# Bounded resume storage

Exact resume stores machine state separately from ordinary in-game progress.
Its cost follows the emulated machine, not the cartridge's size. A game owns its
resume independently of other games and execution slots; an unloaded runtime
still has its durable game/session identity.

The N64 writer previously published a new immutable file for each distinct pause
without retiring its superseded successful generation. Source now applies the
same acknowledgement ordering as the direct GameCube adapter: publish and verify
the complete generation, durably publish the collection reference, then retire
only the known acknowledged predecessor while still owning the profile writer.
Explicit Stop retires its acknowledged scene only after native join and durable
Stop metadata. Normal successful replacement therefore leaves one current scene,
with a temporary overlap during publication.

Rejected, incomplete, incompatible and unacknowledged candidates are preserved.
N64 admits at most three files in its scene directory, including pending files,
and a bounded total derived from its64MiB machine-payload ceiling. If recovery
files consume that budget, another capture is refused rather than deleting an
unknown candidate. Preexisting histories require explicit preservation/disposition;
the application does not guess that every unreferenced file is disposable.

Before writing a new N64 scene, portable Rust policy requires the actual payload,
bounded header, ordinary-image allowance and64MiB free-space reserve. Android
supplies its `statvfs` observation; portable identity/policy owns no Android handle.
The observation is not a reservation. Atomic-write and metadata-sync errors still
retain the paused runtime and all potentially referenced generations.

Ordinary progress has its own authority. It is committed independently even when
a full machine resume cannot fit. A retained scene pins its referenced ordinary
image; current ordinary data always stays protected. Only previous ordinary images
that this writer actually observed acknowledged, and that no retained scene pins,
are retired. Unknown files from an uncertain commit or earlier process stay.
Immutable scene/image publication exclusively creates its pending file, so a retry
also refuses to overwrite an existing incomplete attempt with the same identity.
A separate sixteen-image recovery ceiling prevents uncertain ordinary history
from growing indefinitely; refusal retains the last committed image and paused
runtime. No checkpoint fingerprint is rewritten, and newer saves are never
silently attached to an older machine. A retained old scene may require an explicit
cold start when ordinary progress has advanced.

Host tests exercise forty replacements with changing ordinary data, retained
recovery candidates, identical-generation reuse, missing/damaged replacements,
ordinary-image pinning, incomplete candidates, insufficient space and hard budgets.
Same-identity retry tests also preserve incomplete scene and ordinary-image
bytes exactly. Tests distinguish acknowledged retirement from broad directory cleanup. Target
verification on Pixel4 observes four distinct16MiB title publications retaining
one current scene each; normal joined Stop retires the accepted scene. Protected
ordinary progress stays unchanged. Installed-build bounds remain in [state](state.toml).

GameCube also bounds new scene writes to three
directory files including incomplete/unknown candidates, with a total budget
derived from its existing256MiB payload ceiling. It checks the captured payload,
bounded header and64MiB free-space reserve after the native card/SRAM flush.
The Android platform adapter supplies filesystem availability for both backends;
portable Rust owns each backend's admission policy. A verified identical complete
GameCube scene needs no new write and can be reused even under storage pressure.
Existing oversized histories are retained and block new writes, never swept.
The initial budget change retained GameCube recovery handling. The subsequent
source continuation below separates verified checkpoint-only refusal from native
or ordinary-data failure.
Routine storage pressure must offer a clear decision while preserving current
progress, rather than silently evicting another game's unsaved scene.

Portable Rust now additionally admits new machine files against a shared1GiB
collection budget. It inventories both scene directories under every retained
profile, including orphaned profiles and incomplete, legacy or unknown regular
files. Logical file lengths count conservatively; media, ordinary saves and host
preferences remain separate. Existing over-budget histories stay intact and
block new writes. A bounded inventory refuses unsafe entries and symlinks instead
of following them or guessing they are disposable.

Both backend publication boundaries receive the actual host free-space observation
and the remaining collection allowance separately, after ordinary progress commits.
A new bounded container must fit both budgets, including overlap with its old
scene. Verified identical complete files need no new allocation and remain reusable
when the quota is full. Per-profile file/byte bounds and the64MiB filesystem reserve
still apply. These observations are not reservations: the current foreground owner
serializes capture/publication; future simultaneous writers need reservations at
this boundary without redefining game or save identity.

The library's Resume storage page shows aggregate and per-game machine usage.
Closing an unloaded stored place requires a separate explicit confirmation that
its unsaved position will be lost while its game and current in-game saves stay.
The confirmation names the exact game/profile/media/scene and is invalidated by
a changed reference or recovery state. Rust acquires the profile writer, validates
the complete retained container and ownership, then durably clears its session
reference before unlinking only that file. An uncertain metadata write retains
the file and restores the prior in-memory reference; cleanup failure after durable
metadata reports retained recovery bytes without rolling the metadata back.
Live games use normal Stop/native join. Quarantined, incomplete, damaged and
unknown candidates remain for review; this page is not a general recovery-data
purge. Closing an incompatible old place does not load its core, attach newer
saves to it, or modify ordinary progress. Storage pressure never silently discards
another game's unsaved place.

