# Pinned N64 integration

Selected core: [Mupen64Plus-Next at 15bada85906593376e8fd697f1b0b3af3a20f512](https://github.com/libretro/mupen64plus-libretro-nx/tree/15bada85906593376e8fd697f1b0b3af3a20f512).
[source-lock.json](source-lock.json) records the root tree, inspected blobs and
optional gitlink. The pin has no .gitmodules; no dependency URL is invented for
its excluded gnulib gitlink. No RetroArch frontend is imported.

The ARM64 GLES3/HLE core compiles and combined Android packages are signed with
the retained identity and privately preserved with corresponding source. Rust
owns the libretro adapter, Android EGL/GLES/audio, N64 controls and independent
profile/save/scene routing. Sourceb62b842/APK67e604e9 reached actual N64 execution
after correcting the hardware-context enum, then ART aborted AudioTrack JNI on
the core's alternate stack. Current source hands copied PCM back to the host
stack before managed delivery. Its signed candidate now reaches first-room play
with observed analog movement, owner-confirmed clear sound and a cold reload of
the newly created ordinary file. The game pause transition exposed GL error0502;
the host now diagnoses flags already present at the core handoff separately from
its framebuffer-copy/control drawing errors. Installed source1dc7db2/APK1cb108de
now passes the affected in-game pause/Save route, normal joined Stop, verified
full-process ordinary-save cold reload directly into the room, and analog movement.
One preexisting backend GL0502 flag was observed at game pause while host
presentation continued; its renderer cause is not claimed fixed. Independent
GameCube profiles were unchanged and actual return passed. Later focused checks
verified sequential native-unloaded scene return, fresh-process N64 and GameCube
scenes, and GameCube/N64/GameCube switching with independent ordinary saves.
Installed source87f3641/APK97930edd passes N64 layout drag/resize, Save/Cancel
and fresh-process native mapping, plus the shared GameCube editor boundary.
Actual-target isolated stick+A input, independent A release and cancellation to
neutral are observed; current ordinary data and actual return were verified. Concurrent native instances, broader
compatibility and performance are not inferred.

## Source and reproduction

All 2275 original files were checked against upstream Git blobs. The original
archive's single Windows project CRLF expansion was normalized to the exact Git
blob; source-files.sha256 records that original content. Maintained patches and
patched-files.sha256 separately record checked buffers/results, optional logger
fallback, owned directories, completion-bearing shutdown and coherent state-job
yields. Modified files retain dated change notices and upstream terms.

`cargo xtask mupen-source-check` verifies the complete active graph.
`python3 native/mupen/check_boundary.py` compiles the actual selected header,
state entrypoints and deinit against bounded fixtures, and reproduces both patches
from the retained original archive. It checks ABI layouts, failed results, buffer
admission, completed-job yields, unopened-core shutdown and failed-join retention.
These checks do not execute a console or GPU.

`cargo xtask build-mupen` uses reviewed NDK30, API28, arm64-v8a, GLES3 and one job,
with isolated build/ntx-mupen outputs and 16 KiB ELF alignment. The expanded graph
has [274 compilation inputs](selected-sources.txt), no parallel RSP/RDP, Angrylion
or gnulib, and no M64P_PARALLEL asynchronous save worker. Upstream Android defaults
are overridden explicitly; the selected copy workqueue executes synchronously.
Candidate preservation retains original and complete patched Mupen source,
first-party/Dolphin/dependency bundles, Rust vendor inputs and exact tool/build
bindings. The unsigned source-rebuild route needs no private signer.

## Actual backend boundaries

| Boundary | Implemented ownership and limit |
| --- | --- |
| Media | need_fullpath=false. Scoped cartridge bytes use path=NULL, preventing optional .ndd/.gb/.sav sibling probes. Rust validates bounded z64/v64/n64 magic and fingerprints canonical big-endian bytes without changing GameId or originals. |
| Ordinary storage | RETRO_MEMORY_SAVE_RAM is 296960 bytes: EEPROM2048, four32768-byte Controller Paks, SRAM32768 and FlashRAM131072. Rust owns immutable image generations and an atomic ordinary reference under the profile writer. This is not a universal external native-save converter. |
| Initialization | The core formats saved_memory at load. Rust injects the current image after formatting and before execution; a pending restore reseeds that exact bound generation after cold initialization and validates again. |
| Capture/restore | Selected state includes device protocol and FlashRAM page-buffer/mode/status. Completed jobs yield after clearing the job and before further guest execution. Capture commits ordinary bytes from that boundary before publishing a scene. Restore requires exact core/options/architecture/media/game/profile and actual current ordinary generation; failed mutation is quarantined from publication. |
| Graphics/audio | Android owns a retained EGL3 context, offscreen framebuffer, aspect composition, replacement window surfaces and PCM stereo AudioTrack. The core's threaded renderer is explicitly disabled; callbacks and core calls stay on the native owner thread. Libco callbacks may use an alternate stack: PCM is copied into a bounded Rust queue and AudioTrack JNI runs only after returning to the outer host stack. Android handles never enter durable game identity. |
| Input | One analog stick, digital C-up/down/left/right, independent digital L/R and Z, D-pad, A/B/Start. Default core mapping is fixed explicitly; pointer capture, quick presses, simultaneous input and cancellation are Rust policy. No GameCube C-stick/analog-shoulder or PS1 mask transplant. |
| Shutdown | An unopened coroutine is never started just to stop it. Native stopped state is acknowledged before library/context release. Failed join/publication retains native memory, host resources and writer ownership; successful teardown keeps the writer through durable session publication. |
| Scheduling | Multiple independent SavedSessions/profiles share one foreground runtime policy. Native globals permit one Mupen instance per process until isolation is qualified; that is a runtime capability, not a one-game identity model. Background pause/silence is the default. |

The core and frontend share a GL error flag, which is not cleared by state unbind.
The host drains and bounds diagnostics for preexisting invalid-command flags before
its own GL work, then checks framebuffer copy and overlay separately. Memory/context
failures still stop presentation. This follows the actual
[GLES error semantics](https://registry.khronos.org/OpenGL-Refpages/es3.0/html/glGetError.xhtml);
it does not prove that a backend-rendered frame is correct or qualify a game menu.

## Terms and resources

[RESOURCE_AUDIT.md](RESOURCE_AUDIT.md) records selected components and actual
resource provenance. In particular, GLideNHQ xBRZ requires GPLv3; its MAME exception
does not apply to ntx. GPLv3 text, component licenses, source-header notices,
change notices and exact corresponding source accompany the combined candidate.
Original first-party Rust remains MIT and borrowed files retain their terms.
No cartridge, firmware, save, texture pack, copied branding or private device
content is bundled. Public binary/store/legal commitments remain separately gated.
