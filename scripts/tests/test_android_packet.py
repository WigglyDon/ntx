# SPDX-License-Identifier: MIT
"""Offline regression fixtures: no device alias, ADB or private data."""
import importlib.util
from pathlib import Path
import subprocess
import struct
import hashlib
import io
import json
import tarfile
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "android_packet", Path(__file__).resolve().parents[1] / "android_packet.py")
packet = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(packet)
OWN = packet.PACKAGE
RESUMED = f"Display #0 (activities):\nmResumedActivity: ActivityRecord{{1 u0 {OWN}/.NtxActivity t1}}"
WINDOW = f"Display: mDisplayId=0 rootTasks=1\nmCurrentFocus=Window{{1 u0 {OWN}/{OWN}.NtxActivity}}\nDisplayFrames w=800 h=400 r=0\nmTopFocusedDisplayId=0"
OTHER = "com.example.other/.OtherActivity"
VIEWPORT = "DisplayViewport{type=INTERNAL, valid=true, isActive=true, displayId=0, uniqueId='local:123', orientation=0}"
XML = f'''<?xml version="1.0"?><hierarchy><node bounds="[0,0][800,400]">
<node text="Resume" package="{OWN}" class="android.widget.Button"
enabled="true" clickable="true" bounds="[40,80][300,160]"/>
<node text="Stop game" package="{OWN}" class="android.widget.Button"
enabled="true" clickable="true" bounds="[400,200][700,280]"/>
</node></hierarchy>'''


class Fixture:
    def __init__(self, root):
        self.now = 100
        self.calls = []
        self.timeouts = []
        self.resumed, self.windows = RESUMED, WINDOW
        self.after_capture = None
        self.viewports = VIEWPORT
        self.after_viewports = None
        self.after_dump = None
        self.fail = False
        self.live = True
        self.clean = False
        self.display_states = ["OFF"]
        self.keyguard_states = ["true"]
        self.retirement = None
        self.budget = packet.Budget(self.now, 1300)
        self.driver = packet.Driver(root, self.budget, self.invoke,
                                    lambda: self.now, self.sleep)

    def sleep(self, seconds):
        self.now += seconds

    def invoke(self, args, timeout):
        self.calls.append(args)
        self.timeouts.append(timeout)
        if self.fail:
            raise subprocess.TimeoutExpired(args, timeout)
        joined = " ".join(args)
        out = b""
        if args == ["shell", packet.FOCUS_QUERY]:
            out = (self.windows + "\n" + self.resumed).encode()
        elif "dumpsys activity" in joined:
            out = self.resumed.encode()
        elif "dumpsys window displays |" in joined:
            out = self.windows.encode()
        elif "uiautomator dump" in joined:
            out = XML.encode()
            if self.after_dump:
                self.windows = self.after_dump
        elif args[:4] == ["shell", "-T", "screencap", "-d"]:
            out = b"\x89PNG\r\n\x1a\n" + b"\0\0\0\rIHDR" + struct.pack(">II", 800, 400)
            if self.after_capture:
                self.windows = self.after_capture
            if self.after_viewports is not None:
                self.viewports = self.after_viewports
        elif "DisplayViewport{" in joined:
            out = self.viewports.encode()
        elif "collection.json" in joined and "sha256sum" not in args:
            if self.retirement:
                return subprocess.CompletedProcess(args, 0, self.retirement["collection"], b"")
            value = "true" if self.clean else "false"
            out = ('{"selected":"fixture","entries":[{"game_id":"fixture",'
                   f'"stopped":{value},"runtime_incomplete":false,'
                   '"restart_required":false}]}').encode()
        elif "pidof" in joined:
            out = b"1000" if self.live else b""
        elif "force-stop" in joined:
            self.live = False
        elif "resolve-activity" in joined:
            out = b"com.example.home/.HomeActivity"
        elif "sha256sum" in args and self.retirement:
            out = "".join(f"{self.retirement['hashes'][p]}  {p}\n" for p in args[4:]).encode()
        elif "/proc/1000/stat" in args and self.retirement:
            out = ("1000 (ntx) S " + "0 " * 18 + str(self.retirement["ticks"])).encode()
        elif "logcat" in joined and self.retirement:
            out = self.retirement["logs"].encode()
        elif "dumpsys display" in joined:
            state = self.display_states.pop(0) if len(self.display_states) > 1 else self.display_states[0]
            out = ("Display State=" + state).encode()
        elif "dumpsys window policy" in joined:
            state = self.keyguard_states.pop(0) if len(self.keyguard_states) > 1 else self.keyguard_states[0]
            out = ("showing=" + state + " secure=false").encode()
        return subprocess.CompletedProcess(args, 0, out, b"")


class InterruptedCleanupTests(unittest.TestCase):
    def setUp(self):
        self.job_id = "a" * 32
        self.cleanup = {"pid": 71, "start_ticks": 120}
        self.supervisor = {"pid": 70, "start_ticks": 110}
        self.job = {"id": self.job_id, "state": "RETURNING", "phase": "cleanup",
                    "child": self.cleanup, "supervisor": self.supervisor,
                    "children_retired": True, "remaining_children": []}
        self.inflight = {"sequence": 14, "op": "capture"}

    def admit(self, operation=None, job=None):
        return packet.interrupted_capture_cleanup(
            self.inflight if operation is None else operation,
            self.job if job is None else job, self.job_id,
            self.cleanup, self.supervisor)

    def test_retired_observation_preserves_the_original_failed_operation(self):
        original = json.dumps(self.job, sort_keys=True)
        proof = self.admit()
        proof["op"] = "preserved elsewhere"
        self.assertEqual(self.inflight, {"sequence": 14, "op": "capture"})
        self.assertEqual(json.dumps(self.job, sort_keys=True), original)

    def test_input_and_lifecycle_interruptions_are_not_capture_cleanup(self):
        for op in ("contact", "guest_button", "open_save", "ordinary_stop", "reopen", "observe"):
            with self.subTest(op=op), self.assertRaises(packet.Refused):
                self.admit({"sequence": 14, "op": op})
        for value in (True, 0, 161, "14"):
            with self.subTest(sequence=value), self.assertRaises(packet.Refused):
                self.admit({"sequence": value, "op": "capture"})

    def test_prepared_command_ceiling_preserves_default_and_identity_checks(self):
        operation = {"sequence": 200, "op": "capture"}
        with self.assertRaises(packet.Refused):
            self.admit(operation)
        args = (operation, self.job, self.job_id, self.cleanup, self.supervisor)
        self.assertEqual(packet.interrupted_capture_cleanup(
            *args, sequence_limit=256), operation)
        for limit in (True, 0, 257, "256"):
            with self.subTest(limit=limit), self.assertRaises(packet.Refused):
                packet.interrupted_capture_cleanup(*args, sequence_limit=limit)
        for change in ({"children_retired": False}, {"state": "RUNNING"}):
            with self.subTest(change=change), self.assertRaises(packet.Refused):
                packet.interrupted_capture_cleanup(
                    operation, dict(self.job, **change), self.job_id,
                    self.cleanup, self.supervisor, sequence_limit=256)
        with self.assertRaises(packet.Refused):
            packet.interrupted_capture_cleanup(
                dict(operation, sequence=257), self.job, self.job_id,
                self.cleanup, self.supervisor, sequence_limit=256)

    def test_live_missing_or_different_worker_does_not_authorize_retirement(self):
        changes = ({"id": "b" * 32}, {"state": "RUNNING"}, {"phase": "work"},
                   {"children_retired": False}, {"children_retired": 1},
                   {"remaining_children": [self.supervisor]}, {"child": None},
                   {"child": dict(self.cleanup, start_ticks=121)},
                   {"supervisor": dict(self.supervisor, start_ticks=111)})
        for change in changes:
            with self.subTest(change=change), self.assertRaises(packet.Refused):
                self.admit(job=dict(self.job, **change))
        for key in self.job:
            missing = {k: v for k, v in self.job.items() if k != key}
            with self.subTest(missing=key), self.assertRaises(packet.Refused):
                self.admit(job=missing)


