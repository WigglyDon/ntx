> NTX-5 includes pinned Mupen64Plus-Next N64 integration. Current selected-source,
> license/resource and package checks are recorded in [the N64 audit](../native/mupen/README.md)
> and [state](state.toml). Public binary release remains separately gated.

# Distribution and provenance

Goal: a public free-source player, eventually through Google Play and the ordinary
Apple App Store. This seed is not legal clearance, marketplace approval or a binary
release. Recheck actual terms and requirements when preparing submission.

## Engineering ownership is not relicensing

Original ntx first-party (including historical gcx) work uses MIT. Dolphin declares GPL-2.0-or-later [S1].
Keep per-file and dependency terms, attribution, change notices and exact source
inputs. A distributed combined application must meet all applicable obligations,
including corresponding-source/build material where required. Rust, FFI, dynamic
linking or a worker process does not automatically make a covered combination
independent proprietary software. Translating borrowed implementation does not
remove its license or authorship. Resolve real ambiguity before release.

Targeted fixes, forks, proper internal APIs and justified Rust replacements are
permitted. Do not ask upstream permission merely to exercise rights already granted
by its license, but do not imply a trademark grant or blanket store permission.
Record why the maintained patch exists and reproduce the exact binary/source pair.
Keep an unmodified source reference for comparison; do not disguise changed builds.

No Dolphin or Nintendo logo/artwork as our identity. ntx is original project branding;
name selection is not a search or certification of trademark availability. Source
reuse from public PSX requires a per-file license/provenance check, not copying the
whole repository and claiming it is all MIT. No shared private files or app signer.

## Complete installation without proprietary payloads

Evaluate Dolphin's existing skipped-IPL / emulated boot / DSP-HLE route first [S2].
Audit all included fonts, shader sources, generated data and any firmware-like
resources. Bundle qualified licensed machinery; do not require users to assemble
cores or a developer toolchain. Preserve source/build provenance for generated
assets. No proprietary IPL, Nintendo boot audio/menu, game image or private save
is included. An optional privately supplied original IPL remains private.

No firmware hunt, pirated image retrieval or claim that game ownership grants every
copying/circumvention/distribution right. Respect relevant law and distribution
terms; obtain specialist review for unresolved rights rather than inventing a
worldwide assurance. First-party functional reimplementation is allowed engineering,
not permission to copy proprietary code and rename it Rust.

## Android

Bundle the native engine and resources in the signed app; no native-core downloader.
Use public platform APIs, a complete scoped document/folder path and only necessary
permissions. Do not assume broad storage scanning qualifies for all-files permission.
No accounts, analytics or network needed for offline play. Disable/remove inherited
telemetry, online achievements, update checks and cover networking when unnecessary;
verify the resulting build and manifest rather than relying on our UI omission.

Check current Play target-SDK and AAB requirements at release [S7]. Verify ELF and
APK alignment as well as native runtime behavior, fast-memory mappings and JIT
protection granularity on supported host pages [S6]. A guest page/block size is not
Android's host page size. Do not copy a historical 4 KiB constant into the new host.
Native fault paths and JNI shims must work on device, not only build on Linux.

Game icons use the system-confirmed pinned-shortcut flow. Ordinary pause does not
need a sham foreground service. Background emulation is not an automatic port of
PSX's optional media behavior and is outside the initial scope.

## Apple

Retain shared Rust semantics and a native platform boundary. Apple's emulator
category rules do not grant rights to games/firmware/dependencies [S8]. Current
DolphiniOS maintainers say JIT prevents their App Store release [S5]. That is a
current constraint to investigate, not an approval for ntx or a permanent impossibility.
A signed stock-iOS build must demonstrate usable execution without a debugger or
special setup assumed unavailable to ordinary users. Full iOS work is not initially
dispatched; do not block the Android slice on it or silently redefine the goal as
sideload-only. iPhone Home Screen shortcuts and execution feasibility are different.

## Release inventory

Before any distributed binary: exact engine/dependency versions, corresponding
source and reproducible build instructions, retained licenses/change notices,
original branding, accessible controls, truthful screenshots and privacy disclosure,
permission review, support/recovery instructions and tested update/save retention.
Store enrollment, legal commitments, fees, public APK releases and submission need
their actual separate approval. Normal public source integration is authorized.
