#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""The existing ntx-local JSON test helper, with scoped UI/deadline guards.

Run only inside an admitted android-device transaction. This does not grant a
window, select a device, change shared tooling, or implement the ntx application.
"""
import json
import os
from pathlib import Path
import re
import select
import subprocess
import struct
import hashlib
import tarfile
import tempfile
import sys
import time
import xml.etree.ElementTree as ET

PACKAGE = "com.machinerhythm.ntx"
ACTIVITY = PACKAGE + "/.NtxActivity"
FOCUS_QUERY = ("dumpsys window displays | grep -E 'Display:|mCurrentFocus=|mTopFocusedDisplayId=|DisplayFrames'; "
               "dumpsys activity activities | grep -E 'Display #[0-9]+|mResumedActivity:|topResumedActivity='")


class Refused(RuntimeError):
    pass


def write_private_json(path, value):
    """Durably publish a private receipt; report every persistence failure."""
    path = Path(path)
    fd, temporary = tempfile.mkstemp(prefix=path.name + ".", suffix=".writing",
                                     dir=path.parent)
    try:
        with os.fdopen(fd, "w") as output:
            json.dump(value, output, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        parent = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(parent)
        finally:
            os.close(parent)
    finally:
        # A failed serialization/write must leave the previous receipt intact.
        # After replacement this unique temporary no longer exists.
        if os.path.exists(temporary):
            os.unlink(temporary)


def physical_displays_off(text):
    states = re.findall(r"\bDisplay State=([A-Z_]+)\b", text)
    return bool(states) and all(state == "OFF" for state in states)


def interrupted_capture_cleanup(inflight, job, expected_job, cleanup_process,
                                supervisor_process, *, sequence_limit=160):
    """Recognize a retired observation worker, never grant custody or new work.

    The caller must hold the inherited lease and obtain both live process
    identities itself. The supervisor's current child must be this cleanup
    process; children_retired describes the preceding work, not this child.
    Preserve the interrupted operation separately before cleanup proceeds.
    The caller supplies its pinned command-reader bound; old callers retain 160.
    """
    def identity(value):
        return (isinstance(value, dict) and set(value) == {"pid", "start_ticks"}
                and all(type(value[k]) is int and value[k] > 0 for k in value))
    if (type(sequence_limit) is not int or not 1 <= sequence_limit <= 256
            or not isinstance(expected_job, str)
            or not re.fullmatch(r"[a-f0-9]{32}", expected_job)
            or not isinstance(inflight, dict) or set(inflight) != {"sequence", "op"}
            or inflight["op"] != "capture" or type(inflight["sequence"]) is not int
            or not 1 <= inflight["sequence"] <= sequence_limit
            or not isinstance(job, dict)
            or not identity(cleanup_process) or not identity(supervisor_process)
            or cleanup_process["pid"] == supervisor_process["pid"]
            or job.get("id") != expected_job or job.get("state") != "RETURNING"
            or job.get("phase") != "cleanup" or job.get("children_retired") is not True
            or job.get("remaining_children") != [] or job.get("child") != cleanup_process
            or job.get("supervisor") != supervisor_process):
        raise Refused("interrupted capture lacks actual retired-work/owned-cleanup proof")
    return dict(inflight)


def cleanup_shade_context(focus, display, keyguard):
    """Only an unlocked SystemUI shade above ntx on the default lab display.

    Non-content metadata is sufficient. This is not own_focus(), a gameplay
    input exception, or permission to dismiss another app's overlay. Recheck
    the same context before one stock collapse and require own_focus afterward.
    """
    if any(not isinstance(value, str) for value in (focus, display, keyguard)):
        raise Refused("shade cleanup metadata unavailable")
    states = re.findall(r"\bDisplay State=([A-Z_]+)\b", display)
    if (not states or any(state != "ON" for state in states)
            or re.findall(r"\bsecure=(true|false)\b", keyguard) != ["false"]
            or re.findall(r"\bshowing=(true|false)\b", keyguard) != ["false"]):
        raise Refused("shade cleanup requires observed on/unlocked/nonsecure state")
    windows = re.split(r"Display: mDisplayId=(\d+)[^\n]*", focus)
    activities = re.split(r"Display #(\d+)[^\n]*", focus)
    if (windows[1::2] != ["0"] or activities[1::2] != ["0"]
            or set(re.findall(r"mTopFocusedDisplayId=(\d+)", focus)) - {"0"}):
        raise Refused("shade cleanup requires the unique default display")
    current = re.findall(r"mCurrentFocus=([^\n]+)", windows[2])
    frames = re.findall(r"DisplayFrames w=(\d+) h=(\d+) r=(\d+)", windows[2])
    if (len(current) != 1
            or not re.fullmatch(r"Window\{[a-fA-F0-9]+ u0 NotificationShade\}", current[0].strip())
            or len(frames) != 1 or any(int(n) <= 0 for n in frames[0][:2])
            or int(frames[0][2]) not in (0, 1, 2, 3)
            or component(activities[2]) != (PACKAGE, PACKAGE + ".NtxActivity")):
        raise Refused("shade is not above the exact owned resumed activity")
    return {"display": 0, "window": current[0].strip(),
            "geometry": [int(n) for n in frames[0]], "activity": ACTIVITY}


def active_physical_viewport(text, logical):
    """Resolve only the focused logical display's currently active local panel."""
    panels = set()
    for view in re.findall(r"DisplayViewport\{[^}]+\}", text):
        if not all(re.search(r"\b" + field + r"=true\b", view)
                   for field in ("valid", "isActive")):
            continue
        if not re.search(r"\bdisplayId=" + re.escape(str(logical)) + r"\b", view):
            continue
        panel = re.search(r"\buniqueId='local:([0-9]+)'", view)
        if panel:
            panels.add(panel.group(1))
    if len(panels) != 1:
        raise Refused("active physical viewport for focused display missing or ambiguous")
    return panels.pop()


