# Modding the Magicka rewrite

This guide is for mod authors. It covers what a mod is, how mods load and
override each other, the KeyValues (KV) definition formats, the
server-side Lua API, mod UI, multiplayer behaviour, and the compatibility
policy. Everything here is a **rewrite extension**: none of it describes
the original game.

Working samples ship in `mods/`:

| Mod | Shows |
| --- | --- |
| `example.my_magicks` | A new Magick (KV + Lua), a presentation effect, a retuned retail staff with a Lua passive modifier, localization |
| `example.arena` | A Lua-driven level: watched area, enemy wave, objectives, a native trigger action |
| `boosted` | A full game mode: currency, weighted upgrade offers, purchases, persistence, a UI panel |
| `example.frost_comet` | A new Magick, Frost Comet Storm, written only with the world operations (§5.5) |
| `example.lua_magicks` | Every built-in Magick re-implemented in Lua, replacing the native handlers (§5.5) |

Check your work with the author tool (from ``):

```sh
cargo run --locked -p magicka-script --bin magicka-modtool -- \
    --base game/base --mods mods --enable example.my_magicks validate
```

`validate` compiles every definition, checks every Lua file's syntax,
boots the server Lua VM with your mod and prints all problems with their
file and line. It also prints the rules hash that clients must match.
`order`, `resolve <logical path>`, `show <definition id>` and `hash` help
find out why something did or did not take effect.

## 1. Running mods

```sh
# Host (playable or dedicated) and clients use the same mod arguments.
magicka-viewer … --network-host 0.0.0.0:17700 --mods mods --enable boosted
magicka-viewer … --network-join host:17700   --mods mods --enable boosted
magicka-server --content … --level Havindr.lvl --mods mods --enable boosted
```

Without `--enable`, the mods listed in `<mods>/load_order.toml` load. The
authored base definitions in `game/base` mount below every
mod (override with `--mods-base`). Offline play runs mods too.

**Gameplay runs only on the host.** Clients never run gameplay Lua. A
client must load the same mods so that its *rules hash* matches the host's;
otherwise the host refuses it with a "different game content" reason.
Clients use their copy of the mods for presentation: names, combinations,
effect assets and UI panels.

## 2. Package layout

```text
mods/
  load_order.toml                 enabled = [ "a.mod", "b.mod" ]
  my_mod/                         directory name is free; the Id is in mod.kv
    mod.kv                        manifest
    scripts/npc/items/*.kv        definitions (see §4)
    scripts/npc/abilities/*.kv
    scripts/npc/modifiers/*.kv
    scripts/npc/magicks/*.kv
    scripts/npc/spells/*.kv
    scripts/npc/units/*.kv
    scripts/npc/effects/*.kv
    scripts/npc/patches/*.kv      typed patches of other mods' definitions
    scripts/levels/*.kv           level definitions
    scripts/vscripts/<mod id>/init.lua   server entry point
    scripts/vscripts/…            other Lua (require, handlers)
    ui/*.kv                       UI panels
    localization/<language>/*.kv  strings
```

**Authoritative content** is everything under `scripts/npc`,
`scripts/vscripts` and `scripts/levels`. It is hashed and must match
between host and clients, and it is pinned in memory when a match
starts. Everything else is presentation.

### Manifest (`mod.kv`)

```kv
"Mod"
{
    "Id"              "example.my_magicks"   // [a-z0-9._-], ≤ 64 chars
    "Version"         "1.0.0"                // semver
    "ApiVersion"      "1"                    // must be supported (§9)
    "DisplayNameKey"  "#example_mod_name"
    "ServerScripts"   "1"                    // has scripts/vscripts/<Id>/init.lua
    "ClientAssets"    "1"
    "ClientOnly"      "0"                    // 1: presentation only, no authoritative files
    "Dependencies"    { "base.game" ">=1.0.0" "other.mod" "^2" }
    "Conflicts"       { "rival.mod" "*" }
}
```

### Load order and overrides

- Missing dependencies, unmet version ranges, declared conflicts and
  dependency cycles are errors before the match starts. Dependencies are
  never enabled for you.
