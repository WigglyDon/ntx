> Historical source-selection research, retained at its original scope. Current work follows AGENTS.md, NTX-5 and docs/ARCHITECTURE.md. Legacy product names here describe that research.

# Selective PSX transfer, not a merger

Behavioral reference: Don reports that PSX's PS1 player resumes independent games
across closures, fresh processes and folds and feels good in ordinary play.
Historical public source binding: `WigglyDon/psx` at `1e4ba86fa0a5fc1470837b6bcc06bd20b6642c11`.
This is a retained reference, not a claim to be its latest branch head or to prove
every source path was tested. Re-read current public files before actual reuse.

## Landscape adaptation, 2026-10-22

Owner requested the public PSX layout as gcx's clean default landscape reference.
Reviewed public revision `4277d4ca651a4c9c2e8e88113a0ef092e330f01c`, MIT LICENSE
(Machine Rhythm contributors, 2026), and `crates/player/src/input.rs`, blob
`c3d90f7ca21e1a18b1f945cef999287af054ec9c`. Relevant owners: `side_rails`, the
density-independent control metrics, `ps2_camera_rails` spacing, `PlayerViewport`
and high, centered clear-picture placement. Also inspected the public Android
platform's fullscreen/inset handling; it is not transplanted into gcx.

`crates/gcx/src/touch.rs` adapts the side-rail arrangement and spacing to a bounded
GameCube layout: large green A, red B, gray X/Y and L/R, purple Z, gray Control
Stick and yellow C Stick; independent trigger travel/clicks, a captured eight-way
D-pad, Start and a separate gcx Pause action. No PS1 masks, PS2 stick clicks,
fixed PS1 aspect, saved profiles, GL or emulator/storage code is transferred.
Rust uses actual display density and the engine's reported aspect. The MIT
copyright notice is preserved in the root license and therefore APK notices.
Host tests cover non-overlap, bounds/minimum targets, all 18 reachable channels,
diagonal D-pad and simultaneous independent trigger release. The candidate now
runs in landscape on Pixel 4 as162d6b4/be16792a; its native colors/labels and clear side rails were
visually inspected, and the owner reported it works. Its system bars overlapped
the upper trigger area and bottom picture. Later52e240d/74b2ac4 passes hid bars
and aligned Pause correctly but the inset picture was rejected. Edge-to-edge
picture was subsequently accepted on58335ff/f4f60fa0. Independent physical
qualification of every control remains due.

This is selected public-source adaptation. PSX's checkout, private files, app
resources and active work were not read or changed for it.

## Full drawable picture, 2026-10-22

After rejecting the inset picture, the owner clarified edge-to-edge like PSX.
Reviewed the same public revision4277d4ca651a4c9c2e8e88113a0ef092e330f01c:
`input.rs` PlayerViewport/place_game_in/clear_picture (blob above), Android
`mod.rs` resize/picture-inset split (blob8fe79a4625b05659fb9a703d986b66124c9c6b55),
`graphics.rs` picture fit (blob87b1a704bc1b5d38502512ebb905ccb16dec5310), and
`platform.rs` fullscreen API calls (blob26342f631287aaff509b5199d87d30ce741ed607).
The public input blob was unchanged at readback. These MIT files distinguish
the full picture viewport from the gesture-safe control surface; transient bars
overlay the stable picture instead of reducing it.

`crates/gcx/src/viewport.rs` adapts the centered clear_picture calculation around
real control rectangles. It takes Dolphin's reported aspect rather than PSX's
fixed4:3 and uses the actual window/drawable mapping. Control insets never become
picture margins; no stretch, crop, private preset, GL, core or persistence code is
copied. The original MIT notice remains in LICENSE and packaged notices. New host
fixtures check full drawable height for a1.25 aspect fixture, multiple engine
aspects without overlap, stable bar-reveal picture and shared input mapping.
The original placeholder preview uses this actual Geometry. The owner accepted
the picture size on Pixel4; an existing-card cold reload passed. A system Home
handle remained across the picture centre after bar reveal. No sibling checkout
or private files were read.

The same public platform.rs blob was re-read for that artifact. Its fullscreen
owner uses BEHAVIOR_DEFAULT on API31+ to keep navigation gestures available with
hidden bars, retaining transient behavior on API30. The bounded gcx policy
candidate adopts that version guard; it does not copy PSX storage, permissions,
background services or frontend. This policy choice is not a verified cause/fix
of the artifact. Geometry, stable control clearance and picture bounds stay intact.

