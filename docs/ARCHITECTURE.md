# ntx architecture contracts

Binding authority: [NTX-5](https://github.com/WigglyDon/ntx/issues/5) and
[amendment6505289270](https://github.com/WigglyDon/ntx/issues/5#issuecomment-6505289270).
These are required current contracts. Active identity, schema-2 routing and
offline staged import are implemented. A renamed GameCube package is built and
preserved; Pixel4 app-side data migration is verified with the original IDs and
ordinary bytes retained. Document regrant and a short current-package GameCube
boundary are verified. The pinned N64 core compiles
and the Rust execution/Android resource/save/scene implementation is in source;
A combined package is linked/audited; device acceptance remains unfinished. The preserved GameCube baseline is documented in
[historical architecture](history/GCX_ARCHITECTURE.md) and native/README.md.

## Three independent axes

| Axis | Owner and examples |
| --- | --- |
| Host platform/device | Window/surface, audio delivery, scoped media access, lifecycle, launch integration; Android now, other host adapters later |
| Emulated system | Media/controller/save semantics: GameCube and Nintendo64 |
| Execution backend | Console execution and machine format: direct Dolphin and Mupen64Plus-Next; possible FN64 N64 adapter later |

Portable Rust owns collection, durable identities, ordinary-save policy, resume
intent, input semantics and scheduling. Host and backend adapters provide explicit
capabilities. Keep Android content grants, PIDs, native instances, GPU/JIT resources
and process slots out of portable identity. Reacquire host media access without
creating a different game. Do not bind the universal model to Android or one GPU
API; platform agnosticism does not claim that all ports already work.

The portable collection validates opaque media locators without requiring an
Android URI scheme. A verified relocation can replace a host binding while
retaining IDs, profiles and scene references. Android's adapter accepts only its
scoped content documents and requires its own grant; accepting a locator in the
model supplies no access capability or implemented desktop/other-host port.

## Independent per-game continuity

- GameId: durable game/media identity, independent of host and backend.
- SaveProfile: explicitly owned ordinary progress; exactly one writer through
  native teardown and durable metadata publication.
- SavedSession: that game's continuation, pause intent, selected media and
  checkpoint compatibility; outlives its runtime.
- LiveRuntime: temporary execution/resources assigned to a session.

Many independent games can be open on the same or different systems. Executing,
resident-paused, checkpointed/unloaded and recovery-pending are session states or
placements. One foreground slot is the first scheduling policy, not a permanent
singleton architecture or one session per console. Later bounded slots/process
isolation must not redefine IDs or place progress in console-global writable
storage. Native globals may require process isolation; no unqualified double
instantiation. Background pause/silence is the default, with no fake keepalive.

The portable scheduling module decides pause/play for the first foreground slot
from its bound GameId, selected session, resume/recovery state and reported host
availability. Android supplies foreground/window/player visibility and applies
the decision to either backend. A different selected game cannot execute the old
runtime. Portable frame deadlines own N64 pacing; Android supplies event waits.
Deadlines retain phase through ordinary wake-up jitter and rebase after a whole
period is missed, without a catch-up burst. Resume resets phase. Bounded runtime
diagnostics retain only aggregates for the first4096 elapsed host observations:
active dispatch intervals/lateness, core calls, presentation/swap and outer-stack
audio delivery. Core duration includes its presentation callbacks; callback and
dispatch counts need not match. Pause gaps are excluded from active intervals.
These observations include startup and are not display-frame, audible-output or
measured speed claims. No per-frame logs or game data are collected by them.
The narrower core shader probe passes through compile/link and status-query
addresses resolved by the existing host ABI. It changes no GL state/error flags,
core options or checkpoint fingerprints. Each slow core call is correlated with
its own shader-call elapsed time; unrelated maxima are never subtracted. The
probe follows the non-Send runtime owner thread and retires after native join.
Neither decision embeds Android handles or changes durable identities.

## Ordinary progress and machine state

Keep immutable media, ordinary game-owned saves, machine checkpoints/consistency
metadata and host preferences separate. A layout edit must not change save identity.
N64 cartridge/Controller Pak storage and GameCube card/GCI persistence use their
actual formats and backend owners, not a universal raw-card assumption.

Inspect each selected core's capture/restore boundaries: persistent bytes, caches,
device protocol state and pending writes. Capture/publish a coherent generation.
Before restore validate its relationship to actual current ordinary files. A stale
or partial restore must not flush invalid bytes over newer progress during cleanup.
Do not blindly attach newer saves to an older machine or rewrite fingerprints to
force a load. Retain incompatible checkpoints/errors and offer explicit recovery
or cold start from current saves. Ordinary conversion and exact machine-state
compatibility are separate; neither cross-backend nor cross-architecture state
compatibility is promised.

## Two real adapters

Retain the inspected direct Dolphin GameCube native adapter and its patched source,
resources, threads, render/audio/input/card/state boundaries and provenance. New
first-party ownership stays Rust; useful C/C++ remains. Use existing emulated boot
and DSP-HLE. No BIOS campaign, Dolphin-to-libretro conversion or whole-core rewrite.

For N64 pin/review Mupen64Plus-Next and dependencies, starting with its Android
ARM64 GLES3/GLideN64 route behind a small Rust libretro host. Use core/API, not
RetroArch UI. Own the hardware context lifecycle, core options/environment,
scoped content, audio, N64 input and save publication at actual boundaries. Do not
force it into Dolphin Vulkan or run emulation on the UI thread. Pause/quiesce,
flush, capture/restore and stop/join acknowledge actual completion or failure.

Collection::decode explicitly upgrades schema-1 entries to GameCube/Dolphin in
schema 2 while retaining IDs and progress. The unsafe direct adapter declarations
now live under backend::dolphin; N64 cartridge/input/save-format policy lives in
n64. The small backend::mupen adapter uses a qualified process slot, callback ABI
and instance-owned backend directories. Android supplies EGL3/offscreen drawing,
surface replacement, AudioTrack delivery and measured system-correct touch input.
Mupen's libco callbacks can run on an alternate stack on that same owner thread.
The audio callback only copies PCM into an instance-owned bounded Rust queue;
Android drains it through AudioTrack JNI after the core returns to the host stack.
Pause clears queued samples before silencing output. This repairs the observed
managed-runtime boundary; actual device qualification remains required.
N64 ordinary generations and checkpoints have their own Rust owners and do not
reuse Dolphin cards or machine containers. Exact native/options/architecture/
media/profile/ordinary bindings are checked; failed restores cannot publish native
ordinary memory. State jobs yield at completion before further guest execution.
A failed native join retains code/context/host and profile writer ownership.
Linked/device results remain separately required. A future
FN64 adapter can use the same N64 entries with validated ordinary-save conversion;
it does not inherit Mupen checkpoint compatibility. Do not alter FN64 itself.

## Rename and Android migration

Canonical package/JNI is com.machinerhythm.ntx, root ntx and link scheme
machinerhythm-ntx. Rename active crates, modules, exports, native libraries, build
inputs/artifacts, labels and current docs comprehensively. Isolate legacy readers
and fixtures; preserve vendor names, authorship, historical hashes and records.

New Android package identity does not inherit old app-private files or URI grants,
even with the same signer. Preserve the current source independently, stage/validate
imports, publish atomically, make retries idempotent and reject overwriting newer
target progress. Preserve GameIds/SaveProfileIds, preferences, ordinary files and
complete generations. Relink media without ROM duplication or new IDs. Old links
can parse compatibly; package-targeted pinned icons may require re-pinning. Retire
legacy installations only after verified migration/rollback under actual custody.

## Verification scope

Use affected-boundary fixtures and a few representative journeys: two same-system
games remain independent; cross-system switching restores the correct session;
process loss preserves durable continuation; stale/failed restore cannot damage
newer ordinary saves. Qualify supported residency separately from checkpoint
fallback. Do not block the N64 vertical slice on a process pool, all-platform
campaign or quest automation. The current owner assignment and handoff order
identity/migration, first playable N64, then supported continuity/alpha comfort.

### Layout editing and picture preferences

`layout_edit::Draft` owns bounded undo and geometric gestures for either console.
Console adapters retain their own targets and isolated test pads. A game-field
selection has the same move/resize, collision, reset, undo and cancellation
contracts as a control selection. Group transforms preserve relative placement;
rejected edits never move other controls. Android owns Canvas/Bitmap/window
resources; a paused frame is ephemeral, scoped to the current activity/menu/runtime
and never enters game persistence. The tools dock overlays the play-size surface.

Optional `picture` rectangles in each system's per-display controls profile are
normalized against the full drawable, independently of control-safe margins.
Both backend presentation paths fit their aspect inside that box. The default
maximizes the complete picture before placing controls in unused space;
keep-clear fitting around controls remains an explicit alternative. Controls,
ordinary saves and machine checkpoints remain separate. Existing version1
profiles without a policy keep their earlier behavior and are never rewritten
at load. GameCube saved-picture admission uses the same buffer-adjusted aspect
as presentation, including coherent drawable rounding.


The owner clarified the family-wide default on2026-11-03: maximize the complete
native-aspect picture first, then place controls in unused space without overlap.
`PicturePolicy::GameFirst` implements that contract. Preferred console controls
are fitted into actual free side/bottom regions; a compact fallback stacks whole
semantic groups rather than flattening their directional/face relationships.
The immutable presented GameCube geometry records the actual backend aspect;
changing it cancels obsolete captures. N64 uses the same portable placement policy
through its own typed targets. The editor rebases from these runtime adapters at
actual frame/density/origin, instead of inventing a second layout.

Old version1 profiles without a policy are isolated compatibility reads of their
previous keep-clear behavior. New profiles persist explicit policy. Keep-clear
picture fitting and overlap are user choices, and geometry with no usable spare
space opens the existing editor with Save disabled until a valid choice. Native
video delivery waits through missing measured geometry instead of treating a
transient host measurement as a console execution failure. None of these host
preferences changes a GameId, SaveProfile or checkpoint fingerprint.

The portable geometry includes portrait fixtures; the current Android activity
requests landscape orientation. Those fixtures do not establish a portrait
Android player or device qualification.