- Mods load in a stable topological order: dependencies first, otherwise
  the order you listed them in.
- **Later mods win for the same logical path.** Paths are compared in
  lowercase with `/` separators, so `Scripts\NPC\Items.kv` and
  `scripts/npc/items.kv` are the same file. A collision replaces the whole
  file; files are never merged. To change single fields of another mod's
  definition, use a typed patch (§4.3).
- Give files names unique to your mod (for example
  `localization/english/<mod id>.kv`), unless you mean to replace someone
  else's file.
- Absolute paths, `..`, drive letters, case collisions inside one mod and
  symlinks pointing outside the mod are rejected.

## 3. KeyValues syntax

```kv
#base "../include/shared.kv"      // top level only, relative, through the mod filesystem
"Root"
{
    "key"   "value"               // quoted strings: escapes \" \\ \n \t
    bare_key 12.5                 // bare tokens: letters, digits, _ . - : + / #
    "block" { … }                 /* block comments too */
}
```

Not supported (rejected, never guessed): `[$PLATFORM]` conditionals,
`#include`, unterminated strings and stray braces. Every error reports
`file:line:column`. Files included with `#base` belong outside the
scanned definition directories; otherwise they register twice.

## 4. Definitions

IDs are `namespace:name`. An unqualified key takes your mod's ID as its
namespace (`base` for the base layer). Unknown fields, out-of-range
numbers, non-integers in integer fields and unknown words are errors.

### 4.1 Numbers and the stat resolver

Every numeric field has a schema: type, units, legal range, whether
upgrades may change it, whether negative values mean something, how
contributions combine, and whether it affects gameplay or only
presentation. Values are resolved per holder in this order: base
definition (after patches), then per-player upgrades, then temporary
modifiers, then the stacking rule and range clamp. Base definitions are
never changed at runtime.

### 4.2 Types

| Root | Fields (numbers in *italics*) |
| --- | --- |
| `Items` | `Retail` (retail item name it overrides), `Slot` weapon/staff, `Model`, `Sounds`, `Ability`, `Modifiers` (held → applied), *BlockValue CooldownTime PassiveValue MeleeRange RangedRange RangedElevation Homing GunRange GunClip GunRate GunAccuracy TracerVelocity Scale* |
| `Abilities` | `Behavior` passive/active/toggle, `Target` none/point/unit/direction, handler, `Values { key "10 20 30" }`, `Modifiers`, *Cooldown CastRange Radius Duration Damage* |
| `Modifiers` | `Stacking` refresh/stack/independent, `Debuff`, `Hidden`, `Properties { MoveSpeedPercent DamagePercent DamageTakenPercent HealthRegen MaxHealthBonus CooldownPercent }`, handler, `Effect`, *Duration MaxStacks TickInterval* |
| `Magicks` | `Combination "fire,earth,…"`, `CastType`, handler, `Animation`, `Particle`, `Sound`, `UnlockedByDefault`, *Cooldown* |
| `Spells` | `Elements`, `Delivery` projectile/spray/beam/area/self/weapon, `Effects`, handler, *DamageMultiplier Range Radius* |
| `Units` | `Template` (retail character), `Model`, `Abilities`, `Equipment { slot item }`, *Health MoveSpeed Radius* |
| `Effects` | `Kind` emitter/sound, `Asset`, `Attachment`, `Lifecycle` oneshot/loop, *Duration Scale* (presentation) |
| `Levels` | `Level` (file), `Scenes`, handler, `Objectives`, `Assets` |

Modifier properties act on the holder natively each tick:
`MoveSpeedPercent` scales movement after native status effects;
`HealthRegen` heals (or drains) per second; `DamageTakenPercent` scales
every positive hit the native impact and direct-damage paths deliver;
`MaxHealthBonus` joins the native per-frame maximum (and reverts when the
modifier ends); `DamagePercent` joins the holder's native damage-boost
list, so it scales the elemental damage of every spell, melee hit and
projectile they deliver; `CooldownPercent` scales the cooldown of every
item they hold.

