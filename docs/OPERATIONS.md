# ntx operations

Continue this checkout under NTX-5/amendment6505289270 and AGENTS.md. Preserve
unknown edits and the ignored retained source/progress/signing/recovery set.
The local supplement is located by docs/NTX_HANDOFF.md. Resolve relocated paths
from this checkout; never revive retired storage or create an old-path alias.
Public source commits/pushes are authorized. No public binary/store release,
paid service, sibling mutation or second implementation worker is authorized.

Before downloads, substantial builds, archives or backups, inspect current host
custodian policy, filesystem capacity/allocation/metadata reserve, competing jobs,
memory and expected peak growth. A small check grants no large build reservation.
Use one native build job, existing shared tools and bounded project-owned outputs.
No global cleanup, peer preemption, cache purge, filesystem repair or automatic
heavy build. Keep old cached outputs as historical until exact inactive output
cleanup is separately justified. New CMake outputs are build/ntx-host and
build/ntx-android; package outputs are build/ntx-android-package.

Use focused host tests and warnings-denied relevant lints for changed boundaries.
`cargo test --offline --locked -p ntx -p xtask` checks portable policy and build
helpers. When using the retained offline vendor set, add
`--config .local/vendor-config.toml`. `cargo xtask source-check` verifies the
actual pinned Dolphin tree and patch. Use newly compiled xtask after relocation.
Do not replay earned boot/save/scene/timing or retired quest campaigns.

`cargo build --offline --locked -p ntx --bin ntx-checkpoint-inspect` builds a
read-only GameCube continuation inspector without a native execution library.
Pass a protected offline profile copy, checkpoint ID and an independently trusted
binding JSON. It validates the complete container/current ordinary generation;
it never boots or restores a machine and supplies no phone custody. This keeps
cleanup/recovery verification independent from native boot fixtures or relocated
cached executables. N64 has its own container/ordinary owner; this tool does not
reinterpret an N64 checkpoint as a Dolphin scene.

`cargo build --offline --locked -p ntx --bin ntx-n64-checkpoint-inspect`
builds the separate native-free N64 inspector. Pass a protected local profile
copy, checkpoint ID and independently trusted N64 binding JSON. The copy must
retain its existing N64 directory and writer.lock; the inspector takes only that
copy's lock and validates the actual container and current ordinary generation.
It never boots/restores a core or publishes save data. Inspection alone proves
neither native quiescence/join nor permission to retire a device process.

Canonical Android package is com.machinerhythm.ntx, activity NtxActivity, private
root files/ntx and links machinerhythm-ntx://play/GameId. Legacy installations
remain recovery sources. New package identity confers no old private data or
document grants. See [migration](IDENTITY_MIGRATION.md); no source uninstall,
clear, checkpoint fingerprint rewrite or silent discard of unsaved progress.

No phone action follows from checkout startup or a host test. Prepare artifacts,
backups, finite work/cleanup/return and explicit verifiers off-device first. The
installed android-device skill/procedure owns admission, exclusivity, actual undo
and return. Use standing Pixel4 lab authority with normal exclusive admission;
Fold requires a fresh personal window. Device data and results are independent.
Never root, wipe, change security/launcher, touch foreign apps, preempt peers or
use a free lock/connected phone as custody. Preserve source applications and all
current profiles/generations under the held lease; verify physical return and
actual client/lease release. Failed cleanup stays pending with its original error.

The renamed scripts/android_packet.py remains a finite product test helper;
it grants no custody. Its NTX_* environment and exact package/geometry/source
bindings require a fresh prepared task for the new candidate. An explicitly
prepared image-inspection bound may be 1..90 seconds (default30) to accommodate
measured model/tool turnaround. Current focus/display/geometry checks and
single-use contacts still apply; an expired image must be captured and inspected
again. This local helper supplies neither custody nor a device-use window. Old private GCX
recipes are dormant evidence, not runnable commands or renewed authority.
Detailed [prior helper gates](history/GCX_OPERATIONS.md) remain safety evidence;
current installed procedure and actual admission govern device operations.

N64 now has its own Rust drag/resize draft and isolated input preview in the same
editor views. Its normalized per-screen placements are stored in `n64_profiles`
inside `controls.json`, separately from GameCube placements and all game data.
Save validates and atomically publishes preferences; Cancel discards the draft.
Changed layouts neutralize held input before the runtime map is replaced. The
foreground layout editor requests the same app-window screen-on flag as play;
ordinary menus clear it. This changes no device timeout, permission or service.

`ntx-motion-probe` is an optional bounded test transport for the installed scrcpy
5.0 control service, following its pinned upstream touch-message format. It
requires an inherited device-lease FD and an existing owned loopback forward;
it grants no admission or device identity. The prepared task must verify current
own focus/geometry and inspected preview targets, observe both contacts and
independent release/cancel, then retire its control process, forward and staging
file under the same lease. Sent bytes are not proof of observed input. Its
loopback-only socket regression needs a host that permits a temporary localhost
listener; no phone is involved in that test. No scrcpy service is bundled in ntx.

Keep signer/password, private device identifiers, games/firmware, saves,
checkpoints, raw logs/captures and private resource bindings out of public Git.
Publish sanitized outcomes and source/build identities. Report remaining product
and verification gaps on NTX-5 without closing acceptance issues prematurely.

Android packaging retains each exact unstripped native input, including the Rust
player and C++ runtime, under the private content-addressed `.local/symbols` tree
before stripping the APK copy. `native-libraries.tsv` binds both hashes. Rebuilds
retain earlier generations; a changed retained file or symlink refuses packaging
instead of overwriting evidence. These files are private diagnostics, not APK
payload or a public binary release.
