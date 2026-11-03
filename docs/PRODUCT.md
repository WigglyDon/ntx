# Product: Nintendo games as durable applications

ntx is the Nintendo member of the five emulator products, sharing PSX's established
game-first experience and architecture. This is continuation of the existing
player, not a restart, sibling merger or shared studio framework.

Choose a local game, play, leave and return to that game's own progress and scene.
One collection spans the supported systems. Execution backends are implementation
choices, not a mandatory core-configuration UI. Keep original sibling branding,
a comfortable layout/editor, system-correct controls and understandable recovery.

## Active delivery

[NTX-5](https://github.com/WigglyDon/ntx/issues/5) and
[amendment6505289270](https://github.com/WigglyDon/ntx/issues/5#issuecomment-6505289270)
govern. Complete the full ntx identity and safe migration; preserve working direct
Dolphin GameCube support; add N64 now through Mupen64Plus-Next. FN64 may later
replace the N64 adapter without redefining games or ordinary saves. Other Nintendo
systems remain explicit later additions. No all-platform port is being claimed.

Canonical identity: ntx, Android com.machinerhythm.ntx, data root ntx and
machinerhythm-ntx://play/<GameId>. A cosmetic label over permanently active gcx
names is insufficient. Old names remain only in history and isolated migration
compatibility. Package migration must preserve current progress and reacquire
media permission; never silently lose unsaved scenes or uninstall the only source.

## Continuity belongs to each game

Multiple games independently open/resumable on the same and different systems are
the baseline model. One foreground runtime is an initial scheduling policy, not a
permanent singleton. Open, resident-paused, executing, checkpointed/unloaded and
awaiting recovery are different states of the same durable session. Supported
hosts/backends may later provide multiple resident or executing instances.

Ordinary saves record game-committed progress; automatic machine checkpoints
record exact continuation and pause intent. Neither replaces the other. Separate
both from immutable media and layout preferences. Keep errors/incompatible states
recoverable, reject stale generations against newer ordinary progress and offer
explicit cold start. Save identity follows the game, not the backend or host.

## Alpha comfort

Maximize the game picture using actual drawable/aspect; keep controls comfortably
clear and correctly mapped, with simultaneous input, interruption release, editable
profiles and consistent preferred haptics. Pause/Resume/Stop remain reachable.
GameCube has two sticks and distinct analog/click L/R. N64 has one analog stick,
four C buttons and digital shoulders; its persistence is cartridge/Controller Pak,
not a GameCube card. Physical controllers are useful, never a required workaround.

Prioritize real user outcomes over qualification bookkeeping. Reuse earned boot,
ordinary-save, scene-return and timing evidence at its actual build bounds.
Speedrunner/quest-progression automation is retired. Preserve historical defects
without requiring that every unrepeatable report be disproved before product work.
Use short representative changed-behavior checks, protected saves and honest limits.
Optional shortcut limitations do not block the whole alpha.


## Default picture and touch placement

Every game defaults to the largest complete picture at its backend-reported
aspect within the full drawable: full height in landscape and full width in
portrait when that aspect permits. Do not reserve control rails or gesture/content
margins inside the picture, stretch it, or crop gameplay. Place console-correct
touch controls in the unused side/bottom space without overlap. Preserve semantic
button groups when a narrow region needs a compact arrangement. Rendering,
editing and hit testing use the same measured geometry.

Existing custom layouts retain their explicit meaning. Fitting the picture around
controls and allowing overlap are explicit alternatives. If the drawable leaves
no usable control space, pause for that choice; do not silently change the default
or publish an unusable layout. Picture/control preferences remain separate from
game identity, ordinary saves and machine continuation.
