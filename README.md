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
preview. One canonical-package GameCube Home/reentry regression remains. See the
[migration boundary](docs/IDENTITY_MIGRATION.md), [acceptance](docs/ACCEPTANCE.md)
and [N64 pin/audit](native/mupen/README.md).

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
