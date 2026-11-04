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

These N64 source limits do not claim a finished storage-management UI or a global
quota for every backend. GameCube's existing successful-retirement ordering is
retained; broader recovery budgets/free-space admission remain separate work.
Routine storage pressure must offer a clear decision while preserving current
progress, rather than silently evicting another game's unsaved scene.

Per-game bounds do not make an arbitrarily large collection free: each game with
a retained exact resume still needs space, plus temporary publication headroom.
A collection-wide budget and clear user choices for releasing resume space while
keeping ordinary saves remain unfinished. Storage pressure must never silently
discard another game's unsaved place.