Every type also accepts `NameKey`, `DescriptionKey` and `Icon`. A
**handler** is either `"Builtin" "<native name>"` or `"ScriptFile"
"<path under scripts/vscripts>"` with `"Handler" "<function>"`.

**Assets.** A path starting with `content:` names a file in the player's
own retail content; it is checked when used. Any other path must exist in
the mod filesystem. Retail assets are never redistributed.

**Magicks.** Built-in Magicks are registered first, as `base:<name>` (for
example `base:haste`, `base:meteors`; the names are listed in
`example.lua_magicks/scripts/npc/magicks/builtins.kv`). A new combination
must be 1–5 elements that the real element queue keeps as typed (no
merging, cancelling or second shield), and it must not collide with any
other Magick.

Defining `base:<name>` with a `ScriptFile`/`Handler` **replaces** that
built-in: its combination, unlocks and cast animation stay, but casting
it, by a player, an NPC, a boss or an item, runs your handler instead of
the native effect.

### 4.3 Patches

```kv
"Patches"
{
    "Items"   { "base:clock_staff" { "CooldownTime" "1.5" "Remove" "BlockValue" } }
    "Magicks" { "base:haste"       { "Cooldown" "3" } }    // built-ins keep their native behaviour
}
```

Patches apply in mod load order. Patching an unknown ID or removing a
field the definition lacks is an error.

### 4.4 Localization

`localization/<language>/*.kv` → `"Tokens" { "#key" "text" }`. Lookups
fall back to English. A missing key referenced by a definition is a
warning from `validate`.

## 5. Server Lua

The engine embeds **Lua 5.4** (via `mlua`). It borrows ideas from Valve's
Dota 2 VScript API but is not compatible with it (§10).

### 5.1 Sandbox and limits

Available: `assert error ipairs next pairs pcall select tonumber tostring
type xpcall rawequal rawget rawset rawlen setmetatable getmetatable print`
and the `string table math utf8 coroutine` libraries. **Absent:** `os io
debug package load loadfile dofile collectgarbage string.dump`.
`math.random` uses the match's seeded RNG, so the same seed replays the
same game.

`require("a.b")` loads `scripts/vscripts/a/b.lua` through the mod
filesystem, once per mod; cycles are errors. Your own folder is named
after your mod Id, which may contain dots: `require("my.mod.util")` from
mod `my.mod` loads `scripts/vscripts/my.mod/util.lua`. Each mod has its own global
environment.

Each match VM has a 64 MiB memory limit and 5 million instructions per
callback. Each mod is also capped on timers, subscriptions, commands and
spawns per tick, and net-table size. A script that breaks a limit gets an
error in that callback only; the game and other mods keep running. Errors
name the mod, the callback, the entity and the Lua traceback.

Scripts **read** a snapshot of the world and **ask** for changes. Every
request is checked and applied by the engine at a fixed point in the tick,
so a script cannot reach engine internals.

### 5.2 Lifecycle

Global functions in your mod's environment, called in this order:

`OnModLoad → OnMatchSetup → OnGameStart → OnLevelStart(e) →
OnPlayerJoin(p) → … → OnLevelEnd(e) → OnMatchEnd → OnModUnload`

Game and level callbacks: `OnNpcKilled(e)`, `OnTriggerEnter(e)`,
`OnCheckpoint(e)`, `OnWaveStart(e)`, `OnWaveComplete(e)`,
`OnInteract(e)` (`e.interactable` is the interactable's name hash, compare
with `Levels.Hash("name")`; `e.entity` is the interacting wizard),
`OnModRequest(r)` (§6). `OnInteract` currently fires for the host's own
wizard only: clients have no interact input in the protocol yet. Save support: `SaveState() → table` and
`LoadState(table)`.

### 5.3 API reference

Every function may be called as `NS.F(…)` or `NS:F(…)`. Positions are
`{x=, y=, z=}` (or `{x, y, z}`). Entities are handles that check their
generation: `e:IsValid()` is false once the character is gone, and calls
on a stale handle do nothing.