Checkpoint storage refusal is now classified separately from native execution,
ordinary-save and uncertain collection-publication failure. After successful
native capture and an independently verified ordinary commit, a checkpoint-only
space/query/write error retains the healthy paused runtime and offers Resume.
The warning says that the current place is in memory and requires keeping the
app open; Resume then Pause retries storage. It never promises process-death
recovery without a valid durable scene. Changed ordinary data, failed capture,
pending restore and uncertain collection publication retain recovery handling.
Warnings are scoped to the exact live game/runtime and never clear an existing
quarantine. Portable tests and the focused N64 target warning/Resume boundary now pass:
two quota refusals retain same-process Resume, native Stop/join precedes exact
fixture removal, and subsequent normal publication/Stop succeeds. Nine new
captures were reviewed, with current ordinary data and independent profiles kept.
This adds no gameplay/save/audio/performance qualification.

GameCube retirement now also verifies a bounded regular file, recognized current
or legacy container magic and its complete content identity before deletion.
Damaged files and symlinks remain for recovery. Healthy and legacy successful
retirement and missing-file retries retain their existing behavior. This portable
file-boundary repair has host regression coverage; it adds no native-execution
or historical lifecycle qualification.

GameCube scene publication also exclusively creates incomplete files, using
borrowed payload segments so no second full machine image is allocated. A
verified identical complete generation is reused without rewriting it; an
incomplete same-identity attempt stays untouched. The portable suite now has
133 passing library tests at that change. Subsequent recovery/space admission
coverage brings the suite to137: forty acknowledged GameCube replacements stay
at one file; low space, excessive recovery bytes/files and unsafe entries refuse
without changing ordinary data or existing scenes. These changes are source-verified and separate
from the earlier prepared target candidate.

The subsequent GameCube continuation now uses the shared healthy-RAM warning
only after the native callback supplies a complete capture, after its successful
card/SRAM flush. The exact committed ordinary generation must verify both before
publication and after any scene error. Native recovery flags, absent callbacks,
invalid payload/binding, changed saves and disagreement between native and writer
acknowledgements require recovery. Collection-reference publication remains a
separate fatal boundary. No accepted scene reference is changed by a refused
write. The shared GameId/runtime-scoped UI can offer Resume while retaining
writer ownership and all prior recovery files; RAM is not process-death recovery.
140 library tests and relevant host/ARM64 lints pass. Installed6829 now passes
the affected GameCube boundary: two quota refusals keep same-process Resume,
normal native join/process absence precedes exact fixture removal, and a fresh
title publishes normally before Stop retires it. Eight new captures were reviewed;
cards and independent ordinary/profile data stayed protected, with normal native
SRAM updates retained and actual return verified. No gameplay/save/restore/timing
campaign or historical-cure qualification follows.

The collection quota and storage-page source has affected filesystem fixtures for
cross-system/orphan totals, sparse oversized histories, publication headroom,
unsafe entries, identical reuse, both ordinary-verified storage refusals and
explicit close. Tests preserve newer ordinary progress, identities, errors and
unknown candidates; live locks, stale confirmations, damaged/symlinked containers,
uncertain metadata and failed retirement retain their safe boundaries. Host and
ARM64 warnings-denied Rust lints and151 library tests pass. The affected standalone
N64 inspector fixture passes; existing loopback-only transport tests pass on the
host after the sandbox refused localhost binding. The corrected sourcea78415a
package1bb628aa is signed/checked and privately preserved with exact source and
raw native inputs. Its affected Pixel4 UI now passes: Library with no runtime
shows16.0MiB; Keep retains the exact place/reference/ordinary bytes, then confirmed
Close clears its durable reference before retiring only that file. The page shows
0.0MiB and both tested profiles end with zero scenes. All15new images were reviewed;
cards, independent progress/preferences/source/IDs and actual return verify. Both
publication adapters ran; the short GC image was black with controls, so no new
visible-game qualification follows. Global saturation remains host-fixture verified;
no actual disk fill or gameplay/Save/restore/performance campaign occurred.

An additional cross-system fixture holds the foreground N64 profile writer through
quota refusal, explicitly closes another game's unloaded GameCube place and then
admits the N64 retry. The held writer, selected identity, live session metadata,
current ordinary generation and unknown sparse recovery candidate remain intact.
This exercises release while another game is resident without instantiating two
native cores. The prepared package's production code is unchanged by this test.