class Budget:
    def __init__(self, now, outer_end, helper_seconds=1020, cleanup_seconds=300,
                 return_seconds=60):
        self.hard_end = min(now + helper_seconds, outer_end)
        self.cleanup_at = self.hard_end - cleanup_seconds
        self.commands_end = self.hard_end - return_seconds
        self.forced_cleanup = False
        if self.cleanup_at <= now:
            raise Refused("insufficient admitted cleanup reserve")

    def cleanup(self, now):
        return self.forced_cleanup or now >= self.cleanup_at

    def admit(self, now, cleanup, seconds):
        if now >= self.commands_end:
            raise Refused("command window ended; return reserve only")
        if not cleanup and (self.cleanup(now) or now + seconds > self.cleanup_at):
            self.forced_cleanup = True
            raise Refused("CLEANUP_REQUIRED; no new tests or owner waiting")

    def owner_wait(self, now, seconds):
        if self.cleanup(now):
            raise Refused("CLEANUP_REQUIRED; owner feedback cannot extend the window")
        return max(0, min(15, seconds, self.cleanup_at - now))

    def input_boundary(self, now, automated):
        # An unattended packet must enter its finalizer without waiting for the
        # agent to notice stdout. Its existing same-transaction recovery script
        # gets the ORIGINAL remaining reserve, never a renewed deadline.
        if automated and self.cleanup(now):
            return now
        return self.commands_end if self.cleanup(now) else self.cleanup_at


def active_bound_entry(collection, binding):
    """Current ntx intent for one source/frame-guarded test input, not FPS proof."""
    if (any(not re.fullmatch(r"[a-f0-9]{32}", binding.get(k, ""))
            for k in ("game_id", "profile_id"))
            or collection.get("selected") != binding["game_id"]):
        raise Refused("current selected game binding differs")
    entries = [e for e in collection.get("entries", []) if e.get("game_id") == binding["game_id"]]
    if (len(entries) != 1 or entries[0].get("profile_id") != binding["profile_id"]
            or any(entries[0].get(k) is not False for k in ("paused", "stopped", "restart_required"))):
        raise Refused("current bound game is not logically active")
    return entries[0]


def playing_presentation_ready(trace):
    """Require current playing intent and a frame newer than invalidation.

    Historical geometry before the latest surface/measurement is not readiness.
    A capped presentation history cannot authorize a native coordinate contact.
    """
    if any(marker in trace for marker in ("trace cap", "trace limit", "suppressed", "truncated")):
        return False
    states = re.findall(r"native\.pause\.end (paused|playing)", trace)
    ready = trace.rfind("geometry present.ready")
    measurements = list(re.finditer(r"geometry measurement=(?:true|false) presented=false", trace))
    invalid = max(trace.rfind("surface.init"), trace.rfind("surface.terminate"),
                  trace.rfind("geometry measurement=false"),
                  measurements[-1].start() if measurements else -1,
                  trace.rfind("geometry present.unavailable"))
    return bool(states and states[-1] == "playing" and ready > invalid)


def merge_diagnostic_history(previous, current, previous_process, current_process):
    """Retain scoped history only across a fresh, exactly overlapping read.

    Logd may evict a prefix while retaining the previous tail. This preserves
    invalidations as well as readiness; no overlap or process change refuses.
    It neither proves current frame rate nor replaces fresh UI/display checks.
    """
    if (not isinstance(current_process, (tuple, list)) or len(current_process) != 2
            or any(not isinstance(n, str) or not n.isdecimal() for n in current_process)
            or (previous and tuple(previous_process) != tuple(current_process))):
        raise Refused("diagnostic process identity changed")
    pattern = re.compile(r"^\d\d-\d\d \d\d:\d\d:\d\d\.\d+\s+"
                         + re.escape(current_process[0]) + r"\s+\d+\s+[VDIWEF]\s+ntx\s*:")
    def records(text):
        if not isinstance(text, str) or len(text.encode()) > 1024 * 1024:
            raise Refused("diagnostic history byte bound exceeded")
        rows = [line for line in text.splitlines() if line and line != "--------- beginning of main"]
        if len(rows) > 2048 or any(not pattern.match(line) for line in rows):
            raise Refused("diagnostic scope/record bound changed")
        return rows
    old, fresh = records(previous), records(current)
    if not old:
        return "\n".join(fresh)
    for overlap in range(min(len(old), len(fresh)), 0, -1):
        if old[-overlap:] == fresh[:overlap]:
            if set(old).intersection(fresh[overlap:]):
                raise Refused("diagnostic history reordered or ambiguously repeated")
            merged = "\n".join(old + fresh[overlap:])
            records(merged)
            return merged
    raise Refused("diagnostic history continuity lost; no cached readiness fallback")


def matching_inspected_region(reference_size, reference_rgb, current_size, current_rgb, bounds):
    """Match a complete previously inspected control against fresh RGB pixels.

    This is a task-fixture check, not input authority or a geometry transform.
    The caller still needs its named control scope, exact source, current native
    geometry, fresh focus/display/capture and the ordinary single-use tap guards.
    Changed game pixels outside the inspected control may differ. Any changed
    control pixel, frame size, malformed plane or invalid region refuses.
    """
    if (not isinstance(current_size, (tuple, list)) or len(current_size) != 2
            or reference_size != current_size):
        return False
    if any(type(n) is not int or not 1 <= n <= 8192 for n in current_size):
        return False
    width, height = current_size
    if (not isinstance(reference_rgb, (bytes, bytearray))
            or not isinstance(current_rgb, (bytes, bytearray))
            or len(reference_rgb) != width * height * 3 or len(current_rgb) != len(reference_rgb)):
        return False
    if (not isinstance(bounds, (tuple, list)) or len(bounds) != 4
            or any(type(n) is not int for n in bounds)):
        return False
    left, top, right, bottom = bounds
    if not (0 <= left < right <= width and 0 <= top < bottom <= height
            and right - left >= 32 and bottom - top >= 32):
        return False
    def digest(rgb):
        view = memoryview(rgb)
        result = hashlib.sha256()
        for row in range(top, bottom):
            result.update(view[(row * width + left) * 3:(row * width + right) * 3])
        return result.digest()
    return digest(reference_rgb) == digest(current_rgb)