| Namespace | Functions |
| --- | --- |
| entity methods | `IsValid IsAlive GetHealth GetMaxHealth GetName GetPlayer GetFaction GetPosition GetForward HasStatus(name) IsEthereal HasModifier(id) GetModifierStacks(id) GetHandle` |
| `GameRules` | `GetGameTime() GetTick() GetLevel() GetScene() SendMessage(text, player?)` |
| `Players` | `GetAll() Get(slot)` → `{slot, name, profile_id, entity, magicks}` |
| `Entities` | world operations (§5.5); `GetAll() FindInRadius(pos, r) Damage(e, amount, {element, source}) Heal(e, amount) Teleport(e, pos) Spawn(unit, pos, tag) Remove(e)` |
| `Abilities` | `GetValue(id, key, level)` |
| `Modifiers` | `Apply(e, id, {duration, stacks, caster}) Remove(e, id) Has(e, id) GetStacks(e, id)` |
| `Spells` / `Elements` | world operations (§5.5); `CastElements(caster, {"fire", …}, {delivery, target_position})`: a wizard casts through its own input (it waits until the wizard is free); an NPC with spell abilities casts on its next attack through its native spellcasting; `IsTypeable(list)` |
| `Magicks` | `Invoke(caster, id, {target_position}) Unlock(player, id) Get(id)` |
| `Events` | `Subscribe(name, fn, priority?) → token`, `AddFilter(name, fn, priority?)`, `Unsubscribe(token)`, `Fire(name, data)` (queued) |
| `Timers` | `After(s, fn) Every(s, fn)` (return `false` to stop), `Cancel(id)`; simulation time |
| `NetTables` | `Set(table, key, value) Get(table, key)`: server-owned state replicated to every client for UI |
| `Particles` / `Audio` | `Create(effect, {position, owner})` / `Play(sound, {position, owner})`: one-shot presentation that clients play once each |
| `Levels` | `Action(class, args) DefineCondition(class, args) → id ConditionMet(id) AddTrigger(xml) WatchArea(name, pos, r) SpawnWave(name, {{unit, position}, …}) SetObjective(id, text, done)` |
| `Upgrades` | `Keys() Items(player) Apply(player, target, key, "add"/"percent"/"multiply"/"set", v) → id Remove(id) List(player)` |
| `Settings` | `Load("npc/my_mod/settings.kv")` → table (authoritative KV only) |
| `Localization` | `Get(key, language?)` |
| `Random` | `Int(a, b) Float()` (seeded) |
| `World` | world operations (§5.5); `IsIndoors()` |

`Levels.Hash(name)` returns the native name hash of a trigger, area or
interactable (case-insensitive). `Levels.Action` and
`Levels.DefineCondition` use the native level-script
classes (for example `Counter`, `EndGame`, `TriggerCheckpoint`) with the
same property names as authored level XML. Native level triggers keep
running alongside Lua.

### 5.4 Events and filters

Events are notifications after the fact; they cannot change what
happened. **Filters** run before an engine action and return `true`/`nil`
(allow), `false` (deny) or a table of changed fields. Only the fields
listed below may change:

| Filter | Changeable | If the filter errors |
| --- | --- | --- |
| `damage` | `amount` | the filter is ignored |
| `heal` | `amount` | the filter is ignored |
| `cast` | – | the filter is ignored |
| `pickup` | – | the action is denied |
| `currency` | `amount` | the action is denied |
| `purchase` | `price` | the action is denied |

Filters run by priority, highest first, then in the order they were
added.

### 5.5 Writing Magicks: world operations

