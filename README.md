# ntx

**Your Nintendo games. Open, play, leave, return.**

ntx continues the same Rust-first public project and repository history formerly
named gcx. It is the Nintendo member of Machine Rhythm's five emulator products.
GameCube works through the maintained direct Dolphin adapter. Nintendo 64 through
Mupen64Plus-Next is the next active implementation, not yet a playable claim.

Active crates/native/JNI/build/branding now use ntx and com.machinerhythm.ntx.
An offline staged migration tool preserves identities, ordinary progress and
legacy recovery scenes; schema 2 separates system/backend routing. The retained
legacy GameCube installation remains retained. The canonical ntx package is also
installed with migrated data. A renamed GameCube APK has been built, signed and preserved with its exact
source. The pinned ARM64 N64 core now compiles, with Rust host, graphics/audio,
controls and independent profile/save/scene routing implemented. A combined GameCube/N64 package is also built and privately preserved with its
source. Pixel4 data migration now preserves both original game/profile pairs and exact
ordinary data, with source retained and return verified. Media regrant and
playable N64/save/continuity acceptance remain unfinished. See the
[migration boundary](docs/IDENTITY_MIGRATION.md) and [N64 pin/audit](native/mupen/README.md).

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
