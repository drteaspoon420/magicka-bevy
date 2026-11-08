//! World operations (docs/MODDING.md §5.5): the native routines the
//! built-in Magicks are made of, exposed one-to-one. Each Lua call queues
//! `Command::World`; the host resolves entities and runs the routine after
//! the handler returns. Arguments are validated by the host (errors go to
//! the log with the mod's name).
use super::*;

/// (namespace, function, minimum arguments, signature for documentation).
pub const WORLD_OPS: &[(&str, &str, usize, &str)] = &[
    (
        "Spells",
        "Hit",
        2,
        "Hit(target, {attack, element, amount, magnitude}, {source, from})",
    ),
    (
        "Spells",
        "Projectile",
        2,
        "Projectile(owner, {elements = {fire = 5}, position, velocity, splash})",
    ),
    (
        "Spells",
        "Lightning",
        1,
        "Lightning(owner, {target | direction, from, damage, range, hit_owner})",
    ),
    (
        "Spells",
        "Thunderbolt",
        3,
        "Thunderbolt(owner, position, direction)",
    ),
    ("Spells", "Strike", 2, "Strike(owner, position)"),
    (
        "Spells",
        "Status",
        2,
        "Status(target, status, {dps, magnitude})",
    ),
    ("Spells", "Push", 1, "Push(center, {owner, pull})"),
    ("Spells", "GreaseField", 1, "GreaseField(position, owner)"),
    ("Entities", "SetInvisible", 2, "SetInvisible(e, seconds)"),
    ("Entities", "SetEthereal", 2, "SetEthereal(e, ethereal)"),
    ("Entities", "Levitate", 1, "Levitate(e)"),
    ("Entities", "Haste", 1, "Haste(e, seconds?)"),
    ("Entities", "Resize", 2, "Resize(e, \"grow\" | \"shrink\")"),
    ("Entities", "Charm", 2, "Charm(e, owner, seconds?)"),
    ("Entities", "Fear", 2, "Fear(e, source entity or position)"),
    (
        "Entities",
        "Confuse",
        2,
        "Confuse(e, from entity or position)",
    ),
    ("Entities", "Polymorph", 1, "Polymorph(e)"),
    ("Entities", "Terminate", 1, "Terminate(e)"),
    (
        "Entities",
        "Revive",
        1,
        "Revive(position, {health, slot, from})",
    ),
    ("Entities", "Blink", 2, "Blink(e, position)"),
    (
        "Entities",
        "Summon",
        2,
        "Summon(unit, position, {owner, undead})",
    ),
    ("Entities", "SpawnNative", 2, "SpawnNative(kind, owner)"),
    ("World", "SetLight", 1, "SetLight(intensity)"),
    ("World", "Flash", 1, "Flash(seconds)"),
    ("World", "Shake", 2, "Shake(magnitude, seconds)"),
    ("World", "Cue", 1, "Cue(name, {position, bank})"),
    ("World", "Effect", 2, "Effect(name, position, direction?)"),
    (
        "World",
        "Weather",
        1,
        "Weather(\"rain\" | \"blizzard\", {owner})",
    ),
    ("World", "TimeWarp", 0, "TimeWarp(owner?)"),
    ("World", "Nullify", 0, "Nullify()"),
    ("World", "Corporealize", 1, "Corporealize(position)"),
];

/// Operations that create characters or simulated entities count against
/// the per-tick spawn limit.
pub fn spawns(op: &str) -> bool {
    matches!(op, "Entities.Summon" | "Entities.SpawnNative")
}

pub(super) fn install(lua: &Lua, safe: &Table) -> mlua::Result<()> {
    for &(namespace, name, min, signature) in WORLD_OPS {
        let table = match safe.raw_get::<Value>(namespace)? {
            Value::Table(t) => t,
            _ => {
                let t = lua.create_table()?;
                t.raw_set("__namespace", namespace)?;
                safe.set(namespace, t.clone())?;
                t
            }
        };
        let op = format!("{namespace}.{name}");
        let f = lua.create_function(move |lua, a: Variadic<Value>| {
            let a = args(a);
            if a.len() < min {
                return Err(err(format!("usage: {namespace}.{signature}")));
            }
            let args = a
                .iter()
                .map(convert::from_lua)
                .collect::<mlua::Result<Vec<_>>>()?;
            with_state(lua, |s| {
                s.issue(Command::World {
                    op: op.clone(),
                    args,
                })
            })?;
            Ok(Value::Nil)
        })?;
        table.set(name, f)?;
    }
    // Reads.
    let world: Table = safe.get("World")?;
    world.set(
        "IsIndoors",
        lua.create_function(|lua, _: Variadic<Value>| with_state(lua, |s| Ok(s.snapshot.indoors)))?,
    )?;
    Ok(())
}