A Magick handler is called with `ctx = {caster, magick, target, target_position}`:
`target` is the native aim (an NPC caster's combat target, else the
character nearest a player's mouse pick; nil when that is the caster),
and `target_position` is the mouse pick or the target's position.

The **world operations** below are the native routines the built-in
Magicks are made of, exposed one-to-one: a Lua Magick and a native one
run the same code. `example.lua_magicks` re-implements all 49 built-ins
with them (start there), and `example.frost_comet` adds a new Magick.

| Namespace | Operation | Native routine |
| --- | --- | --- |
| `Spells` | `Hit(target, {attack, element, amount, magnitude}, {source, from})` | IDamageable.Damage (Votal, KnockDownChance) |
| | `Status(target, status, {dps, magnitude})` | Character.AddStatusEffect (Inflict…Damage) |
| | `Projectile(owner, {elements = {fire = 5}, position, velocity, splash})` | a spell missile (Meteor Shower) |
| | `Lightning(owner, {target, direction, from, damage, range, hit_owner})` | a bolt at `target` (Vlad Zap) or along `direction`/the facing (Chain Lightning's Zap) |
| | `Thunderbolt(owner, position, direction)` | Thunderbolt's aimed bolt (8 m ahead) |
| | `Strike(owner, position)` | one Thunderstorm strike (tallest within 5 m) |
| | `Push(center, {owner, pull})` | TractorPull / TractorPush (15 m) |
| | `GreaseField(position, owner)` | a grease field (Grease Splash) |
| `Entities` | `SetInvisible(e, s) SetEthereal(e, on) Levitate(e) Haste(e, s?) Resize(e, "grow"/"shrink")` | the caster effects |
| | `Charm(e, owner, s?) Fear(e, source) Confuse(e, from) Polymorph(e) Terminate(e)` | the per-target effect of Charm, Fear, Confuse, Polymorph, CTD |
| | `Revive(position, {health, slot, from}) Blink(e, position)` | Revive, Teleport (needs ground) |
| | `Summon(unit, position, {owner, undead})` | a summoned NPC placed like Summon Undead's |
| | `SpawnNative(kind, owner)` | a simulated Magick entity (below) |
| `World` | `SetLight(i) Flash(s) Shake(magnitude, s) Cue(name, {position, bank}) Effect(name, position, direction?)` | scene light, flash, camera, native sound cues and effects |
| | `Weather("rain"/"blizzard", {owner}) TimeWarp(owner?) Nullify() Corporealize(position)` | the global states |

`SpawnNative` kinds (entities with their own per-frame simulation, posed
from the owner as the built-in does): `tornado vortex wave portal phoenix
death elemental earthquake cross slime slime_overkill clone napalm
conflagration propp amalgameddon judgement_spray grease_spray`.

Names: elements `earth water cold fire lightning arcane life shield ice
steam poison`; attacks `damage knockdown pushed knockback piercing
armour_piercing status entanglement stun bleed` (combine with `|`);
statuses `burning wet frozen cold poisoned healing greased steamed
bleeding`. Native sound banks for `Cue`: 16 Spells (default), 32
Characters, 512 Additional.

Operations run **after your handler returns**, on the host, in the order
issued; their results are in the next tick's snapshot. Use `Timers` for
anything that lasts or repeats, and keep your own state for singletons
(see `MeteorS` in `sky.lua`). A rejected operation (stale entity, bad
argument, nothing to apply to) is logged as `mod <id>: <operation>: …`;
the native "fails" (no target, indoors) are yours to signal, usually with
`World.Cue("magick_fail", {position = …})`. `Summon` and `SpawnNative`
count against the per-tick spawn limit.

```lua
-- Frost Comet Storm (example.frost_comet), the core of it:
Spells.Projectile(caster, {
    elements = { ice = 5, cold = 5 },
    position = { x = p.x, y = p.y + 146, z = p.z },
    velocity = { x = 0.01, y = -20, z = -0.01 },
    splash = 6,
})
```

### 5.6 Disabling

When a mod is disabled, `OnModUnload` runs and the engine then removes
everything the mod owned: subscriptions, filters, timers, net-table
entries, modifiers and upgrades. Gameplay values return to their base.

## 6. Client requests and UI

Clients only *ask*. A UI choice becomes `ModRequest { request_id, channel
"mymod.action", data }`. The host calls `OnModRequest(r)` in the mod named
by the channel prefix, with `r = {player, request_id, channel, data}`,
and sends back whatever table it returns. The engine runs each (player,
request ID) **once**: a repeated or retried request gets the stored result
and is never charged twice. Always validate `data` against your own
server state; never trust a price, an offer or a balance a client sends.

Panels in `ui/*.kv` are drawn from net tables; F4 shows them in the
viewer, and the number keys pick from the list.

```kv
"Panels"
{
    "shop"
    {
        "Title"  "#boosted_title"
        "Lines"  { "1" "Gold: {boosted:gold.$slot}" }
        "List"   { "Source" "boosted:offers.$slot"
                   "Item"   "[{rarity}] {key}: {from} -> {to} ({price} gold)"
                   "Action" "boosted.buy" }
    }
}
```

`{table:key}` reads a net table entry, and `$slot` becomes the viewing
player's slot. Inside `Item`, `{field}` reads fields of each element of
the JSON array named by `Source`. Text starting with `#` is localized.

## 7. Multiplayer behaviour

- Gameplay happens on the host; clients receive the results. A late joiner
  gets the current world, objectives and net tables, and never re-runs
  your scripts or replays old particles.
- Mod Magicks travel by definition ID; registry numbers are local to one
  match and never sent.
- Save what must survive in `SaveState`. Closures, entity handles and VM
  objects are not saved. Modifiers, upgrades, net tables, custom Magick
  unlocks and request results are saved by the engine with the campaign.

## 8. Do and don't

- Do keep gameplay decisions on the server, and use `Random` (seeded) for
  them.
- Do check entities with `IsValid()` before relying on old handles.
- Don't put authoritative data in presentation paths: it would not be
  hashed or pinned.
- Don't ship retail assets. Refer to them with `content:` paths.

## 9. Compatibility and versioning policy

| Version | Where | Bumped when | Compatibility |
| --- | --- | --- | --- |
| Wire protocol (12) | `magicka_network::PROTOCOL_VERSION` | any wire change | host and client must match exactly; a mismatch is refused with a reason |
| Mod API (1) | `magicka_mods::manifest::SUPPORTED_API`, `magicka_script::API_VERSION` | a breaking Lua or KV API change | the engine accepts the listed `ApiVersion`s; new fields and functions do not bump it |
| Content schema (1) | `magicka_mods::CONTENT_SCHEMA` | the rules-hash layout changes | changes every rules hash |
| Definition schema (1) | `magicka_definitions::SCHEMA_VERSION` | an incompatible definition-field change | part of the rules hash; old KV is migrated or rejected with a message |
| Campaign saves | `magicka_runtime::saves::VERSION` | the save layout changes | mod state lives under `"mods"`; a save without it loads with no mod state |

Mods are expected to follow semver. A breaking change to your settings or
`SaveState` format is a major version bump, and your `LoadState` should
accept your previous format.

## 10. Coming from Dota 2 / MGMod

| Dota / MGMod | Here |
| --- | --- |
| `addon_game_mode.lua` `Activate` | `scripts/vscripts/<mod id>/init.lua`, `OnModLoad`/`OnGameStart` |
| `npc_abilities_custom.txt` etc. | `scripts/npc/<type>/*.kv` with typed, validated fields |
| `CustomNetTables` | `NetTables` (server-owned; clients read) |
| `CustomGameEventManager` client→server | `ModRequest` → `OnModRequest`, deduplicated |
| `ListenToGameEvent` | `Events.Subscribe` and the global callbacks |
| `SetDamageFilter` etc. | `Events.AddFilter("damage", …)` with fixed changeable fields |
| `Timers` library | `Timers` (simulation time, owned by your mod) |
| Lua modifiers (`modifier_x = class({})`) | KV `Modifiers` + `ScriptFile` with `OnCreated`/`OnIntervalThink`/`OnDestroy` |
| `CDOTA_BaseNPC` methods | entity handle methods (smaller; checked against the snapshot) |
| Panorama | `ui/*.kv` panels bound to net tables |
| MGMod plugin settings | `Settings.Load` from your authoritative KV |

Dota scripts do not run unchanged: there is no `CDOTA_*` class
hierarchy, and no file, OS or network access.