class ShadeCleanupTests(unittest.TestCase):
    def setUp(self):
        self.focus = (WINDOW.replace(f"{OWN}/{OWN}.NtxActivity", "NotificationShade")
                      + "\n" + RESUMED)
        self.display = "Display State=ON"
        self.keyguard = "showing=false secure=false"

    def context(self, focus=None, display=None, keyguard=None):
        return packet.cleanup_shade_context(
            self.focus if focus is None else focus,
            self.display if display is None else display,
            self.keyguard if keyguard is None else keyguard)

    def test_owned_unlocked_shade_is_not_misrepresented_as_game_focus(self):
        proof = self.context()
        self.assertEqual(proof["activity"], packet.ACTIVITY)
        self.assertEqual(proof["geometry"], [800, 400, 0])
        with self.assertRaises(packet.Refused):packet.own_focus(self.focus, self.focus)

    def test_foreign_secure_locked_off_or_ambiguous_overlay_never_matches(self):
        changes = (
            {"focus": self.focus.replace(f"{OWN}/.NtxActivity", OTHER)},
            {"focus": self.focus.replace("NotificationShade", "PermissionDialog")},
            {"focus": self.focus.replace(" u0 NotificationShade", " u10 NotificationShade")},
            {"focus": self.focus + "\nDisplay #1:\n" + OTHER},
            {"focus": self.focus + "\nDisplay: mDisplayId=1 rootTasks=1"},
            {"focus": self.focus.replace("mTopFocusedDisplayId=0", "mTopFocusedDisplayId=1")},
            {"focus": self.focus.replace("DisplayFrames w=800", "DisplayFrames w=0")},
            {"keyguard": "showing=false secure=true"},
            {"keyguard": "showing=true secure=false"},
            {"keyguard": "showing=false secure=false secure=true"},
            {"display": "Display State=OFF"}, {"display": ""})
        for change in changes:
            with self.subTest(change=change), self.assertRaises(packet.Refused):
                self.context(**change)