def matching_inspected_text(reference_size, reference_rgb, current_size, current_rgb,
                            bounds, opaque_colours):
    """Exact inspected text pixels over moving scenery; never a control proof.

    The task pins its text palette/region. Both the foreground shape and every
    selected RGB value must match. Native controls still use complete regions.
    """
    if (not isinstance(opaque_colours, list) or not 1 <= len(opaque_colours) <= 256
            or any(not isinstance(c, list) or len(c) != 3
                   or any(type(v) is not int or not 0 <= v <= 255 for v in c)
                   for c in opaque_colours)
            or not matching_inspected_region(reference_size, reference_rgb,
                                              current_size, reference_rgb, bounds)
            or not isinstance(current_rgb, (bytes, bytearray))
            or len(current_rgb) != len(reference_rgb)):
        return False
    palette = {tuple(c) for c in opaque_colours}
    left, top, right, bottom = bounds
    width = reference_size[0]
    def selected(rgb):
        result = []
        for y in range(top, bottom):
            for x in range(left, right):
                offset = (y * width + x) * 3
                colour = tuple(rgb[offset:offset + 3])
                if colour in palette:
                    result.append((offset, colour))
        return result
    expected = selected(reference_rgb)
    return 64 <= len(expected) <= (right-left)*(bottom-top)*3//4 and expected == selected(current_rgb)


def component(text):
    matches = re.findall(r"\b([\w.]+)/([\w.$]+)", text)
    normalized = {(p, p + a if a.startswith(".") else a) for p, a in matches}
    if len(normalized) != 1:
        raise Refused("missing or ambiguous current component")
    return normalized.pop()


def focused_pair(resumed, windows):
    # Only the current top-focused display is relevant. A full window dump can
    # contain historical ANR focus records before the live display section.
    top = set(re.findall(r"mTopFocusedDisplayId=(\d+)", windows))
    sections = re.split(r"Display: mDisplayId=(\d+)[^\n]*", windows)
    ids = sections[1::2]
    # Android13's `window displays` omits the global top-display field. A
    # single live display is unambiguous; several require its explicit ID.
    if not top and len(ids) == 1:
        top.add(ids[0])
    if len(top) != 1:
        raise Refused("current top-focused display missing or ambiguous")
    display = top.pop()
    current = [sections[i + 1] for i in range(1, len(sections), 2)
               if sections[i] == display]
    if len(current) != 1:
        raise Refused("current focused display section missing or ambiguous")
    focus = re.findall(r"mCurrentFocus=([^\n]+)", current[0])
    if len(focus) != 1:
        raise Refused("focused-window observation missing or ambiguous")
    activities = re.split(r"Display #(\d+)[^\n]*", resumed)
    active = [activities[i + 1] for i in range(1, len(activities), 2)
              if activities[i] == display]
    if len(active) != 1:
        raise Refused("resumed Activity display missing or ambiguous")
    activity = component(active[0])
    window = component(focus[0])
    if window != activity:
        raise Refused("resumed Activity and focused window do not match")
    frames = tuple(re.findall(r"DisplayFrames w=(\d+) h=(\d+) r=(\d+)", current[0]))
    return activity, (display, focus[0].strip(), frames)


def own_focus(resumed, windows):
    activity, window = focused_pair(resumed, windows)
    if activity != (PACKAGE, PACKAGE + ".NtxActivity"):
        raise Refused("expected ntx Activity AND focused window do not match")
    return activity, window


def button_center(xml, text):
    tree = ET.fromstring(xml)
    parents = {child: parent for parent in tree.iter() for child in parent}
    roots = list(tree)
    if not roots:
        raise Refused("no current UI root")
    def bounds(node):
        rectangle = re.fullmatch(r"\[(-?\d+),(-?\d+)\]\[(-?\d+),(-?\d+)\]",
                                 node.attrib.get("bounds", ""))
        if rectangle is None:
            raise Refused("current UI bounds missing")
        return tuple(map(int, rectangle.groups()))
    root = bounds(roots[0])
    matches = [n for n in tree.iter("node") if n.attrib.get("text") == text
               and n.attrib.get("package") == PACKAGE
               and n.attrib.get("class") == "android.widget.Button"
               and n.attrib.get("enabled") == "true"
               and n.attrib.get("clickable") == "true"]
    if len(matches) != 1:
        raise Refused("requested current ntx button missing or ambiguous")
    a, b, c, d = bounds(matches[0])
    ancestor = parents.get(matches[0])
    while ancestor is not None and ancestor.tag == "node":
        left, top, right, bottom = bounds(ancestor)
        a, b, c, d = max(a, left), max(b, top), min(c, right), min(d, bottom)
        ancestor = parents.get(ancestor)
    a, b, c, d = max(a, root[0]), max(b, root[1]), min(c, root[2]), min(d, root[3])
    if a >= c or b >= d:
        raise Refused("button is outside the currently observed layout")
    return (a + c) // 2, (b + d) // 2


def hash_arguments(paths):
    # ADB joins shell arguments: literal validated paths avoid the former sh -c
    # quoting bug, which accidentally ran sha256sum with no input arguments.
    if not paths or any(not re.fullmatch(r"files/ntx/[A-Za-z0-9_./-]+", p)
                        or ".." in p.split("/") for p in paths):
        raise Refused("hash requires explicit literal ntx file paths")
    return ["shell", "run-as", PACKAGE, "sha256sum", *paths]