Candidate reusable concepts: validated opaque GameId/OpenGame requests, input
capture and cancellation, aspect-correct layout/editor behavior, atomic settings,
private-source separation, document grants, launch-intent handling, native Android
lifecycle and focused packaging checks. Check coupling and actual licenses first.
Record the source revision, file path, license, modifications and tests for each
transferred file. Preserve attribution. Do not copy private files or full history.

Do NOT inherit PS1 button masks, card sizes, BIOS bytes, libretro timing/TLS,
assumed 4:3 geometry, snapshot-size limits, firmware restore order or specific GL
state ownership. Do not import PS2 dual-analog semantics as GameCube trigger/click
semantics. xbx's hard-disk/EEPROM snapshot design is not gcx's memory-card design.

Recent PSX failure lessons to include in tests, not claims of shared code defects:
transient tiny native windows can invalidate geometry; a standard-library lock
API may compile but be unsupported on Android; a C++ adapter can hide mangled
exports from a C ABI. Verify actual platform primitives and exported symbols.

Maintain separate app IDs, signers, storage and release/source history. A small
port can reuse source without a studio-wide framework extraction. Do not modify
PSX/xbx/FN64/Kitten or share mutable cards/caches without the respective authority.
The new Master owns gcx decisions and consults actual source when transfer helps.

## Checkpoint concepts, 2026-10-22

Reviewed only the public MIT source at4277d4ca651a4c9c2e8e88113a0ef092e330f01c:
`crates/player/src/resume.rs` blob2fddaeab016e25f51770bb7bd92ea4f333071194 and
`saves.rs` blob4038b2dd4eaf1737cba0f5af071f7ef1f003add6. The reviewed ideas were
complete validated state envelopes, atomic publication and retained recovery.
No checkpoint code was copied. gcx's original Rust implementation uses the
inspected Dolphin protocol, pending events and separate card/SRAM generation.
PSX's128KiB card, RAM-restore order,64MiB snapshot ceiling, PS2 sealing and firmware
branches were rejected as coupled assumptions. MIT attribution for earlier
selected adaptations remains intact; all targeted Dolphin changes remain GPL.

## Responsiveness and UI refinement, 2026-10-23

Owner requested closer PSX quality after confirming ABI7's real checkpoint return,
and separately reported weak layout, unsmooth controls and layout hitching. The
current public input file readback still has blob
`c3d90f7ca21e1a18b1f945cef999287af054ec9c`, identical to the reviewed4277d4c
revision. MIT LICENSE readback is `b7f230132ee7f02cf3a70d97da95362132e6a6fb`.
Also reviewed that pinned public revision's `crates/player/src/ui.rs`
(`6e9e13c8570e08ed0606f7f6272fc0ac07bcccbd`) and `library.rs`
(`6433f62910ac8a20af46496aabae67e449237ee5`). No sibling checkout/private files
or running PSX app were read or changed.

Selected adaptations: following thumb centers and continuous radial dead-zone
rescaling from `StickGesture::track`, captured D-pad direction hysteresis, the
lower primary-thumb band with auxiliary keys beside it, and the compact centered
720-point menu column. GameCube keeps its octagonal gate, named18-channel packet,
separate travel/clicks, original color/label cues and no stick-click gestures.
Game entries precede auxiliary actions, with compact mixed-case native buttons;
the menu and its policy remain Rust-owned. The editor, renderer, entire frontend,
system-specific masks and private presets were not imported.

PSX's pending-press idea cannot be copied at gcx's host-tick boundary: Dolphin
runs independently. ABI8's original Rust pulse policy consumes presses only at
Dolphin's actual complete pad read through a small maintained GPL callback.
Fixed16-pointer storage, immutable shared geometry and equivalent-inset capture
retention remove avoidable allocation/cancellation work. Bounded per-runtime
timing separates Android event age, dispatch-to-pad delay and native poll cadence
from game/display behavior. Tests and future hardware results remain source-bound.

## First alpha presentation and control tools, 2026-10-24