class CaptureBudgetTests(unittest.TestCase):
    def test_receipt_reader_keeps_complete_prior_json_until_replacement(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'receipt.private.json'
            packet.write_private_json(path, {'stage': 'before'})
            replace = packet.os.replace
            def inspect_then_replace(source, target):
                self.assertEqual(json.loads(path.read_text()), {'stage': 'before'})
                self.assertEqual(json.loads(Path(source).read_text()), {'stage': 'after'})
                return replace(source, target)
            with patch.object(packet.os, 'replace', side_effect=inspect_then_replace):
                packet.write_private_json(path, {'stage': 'after'})
            self.assertEqual(json.loads(path.read_text()), {'stage': 'after'})
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_existing_packets_keep_eight_capture_default(self):
        with tempfile.TemporaryDirectory() as root:
            fixture = Fixture(root)
            self.assertEqual(fixture.driver.capture_limit, 8)
            self.assertEqual(fixture.driver.receipt['capture_limit'], 8)
            fixture.driver.captures = 8
            with self.assertRaises(packet.Refused):
                fixture.driver.execute(dict(op='capture', name='extra.png', capture_limit=10))
            self.assertEqual(fixture.calls, [])

    def test_task_admitted_ten_capture_ceiling_is_enforced(self):
        with tempfile.TemporaryDirectory() as root:
            fixture = Fixture(root)
            driver = packet.Driver(root, fixture.budget, fixture.invoke,
                                   lambda: fixture.now, fixture.sleep, capture_limit=10)
            driver.captures = 9
            self.assertEqual(driver.receipt['capture_limit'], 10)
            driver.execute(dict(op='capture', name='tenth.png'))
            self.assertEqual(driver.captures, 10)
            before = len(fixture.calls)
            with self.assertRaises(packet.Refused):
                driver.execute(dict(op='capture', name='eleventh.png'))
            self.assertEqual(len(fixture.calls), before)

    def test_invalid_or_unimplemented_capture_ceiling_refuses_before_transport(self):
        for limit in [0, 161, 10., True]:
            with self.subTest(limit=limit), self.assertRaises(packet.Refused):
                packet.Driver('/unused', None, None, capture_limit=limit)

    def test_longer_inspected_task_keeps_a_finite_constructor_capture_bound(self):
        with tempfile.TemporaryDirectory() as root:
            fixture = Fixture(root)
            driver = packet.Driver(root, fixture.budget, fixture.invoke,
                                   lambda: fixture.now, fixture.sleep, capture_limit=96)
            self.assertEqual(driver.receipt['capture_limit'], 96)
            driver.captures = 95
            driver.execute(dict(op='capture', name='last.png'))
            before = len(fixture.calls)
            with self.assertRaises(packet.Refused):
                driver.execute(dict(op='capture', name='extra.png', capture_limit=97))
            self.assertEqual(driver.captures, 96)
            self.assertEqual(len(fixture.calls), before)

    def test_larger_prepared_navigation_retains_its_own_capture_ceiling(self):
        with tempfile.TemporaryDirectory() as root:
            fixture = Fixture(root)
            driver = packet.Driver(root, fixture.budget, fixture.invoke,
                                   lambda: fixture.now, fixture.sleep, capture_limit=160)
            driver.captures = 159
            driver.execute(dict(op='capture', name='last.png'))
            before = len(fixture.calls)
            with self.assertRaises(packet.Refused):
                driver.execute(dict(op='capture', name='extra.png', capture_limit=161))
            self.assertEqual(driver.captures, 160)
            self.assertEqual(len(fixture.calls), before)

    def test_prepared_two_stop_packet_has_one_aggregate_evidence_budget(self):
        with tempfile.TemporaryDirectory() as root:
            fixture = Fixture(root)
            driver = packet.Driver(root, fixture.budget, fixture.invoke,
                                   lambda: fixture.now, fixture.sleep,
                                   capture_limit=48, byte_limit=384 * 1024**2)
            self.assertEqual(driver.receipt['byte_limit'], 384 * 1024**2)
            driver.captures = 48
            with self.assertRaises(packet.Refused):
                driver.execute(dict(op='capture', name='extra.png'))
            self.assertEqual(fixture.calls, [])
            driver.received = driver.byte_limit
            with self.assertRaises(packet.Refused):
                driver.call(['shell', packet.FOCUS_QUERY])
            self.assertTrue(fixture.budget.forced_cleanup)

    def test_old_byte_limit_and_new_constructor_limit_do_not_follow_commands(self):
        with tempfile.TemporaryDirectory() as root:
            fixture = Fixture(root)
            self.assertEqual(fixture.driver.byte_limit, 256 * 1024**2)
            self.assertEqual(fixture.driver.receipt['byte_limit'], 256 * 1024**2)
        for invalid in (0, -1, True, 1., 768 * 1024**2 + 1):
            with self.subTest(limit=invalid), self.assertRaises(packet.Refused):
                packet.Driver('/unused', None, None, byte_limit=invalid)


class InspectedRegionTests(unittest.TestCase):
    def setUp(self):
        self.size = (128, 96)
        self.rgb = bytes([72, 104, 156]) * (128 * 96)
        self.bounds = (32, 24, 96, 88)

    def matches(self, **changes):
        args = dict(reference_size=self.size, reference_rgb=self.rgb,
                    current_size=self.size, current_rgb=self.rgb, bounds=self.bounds)
        args.update(changes)
        return packet.matching_inspected_region(**args)

    def test_exact_control_pixels_match(self):
        self.assertTrue(self.matches())

    def test_one_changed_control_pixel_refuses(self):
        changed = bytearray(self.rgb)
        changed[(40 * 128 + 50) * 3] ^= 1
        self.assertFalse(self.matches(current_rgb=changed))

    def test_background_outside_control_may_change(self):
        changed = bytearray(self.rgb)
        changed[0] ^= 1
        self.assertTrue(self.matches(current_rgb=changed))

    def test_changed_frame_size_refuses(self):
        self.assertFalse(self.matches(current_size=(96, 128)))

    def test_malformed_frame_sizes_refuse(self):
        for size in [None, (), (128,), (128., 96), (0, 96), (8193, 96)]:
            with self.subTest(size=size):
                self.assertFalse(self.matches(current_size=size))

    def test_malformed_rgb_plane_refuses(self):
        self.assertFalse(self.matches(current_rgb=self.rgb[:-1]))
        self.assertFalse(self.matches(reference_rgb=b''))
        self.assertFalse(self.matches(current_rgb=None))
        self.assertFalse(self.matches(reference_rgb=''))

    def test_invalid_or_tiny_bounds_refuse(self):
        for bounds in [None, (-1, 0, 64, 64), (32, 24, 129, 88), (32, 24, 63, 88),
                       (32, 24, 96, 24), (32., 24, 96, 88), (32, 24, 96)]:
            with self.subTest(bounds=bounds):
                self.assertFalse(self.matches(bounds=bounds))


class PacketTests(unittest.TestCase):
    def test_optional_lifecycle_limit_retains_fresh_state_and_presentation_proof(self):
        trace = ('native.pause.end playing\ngeometry present.ready buffer=2280x1080\n'
                 'lifecycle.optional_limit optional records omitted; native acknowledgment and Stop slots reserved\n')
        self.assertTrue(packet.playing_presentation_ready(trace))

    def test_missing_native_acknowledgments_refuse_even_with_a_fresh_frame(self):
        trace = ('native.pause.end playing\n'
                 'lifecycle.ack_limit trace cap: native state acknowledgments unavailable\n'
                 'geometry present.ready buffer=2280x1080\n')
        self.assertFalse(packet.playing_presentation_ready(trace))

    def test_play_readiness_rejects_historical_frame_after_surface_or_measurement_loss(self):
        first = "surface.init\ngeometry present.ready buffer=2280x1080\nnative.pause.end playing\n"
        self.assertTrue(packet.playing_presentation_ready(first))
        for invalid in ("surface.init", "surface.terminate", "geometry measurement=false presented=false", "geometry measurement=true presented=false", "geometry present.unavailable buffer=2280x1080"):
            stale = first + invalid + "\nnative.pause.end playing\n"
            self.assertFalse(packet.playing_presentation_ready(stale))
            self.assertTrue(packet.playing_presentation_ready(stale + "geometry present.ready buffer=2280x1080\n"))
        self.assertFalse(packet.playing_presentation_ready(first + "native.pause.end paused\n"))
        self.assertFalse(packet.playing_presentation_ready(first + "presentation trace cap reached\n"))

    def test_every_fold_display_must_be_off_and_policy_mstate_is_not_physical_state(self):
        self.assertFalse(packet.physical_displays_off("Display State=ON\nDisplay State=OFF\nmState=OFF"))
        self.assertTrue(packet.physical_displays_off("Display State=OFF\nDisplay State=OFF\nmState=AUTO_BRIGHTNESS_ENABLED"))
        self.assertFalse(packet.physical_displays_off("mState=OFF"))
        self.assertFalse(packet.physical_displays_off("Display State=DOZE\nDisplay State=OFF"))
        self.assertFalse(packet.physical_displays_off("Display State=ON_SUSPEND\nDisplay State=OFF"))

    def test_capture_resolves_only_active_local_panel_for_focused_display(self):
        self.f.viewports += "\n" + VIEWPORT.replace("isActive=true", "isActive=false").replace("local:123", "local:456")
        self.f.viewports += "\n" + VIEWPORT.replace("displayId=0", "displayId=3").replace("local:123", "local:789")
        self.f.driver.execute({"op": "capture", "name": "fold.png"})
        self.assertIn(["shell", "-T", "screencap", "-d", "123", "-p"], self.f.calls)
        self.assertTrue((Path(self.temp.name) / "fold.png").exists())

    def test_capture_refuses_missing_inactive_remote_or_ambiguous_panels_before_pixels(self):
        for views in ("", VIEWPORT.replace("isActive=true", "isActive=false"),
                      VIEWPORT.replace("valid=true", "valid=false"),
                      VIEWPORT.replace("local:123", "virtual:123"),
                      VIEWPORT.replace("displayId=0", "displayId=3"),
                      VIEWPORT + VIEWPORT.replace("local:123", "local:456")):
            with self.subTest(views=views):
                self.f.viewports = views
                with self.assertRaises(packet.Refused):
                    self.f.driver.execute({"op": "capture", "name": "refused.png"})
                self.assertFalse(any("screencap" in c for c in self.f.calls))

    def test_capture_panel_change_discards_pixels_and_enters_cleanup(self):
        for after in ("", VIEWPORT.replace("local:123", "local:456")):
            with self.subTest(after=after):
                self.f.viewports = VIEWPORT
                self.f.after_viewports = after
                with self.assertRaises(packet.Refused):
                    self.f.driver.execute({"op": "capture", "name": "discard.png"})
                self.assertTrue(self.f.budget.forced_cleanup)
                self.assertFalse(list(Path(self.temp.name).glob("*.png")))
                self.f.budget.forced_cleanup = False

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.f = Fixture(self.temp.name)

    def prepare_retirement(self):
        root = self.f.driver.root
        payload = b"original complete native payload fixture"
        checkpoint = hashlib.sha256(payload).hexdigest()
        game, profile = "a" * 32, "b" * 32
        base = f"files/ntx/profiles/{profile}/"
        collection = json.dumps({"schema_version": 1, "selected": game, "entries": [{
            "game_id": game, "profile_id": profile, "checkpoint": checkpoint,
            "paused": True, "runtime_incomplete": True, "stopped": False,
            "restart_required": False}]}).encode()
        files = {"files/ntx/collection.json": collection, base + "save-generation.json": b"generation",
                 base + "GC/SRAM.raw": b"console", base + "GC/MemoryCardA.USA.raw": b"ordinary save",
                 base + f"checkpoints/{checkpoint}.ntx": payload}
        archive = root / "complete.tar"
        with tarfile.open(archive, "w") as tar:
            for name, data in files.items():
                member = tarfile.TarInfo(name); member.size = len(data)
                tar.addfile(member, io.BytesIO(data))
        (root / "verified-generation.private.json").write_text(json.dumps({
            "checkpoint": checkpoint, "archive_sha256": hashlib.sha256(archive.read_bytes()).hexdigest(),
            "archive_bytes": archive.stat().st_size,
            "pause_intent": True,
            "verification": "PASS read-only checkpoint inspection: captured pause intent=true"}))
        (root / "trusted-binding.private.json").write_text(json.dumps({"profile_id": profile, "game_id": game}))
        self.f.retirement = {"collection": collection,
                             "hashes": {p: hashlib.sha256(data).hexdigest() for p, data in files.items()},
                             "ticks": 777, "logs": "native.pause.end paused\nCheckpoint complete: durable\n"}
        self.f.resumed = RESUMED.replace(OWN + "/.NtxActivity", "com.example.home/.HomeActivity")
        self.f.windows = WINDOW.replace(OWN + "/" + OWN + ".NtxActivity", "com.example.home/.HomeActivity")
        return {"op": "retire_checkpoint", "archive": "complete.tar", "pid": 1000, "start_ticks": 777}

    def test_checkpoint_retirement_requires_verified_backup_background_and_current_process(self):
        command = self.prepare_retirement()
        self.f.driver.execute(command)
        self.assertFalse(self.f.live)
        self.assertTrue(self.f.driver.receipt["checkpoint_process_retired"])
        self.assertFalse(self.f.driver.receipt["retirement_is_normal_stop"])
        self.assertEqual(sum("force-stop" in c for c in self.f.calls), 1)

    def test_retirement_refuses_newer_saves_reused_pid_playing_or_foreground_runtime(self):
        for case in ("card", "pid", "playing", "foreground"):
            with self.subTest(case=case), tempfile.TemporaryDirectory() as root:
                self.f = Fixture(root); command = self.prepare_retirement()
                if case == "card":
                    path = next(p for p in self.f.retirement["hashes"] if p.endswith("USA.raw"))
                    self.f.retirement["hashes"][path] = "0" * 64
                elif case == "pid":
                    self.f.retirement["ticks"] = 888
                elif case == "playing":
                    self.f.retirement["logs"] += "native.pause.end playing\n"
                else:
                    self.f.resumed, self.f.windows = RESUMED, WINDOW
                with self.assertRaises(packet.Refused):
                    self.f.driver.execute(command)
                self.assertTrue(self.f.live)
                self.assertFalse(any("force-stop" in c for c in self.f.calls))
                self.assertTrue(self.f.budget.forced_cleanup)

    def test_changed_backup_or_cleanup_flag_cannot_retire_live_runtime(self):
        command = self.prepare_retirement()
        command["cleanup"] = True
        with self.assertRaises(packet.Refused):
            self.f.driver.execute(command)
        command.pop("cleanup")
        with (self.f.driver.root / "complete.tar").open("ab") as file:
            file.write(b"changed")
        with self.assertRaises(packet.Refused):
            self.f.driver.execute(command)
        self.assertEqual(self.f.calls, [])

    def test_bounded_quiescent_recovery_keeps_all_proofs_and_is_never_joined_stop(self):
        command = self.prepare_retirement()
        command.update(op="recover_checkpoint", cleanup=True)
        self.f.budget.forced_cleanup = True
        self.f.driver.execute(command)
        self.assertFalse(self.f.live)
        self.assertTrue(self.f.driver.receipt["quiescent_recovery"])
        self.assertFalse(self.f.driver.receipt["retirement_is_normal_stop"])
        proof = self.f.driver.receipt["retirement_validation"]
        self.assertEqual((proof["pid"], proof["start_ticks"]), ("1000", "777"))
        self.assertIn("native.pause.end paused", Path(proof["quiescence"]).read_text())
        self.assertEqual(proof["actual_file_sha256"], self.f.retirement["hashes"])

    def test_failed_quiescence_is_retained_without_retirement_then_retry_is_distinct(self):
        command = self.prepare_retirement()
        self.f.retirement["logs"] += "native.pause.end playing\n"
        with self.assertRaises(packet.Refused):
            self.f.driver.execute(command)
        proof = self.f.driver.receipt["retirement_observations"][0]
        self.assertIn("native.pause.end playing", Path(proof["quiescence"]).read_text())
        self.assertNotIn("retirement_validation", self.f.driver.receipt)
        self.assertFalse(any("force-stop" in c for c in self.f.calls))
        self.f.retirement["logs"] += "native.pause.end paused\nCheckpoint complete: durable\n"
        command.update(op="recover_checkpoint", cleanup=True)
        self.f.driver.execute(command)
        latest = self.f.driver.receipt["retirement_validation"]
        self.assertNotEqual(latest["quiescence"], proof["quiescence"])
        self.assertEqual(len(self.f.driver.receipt["retirement_observations"]), 2)

    def test_automated_cutoff_finalizes_instead_of_waiting_through_cleanup(self):
        budget = self.f.budget
        self.assertEqual(budget.input_boundary(budget.cleanup_at - 1, True), budget.cleanup_at)
        self.assertEqual(budget.input_boundary(budget.cleanup_at, True), budget.cleanup_at)
        self.assertEqual(budget.input_boundary(budget.cleanup_at + 1, True), budget.cleanup_at + 1)
        self.assertEqual(budget.input_boundary(budget.cleanup_at, False), budget.commands_end)
        budget.forced_cleanup = True
        self.assertEqual(budget.input_boundary(101, True), 101)

    def test_pre_restore_recovery_requires_full_bootstrap_trace_and_refuses_any_restore_or_play(self):
        bootstrap = "activity.init\nruntime.boot.begin\nboot.checkpoint_validate.end ok\nboot.native_paused.end ok\nruntime.boot.end ready_for_restore_or_play\nnative.pause.end paused\n"
        for suffix in ("", "restore.validate.begin\n", "restore.load.end ok_paused\n", "native.pause.end playing\n", "runtime.boot.begin\n"):
            with self.subTest(suffix=suffix), tempfile.TemporaryDirectory() as root:
                self.f=Fixture(root);command=self.prepare_retirement();command.update(op="recover_pending_restore",cleanup=True)
                self.f.retirement["logs"]=bootstrap+suffix
                if suffix:
                    with self.assertRaises(packet.Refused):self.f.driver.execute(command)
                    self.assertTrue(self.f.live)
                else:
                    self.f.driver.execute(command);self.assertFalse(self.f.live)
                    self.assertTrue(self.f.driver.receipt["pending_restore_recovery"])
                    self.assertFalse(self.f.driver.receipt["retirement_is_normal_stop"])

    def test_cleanup_at_locked_screen_off_needs_no_resumed_activity_or_wake(self):
        command=self.prepare_retirement();command.update(op="recover_checkpoint",cleanup=True)
        self.f.windows="Display: mDisplayId=0\nmCurrentFocus=Window{1 u0 NotificationShade}"
        self.f.resumed="Display #0 (activities):"
        self.f.driver.execute(command)
        self.assertFalse(self.f.live)
        self.assertEqual(self.f.driver.receipt["retirement_background"],"physical_display_off_and_keyguard_showing")
        self.assertFalse(any("input" in c or "wake" in c for c in self.f.calls))

    def test_earliest_deadline_and_real_cleanup_reserve(self):
        budget = packet.Budget(100, 800)
        self.assertEqual((budget.hard_end, budget.cleanup_at, budget.commands_end),
                         (800, 500, 740))
        self.assertEqual(budget.owner_wait(498, 60), 2)
        with self.assertRaises(packet.Refused):
            budget.owner_wait(500, 1)
        with self.assertRaises(packet.Refused):
            budget.admit(480, False, 30)
        budget.admit(500, True, 30)
        with self.assertRaises(packet.Refused):
            budget.admit(740, True, 1)
        with self.assertRaises(packet.Refused):
            packet.Budget(100, 399)

    def test_wait_crosses_to_cleanup_but_never_waits_after_it(self):
        self.f.now = self.f.budget.cleanup_at - 2
        answer = self.f.driver.execute({"op": "wait_owner", "seconds": 60})
        self.assertTrue(answer["cleanup_required"])
        with self.assertRaises(packet.Refused):
            self.f.driver.execute({"op": "wait_owner"})
        with self.assertRaises(packet.Refused):
            self.f.driver.execute({"op": "capture", "name": "late.png"})
        with self.assertRaises(packet.Refused):
            self.f.driver.execute({"op": "ntx_button", "text": "Resume", "cleanup": True})
        self.assertEqual(self.f.calls, [])

    def test_requested_short_timeout_is_also_the_actual_operation_deadline(self):
        self.f.now = self.f.budget.cleanup_at - 2
        self.f.driver.execute({"op": "hash_files", "paths": ["files/ntx/card.raw"],
                               "timeout": 1})
        self.assertEqual(self.f.driver.operation_end, self.f.budget.cleanup_at - 1)
        self.assertEqual(self.f.timeouts, [1])

    def test_live_display_section_excludes_historical_and_other_display_focus(self):
        windows = ("historical ANR mCurrentFocus=Window{old u0 " + OTHER + "}\n" +
                   "Display: mDisplayId=4 rootTasks=1\nmCurrentFocus=Window{other u0 " + OTHER + "}\n" + WINDOW)
        activities = "Display #4 (activities):\ntopResumedActivity=" + OTHER + "\n" + RESUMED
        self.assertEqual(packet.own_focus(activities, windows)[0], (OWN, OWN + ".NtxActivity"))
        with self.assertRaises(packet.Refused):
            packet.own_focus(activities, windows.replace("mTopFocusedDisplayId=0", "mTopFocusedDisplayId=4"))

    def test_android13_single_live_display_needs_no_global_top_field(self):
        windows = WINDOW.replace("mTopFocusedDisplayId=0", "")
        self.assertEqual(packet.own_focus(RESUMED, windows)[0], (OWN, OWN + ".NtxActivity"))
        with self.assertRaises(packet.Refused):
            packet.own_focus(RESUMED, windows + "\nDisplay: mDisplayId=4 rootTasks=1\n")

    def test_resumed_and_focused_must_both_be_own_activity(self):
        for resumed, windows in ((RESUMED, f"mCurrentFocus=Window{{1 u0 {OTHER}}}"),
                                 (f"mResumedActivity: {OTHER}", WINDOW)):
            with self.assertRaises(packet.Refused):
                packet.own_focus(resumed, windows)
        # Android can report the same resumed component twice, in two formats.
        self.assertEqual(packet.own_focus(RESUMED + "\ntopResumedActivity=" +
                                         OWN + "/" + OWN + ".NtxActivity", WINDOW)[0],
                         (OWN, OWN + ".NtxActivity"))

    def test_coherent_foreign_refusal_retains_first_predicate_without_extra_query(self):
        own = f"{OWN}/{OWN}.NtxActivity"
        self.f.windows = WINDOW.replace(own, OTHER)
        self.f.resumed = RESUMED.replace(f'{OWN}/.NtxActivity', OTHER)
        with self.assertRaises(packet.Refused):self.f.driver.focus()
        proof = self.f.driver.receipt['foreground_refusal_observation']
        self.assertEqual(proof['component'],list(packet.component(OTHER)))
        self.assertEqual((proof['at'],proof['display']),(100,'0'))
        self.assertEqual(len(self.f.calls),1)
        self.assertTrue(self.f.driver.budget.forced_cleanup)
        self.assertTrue(self.f.driver.mismatch)
        self.f.windows = WINDOW.replace(own,'second.app/.Second')
        self.f.resumed = RESUMED.replace(f'{OWN}/.NtxActivity','second.app/.Second')
        with self.assertRaises(packet.Refused):self.f.driver.focus()
        self.assertEqual(self.f.driver.receipt['foreground_refusal_observation'],proof)
        self.assertEqual(len(self.f.calls),2)
        self.assertFalse(any('input' in call or 'screencap' in call for call in self.f.calls))

    def test_empty_viewport_read_retries_observation_without_replaying_capture(self):
        invoke = self.f.driver.invoke
        observations = 0
        def transient(args, timeout):
            nonlocal observations
            if args == ["shell", "dumpsys display | grep 'DisplayViewport{' "]:
                observations += 1
                if observations == 2:
                    self.f.calls.append(args)
                    return subprocess.CompletedProcess(args, 1, b"", b"")
            return invoke(args, timeout)
        self.f.driver.invoke = transient
        self.f.driver.execute({"op": "capture", "name": "viewport.png"})
        self.assertEqual(observations, 3)
        self.assertEqual(sum("screencap" in call for call in self.f.calls), 1)
        self.assertFalse(self.f.driver.budget.forced_cleanup)
        self.assertFalse(any("input" in call for call in self.f.calls))

    def test_missing_viewport_retries_remain_bounded_and_do_not_authorize_input(self):
        invoke = self.f.driver.invoke
        observations = 0
        def missing(args, timeout):
            nonlocal observations
            if args == ["shell", "dumpsys display | grep 'DisplayViewport{' "]:
                observations += 1
                if observations > 1:
                    self.f.calls.append(args)
                    return subprocess.CompletedProcess(args, 1, b"", b"")
            return invoke(args, timeout)
        self.f.driver.invoke = missing
        with self.assertRaises(packet.Refused):
            self.f.driver.execute({"op": "capture", "name": "viewport.png"})
        self.assertEqual(observations, 4)
        self.assertTrue(self.f.driver.budget.forced_cleanup)
        self.assertFalse(any("input" in call for call in self.f.calls))

    def test_mismatch_blocks_capture_and_touch_before_mutation(self):
        self.f.windows = f"mCurrentFocus=Window{{1 u0 {OTHER}}}"
        for command in ({"op": "capture", "name": "wrong.png"},
                        {"op": "ntx_button", "text": "Resume", "cleanup": True}):
            with self.assertRaises(packet.Refused):
                self.f.driver.execute(command)
        self.assertTrue(self.f.driver.mismatch)
        self.assertFalse(any("screencap" in c or "input" in c for c in self.f.calls))
        self.assertFalse(list(Path(self.temp.name).glob("*.png")))

    def test_focus_change_during_capture_discards_pixels(self):
        self.f.after_capture = f"mCurrentFocus=Window{{1 u0 {OTHER}}}"
        with self.assertRaises(packet.Refused):
            self.f.driver.execute({"op": "capture", "name": "discard.png"})
        self.assertFalse(list(Path(self.temp.name).glob("*.png")))

    def test_native_tap_is_single_use_and_bound_to_current_inspected_image(self):
        self.f.driver.execute({"op": "capture", "name": "native.png"})
        tap = {"op": "ntx_tap", "capture": "native.png", "target": "Pause",
               "bounds": [30, 300, 90, 390]}
        self.f.driver.execute(tap)
        self.assertEqual(self.f.calls[-1], ["shell", "input", "tap", "60", "345"])
        with self.assertRaises(packet.Refused):
            self.f.driver.execute(tap)

    def test_stick_contact_is_named_finite_and_single_use(self):
        self.f.driver.execute(dict(op='capture', name='stick.png'))
        command = dict(op='ntx_stick', target='C Stick', capture='stick.png',
                       bounds=[30, 290, 150, 390], delta=[20, -15], duration_ms=500)
        self.f.driver.execute(command)
        self.assertEqual(self.f.calls[-1],
                         ['shell', 'input', 'swipe', '90', '340', '110', '325', '500'])
        with self.assertRaises(packet.Refused):
            self.f.driver.execute(command)

    def test_stick_contact_refuses_unbounded_arbitrary_and_cleanup_routes(self):
        self.f.driver.execute(dict(op='capture', name='stick.png'))
        command = dict(op='ntx_stick', target='Control Stick', capture='stick.png',
                       bounds=[30, 290, 150, 390], delta=[20, 0], duration_ms=500)
        for change in ({'target': 'A'}, {'cleanup': True}, {'delta': [0, 0]},
                       {'delta': [34, 0]}, {'delta': [25, 25]}, {'delta': [True, 0]},
                       {'delta': [1]}, {'delta': '20,0'}, {'duration_ms': 99},
                       {'duration_ms': 1001}, {'duration_ms': True}):
            with self.subTest(change=change), self.assertRaises(packet.Refused):
                self.f.driver.execute(dict(command, **change))
        self.assertFalse(any('input' in call for call in self.f.calls))

    def test_explicit_stick_hold_is_one_bounded_stationary_contact_and_single_use(self):
        self.f.driver.execute(dict(op='capture', name='held-stick.png'))
        command = dict(op='ntx_stick', target='Control Stick', capture='held-stick.png',
                       bounds=[30, 290, 150, 390], delta=[20, -15], duration_ms=500,
                       motion='hold')
        self.f.driver.execute(command)
        self.assertEqual(self.f.calls[-1],
                         ['shell', 'input', 'swipe', '110', '325', '110', '325', '500'])
        self.assertEqual(self.f.driver.actions, 1)
        with self.assertRaises(packet.Refused):self.f.driver.execute(command)

    def test_analog_travel_is_separate_balanced_finite_and_single_use(self):
        self.f.driver.execute(dict(op='capture', name='analog.png'))
        command = dict(op='ntx_trigger', target='L analog', capture='analog.png',
                       bounds=[30, 80, 130, 140], travel_percent=35, duration_ms=100)
        self.f.driver.execute(command)
        self.assertEqual(self.f.calls[-1],
                         ['shell', 'input', 'swipe', '65', '110', '65', '110', '100'])
        self.assertEqual(self.f.driver.actions, 1)
        with self.assertRaises(packet.Refused):self.f.driver.execute(command)

    def test_analog_contact_rejects_ambiguous_button_travel_and_cleanup(self):
        self.f.driver.execute(dict(op='capture', name='analog.png'))
        command = dict(op='ntx_trigger', target='L analog', capture='analog.png',
                       bounds=[30, 80, 130, 140], travel_percent=95, duration_ms=100)
        for change in ({'target':'L click'}, {'target':'L'}, {'cleanup':True},
                       {'travel_percent':True}, {'travel_percent':0}, {'travel_percent':100},
                       {'duration_ms':True}, {'duration_ms':99}, {'duration_ms':501},
                       {'bounds':[30,80,32,140]}):
            with self.subTest(change=change),self.assertRaises(packet.Refused):
                self.f.driver.execute(dict(command, **change))
        self.assertFalse(any('input' in call for call in self.f.calls))

    def test_right_analog_near_full_travel_stays_inside_its_bar(self):
        self.f.driver.execute(dict(op='capture', name='right-analog.png'))
        self.f.driver.execute(dict(op='ntx_trigger', target='R analog',
            capture='right-analog.png', bounds=[30, 80, 130, 140],
            travel_percent=99, duration_ms=500))
        self.assertEqual(self.f.calls[-1],
            ['shell', 'input', 'swipe', '129', '110', '129', '110', '500'])

    def test_analog_contact_keeps_foreground_freshness_and_frame_guards(self):
        self.f.driver.execute(dict(op='capture', name='analog.png'))
        command = dict(op='ntx_trigger', target='R analog', capture='analog.png',
                       bounds=[30, 80, 130, 140], travel_percent=95, duration_ms=100)
        self.f.now +=31
        with self.assertRaises(packet.Refused):self.f.driver.execute(command)
        self.f.now -=31
        self.f.windows = f"mCurrentFocus=Window{{1 u0 {OTHER}}}"
        with self.assertRaises(packet.Refused):self.f.driver.execute(command)
        self.assertFalse(any('input' in call for call in self.f.calls))

    def test_only_explicit_digital_l_click_is_admitted_as_a_finite_button_contact(self):
        self.f.driver.execute(dict(op='capture', name='l-click.png'))
        command = dict(op='ntx_tap', target='L click', capture='l-click.png',
                       bounds=[30, 80, 90, 140], press_ms=100)
        for change in ({'target': 'L analog'}, {'target': 'L'}, {'target': 'R click'},
                       {'cleanup': True}, {'press_ms': 251}):
            with self.subTest(change=change), self.assertRaises(packet.Refused):
                self.f.driver.execute(dict(command, **change))
        self.assertFalse(any('input' in call for call in self.f.calls))
        self.f.driver.execute(command)
        self.assertEqual(self.f.calls[-1],
                         ['shell', 'input', 'swipe', '60', '110', '60', '110', '100'])
        with self.assertRaises(packet.Refused):self.f.driver.execute(command)

    def test_hold_retains_fresh_frame_geometry_target_duration_and_well_guards(self):
        self.f.driver.execute(dict(op='capture', name='held-stick.png'))
        command = dict(op='ntx_stick', target='C Stick', capture='held-stick.png',
                       bounds=[30, 290, 150, 390], delta=[20, -15], duration_ms=500,
                       motion='hold')
        for change in ({'motion': 'other'}, {'motion': True}, {'motion': None},
                       {'target': 'A'}, {'cleanup': True}, {'delta': [34, 0]},
                       {'duration_ms': 1001}):
            with self.subTest(change=change), self.assertRaises(packet.Refused):
                self.f.driver.execute(dict(command, **change))
        self.f.now += 31
        with self.assertRaises(packet.Refused):self.f.driver.execute(command)
        self.f.now -= 31
        self.f.windows = WINDOW.replace('r=0', 'r=1')
        with self.assertRaises(packet.Refused):self.f.driver.execute(command)
        self.assertFalse(any('input' in call for call in self.f.calls))

    def test_stick_capture_expiry_and_window_change_refuse(self):
        self.f.driver.execute(dict(op='capture', name='stick.png'))
        command = dict(op='ntx_stick', target='Control Stick', capture='stick.png',
                       bounds=[30, 290, 150, 390], delta=[20, 0], duration_ms=500)
        self.f.now += 31
        with self.assertRaises(packet.Refused):
            self.f.driver.execute(command)
        self.f.now -= 31
        self.f.windows = WINDOW.replace('r=0', 'r=1')
        with self.assertRaises(packet.Refused):
            self.f.driver.execute(command)
        self.assertFalse(any('input' in call for call in self.f.calls))

    def test_prepared_longer_stick_contact_keeps_its_task_bound_and_single_use(self):
        driver = packet.Driver(self.temp.name, self.f.budget, self.f.invoke,
                               lambda: self.f.now, self.f.sleep, stick_ms_limit=4000)
        driver.execute(dict(op='capture', name='long-stick.png'))
        command = dict(op='ntx_stick', target='Control Stick', capture='long-stick.png',
                       bounds=[30, 290, 150, 390], delta=[20, 0], duration_ms=4001)
        with self.assertRaises(packet.Refused):
            driver.execute(command)
        command['duration_ms'] = 4000
        driver.execute(command)
        self.assertEqual(self.f.calls[-1][-1], '4000')
        self.assertEqual(driver.receipt['stick_ms_limit'], 4000)
        with self.assertRaises(packet.Refused):
            driver.execute(command)

    def test_stick_contact_default_and_invalid_constructor_bounds(self):
        self.assertEqual(self.f.driver.stick_ms_limit, 1000)
        for value in (0, 99, 4001, True, 4000.):
            with self.subTest(value=value), self.assertRaises(packet.Refused):
                packet.Driver('/unused', None, None, stick_ms_limit=value)

    def test_focus_metadata_is_batched_without_dropping_either_observation(self):
        self.f.driver.focus()
        self.assertEqual(self.f.calls, [["shell", packet.FOCUS_QUERY]])
        self.f.resumed = RESUMED.replace(OWN + "/.NtxActivity", OTHER)
        with self.assertRaises(packet.Refused):
            self.f.driver.focus()
        self.assertTrue(self.f.driver.mismatch)

    def test_held_test_stimulus_is_one_bounded_stationary_contact(self):
        self.f.driver.execute({"op": "capture", "name": "native.png"})
        self.f.driver.execute({"op": "ntx_tap", "capture": "native.png", "target": "Start",
                               "bounds": [30, 300, 90, 390], "press_ms": 120})
        self.assertEqual(self.f.calls[-1], ["shell", "input", "swipe", "60", "345", "60", "345", "120"])
        with self.assertRaises(packet.Refused):
            self.f.driver.execute({"op": "ntx_tap", "capture": "native.png", "target": "Start",
                                   "bounds": [30, 300, 90, 390], "press_ms": 120})

    def test_held_test_stimulus_refuses_invalid_duration_before_input(self):
        self.f.driver.execute({"op": "capture", "name": "native.png"})
        for invalid in (-1, 251, True, "120", 0.5):
            with self.assertRaises(packet.Refused):
                self.f.driver.execute({"op": "ntx_tap", "capture": "native.png", "target": "Start",
                                       "bounds": [30, 300, 90, 390], "press_ms": invalid})
        self.assertFalse(any("input" in c for c in self.f.calls))

    def test_pause_cannot_hold_contact_across_opening_a_dialog_window(self):
        self.f.driver.execute({"op": "capture", "name": "native.png"})
        with self.assertRaises(packet.Refused):
            self.f.driver.execute({"op": "ntx_tap", "capture": "native.png", "target": "Pause",
                                   "bounds": [30, 300, 90, 390], "press_ms": 120})
        self.assertFalse(any("input" in c for c in self.f.calls))

    def test_native_tap_refuses_stale_or_rotated_capture(self):
        self.f.driver.execute({"op": "capture", "name": "native.png"})
        tap = {"op": "ntx_tap", "capture": "native.png", "target": "Pause",
               "bounds": [30, 300, 90, 390]}
        self.f.now += 31
        with self.assertRaises(packet.Refused):
            self.f.driver.execute(tap)
        self.f.now -= 31
        self.f.windows = WINDOW.replace("w=800 h=400 r=0", "w=400 h=800 r=1")
        with self.assertRaises(packet.Refused):
            self.f.driver.execute(tap)
        self.assertFalse(any("input" in c for c in self.f.calls))

    def test_prepared_inspection_window_handles_model_latency_but_stays_single_use(self):
        self.f.driver.inspection_seconds = 90
        self.f.driver.execute({"op": "capture", "name": "native.png"})
        self.f.now += 84
        tap = {"op": "ntx_tap", "capture": "native.png", "target": "Pause",
               "bounds": [30, 300, 90, 390]}
        self.f.driver.execute(tap)
        self.assertEqual(self.f.calls[-1], ["shell", "input", "tap", "60", "345"])
        with self.assertRaises(packet.Refused):
            self.f.driver.execute(tap)

    def test_prepared_inspection_window_still_refuses_expiry_and_focus_change(self):
        self.f.driver.inspection_seconds = 90
        self.f.driver.execute({"op": "capture", "name": "native.png"})
        tap = {"op": "ntx_tap", "capture": "native.png", "target": "Pause",
               "bounds": [30, 300, 90, 390]}
        self.f.now += 91
        with self.assertRaises(packet.Refused):
            self.f.driver.execute(tap)
        self.f.now -= 7
        self.f.windows = WINDOW.replace(OWN, "com.example.other")
        with self.assertRaises(packet.Refused):
            self.f.driver.execute(tap)
        self.assertFalse(any("input" in c for c in self.f.calls))

    def test_inspection_window_must_be_finite_and_explicitly_prepared(self):
        for value in (0, -1, 91, True, 30.0, "90", None):
            with self.subTest(value=value), self.assertRaises(packet.Refused):
                packet.Driver(self.f.driver.root, self.f.budget, self.f.invoke,
                              inspection_seconds=value)
        default = packet.Driver(self.f.driver.root, self.f.budget, self.f.invoke)
        prepared = packet.Driver(self.f.driver.root, self.f.budget, self.f.invoke,
                                 inspection_seconds=90)
        self.assertEqual(default.inspection_seconds, 30)
        self.assertEqual(prepared.receipt["inspection_seconds"], 90)

    def test_native_image_age_is_rechecked_at_dispatch_after_slow_focus_queries(self):
        self.f.driver.execute({"op": "capture", "name": "native.png"})
        self.f.now += 10
        invoke = self.f.driver.invoke
        def slow_focus(args, timeout):
            result = invoke(args, timeout)
            if "dumpsys window displays" in " ".join(args):
                self.f.now += 25
            return result
        self.f.driver.invoke = slow_focus
        with self.assertRaises(packet.Refused):
            self.f.driver.execute({"op": "ntx_tap", "capture": "native.png", "target": "Pause",
                                   "bounds": [30, 300, 90, 390]})
        self.assertFalse(any("input" in c for c in self.f.calls))

    def test_each_button_uses_fresh_observed_bounds_and_focus(self):
        self.f.driver.execute({"op": "ntx_button", "text": "Resume"})
        self.f.driver.execute({"op": "ntx_button", "text": "Stop game", "cleanup": True})
        self.assertEqual([c for c in self.f.calls if "tap" in c],
                         [["shell", "input", "tap", "170", "120"],
                          ["shell", "input", "tap", "550", "240"]])
        self.assertEqual(sum("dumpsys activity" in " ".join(c) for c in self.f.calls), 4)

    def test_scroll_view_clips_button_even_when_button_is_inside_the_screen(self):
        xml = XML.replace('<node text="Stop game"',
                          '<node bounds="[300,10][750,190]"><node text="Stop game"')
        xml = xml.replace('bounds="[400,200][700,280]"/>',
                          'bounds="[400,200][700,280]"/></node>')
        with self.assertRaises(packet.Refused):
            packet.button_center(xml, "Stop game")
        visible = xml.replace('[300,10][750,190]', '[300,10][750,250]')
        self.assertEqual(packet.button_center(visible, "Stop game"), (550, 225))

    def test_negative_offscreen_bounds_cannot_become_positive_tap_coordinates(self):
        xml = XML.replace('[400,200][700,280]', '[400,-280][700,-200]')
        with self.assertRaises(packet.Refused):
            packet.button_center(xml, "Stop game")

    def test_focus_change_after_layout_aborts_the_input_batch(self):
        self.f.after_dump = f"mCurrentFocus=Window{{1 u0 {OTHER}}}"
        with self.assertRaises(packet.Refused):
            self.f.driver.execute({"op": "ntx_button", "text": "Resume"})
        self.assertFalse(any("input" in c for c in self.f.calls))
        with self.assertRaises(packet.Refused):
            packet.button_center(XML, "missing stale button")

    def test_same_package_new_window_invalidates_the_observed_button_layout(self):
        self.f.after_dump = WINDOW.replace("Window{1", "Window{2")
        with self.assertRaises(packet.Refused):
            self.f.driver.execute({"op": "ntx_button", "text": "Resume"})
        self.assertFalse(any("input" in c for c in self.f.calls))

    def test_focused_app_without_a_current_focused_window_is_not_admission(self):
        self.f.windows = f"mFocusedApp=ActivityRecord{{1 u0 {OWN}/.NtxActivity t1}}"
        with self.assertRaises(packet.Refused):
            self.f.driver.execute({"op": "capture", "name": "missing-focus.png"})
        self.assertFalse(any("screencap" in c for c in self.f.calls))
        self.assertFalse(list(Path(self.temp.name).glob("*.png")))
        proof = self.f.driver.receipt["focus_refusals"]
        self.assertEqual(len(proof), 1)
        self.assertIn("mFocusedApp=", Path(proof[0]).read_text())

    def test_raw_batches_and_option_variants_cannot_bypass_guards(self):
        for args in (["shell", "input", "--display", "0", "tap", "1", "2"],
                     ["shell", "input tap 1 2; input keyevent HOME"],
                     ["shell", "sh", "-c", "input tap 1 2"],
                     ["exec-out", "screencap", "-p"],
                     ["shell", "am", "start", "-n", OTHER],
                     ["shell", "uiautomator", "dump"],
                     ["shell", "run-as", OWN, "sh", "-c", "sha256sum"]):
            with self.assertRaises(packet.Refused):
                self.f.driver.execute({"op": "adb", "args": args})
        self.assertEqual(self.f.calls, [])

    def test_unexpected_app_cannot_be_displaced_by_ordinary_launch(self):
        self.f.resumed = "mResumedActivity: " + OTHER
        self.f.windows = f"mCurrentFocus=Window{{1 u0 {OTHER}}}"
        with self.assertRaises(packet.Refused):
            self.f.driver.execute({"op": "launch"})
        self.assertFalse(any("am" in c for c in self.f.calls))

    def test_authorized_initial_own_entry_cannot_override_later_mismatch(self):
        self.f.resumed = RESUMED.replace(OWN + "/.NtxActivity", OTHER)
        self.f.windows = WINDOW.replace(OWN + "/" + OWN + ".NtxActivity", OTHER)
        self.f.driver.execute({"op": "launch", "initial_entry": True})
        self.assertEqual(sum("am" in c for c in self.f.calls), 1)
        self.assertIn("-W", next(c for c in self.f.calls if "am" in c))
        with self.assertRaises(packet.Refused):
            self.f.driver.execute({"op": "launch", "initial_entry": True})
        self.assertEqual(sum("am" in c for c in self.f.calls), 1)

    def test_hashing_always_has_literal_file_arguments(self):
        paths = ["files/ntx/profiles/fixture/cards/MemoryCardA.USA.raw"]
        args = packet.hash_arguments(paths)
        self.assertEqual(args, ["shell", "run-as", OWN, "sha256sum", *paths])
        for invalid in ([], ["files/ntx/*.raw"], ["files/ntx/a b"],
                        ["files/ntx/../private"], ["files/ntx/$(echo x)"],
                        ["/files/ntx/card"]):
            with self.assertRaises(packet.Refused):
                packet.hash_arguments(invalid)
        self.f.driver.execute({"op": "hash_files", "paths": paths})
        self.assertEqual(self.f.calls, [args])

    def test_unclean_live_writer_is_retained_and_foreign_ui_untouched(self):
        self.f.windows = f"mCurrentFocus=Window{{1 u0 {OTHER}}}"
        self.f.driver.finish()
        self.assertFalse(self.f.driver.receipt["handback"])
        self.assertIn("pending_recovery", self.f.driver.receipt)
        self.assertFalse(any("force-stop" in c or "input" in c for c in self.f.calls))

    def test_timeout_stops_all_further_device_commands(self):
        self.f.fail = True
        with self.assertRaises(subprocess.TimeoutExpired):
            self.f.driver.execute({"op": "hash_files", "paths": ["files/ntx/card.raw"]})
        count = len(self.f.calls)
        self.f.driver.finish()
        self.assertEqual(len(self.f.calls), count)
        self.assertFalse(self.f.driver.receipt["handback"])
        with self.assertRaises(packet.Refused):
            self.f.driver.call(["shell", "pidof", OWN])
        self.assertEqual(len(self.f.calls), count)

    def test_asynchronous_sleep_transition_requires_fresh_return_observations(self):
        self.f.clean = True
        self.f.display_states = ["ON", "ON", "OFF"]
        self.f.keyguard_states = ["false", "true"]
        self.f.driver.finish()
        self.assertTrue(self.f.driver.receipt["handback"])
        self.assertAlmostEqual(self.f.now, 100.3)
        self.assertEqual(sum("KEYCODE_SLEEP" in c for c in self.f.calls), 1)

    def test_unsettled_display_return_has_finite_attempts_and_pending_reason(self):
        self.f.clean = True
        self.f.display_states = ["ON"]
        self.f.keyguard_states = ["false"]
        self.f.driver.finish()
        self.assertFalse(self.f.driver.receipt["handback"])
        self.assertIn("pending_recovery", self.f.driver.receipt)
        self.assertEqual(sum("dumpsys display" in " ".join(c) for c in self.f.calls), 5)
        self.assertAlmostEqual(self.f.now, 100.9)

    def test_clean_metadata_then_client_retirement_and_return(self):
        self.f.clean = True
        self.f.driver.finish()
        self.assertTrue(self.f.driver.receipt["clean_stop_metadata"])
        self.assertTrue(self.f.driver.receipt["clients_retired"])
        self.assertTrue(self.f.driver.receipt["handback"])
        self.assertTrue(self.f.driver.receipt["recovery_resolved"])
        self.assertEqual(sum("force-stop" in c for c in self.f.calls), 1)

    def test_verified_new_install_can_close_empty_library_without_claiming_stop(self):
        self.f.driver.receipt["verified_new_install"] = True
        invoke = self.f.driver.invoke
        def new_install(args, timeout):
            if "cat" in args and "files/ntx/collection.json" in args:
                self.assertEqual(args, ["shell", "run-as", OWN, "cat", "files/ntx/collection.json"])
                return subprocess.CompletedProcess(args, 1, b"", b"cat: No such file or directory")
            return invoke(args, timeout)
        self.f.driver.invoke = new_install
        self.f.driver.finish()
        self.assertTrue(self.f.driver.receipt["handback"])
        self.assertTrue(self.f.driver.receipt["new_install_without_game"])
        self.assertFalse(self.f.driver.receipt["clean_stop_metadata"])

    def test_missing_file_message_in_success_stdout_is_not_empty_install_proof(self):
        self.f.driver.receipt["verified_new_install"] = True
        invoke = self.f.driver.invoke
        def merged_error(args, timeout):
            if "cat" in args and "files/ntx/collection.json" in args:
                return subprocess.CompletedProcess(args, 0, b"cat: No such file or directory", b"")
            return invoke(args, timeout)
        self.f.driver.invoke = merged_error
        self.f.driver.finish()
        self.assertFalse(self.f.driver.receipt["handback"])
        self.assertTrue(self.f.live)
        self.assertFalse(any("force-stop" in c for c in self.f.calls))

    def test_missing_collection_without_new_install_proof_cannot_retire_client(self):
        invoke = self.f.driver.invoke
        def missing(args, timeout):
            if "cat" in args and "files/ntx/collection.json" in args:
                return subprocess.CompletedProcess(args, 1, b"", b"cat: No such file or directory")
            return invoke(args, timeout)
        self.f.driver.invoke = missing
        self.f.driver.finish()
        self.assertFalse(self.f.driver.receipt["handback"])
        self.assertTrue(self.f.live)
        self.assertFalse(any("force-stop" in c for c in self.f.calls))


class DiagnosticHistoryTests(unittest.TestCase):
    identity = ('1234', '5678')

    def line(self, number, body):
        return f'10-26 12:00:00.{number:03d} 1234 1235 E ntx : Lifecycle +{number}us {body}'

    def test_evicted_prefix_requires_exact_tail_and_retains_all_records(self):
        old = [self.line(i, str(i)) for i in range(8)]
        fresh = old[-2:] + [self.line(8, 'new')]
        merged = packet.merge_diagnostic_history('\n'.join(old), '\n'.join(fresh),
                                                  self.identity, self.identity)
        self.assertEqual(merged.splitlines(), old + fresh[-1:])

    def test_new_surface_invalidation_stays_after_old_readiness(self):
        old = [self.line(1, 'native.pause.end playing'),
               self.line(2, 'geometry present.ready'), self.line(3, 'input')]
        fresh = old[-2:] + [self.line(4, 'surface.terminate')]
        merged = packet.merge_diagnostic_history('\n'.join(old), '\n'.join(fresh),
                                                  self.identity, self.identity)
        self.assertFalse(packet.playing_presentation_ready(merged))
        fresh += [self.line(5, 'geometry present.ready')]
        merged = packet.merge_diagnostic_history(merged, '\n'.join(fresh),
                                                  self.identity, self.identity)
        self.assertTrue(packet.playing_presentation_ready(merged))

    def test_empty_disjoint_or_reordered_reads_refuse_cached_readiness(self):
        old = [self.line(i, str(i)) for i in range(4)]
        for fresh in ([], [self.line(5, 'new')], old[:2], old[-1:] + old[-2:-1]):
            with self.subTest(fresh=fresh), self.assertRaises(packet.Refused):
                packet.merge_diagnostic_history('\n'.join(old), '\n'.join(fresh),
                                                self.identity, self.identity)

    def test_process_reuse_and_unscoped_lines_refuse(self):
        old = self.line(1, 'old')
        for identity in (('1234', '5679'), ('4321', '5678'), None):
            with self.subTest(identity=identity), self.assertRaises(packet.Refused):
                packet.merge_diagnostic_history(old, old, self.identity, identity)
        for extra in ('foreign text', old.replace('1234', '4321'), '--------- truncated'):
            with self.subTest(extra=extra), self.assertRaises(packet.Refused):
                packet.merge_diagnostic_history('', extra, None, self.identity)

    def test_first_read_and_history_growth_remain_bounded(self):
        self.assertEqual(packet.merge_diagnostic_history('', '--------- beginning of main\n'
                                                         + self.line(1, 'first'), None, self.identity),
                         self.line(1, 'first'))
        for oversized in ('x' * (1024 * 1024 + 1), '\n'.join(self.line(i, str(i)) for i in range(2049))):
            with self.assertRaises(packet.Refused):
                packet.merge_diagnostic_history('', oversized, None, self.identity)

    def test_prepared_three_stop_byte_bound_cannot_be_raised_by_command(self):
        with tempfile.TemporaryDirectory() as root:
            fixture = Fixture(root)
            driver = packet.Driver(root, fixture.budget, fixture.invoke,
                                   lambda: fixture.now, fixture.sleep, byte_limit=576 * 1024**2)
            self.assertEqual(driver.receipt['byte_limit'], 576 * 1024**2)
            driver.received = driver.byte_limit
            with self.assertRaises(packet.Refused):
                driver.call(['shell', packet.FOCUS_QUERY])
            self.assertTrue(fixture.budget.forced_cleanup)


class ActiveBoundEntryTests(unittest.TestCase):
    def fixture(self):
        binding = dict(game_id='a' * 32, profile_id='b' * 32)
        entry = dict(binding, paused=False, stopped=False, restart_required=False,
                     runtime_incomplete=True)
        return binding, entry, dict(selected=binding['game_id'], entries=[entry])

    def test_live_writer_intent_is_separate_from_a_previous_incomplete_runtime(self):
        binding, entry, collection = self.fixture()
        self.assertIs(packet.active_bound_entry(collection, binding), entry)

    def test_each_stop_pause_error_or_missing_flag_refuses(self):
        for key in ('paused', 'stopped', 'restart_required'):
            for value in (True, None, 0):
                binding, entry, collection = self.fixture()
                entry[key] = value
                with self.subTest(key=key, value=value), self.assertRaises(packet.Refused):
                    packet.active_bound_entry(collection, binding)

    def test_selection_profile_duplicates_and_invalid_identity_refuse(self):
        for change in ('selected', 'profile', 'duplicate', 'invalid'):
            binding, entry, collection = self.fixture()
            if change == 'selected':
                collection['selected'] = 'c' * 32
            elif change == 'profile':
                entry['profile_id'] = 'c' * 32
            elif change == 'duplicate':
                collection['entries'].append(dict(entry))
            else:
                binding['game_id'] = 'invalid'
            with self.subTest(change=change), self.assertRaises(packet.Refused):
                packet.active_bound_entry(collection, binding)


class InspectedTextTests(unittest.TestCase):
    colours = [[255, 255, 139], [254, 254, 139]]
    bounds = [0, 0, 32, 32]

    def plane(self, background):
        pixels = [tuple(background)] * 1024
        for y in range(5, 15):
            for x in range(6, 16):
                pixels[y * 32 + x] = tuple(self.colours[(x + y) % 2])
        return bytearray(v for pixel in pixels for v in pixel)

    def matches(self, reference, current):
        return packet.matching_inspected_text((32, 32), reference, (32, 32), current,
                                               self.bounds, self.colours)

    def test_moving_background_is_excluded_but_complete_control_match_stays_exact(self):
        reference, current = self.plane([0, 50, 150]), self.plane([80, 90, 100])
        self.assertTrue(self.matches(reference, current))
        self.assertFalse(packet.matching_inspected_region((32, 32), reference,
                                                          (32, 32), current, self.bounds))

    def test_changed_foreground_colour_shape_missing_or_uniform_text_refuses(self):
        reference = self.plane([0, 50, 150])
        changed = bytearray(reference)
        offset = (5 * 32 + 6) * 3
        changed[offset:offset+3] = bytes(self.colours[0])
        self.assertFalse(self.matches(reference, changed))
        changed[offset:offset+3] = bytes([0, 50, 150])
        self.assertFalse(self.matches(reference, changed))
        self.assertFalse(self.matches(reference, bytes([0, 50, 150]) * 1024))
        self.assertFalse(self.matches(bytes(self.colours[0]) * 1024, bytes(self.colours[0]) * 1024))

    def test_palette_geometry_and_plane_bounds_are_checked(self):
        plane = self.plane([0, 50, 150])
        for palette in ([], [[True, 0, 0]], [[256, 0, 0]], [[1, 2]], [[1, 2, 3]] * 257):
            with self.subTest(palette=palette[:2]):
                self.assertFalse(packet.matching_inspected_text((32, 32), plane,
                                 (32, 32), plane, self.bounds, palette))
        self.assertFalse(packet.matching_inspected_text((32, 32), plane,
                         (16, 64), plane, self.bounds, self.colours))
        self.assertFalse(self.matches(plane, plane[:-1]))


if __name__ == "__main__":
    unittest.main()