class Driver:
    def __init__(self, root, budget, invoke, clock=time.monotonic, sleep=time.sleep, *,
                 capture_limit=8, byte_limit=268435456, stick_ms_limit=1000,
                 inspection_seconds=30):
        # These are prepared task bounds, not authority or policy overrides.
        # Existing callers retain their original eight/ten-image and byte caps.
        if type(capture_limit) is not int or not 1 <= capture_limit <= 160:
            raise Refused("invalid admitted capture ceiling")
        if type(byte_limit) is not int or not 1 <= byte_limit <= 805306368:
            raise Refused("invalid admitted collected-byte ceiling")
        if type(stick_ms_limit) is not int or not 100 <= stick_ms_limit <= 4000:
            raise Refused("invalid admitted stick-contact ceiling")
        if type(inspection_seconds) is not int or not 1 <= inspection_seconds <= 90:
            raise Refused("invalid admitted image-inspection bound")
        self.capture_limit = capture_limit
        self.byte_limit = byte_limit
        self.stick_ms_limit = stick_ms_limit
        self.inspection_seconds = inspection_seconds
        self.root, self.budget, self.invoke = Path(root), budget, invoke
        self.clock, self.sleep = clock, sleep
        self.operation_end = budget.commands_end
        self.timeout = False
        self.actions, self.captures, self.received = 0, 0, 0
        self.mismatch = False
        self.last_capture = None
        self.own_ui_observed = False
        self.launched = False
        self.receipt = {"started": time.time(), "operations": [], "handback": False,
                        "capture_limit": capture_limit,
                        "byte_limit": byte_limit,
                        "stick_ms_limit": stick_ms_limit,
                        "inspection_seconds": inspection_seconds,
                        "phase": "admitted", "recovery_resolved": False}

    def save(self):
        p = self.root / "receipt.private.json"
        write_private_json(p, self.receipt)

    def call(self, args):
        if self.timeout:
            raise Refused("timed-out transport requires transaction retirement")
        timeout = min(30, self.operation_end - self.clock(),
                      self.budget.hard_end - self.clock())
        if timeout <= 0:
            raise Refused("operation deadline reached")
        try:
            result = self.invoke(args, timeout)
        except subprocess.TimeoutExpired:
            # End the helper; the existing transaction wrapper retires its own
            # foreground process group. Do not issue more commands concurrently
            # with a possibly surviving timed-out adb descendant.
            self.timeout = True
            raise
        self.received += len(result.stdout) + len(result.stderr)
        if self.received > self.byte_limit:
            self.budget.forced_cleanup = True
            raise Refused("collected-byte ceiling reached")
        return result

    def focus(self):
        try:
            observed = self.observe_focus()
            if observed[0] != (PACKAGE, PACKAGE + ".NtxActivity"):
                # Private predicate metadata only; preserve the first refusal.
                # This is not readiness, a UI capture or authority to act there.
                self.receipt.setdefault("foreground_refusal_observation", {
                    "at": self.clock(), "component": list(observed[0]),
                    "display": observed[1][0], "frames": observed[1][2],
                    "expected": [PACKAGE, PACKAGE + ".NtxActivity"],
                })
                raise Refused("expected ntx Activity AND focused window do not match")
            self.own_ui_observed = True
            return observed
        except Refused:
            self.mismatch = True
            self.budget.forced_cleanup = True
            self.receipt["foreground_mismatch"] = True
            raise

    def observe_focus(self):
        # Both current metadata views share one wrapped ADB invocation. This
        # retains target/lease checks and avoids repeating costly USB admission
        # for each read; differing Activity/window/display still refuses.
        result = self.call(["shell", FOCUS_QUERY])
        if result.returncode:
            raise Refused("current focus metadata query failed")
        data = result.stdout.decode()
        try:
            return focused_pair(data, data)
        except Refused:
            failures = self.receipt.setdefault("focus_refusals", [])
            if len(failures) < 8:
                evidence = self.root / f"focus-refusal-{len(failures)}.private.txt"
                with evidence.open("xb") as file:
                    file.write(result.stdout)
                evidence.chmod(0o600)
                failures.append(str(evidence))
                self.save()
            raise

    def action(self, args):
        if self.actions >= 100:
            raise Refused("UI action ceiling reached")
        self.actions += 1
        self.last_capture = None
        return self.call(args)

    def capture_panel(self, focus):
        query = ["shell", "dumpsys display | grep 'DisplayViewport{' "]
        def observe():
            attempts = []
            for attempt in range(3):
                result = self.call(query)
                attempts.append(result.stdout + b"\nSTDERR\n" + result.stderr)
                if result.stdout.strip() or attempt == 2:
                    break
                # A missing read-only observation supplies no viewport. Retry
                # only that observation, within the current operation deadline;
                # a changed/ambiguous viewport is still an immediate refusal.
                self.sleep(0.1)
            return result, b"\nREAD_RETRY\n".join(attempts)

        before, before_evidence = observe()
        if before.returncode:
            raise Refused("active viewport query failed")
        panel = active_physical_viewport(before.stdout.decode(), focus[1][0])
        # shell-v2 keeps multi-panel warnings on stderr, outside the PNG stream.
        result = self.call(["shell", "-T", "screencap", "-d", panel, "-p"])
        after, after_evidence = observe()
        evidence = self.root / f"capture-viewport-{self.captures}.private.txt"
        evidence.write_bytes(before_evidence + b"\nAFTER\n" + after_evidence +
                             b"\nCAPTURE_STDERR\n" + result.stderr)
        evidence.chmod(0o600)
        try:
            unchanged = (after.returncode == 0 and
                         active_physical_viewport(after.stdout.decode(), focus[1][0]) == panel)
        except Refused:
            unchanged = False
        if not unchanged:
            self.budget.forced_cleanup = True
            raise Refused("capture discarded: active physical viewport changed")
        if result.returncode:
            raise Refused("physical-panel capture failed")
        return result

    def retire_checkpoint(self, command):
        """Retire only our backed-up quiescent runtime; never claim joined Stop."""
        name = command["archive"]
        if Path(name).name != name:
            raise Refused("checkpoint backup must be in this admitted evidence directory")
        archive = self.root / name
        record = json.loads((self.root / "verified-generation.private.json").read_text())
        binding = json.loads((self.root / "trusted-binding.private.json").read_text())
        if archive.is_symlink() or not (0 < archive.stat().st_size == record["archive_bytes"] <= 268435456):
            raise Refused("verified checkpoint backup size/type changed")
        with archive.open("rb") as file:
            if hashlib.file_digest(file, "sha256").hexdigest() != record["archive_sha256"]:
                raise Refused("verified backup changed before retirement")
        profile = binding["profile_id"]
        game = binding["game_id"]
        checkpoint = record["checkpoint"]
        if not (re.fullmatch(r"[0-9a-f]{32}", profile) and re.fullmatch(r"[0-9a-f]{32}", game)
                and re.fullmatch(r"[0-9a-f]{64}", checkpoint)):
            raise Refused("invalid checkpoint/profile binding")
        base = f"files/ntx/profiles/{profile}/"
        required = {"files/ntx/collection.json", base + "save-generation.json",
                    base + "GC/SRAM.raw", base + f"checkpoints/{checkpoint}.ntx"}
        hashes = {}
        with tarfile.open(archive) as backup:
            members = backup.getmembers()
            if sum(m.size for m in members) > 268435456:
                raise Refused("checkpoint backup expansion exceeds admitted bytes")
            for member in members:
                if member.isfile():
                    if member.name in hashes:
                        raise Refused("ambiguous backup member")
                    hashes[member.name] = hashlib.file_digest(backup.extractfile(member), "sha256").hexdigest()
            cards = {p for p in hashes if re.fullmatch(re.escape(base) + r"GC/MemoryCardA\.(USA|EUR|JAP)\.raw", p)}
            if len(cards) != 1 or set(hashes) != required | cards:
                raise Refused("backup must contain the complete selected generation only")
            if hashes[base + f"checkpoints/{checkpoint}.ntx"] != checkpoint:
                raise Refused("backup checkpoint bytes do not match their identity")
            collection = json.load(backup.extractfile("files/ntx/collection.json"))
        entry = next(e for e in collection["entries"] if e["game_id"] == collection["selected"])
        intent = str(entry["paused"]).lower()
        if not (entry["game_id"] == game and entry["profile_id"] == profile
                and entry["checkpoint"] == checkpoint and entry["runtime_incomplete"]
                and not entry["stopped"] and not entry["restart_required"]
                and record["pause_intent"] == entry["paused"]
                and "PASS read-only checkpoint inspection:" in record["verification"]
                and f"captured pause intent={intent}" in record["verification"]):
            raise Refused("backup lacks a matching independently validated live checkpoint")
        # Cleanup can arrive at an already screen-off keyguard. Android then
        # reports NotificationShade and no resumed Activity; do not wake it just
        # to manufacture a Home component. No UI is touched on this path.
        locked = False
        if command["op"] in ("recover_checkpoint", "recover_pending_restore"):
            display = self.call(["shell", "dumpsys display | grep -E 'Display State=|mState='"]).stdout.decode()
            policy = self.call(["shell", "dumpsys window policy | grep -E 'secure=|showing='"]).stdout.decode()
            locked = physical_displays_off(display) and "showing=true" in policy
            if locked:
                self.receipt["retirement_background"] = "physical_display_off_and_keyguard_showing"
        if not locked:
            current, _ = self.observe_focus()
            home = self.call(["shell", "cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.HOME"])
            if current != component(home.stdout.decode()):
                raise Refused("checkpoint retirement requires observed Home or locked screen-off cleanup")
        actual = self.call(hash_arguments(sorted(hashes)))
        lines = [line.split() for line in actual.stdout.decode().splitlines()]
        if actual.returncode or any(len(line) != 2 for line in lines) or {p: h for h, p in lines} != hashes:
            raise Refused("current checkpoint/saves/metadata differ from the protected backup")
        pid = str(command["pid"])
        ticks = str(command["start_ticks"])
        if not (pid.isdecimal() and ticks.isdecimal()):
            raise Refused("invalid current process observation")
        if self.call(["shell", "pidof", PACKAGE]).stdout.decode().strip() != pid:
            raise Refused("runtime process changed; reobserve before retirement")
        stat = self.call(["shell", "cat", f"/proc/{pid}/stat"]).stdout.decode()
        if stat.rsplit(")", 1)[-1].split()[19] != ticks:
            raise Refused("runtime process start changed")
        pending_restore = command["op"] == "recover_pending_restore"
        lines = 192 if pending_restore else 64
        logs = self.call(["shell", f"logcat -d --pid={pid} -s ntx:I '*:S' | tail -{lines}"]).stdout.decode()
        states = re.findall(r"native\.pause\.end (paused|playing)", logs)
        observations = self.receipt.setdefault("retirement_observations", [])
        if len(observations) >= 8:
            raise Refused("retirement observation ceiling reached")
        evidence = self.root / f"retirement-quiescence-{len(observations)}.private.txt"
        with evidence.open("xb") as file:
            file.write(logs.encode())
        evidence.chmod(0o600)
        proof = {"archive_sha256": record["archive_sha256"], "actual_file_sha256": hashes,
                 "pid": pid, "start_ticks": ticks, "quiescence": str(evidence)}
        observations.append(proof)
        self.save()
        if pending_restore:
            # A departure before FIRST restore has no new capture to publish.
            # Retire only a fully traced paused bootstrap, with the existing
            # independently inspected checkpoint and every ordinary file still
            # matching the complete backup above. Never use this after play,
            # partial restore, trace truncation or another runtime boot.
            markers = ("activity.init", "boot.checkpoint_validate.end ok",
                       "boot.native_paused.end ok", "runtime.boot.end ready_for_restore_or_play")
            publication = (all(marker in logs for marker in markers)
                           and logs.count("runtime.boot.begin") == 1
                           and not any(marker in logs for marker in
                                       ("restore.validate.begin", "restore.load", "native.pause.end playing",
                                        "checkpoint.capture.begin", "runtime.stop")))
        else:
            publication = "Checkpoint complete:" in logs
        if not states or states[-1] != "paused" or not publication:
            raise Refused("current runtime quiescence/publication not observed")
        self.receipt["retirement_validation"] = proof
        self.save()
        result = self.action(["shell", "am", "force-stop", PACKAGE])
        if result.returncode or self.call(["shell", "pidof", PACKAGE]).stdout.strip():
            raise Refused("checkpoint retirement did not reach process absence")
        self.receipt["checkpoint_process_retired"] = True
        self.receipt["retirement_is_normal_stop"] = False
        return result

    def execute(self, command):
        op = command["op"]
        cleanup = bool(command.get("cleanup"))
        if op == "wait_owner":
            self.sleep(self.budget.owner_wait(self.clock(), command.get("seconds", 15)))
            return {"cleanup_required": self.budget.cleanup(self.clock())}
        seconds = max(0, min(30, command.get("timeout", 30)))
        self.budget.admit(self.clock(), cleanup, seconds)
        self.operation_end = min(self.budget.commands_end, self.clock() + seconds)
        if op == "capture":
            if self.captures >= self.capture_limit:
                raise Refused("image ceiling reached")
            self.captures += 1
            before = self.focus()
            result = self.capture_panel(before)
            # Discard rather than save/return an image if focus changed during it.
            if self.focus() != before:
                self.mismatch = True
                self.budget.forced_cleanup = True
                raise Refused("capture discarded: focused window changed")
            if len(result.stdout) < 24 or result.stdout[:8] != b"\x89PNG\r\n\x1a\n" or result.stdout[12:16] != b"IHDR":
                raise Refused("capture did not return a PNG")
            dimensions = struct.unpack(">II", result.stdout[16:24])
            name = command["name"]
            if Path(name).name != name or not name.endswith(".png"):
                raise Refused("invalid local capture name")
            p = self.root / name
            p.write_bytes(result.stdout)
            p.chmod(0o600)
            self.last_capture = (name, self.clock(), before, dimensions)
            return {"capture": str(p), "bytes": len(result.stdout)}
        if op in ("ntx_tap", "ntx_stick", "ntx_trigger"):
            # Automated native-button checks use an agent-inspected, just-taken
            # own image. Bind one tap to its live window AND display geometry;
            # never reuse a guessed historical layout or a batch of points.
            admitted = (("Control Stick", "C Stick") if op == "ntx_stick"
                        else (("L analog", "R analog") if op == "ntx_trigger"
                              else ("Pause", "Start", "A", "B", "L click")))
            if command["target"] not in admitted:
                raise Refused("unadmitted native control")
            if cleanup and command["target"] != "Pause":
                raise Refused("cleanup native input is Pause only")
            capture = self.last_capture
            if (not capture or capture[0] != command["capture"]
                    or self.clock() - capture[1] > self.inspection_seconds):
                age = None if not capture else round(self.clock() - capture[1], 3)
                raise Refused(f"native tap requires a fresh inspected own capture; age_seconds={age}")
            focus = self.focus()
            if focus != capture[2]:
                self.mismatch = True
                self.budget.forced_cleanup = True
                raise Refused("native tap aborted: live window/display changed")
            frames = focus[1][2]
            if len(frames) != 1 or tuple(map(int, frames[0][:2])) != capture[3]:
                raise Refused("native capture/display geometry is not coherent")
            if self.clock() - capture[1] > self.inspection_seconds:
                raise Refused(f"native capture expired during focus verification; age_seconds={self.clock()-capture[1]:.3f}")
            bounds = command["bounds"]
            if len(bounds) != 4 or any(type(v) is not int for v in bounds):
                raise Refused("native button requires observed integer bounds")
            a, b, c, d = bounds
            width, height = capture[3]
            if not (0 <= a < c <= width and 0 <= b < d <= height):
                raise Refused("native button outside current image")
            x, y = (a+c)//2, (b+d)//2
            if op == "ntx_trigger":
                percent = command.get("travel_percent")
                duration = command.get("duration_ms")
                if type(percent) is not int or not 1 <= percent <= 99 or c - a < 3:
                    raise Refused("analog travel requires an interior percentage 1..99")
                if type(duration) is not int or not 100 <= duration <= 500:
                    raise Refused("analog contact must be 100..500ms")
                # Rust owns the horizontal travel mapping. A bounded stock
                # DOWN/MOVE/UP contact releases it; no digital click is emitted.
                x = min(c - 1, max(a + 1, round(a + (c - a) * percent / 100)))
                result = self.action(["shell", "input", "swipe", str(x), str(y),
                                      str(x), str(y), str(duration)])
            elif op == "ntx_stick":
                delta = command.get("delta")
                duration = command.get("duration_ms")
                if (not isinstance(delta, list) or len(delta) != 2
                        or any(type(v) is not int for v in delta)
                        or not any(delta)
                        or sum(v*v for v in delta) > (min(c-a, d-b)//3)**2):
                    raise Refused("stick displacement must stay inside its inspected thumb well")
                if type(duration) is not int or not 100 <= duration <= self.stick_ms_limit:
                    raise Refused("stick duration exceeds this task's admitted contact bound")
                motion = command.get("motion", "ramp")
                if motion not in ("ramp", "hold"):
                    raise Refused("unprepared stick motion")
                start_x, start_y = (x+delta[0], y+delta[1]) if motion == "hold" else (x, y)
                # Stock swipe supplies down/move/up as one finite command.
                # Equal endpoints hold the inspected displacement from DOWN;
                # the default retains the earlier center-to-target ramp.
                # The fixture must prove the complete named well, source,
                # settings and live geometry; no arbitrary screen trajectory.
                result = self.action(["shell", "input", "swipe", str(start_x), str(start_y),
                                      str(x+delta[0]), str(y+delta[1]), str(duration)])
            else:
                press = command.get("press_ms", 0)
                if type(press) is not int or not 0 <= press <= 250:
                    raise Refused("test press duration must be 0..250ms")
                if press and command["target"] == "Pause":
                    raise Refused("Pause opens a new window; held contact is game controls only")
                x, y = str(x), str(y)
                result = self.action(["shell", "input", "swipe", x, y, x, y, str(press)] if press else
                                     ["shell", "input", "tap", x, y])
        elif op == "ntx_button":
            text = command["text"]
            if cleanup and text not in ("Stop game", "Stop current game"):
                raise Refused("cleanup cannot resume a test")
            before = self.focus()
            path = "/data/local/tmp/ntx-packet-ui.xml"
            result = self.call(["shell", f"uiautomator dump {path} >/dev/null && cat {path}; rm -f {path}"])
            xml = result.stdout.decode()
            try:
                x, y = button_center(xml[xml.index("<?xml"):], text)
            except (ValueError, ET.ParseError) as error:
                raise Refused("no usable current ntx UI layout") from error
            if self.focus() != before:
                self.mismatch = True
                self.budget.forced_cleanup = True
                raise Refused("layout batch aborted: focused window changed")
            result = self.action(["shell", "input", "tap", str(x), str(y)])
        elif op == "launch":
            if self.mismatch and not command.get("owner_reconfirmed"):
                raise Refused("unexpected foreground requires owner clarification")
            current, _ = self.observe_focus()
            home = self.call(["shell", "cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.HOME"])
            initial = (command.get("initial_entry") and not self.launched
                       and not self.own_ui_observed and not self.mismatch)
            if current not in (component(home.stdout.decode()), (PACKAGE, PACKAGE + ".NtxActivity")) and not (initial or command.get("owner_reconfirmed")):
                self.mismatch = True
                self.budget.forced_cleanup = True
                raise Refused("unexpected app before launch; owner clarification required")
            result = self.action(["shell", "am", "start", "-W", "--user", "0", "-a",
                                  "android.intent.action.MAIN", "-c",
                                  "android.intent.category.LAUNCHER", "-n", ACTIVITY])
            self.launched |= result.returncode == 0
        elif op == "system":
            step = command["step"]
            if step == "home":
                self.focus()  # Home departure must originate from the expected game.
                result = self.action(["shell", "input", "keyevent", "KEYCODE_HOME"])
            elif step == "wake_nonsecure":
                policy = self.call(["shell", "dumpsys window policy | grep -E 'secure=|showing='"])
                if "secure=false" not in policy.stdout.decode():
                    raise Refused("nonsecure wake authority does not match observation")
                self.action(["shell", "input", "keyevent", "KEYCODE_WAKEUP"])
                result = self.action(["shell", "wm", "dismiss-keyguard"])
            else:
                raise Refused("unadmitted system UI step")
        elif op == "hash_files":
            result = self.call(hash_arguments(command["paths"]))
        elif op in ("retire_checkpoint", "recover_checkpoint", "recover_pending_restore"):
            if op == "retire_checkpoint" and cleanup:
                raise Refused("checkpoint test retirement is not cleanup Stop")
            if op in ("recover_checkpoint", "recover_pending_restore") and not cleanup:
                raise Refused("bounded quiescent recovery must be explicitly marked cleanup")
            try:
                result = self.retire_checkpoint(command)
                self.receipt["quiescent_recovery"] = op != "retire_checkpoint"
                self.receipt["pending_restore_recovery"] = op == "recover_pending_restore"
            except (OSError, ValueError, TypeError, KeyError, IndexError, StopIteration, tarfile.TarError, Refused) as error:
                self.budget.forced_cleanup = True
                raise Refused(str(error)) from error
        elif op == "adb":
            args = command["args"]
            joined = " ".join(args)
            if (re.search(r"\binput\b", joined) and args != ["shell", "dumpsys", "input"]
                    or re.search(r"\bam\b|\bwm\s+dismiss-keyguard", joined)
                    or "screencap" in joined or "uiautomator" in joined
                    or "sha256sum" in joined):
                raise Refused("raw UI/capture/hash batch refused; use guarded operations")
            result = self.call(args)
        else:
            raise Refused("unknown operation")
        name = command.get("save", f"operation-{len(self.receipt['operations']):03}.private.txt")
        if Path(name).name != name:
            raise Refused("invalid local output name")
        p = self.root / name
        p.write_bytes(result.stdout if command.get("binary") else
                      result.stdout + b"\nSTDERR\n" + result.stderr)
        p.chmod(0o600)
        self.receipt["operations"].append({"at": time.time(), "op": op,
                                           "returncode": result.returncode, "output": str(p)})
        self.save()
        return {"returncode": result.returncode, "saved": str(p),
                "stdout": "" if command.get("binary") else result.stdout.decode(errors="replace")[:command.get("print_limit", 1500)]}

    def finish(self):
        self.operation_end = self.budget.hard_end
        if self.timeout:
            self.receipt["pending_recovery"] = "command timed out; no further device commands"
            return
        try:
            # Collection is text: shell-v2 preserves the remote exit status and
            # stderr. exec-out merges a missing-file error into stdout and may
            # return zero, which is not evidence of a readable collection.
            result = self.call(["shell", "run-as", PACKAGE, "cat", "files/ntx/collection.json"])
            # The admitted installer may prove package absence before a new
            # install. Before any import, that build has no saved collection or
            # runtime: imports persist the collection before native boot.
            fresh = self.receipt.get("verified_new_install", False)
            absent = result.returncode and b"No such file" in result.stderr
            if absent and fresh:
                data = {"entries": [], "selected": None}
            elif result.returncode:
                raise Refused("collection unavailable; no client retirement")
            else:
                data = json.loads(result.stdout)
            entry = next((e for e in data["entries"] if e["game_id"] == data.get("selected")), None)
            empty = fresh and data["entries"] == [] and data.get("selected") is None
            clean = bool(entry and entry.get("stopped") and not entry.get("runtime_incomplete")
                         and not entry.get("restart_required"))
            self.receipt["clean_stop_metadata"] = clean
            self.receipt["new_install_without_game"] = empty
            pid = self.call(["shell", "pidof", PACKAGE]).stdout.strip()
            if pid and (clean or empty):
                self.call(["shell", "am", "force-stop", PACKAGE])
                pid = self.call(["shell", "pidof", PACKAGE]).stdout.strip()
            if pid:
                # Never replace normal Stop with killing a live profile writer.
                self.receipt["pending_recovery"] = "live runtime retained; normal Stop not verified"
                try:
                    self.focus()
                    self.action(["shell", "input", "keyevent", "KEYCODE_HOME"])
                except Refused:
                    pass  # Do not act on an unexpected app to finish cleanup.
                return
            self.receipt["clients_retired"] = True
            # Screen-off is an explicit return step, only from ntx or the observed
            # current Home. No unrelated-app screenshot or touch is used.
            display = self.call(["shell", "dumpsys display | grep -E 'Display State=|mState='"]).stdout.decode()
            if not physical_displays_off(display):
                home = self.call(["shell", "cmd package resolve-activity --brief -a android.intent.action.MAIN -c android.intent.category.HOME"]).stdout.decode()
                current, _ = self.observe_focus()
                if current not in (component(home), (PACKAGE, PACKAGE + ".NtxActivity")):
                    raise Refused("unexpected app during return; no screen input")
                self.action(["shell", "input", "keyevent", "KEYCODE_SLEEP"])
            # Sleep/keyguard transitions are asynchronous. Re-observe finitely,
            # within the same remaining hard deadline; never infer success from
            # the key-event command's return code.
            for attempt in range(4):
                display = self.call(["shell", "dumpsys display | grep -E 'Display State=|mState='"]).stdout.decode()
                keyguard = self.call(["shell", "dumpsys window policy | grep -E 'secure=|showing='"]).stdout.decode()
                self.receipt["display_off"] = physical_displays_off(display)
                self.receipt["keyguard_showing"] = "showing=true" in keyguard
                if self.receipt["display_off"] and self.receipt["keyguard_showing"]:
                    break
                if attempt < 3 and self.clock() + 0.3 < self.budget.hard_end:
                    self.sleep(0.3)
            self.receipt["handback"] = self.receipt["display_off"] and self.receipt["keyguard_showing"]
            self.receipt["recovery_resolved"] = self.receipt["handback"]
            if not self.receipt["handback"]:
                self.receipt["pending_recovery"] = "physical display/keyguard return not verified"
        except (Refused, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as error:
            self.receipt["pending_recovery"] = str(error)


def main():
    alias, root = sys.argv[1:3]
    root = Path(root)
    root.mkdir(mode=0o700, parents=True, exist_ok=False)
    fd = int(os.environ["CODEX_ANDROID_LEASE_FD"])
    budget = Budget(time.monotonic(), float(os.environ["NTX_OUTER_DEADLINE_MONO"]))
    automated = os.environ.get("NTX_AUTOMATED_PACKET") == "1"
    def invoke(args, timeout):
        return subprocess.run(["android-device", "adb", alias, "--", *args],
                              stdin=subprocess.DEVNULL, capture_output=True,
                              timeout=timeout, pass_fds=(fd,))
    driver = Driver(root, budget, invoke)
    driver.receipt["verified_new_install"] = os.environ.get("NTX_VERIFIED_NEW_INSTALL") == "1"
    task = os.environ["NTX_DEVICE_TASK"]
    if not re.fullmatch(r"ntx-[a-z0-9-]{1,100}", task):
        raise Refused("invalid own task record name")
    canonical = Path.home() / ".local/state/codex-android/tasks" / task / "receipt.json"
    def canonical_pending():
        canonical.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        r = dict(driver.receipt, alias=alias, returned=driver.receipt["handback"],
                 lease_released=False, task_owned_resources_pending=not driver.receipt.get("clients_retired", False),
                 return_observations_pending=not driver.receipt["handback"], source_evidence=str(root))
        canonical.write_text(json.dumps(r, indent=2)); canonical.chmod(0o600)
    driver.save(); canonical_pending()
    print("LEASE_ACQUIRED; new work stops at earliest deadline minus300s; return reserve60s; "
          f"automatic_cleanup={automated}", flush=True)
    cleanup_announced = False
    try:
        while time.monotonic() < budget.commands_end:
            now = time.monotonic()
            if budget.cleanup(now) and not cleanup_announced:
                driver.receipt["cleanup_entered"] = time.time()
                print("CLEANUP_REQUIRED; owner waiting ended; cleanup operations only", flush=True)
                cleanup_announced = True
            boundary = budget.input_boundary(now, automated)
            if boundary <= now:
                break
            ready, _, _ = select.select([sys.stdin], [], [], min(15, max(0, boundary-now)))
            if not ready:
                continue
            line = sys.stdin.readline()
            if not line:
                break
            command = json.loads(line)
            if command["op"] == "finish":
                break
            try:
                result = driver.execute(command)
                print(json.dumps(result), flush=True)
            except Refused as error:
                print(json.dumps({"REFUSED": str(error), "cleanup_required": budget.cleanup(time.monotonic())}), flush=True)
            driver.save()
    finally:
        driver.finish()
        driver.receipt.update(phase="returned" if driver.receipt["handback"] else "pending-recovery",
                              finished=time.time(), actions=driver.actions,
                              captures=driver.captures, collected_bytes=driver.received)
        driver.save(); canonical_pending()
        print(json.dumps({"FINISHED": True, "handback": driver.receipt["handback"],
                          "pending_recovery": driver.receipt.get("pending_recovery")}), flush=True)


if __name__ == "__main__":
    main()