Owner clarified that gcx is the app to polish and that these emulators are product
siblings. Read public psx at `123e831e4afb6c7cec2d8e05a030e3cd82a72bba`:
MIT LICENSE `b7f230132ee7f02cf3a70d97da95362132e6a6fb`, ui.rs
`6e9e13c8570e08ed0606f7f6272fc0ac07bcccbd`, input.rs
`c3d90f7ca21e1a18b1f945cef999287af054ec9c`, settings.rs
`a05874320ab63366e7c0e9cb14a56f9a6a4dc284`, brand README
`1cc40f6c4fac2d834ef9454a7b16ffe8edfed7fc` and icon.svg
`b3f50733cfcc8bce7a780cb5f839b9bd285ac98a`.

The green A icon adapts the public MIT icon's warm-plastic/button composition;
gcx's A path, green cap and themed vectors are drawn here. LICENSE retains the
Machine Rhythm attribution and the APK retains that notice. No console logo,
startup artwork or private asset is used. Lowercase gcx, restrained dark surfaces,
game-first rows and a centered column give the apps a family resemblance.

The source review selected editor affordances (move/resize, a separate input
preview, per-screen persistence, reset and explicit save/cancel), square-display
shoulder docking and visual feedback that does not delay logical input. gcx's
editor/settings implementation is original Rust and uses its own named GameCube
controls. No PSX editor, masks, emulation, storage, game data or entire frontend
was copied. The Java additions only forward View/Canvas/SeekBar callbacks; all
state, drawing decisions, validation and persistence remain Rust-owned.

The narrow legacy layout clipped bottom targets and overlapped C Stick targets.
A bounded shared GameCube deck replaces it; tall Fold surfaces dock shoulders in
the middle and keep the primary stick near A. Both sticks, Z, Start, eight-way
D-pad and separate L/R travel/clicks remain independent. Saved overrides address
logical targets in normalized control-safe coordinates; drawing and input share
the admitted geometry. Editing/test input never reaches the emulating machine.
These are source/host observations until the new alpha earns its device results.

## Fold refinement, 2026-10-25

After actual Fold world play exposed crowding, gcx's original Rust refinement
repackages the existing GameCube targets into a172-point deck and contains
following-stick graphics inside their touch rectangles. It builds on the already
reviewed MIT adaptation above; no additional PSX source or private presets were
copied. The engine aspect, GameCube trigger/click channels, independent pointer
capture and original input-following policy are retained. The actual shared
Geometry example now includes a Fold-size synthetic picture fixture. Host checks
are not a physical Fold layout, haptics or performance acceptance. Sibling work,
files and device settings were untouched.

## Full-size layout editing and game field, 2026-11-02

Owner requested PSX controller editing/layout quality, with equal priority for
its game field. Read the clean public MIT reference checkout at
`ac74cc06602a3e5c00b14d77404a93c94cebb6bc`: `crates/player/src/input.rs`
(blob `6a867ce3a1177d5d76a866626deabf61583cfc52`), `ui.rs`
(`3571c9916439954cccdc2e3a25e410e160e91061`), `controller_profiles.rs`
(`89cc72474835a19a3c5b4dc03f1769a7d4870a17`) and MIT LICENSE
(`b7f230132ee7f02cf3a70d97da95362132e6a6fb`). No sibling files were changed.

Selected concepts: a play-size editing surface, a movable tools dock, grouped
face/C/directional buttons, opposite-corner resize, centered grid and alignment
guides, explicit mirror, scoped reset, bounded draft undo and separate input test.
The picture is a selectable move/resize target with the same collision priority
and undo/cancel policy as controls. The clear-picture solver already adapted here
is shared by both renderers; custom picture boxes fit the actual backend aspect.
An optional normalized picture field extends each system's display preference;
old version1 preferences remain readable. No saves, profile/game identities,
snapshot formats, core input masks or PSX frontend were transferred.

The implementation is original Rust in `layout_edit`, the existing console
editors, Android Canvas/JNI UI and backend presentation adapters. A small Java
listener only forwards PixelCopy completion; Rust requests a bounded in-memory
copy of the current app's paused game rectangle, validates activity/menu/runtime
ownership and draws it inside the edited game field. A missing copy leaves a
neutral game field. Nothing is captured to storage by this player feature.
Machine Rhythm MIT attribution remains in LICENSE and packaged notices.

Focused portable fixtures check grouped movement/scaling, anchored corners,
one undo per gesture, cancellation, collision priority, spacing/mirror, snapping
escape and preference reload into both renderers across safe insets/density.
These source checks alone do not qualify the physical editor or a Fold layout.
