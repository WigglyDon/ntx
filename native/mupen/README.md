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
yields. The mechanical shader-dispatch patch routes four GL calls through the
existing host resolver, retaining their original direct fallback. The selected
GLES wrappers otherwise import these base calls directly, bypassing host probes.
All measurement/correlation policy remains Rust. Modified files retain dated
change notices and upstream terms.

The cache-directory bridge uses the existing host save-directory callback for
derived renderer caches. Rust supplies each profile's `N64/backend` directory;
system inputs and ordinary committed images remain separate. The host requests
shader storage explicitly for this selected GLES3 pin and checks its bounded
v5 key/v0x3B binary envelopes before native use, in both `backend/shaders`
and the native loader’s fallback directly in `backend`. Each location has its own
fresh recovery directory for malformed derivatives, preserving duplicate names. GPU/option checks and binary loading remain native;
options and core fingerprints change honestly, without reidentifying games or
forcing older machine scenes to load. One Pixel4 same-build title pair verifies cache creation/warm admission and
reduces slow core calls above40ms from24 to7 in the first4096 window. Seven
residual stalls remain; driver warmth/preloading and whole-game limits are
recorded in [current state](../../docs/state.toml).

`cargo xtask mupen-source-check` verifies the complete active graph.
`python3 native/mupen/check_boundary.py` compiles the actual selected header,
state entrypoints and deinit against bounded fixtures, and reproduces all patches
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

The optional compiler-observer bridge wraps the existing `new_recompile_block`
entry while retaining its algorithm, arguments and results. A
nullable callback reports only begin/end; it exports no guest addresses or bytes.
Rust owns first4096-call correlation and a131072-event cap, rejects incomplete or
nested scopes, and unregisters between native calls after its bound. Timing is
elapsed host time, not proof of CPU/GPU attribution or a compiler remedy. The
actual wrapper/registrar fixture checks returned results and callback retirement.
Pass/cache notifications are separately declared; removing only those statements
reproduces the original compiler and cache bodies byte for byte. Cache time can
overlap link/expiration and must not be summed with enclosing pass time.
The [Arm cache-maintenance explanation](https://developer.arm.com/community/arm-community-blogs/b/architectures-and-processors-blog/posts/caches-self-modifying-code-implementing-clear-cache)
is reviewed context; the selected maintenance instructions, barriers and A53
workaround remain unchanged pending actual subphase evidence.

The cache-batch repair completes a trailing dirty-page run even when it reaches
bit31 of its128KiB bookkeeping group. Previously that run could be marked clean
without calling the existing maintenance primitive. Unsigned bitmap shifts also
avoid the sign-bit undefined behavior. `check_cache_batch.py` compiles the actual
batcher and checks all64 single-bit positions across two groups, complete tails,
disjoint masks and empty masks before flags clear, in normal and UBSan trap mode.
The primitive's instructions/barriers/A53 workaround are retained. A bounded
completion counter can establish whether the repaired path executes on target;
no speed benefit or historical-incident cure is presumed from the fixture.

The register-culling repair replaces two repeated membership-only searches of
immutable entry maps with a bounded two-word membership mask. It retains the
original excluded-register rule, signed-byte range and final caller loop index;
all downstream compiler operations remain unchanged. The actual helper is checked
against the retained getter for every signed-byte code/slot, duplicates, selected
and in-range exclusions, changing maps and aliasing, with normal/UBSan-trap runs.
The full compiler-body comparison permits only declared culling substitutions
and observation statements. The first immutable-map change preserves the actual
title route but shows no material timing gain; it is not a measured hitch cure.

The live-query count patch targets the remaining repeated getter calls. Counts
start uninitialized and are evaluated only at the original query point. Every
relevant map erase updates an initialized count, including when a free-register
query gains a match; duplicate and excluded slots retain their original meaning.
Actual helpers are checked against the unchanged getter through over1million
changing-map steps per mode, with pre-query erases, short-circuit lookup and
negative/free/duplicate/excluded values in normal and UBSan trap mode. Compiler
remainder comparison checks the declared call sites and every map erase; no
compiler yield or persistence format is changed. Its bounded title comparison
reduces culling maximum41.056ms to28.821ms; five compiler calls still exceed40ms.
This is a targeted improvement, not a whole-game hitch cure.

The incoming-branch index targets the separately measured allocation pass. It
indexes immutable decoded destinations once per block in two4096-entry arrays
(32KiB), then reads provisional and allocated register-analysis values live.
Forward branches retain the original provisional reset-to-one rule; other queries
retain their original cutoff and AND behavior. The actual helpers match all four
original searches through283350 changing-value queries per mode, including
shrinking/growing blocks, empty/dense destinations and address wraparound, in
normal and UBSan trap runs. Exact comparisons cover the provisional function and
the remainder of the compiler. No new yield, block limit or persistence format;
target comparison shows no material title-window allocation gain (31us mean,
19.779ms maximum versus31us/19.600ms). The exact-output/linear analysis bound
remains; no hitch cure is advertised.

The debug bridge passes its already-formatted message to the optional varargs
logger using a literal `%s` format. Percent directives inside the payload remain
text, including after bounded truncation. The actual callback fixture checks
percent-bearing and long messages plus an absent logger, and rejects the original
bridge before executing unsafe formatting. Normal/UBSan trap checks cover this
mechanical ABI repair; no severity policy or performance change is claimed.


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
