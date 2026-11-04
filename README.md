# ntx

**Your Nintendo games. Open, play, leave, return.**

ntx continues the same Rust-first public project and repository history formerly
named gcx. It is the Nintendo member of Machine Rhythm's five emulator products.
GameCube works through the maintained direct Dolphin adapter. Nintendo 64 through
Mupen64Plus-Next now has a verified playable Ocarina slice on Pixel4.

Canonical source, Android package/JNI, data and links now use ntx. Protected
Pixel4 migration preserves both original GameCube game/profile identities and
ordinary progress; the source installation and recovery material are retained.
Ocarina shares the collection with both GameCube games. Its controls, in-game
Save, ordinary cold reload, fresh-process scene return and sequential cross-system
switching are verified. Clear sound retains its prior human-observed build bounds;
intro hitches and historical lifecycle incidents remain disclosed.

The installed N64 Rust editor now passes drag/resize, Save/Cancel and fresh-process
layout persistence, with the shared GameCube editor preserved. Actual-target
stick/button input, independent release and cancellation also pass in the isolated
preview. A bounded canonical-package GameCube title/attract Home/return and
Pause/Resume/Stop route is also observed, with current cards unchanged. See the
[migration boundary](docs/IDENTITY_MIGRATION.md), [acceptance](docs/ACCEPTANCE.md)
and [N64 pin/audit](native/mupen/README.md).

Controller editing now uses a play-size canvas with grouped movement, corner
resize, snapping, mirror, undo and scoped reset. The game field has the same
selection and editing priority as controls, with aspect-correct preferences used
by both renderers. Pixel4 observes GC field move/resize/undo/Save and N64 paused
image/resize/Save/native-picture mapping. The short-screen group repair,
Save/Cancel/cold editor reload and movable tools dock are observed on Pixel4.
The default maximizes the complete native-aspect picture first—height in landscape,
width in portrait—then fits controls in unused space without overlap. Pixel4
checks verify full-height pictures through both active renderers, N64 grouped
editing, GameCube field resize, saved native placement and Cancel. See
[current state](docs/state.toml) for exact build and verification bounds. Android
currently requests landscape; portrait geometry has portable model coverage.

N64 deadlines retain cadence through wake-up jitter. A bounded same-installation
cold/warm title pair verifies shader-cache reuse and reduces core calls above40ms
from24 to7. Same-call compiler measurements now identify register culling and
assembly as the largest measured compilation passes. Lazy live-query counts
reduce the observed culling maximum from41.056ms to28.821ms; compiler calls still exceed40ms. A follow-up incoming-branch
index shows no material title-window gain. This is not a whole-game hitch cure.
Intro hitches and historical incidents remain open. See the
[latest result](https://github.com/WigglyDon/ntx/issues/5#issuecomment-6621210896)
and [current state](docs/state.toml) for exact build and observation limits.

Resume storage is being bounded explicitly: the N64 source repair retires known
superseded scenes only after durable publication, preserves recovery candidates,
and checks space before new scene writes. See [storage policy](docs/RESUME_STORAGE.md)
for source/installed verification bounds.

Read [AGENTS.md](AGENTS.md), [successor handoff](docs/NTX_HANDOFF.md),
[NTX-5](https://github.com/WigglyDon/ntx/issues/5) and its
[architecture amendment](https://github.com/WigglyDon/ntx/issues/5#issuecomment-6505289270).
Continue this project; do not bootstrap a replacement or repeat completed tests.

- [Product](docs/PRODUCT.md): comfortable game-first alpha and active scope.
- [Architecture](docs/ARCHITECTURE.md): host/system/backend and independent games.
- [Current state](docs/state.toml), [issue identities](docs/github-state.toml).
- [Acceptance](docs/ACCEPTANCE.md): earned results and unresolved defects.
- [Operations](docs/OPERATIONS.md), [distribution](docs/DISTRIBUTION.md),
  [native reproduction](native/README.md), [third-party terms](THIRD_PARTY.md).

Original first-party work is [MIT](LICENSE); borrowed components keep their actual
terms, including Dolphin's GPL obligations. No proprietary games, firmware or
Nintendo artwork are supplied. No public binary/store release is implied.
