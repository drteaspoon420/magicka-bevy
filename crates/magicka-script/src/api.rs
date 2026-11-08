//! The engine API namespaces (plan 8.2). Each function accepts both
//! `NS.Func(...)` and `NS:Func(...)`. Reads come from the tick's snapshot and
//! the definition registries; writes are queued commands.
use super::*;

type Args = Variadic<Value>;

fn namespace(lua: &Lua, safe: &Table, name: &str) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.raw_set("__namespace", name)?;
    safe.set(name, t.clone())?;
    Ok(t)
}

fn func<F>(lua: &Lua, t: &Table, name: &str, f: F) -> mlua::Result<()>
where
    F: Fn(&Lua, Vec<Value>) -> mlua::Result<Value> + Send + 'static,
{
    t.set(
        name,
        lua.create_function(move |lua, a: Args| f(lua, args(a)))?,
    )
}

fn entity_value(lua: &Lua, e: Option<EntityRef>) -> mlua::Result<Value> {
    match e {
        Some(e) => Ok(Value::UserData(lua.create_userdata(e)?)),
        None => Ok(Value::Nil),
    }
}
fn scalar(v: &Value) -> mlua::Result<String> {
    Ok(match v {
        Value::Boolean(b) => b.to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.to_str()?.to_string(),
        _ => return Err(err("arguments must be scalars")),
    })
}
fn table_arg(v: &Value) -> Option<&Table> {
    match v {
        Value::Table(t) => Some(t),
        _ => None,
    }
}
fn subscribe(lua: &Lua, a: &[Value], filter: bool) -> mlua::Result<Value> {
    let event = read_string(&arg(a, 0), "event name")?;
    let Value::Function(f) = arg(a, 1) else {
        return Err(err("a subscription needs a function"));
    };
    let priority = match arg(a, 2) {
        Value::Nil => 0,
        v => read_number(&v, "priority")? as i64,
    };
    if filter && !FILTERS.iter().any(|s| s.name == event) {
        let names: Vec<_> = FILTERS.iter().map(|f| f.name).collect();
        return Err(err(format!("no filter {event:?} (filters: {names:?})")));
    }
    let key = lua.create_registry_value(f)?;
    with_state(lua, |s| {
        let owner = s.owner()?;
        if s.owned_count(owner, false) >= s.limits.subscriptions_per_mod {
            return Err(err("subscription limit reached"));
        }
        s.next_token += 1;
        s.subscriptions.push(Subscription {
            token: s.next_token,
            owner,
            event,
            priority,
            filter,
            func: key,
        });
        Ok(Value::Integer(s.next_token as i64))
    })
}
fn timer(lua: &Lua, a: &[Value], repeat: bool) -> mlua::Result<Value> {
    let delay = read_number(&arg(a, 0), "delay")?;
    if delay < 0.0 || (repeat && delay <= 0.0) {
        return Err(err("timer delay must be positive"));
    }
    let Value::Function(f) = arg(a, 1) else {
        return Err(err("a timer needs a function"));
    };
    let key = lua.create_registry_value(f)?;
    with_state(lua, |s| {
        let owner = s.owner()?;
        if s.owned_count(owner, true) >= s.limits.timers_per_mod {
            return Err(err("timer limit reached"));
        }
        s.next_timer += 1;
        s.timers.insert(
            s.next_timer,
            Timer {
                owner,
                due: s.time + delay,
                interval: repeat.then_some(delay),
                func: key,
            },
        );
        Ok(Value::Integer(s.next_timer as i64))
    })
}
fn player_table(lua: &Lua, p: &PlayerInfo) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("slot", p.slot)?;
    t.set("name", p.name.as_str())?;
    t.set("profile_id", p.profile_id.as_deref())?;
    t.set("entity", entity_value(lua, p.entity)?)?;
    let magicks = lua.create_sequence_from(p.magicks.iter().map(String::as_str))?;
    t.set("magicks", magicks)?;
    Ok(t)
}

pub(super) fn install(lua: &Lua, safe: &Table, random: Function) -> mlua::Result<()> {
    // GameRules
    let t = namespace(lua, safe, "GameRules")?;
    func(lua, &t, "GetGameTime", |lua, _| {
        with_state(lua, |s| Ok(Value::Number(s.time)))
    })?;
    func(lua, &t, "GetTick", |lua, _| {
        with_state(lua, |s| Ok(Value::Integer(s.snapshot.tick as i64)))
    })?;
    func(lua, &t, "GetLevel", |lua, _| {
        let level = with_state(lua, |s| Ok(s.snapshot.level.clone()))?;
        Ok(Value::String(lua.create_string(level)?))
    })?;
    func(lua, &t, "GetScene", |lua, _| {
        let scene = with_state(lua, |s| Ok(s.snapshot.scene.clone()))?;
        Ok(Value::String(lua.create_string(scene)?))
    })?;
    func(lua, &t, "SendMessage", |lua, a| {
        let text = read_string(&arg(&a, 0), "message")?;
        let player = match arg(&a, 1) {
            Value::Nil => None,
            v => Some(read_number(&v, "player")? as usize),
        };
        with_state(lua, |s| s.issue(Command::Message { text, player }))?;
        Ok(Value::Nil)
    })?;

    // Players
    let t = namespace(lua, safe, "Players")?;
    func(lua, &t, "GetAll", |lua, _| {
        let players = with_state(lua, |s| Ok(s.snapshot.players.clone()))?;
        let out = lua.create_table()?;
        for (i, p) in players.iter().enumerate() {
            out.raw_set(i + 1, player_table(lua, p)?)?;
        }
        Ok(Value::Table(out))
    })?;
    func(lua, &t, "Get", |lua, a| {
        let slot = read_number(&arg(&a, 0), "slot")? as usize;
        let p = with_state(lua, |s| {
            Ok(s.snapshot.players.iter().find(|p| p.slot == slot).cloned())
        })?;
        match p {
            Some(p) => Ok(Value::Table(player_table(lua, &p)?)),
            None => Ok(Value::Nil),
        }
    })?;

    // Entities
    let t = namespace(lua, safe, "Entities")?;
    func(lua, &t, "GetAll", |lua, _| {
        let all: Vec<EntityRef> = with_state(lua, |s| {
            Ok(s.snapshot
                .entities
                .iter()
                .filter_map(|e| e.entity)
                .collect())
        })?;
        let out = lua.create_table()?;
        for (i, e) in all.into_iter().enumerate() {
            out.raw_set(i + 1, lua.create_userdata(e)?)?;
        }
        Ok(Value::Table(out))
    })?;
    func(lua, &t, "FindInRadius", |lua, a| {
        let center = read_vec3(&arg(&a, 0))?;
        let radius = read_number(&arg(&a, 1), "radius")? as f32;
        let found: Vec<EntityRef> = with_state(lua, |s| {
            let mut v: Vec<(f32, EntityRef)> = s
                .snapshot
                .entities
                .iter()
                .filter_map(|e| {
                    let d = (0..3)
                        .map(|i| (e.position[i] - center[i]).powi(2))
                        .sum::<f32>()
                        .sqrt();
                    (d <= radius).then_some((d, e.entity?))
                })
                .collect();
            v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            Ok(v.into_iter().map(|(_, e)| e).collect())
        })?;
        let out = lua.create_table()?;
        for (i, e) in found.into_iter().enumerate() {
            out.raw_set(i + 1, lua.create_userdata(e)?)?;
        }
        Ok(Value::Table(out))
    })?;
    func(lua, &t, "Damage", |lua, a| {
        let target = read_entity(&arg(&a, 0))?;
        let amount = read_number(&arg(&a, 1), "amount")? as f32;
        let opts = arg(&a, 2);
        let element = match opt(table_arg(&opts), "element")? {
            Value::Nil => None,
            v => Some(read_string(&v, "element")?),
        };
        let source = match opt(table_arg(&opts), "source")? {
            Value::Nil => None,
            v => Some(read_entity(&v)?),
        };
        with_state(lua, |s| {
            s.issue(Command::Damage {
                target,
                amount,
                element,
                source,
            })
        })?;
        Ok(Value::Nil)
    })?;
    func(lua, &t, "Heal", |lua, a| {
        let target = read_entity(&arg(&a, 0))?;
        let amount = read_number(&arg(&a, 1), "amount")? as f32;
        with_state(lua, |s| s.issue(Command::Heal { target, amount }))?;
        Ok(Value::Nil)
    })?;
    func(lua, &t, "Teleport", |lua, a| {
        let target = read_entity(&arg(&a, 0))?;
        let position = read_vec3(&arg(&a, 1))?;
        with_state(lua, |s| s.issue(Command::Teleport { target, position }))?;
        Ok(Value::Nil)
    })?;
    func(lua, &t, "Spawn", |lua, a| {
        let unit = read_string(&arg(&a, 0), "unit")?;
        let position = read_vec3(&arg(&a, 1))?;
        let tag = match arg(&a, 2) {
            Value::Nil => String::new(),
            v => read_string(&v, "tag")?,
        };
        with_state(lua, |s| {
            s.issue(Command::Spawn {
                unit,
                position,
                tag,
            })
        })?;
        Ok(Value::Nil)
    })?;
    func(lua, &t, "Remove", |lua, a| {
        let target = read_entity(&arg(&a, 0))?;
        with_state(lua, |s| s.issue(Command::Remove { target }))?;
        Ok(Value::Nil)
    })?;

    // Abilities: typed values from the registry.
    let t = namespace(lua, safe, "Abilities")?;
    func(lua, &t, "GetValue", |lua, a| {
        let id = read_id(&arg(&a, 0), "ability")?;
        let key = read_string(&arg(&a, 1), "key")?.to_ascii_lowercase();
        let level = match arg(&a, 2) {
            Value::Nil => 1,
            v => read_number(&v, "level")?.max(1.0) as usize,
        };
        with_state(lua, |s| {
            let Some(ability) = s.defs.abilities.get(&id) else {
                return Err(err(format!("unknown ability {id}")));
            };
            let v = ability
                .values
                .get(&key)
                .map(|list| list[(level - 1).min(list.len() - 1)])
                .or_else(|| {
                    ability
                        .common
                        .numbers
                        .iter()
                        .find(|(k, _)| k.eq_ignore_ascii_case(&key))
                        .map(|(_, v)| *v)
                });
            Ok(v.map_or(Value::Nil, Value::Number))
        })
    })?;

    // Modifiers
    let t = namespace(lua, safe, "Modifiers")?;
    func(lua, &t, "Apply", |lua, a| {
        let target = read_entity(&arg(&a, 0))?;
        let id = read_id(&arg(&a, 1), "modifier")?;
        let opts = arg(&a, 2);
        let duration = match opt(table_arg(&opts), "duration")? {
            Value::Nil => None,
            v => Some(read_number(&v, "duration")?),
        };
        let stacks = match opt(table_arg(&opts), "stacks")? {
            Value::Nil => 1,
            v => read_number(&v, "stacks")?.clamp(1.0, 1000.0) as u32,
        };
        let caster = match opt(table_arg(&opts), "caster")? {
            Value::Nil => None,
            v => Some(read_entity(&v)?),
        };
        with_state(lua, |s| {
            let owner = s.owners[s.owner()?].clone();
            if s.snapshot.entity(target).is_none() {
                return Err(err("cannot apply a modifier to a stale entity"));
            }
            let defs = s.defs.clone();
            let mut events = std::mem::take(&mut s.modifier_events);
            let r = s.modifiers.apply(
                &defs,
                target,
                &id,
                duration,
                stacks,
                caster,
                &owner,
                &mut events,
            );
            s.modifier_events = events;
            r.map(|i| Value::Integer(i as i64)).map_err(err)
        })
    })?;
    func(lua, &t, "Remove", |lua, a| {
        let target = read_entity(&arg(&a, 0))?;
        let id = read_id(&arg(&a, 1), "modifier")?;
        with_state(lua, |s| {
            let mut events = std::mem::take(&mut s.modifier_events);
            s.modifiers.remove(target, &id, &mut events);
            s.modifier_events = events;
            Ok(Value::Nil)
        })
    })?;
    func(lua, &t, "Has", |lua, a| {
        let target = read_entity(&arg(&a, 0))?;
        let id = read_id(&arg(&a, 1), "modifier")?;
        with_state(lua, |s| Ok(Value::Boolean(s.modifiers.has(target, &id))))
    })?;
    func(lua, &t, "GetStacks", |lua, a| {
        let target = read_entity(&arg(&a, 0))?;
        let id = read_id(&arg(&a, 1), "modifier")?;
        with_state(lua, |s| {
            Ok(Value::Integer(s.modifiers.stacks(target, &id) as i64))
        })
    })?;

    // Spells / Elements: the normal elemental path.
    let t = namespace(lua, safe, "Spells")?;
    safe.set("Elements", t.clone())?;
    func(lua, &t, "CastElements", |lua, a| {
        let caster = read_entity(&arg(&a, 0))?;
        let Value::Table(list) = arg(&a, 1) else {
            return Err(err("elements must be a list of names"));
        };
        let mut elements = Vec::new();
        for v in list.sequence_values::<String>() {
            let name = v?.to_ascii_lowercase();
            if magicka_definitions::defs::element_names().all(|e| e != name) {
                return Err(err(format!("unknown element {name:?}")));
            }
            elements.push(name);
        }
        if elements.is_empty() || elements.len() > 5 {
            return Err(err("cast 1..=5 elements"));
        }
        let opts = arg(&a, 2);
        let delivery = match opt(table_arg(&opts), "delivery")? {
            Value::Nil => "projectile".to_string(),
            v => read_string(&v, "delivery")?,
        };
        if magicka_definitions::Delivery::parse(&delivery).is_none() {
            return Err(err(format!("unknown delivery {delivery:?}")));
        }
        let target_position = match opt(table_arg(&opts), "target_position")? {
            Value::Nil => None,
            v => Some(read_vec3(&v)?),
        };
        with_state(lua, |s| {
            s.issue(Command::CastElements {
                caster,
                elements,
                delivery,
                target_position,
            })
        })?;
        Ok(Value::Nil)
    })?;
    func(lua, &t, "IsTypeable", |_, a| {
        let Value::Table(list) = arg(&a, 0) else {
            return Err(err("elements must be a list of names"));
        };
        let mut combo = Vec::new();
        for v in list.sequence_values::<String>() {
            match magicka_definitions::defs::element(&v?) {
                Some(e) => combo.push(e),
                None => return Ok(Value::Boolean(false)),
            }
        }
        Ok(Value::Boolean(
            magicka_definitions::check_combination(&combo).is_ok(),
        ))
    })?;

    // Magicks
    let t = namespace(lua, safe, "Magicks")?;
    func(lua, &t, "Invoke", |lua, a| {
        let caster = read_entity(&arg(&a, 0))?;
        let magick = read_id(&arg(&a, 1), "magick")?;
        let opts = arg(&a, 2);
        let target_position = match opt(table_arg(&opts), "target_position")? {
            Value::Nil => None,
            v => Some(read_vec3(&v)?),
        };
        with_state(lua, |s| {
            if !s.defs.magicks.contains(&magick) {
                return Err(err(format!("unknown magick {magick}")));
            }
            s.issue(Command::InvokeMagick {
                caster,
                magick,
                target_position,
            })
        })?;
        Ok(Value::Nil)
    })?;
    func(lua, &t, "Unlock", |lua, a| {
        let player = read_number(&arg(&a, 0), "player")? as usize;
        let magick = read_id(&arg(&a, 1), "magick")?;
        with_state(lua, |s| {
            if !s.defs.magicks.contains(&magick) {
                return Err(err(format!("unknown magick {magick}")));
            }
            s.issue(Command::UnlockMagick { player, magick })
        })?;
        Ok(Value::Nil)
    })?;
    func(lua, &t, "Get", |lua, a| {
        let id = read_id(&arg(&a, 0), "magick")?;
        let found = with_state(lua, |s| {
            Ok(s.defs.magicks.get(&id).map(|m| {
                json!({
                    "id": m.common.id.as_str(),
                    "combination": m.combination.iter().map(|e| magicka_definitions::element_name(*e)).collect::<Vec<_>>(),
                    "builtin": m.legacy.is_some(),
                    "cooldown": m.common.numbers.get("Cooldown"),
                })
            }))
        })?;
        match found {
            Some(j) => to_lua(lua, &j),
            None => Ok(Value::Nil),
        }
    })?;

    // Events
    let t = namespace(lua, safe, "Events")?;
    func(lua, &t, "Subscribe", |lua, a| subscribe(lua, &a, false))?;
    func(lua, &t, "AddFilter", |lua, a| subscribe(lua, &a, true))?;
    func(lua, &t, "Unsubscribe", |lua, a| {
        let token = read_number(&arg(&a, 0), "token")? as u64;
        with_state(lua, |s| {
            let owner = s.owner()?;
            let before = s.subscriptions.len();
            s.subscriptions
                .retain(|x| !(x.token == token && x.owner == owner));
            Ok(Value::Boolean(s.subscriptions.len() != before))
        })
    })?;
    func(lua, &t, "Fire", |lua, a| {
        let name = read_string(&arg(&a, 0), "event name")?;
        if FILTERS.iter().any(|f| f.name == name) || name.starts_with("engine.") {
            return Err(err(format!("{name} is an engine event")));
        }
        let payload = from_lua(&arg(&a, 1))?;
        with_state(lua, |s| {
            let owner = s.owner()?;
            s.fired.push((owner, name, payload));
            Ok(Value::Nil)
        })
    })?;

    // Timers (simulation time)
    let t = namespace(lua, safe, "Timers")?;
    func(lua, &t, "After", |lua, a| timer(lua, &a, false))?;
    func(lua, &t, "Every", |lua, a| timer(lua, &a, true))?;
    func(lua, &t, "Cancel", |lua, a| {
        let id = read_number(&arg(&a, 0), "timer")? as u64;
        with_state(lua, |s| {
            let owner = s.owner()?;
            let mine = s.timers.get(&id).is_some_and(|t| t.owner == owner);
            if mine {
                s.timers.remove(&id);
            }
            Ok(Value::Boolean(mine))
        })
    })?;

    // NetTables: server-owned replicated state for approved mod UI.
    let t = namespace(lua, safe, "NetTables")?;
    func(lua, &t, "Set", |lua, a| {
        let table = read_string(&arg(&a, 0), "table")?;
        let key = read_string(&arg(&a, 1), "key")?;
        let value = from_lua(&arg(&a, 2))?;
        if table.is_empty() || table.len() > 64 || key.len() > 128 {
            return Err(err("net table and key names must be short"));
        }
        with_state(lua, |s| {
            let owner = s.owner()?;
            if let Some((o, _)) = s.net_tables.get(&table).and_then(|t| t.get(&key)) {
                if *o != owner {
                    return Err(err(format!("{table}.{key} belongs to {}", s.owners[*o])));
                }
            }
            let used: usize = s
                .net_tables
                .values()
                .flat_map(|t| t.iter())
                .filter(|(k, (o, _))| *o == owner && !(k.as_str() == key))
                .map(|(k, (_, v))| k.len() + v.to_string().len())
                .sum();
            if used + key.len() + value.to_string().len() > s.limits.net_table_bytes_per_mod {
                return Err(err("net table size limit reached"));
            }
            let entries = s.net_tables.entry(table.clone()).or_default();
            if value.is_null() {
                entries.remove(&key);
            } else {
                entries.insert(key, (owner, value));
            }
            s.net_dirty.insert(table);
            Ok(Value::Nil)
        })
    })?;
    func(lua, &t, "Get", |lua, a| {
        let table = read_string(&arg(&a, 0), "table")?;
        let key = read_string(&arg(&a, 1), "key")?;
        let v = with_state(lua, |s| {
            Ok(s.net_tables
                .get(&table)
                .and_then(|t| t.get(&key))
                .map(|(_, v)| v.clone()))
        })?;
        match v {
            Some(v) => to_lua(lua, &v),
            None => Ok(Value::Nil),
        }
    })?;

    // Particles / Audio: presentation events.
    for (ns, sound) in [("Particles", false), ("Audio", true)] {
        let t = namespace(lua, safe, ns)?;
        let name = if sound { "Play" } else { "Create" };
        func(lua, &t, name, move |lua, a| {
            let what = read_string(&arg(&a, 0), "effect")?;
            let opts = arg(&a, 1);
            let position = match opt(table_arg(&opts), "position")? {
                Value::Nil => None,
                v => Some(read_vec3(&v)?),
            };
            let owner = match opt(table_arg(&opts), "owner")? {
                Value::Nil => None,
                v => Some(read_entity(&v)?),
            };
            let command = if sound {
                Command::Sound {
                    sound: what,
                    position,
                    owner,
                }
            } else {
                Command::Particle {
                    effect: what,
                    position,
                    owner,
                }
            };
            with_state(lua, |s| s.issue(command))?;
            Ok(Value::Nil)
        })?;
    }

    // Levels: typed native level actions (see script_schema).
    let t = namespace(lua, safe, "Levels")?;
    func(lua, &t, "Action", |lua, a| {
        let name = read_string(&arg(&a, 0), "action")?;
        let mut arguments = Vec::new();
        if let Value::Table(t) = arg(&a, 1) {
            for pair in t.pairs::<String, Value>() {
                let (k, v) = pair?;
                let text = match v {
                    Value::Boolean(b) => b.to_string(),
                    Value::Integer(i) => i.to_string(),
                    Value::Number(n) => n.to_string(),
                    Value::String(s) => s.to_str()?.to_string(),
                    _ => return Err(err(format!("argument {k} must be a scalar"))),
                };
                arguments.push((k, text));
            }
        }
        arguments.sort();
        with_state(lua, |s| s.issue(Command::LevelAction { name, arguments }))?;
        Ok(Value::Nil)
    })?;

    // The native name hash of triggers, interactables and areas
    // (Helper.GetHashCodeCustom of the lowercased name).
    func(lua, &t, "Hash", |_, a| {
        let name = read_string(&arg(&a, 0), "name")?;
        Ok(Value::Integer(i64::from(magicka_sim::hash::custom_hash(
            &name.to_ascii_lowercase(),
        ))))
    })?;
    func(lua, &t, "DefineCondition", |lua, a| {
        let name = read_string(&arg(&a, 0), "condition")?;
        let mut arguments = Vec::new();
        if let Value::Table(t) = arg(&a, 1) {
            for pair in t.pairs::<String, Value>() {
                let (k, v) = pair?;
                arguments.push((k, scalar(&v)?));
            }
        }
        arguments.sort();
        with_state(lua, |s| {
            let owner = s.owner()?;
            s.next_condition += 1;
            let id = format!("{}#{}", s.owners[owner], s.next_condition);
            s.issue(Command::DefineCondition {
                id: id.clone(),
                name,
                arguments,
            })?;
            Ok(Value::String(lua.create_string(&id)?))
        })
    })?;
    func(lua, &t, "ConditionMet", |lua, a| {
        let id = read_string(&arg(&a, 0), "condition id")?;
        with_state(lua, |s| {
            Ok(s.snapshot
                .conditions
                .get(&id)
                .map_or(Value::Nil, |b| Value::Boolean(*b)))
        })
    })?;
    func(lua, &t, "AddTrigger", |lua, a| {
        let xml = read_string(&arg(&a, 0), "trigger xml")?;
        if xml.len() > 64 << 10 {
            return Err(err("trigger xml too large"));
        }
        with_state(lua, |s| s.issue(Command::AddTrigger { xml }))?;
        Ok(Value::Nil)
    })?;
    func(lua, &t, "WatchArea", |lua, a| {
        let name = read_string(&arg(&a, 0), "area")?;
        let position = read_vec3(&arg(&a, 1))?;
        let radius = read_number(&arg(&a, 2), "radius")? as f32;
        if radius <= 0.0 {
            return Err(err("radius must be positive"));
        }
        with_state(lua, |s| {
            s.issue(Command::WatchArea {
                name,
                position,
                radius,
            })
        })?;
        Ok(Value::Nil)
    })?;
    func(lua, &t, "SpawnWave", |lua, a| {
        let name = read_string(&arg(&a, 0), "wave")?;
        let Value::Table(list) = arg(&a, 1) else {
            return Err(err("a wave is a list of {unit=, position=}"));
        };
        let mut units = Vec::new();
        for entry in list.sequence_values::<Table>() {
            let entry = entry?;
            let unit = read_string(&entry.get::<Value>("unit")?, "unit")?;
            let position = read_vec3(&entry.get::<Value>("position")?)?;
            units.push((unit, position));
        }
        if units.is_empty() || units.len() > 64 {
            return Err(err("a wave has 1..=64 units"));
        }
        with_state(lua, |s| s.issue(Command::SpawnWave { name, units }))?;
        Ok(Value::Nil)
    })?;
    func(lua, &t, "SetObjective", |lua, a| {
        let id = read_string(&arg(&a, 0), "objective")?;
        let text = read_string(&arg(&a, 1), "text")?;
        let done = matches!(arg(&a, 2), Value::Boolean(true));
        with_state(lua, |s| {
            let owner = s.owner()?;
            s.net_tables
                .entry("objectives".into())
                .or_default()
                .insert(id, (owner, json!({ "text": text, "done": done })));
            s.net_dirty.insert("objectives".into());
            Ok(Value::Nil)
        })
    })?;

    // Upgrades: per-player overlays on schema-exposed numbers. Base
    // definitions are never changed; the engine resolves each holder's
    // values through the stat resolver every tick.
    let t = namespace(lua, safe, "Upgrades")?;
    func(lua, &t, "Keys", |lua, _| {
        let out = lua.create_table()?;
        for (i, s) in magicka_definitions::ITEM_FIELDS
            .iter()
            .filter(|s| {
                s.upgradable && s.affects == magicka_definitions::schema::Affects::Simulation
            })
            .enumerate()
        {
            let k = lua.create_table()?;
            k.set("key", s.key)?;
            k.set("min", s.min)?;
            k.set("max", s.max)?;
            k.set("units", s.units)?;
            k.set(
                "integer",
                s.kind == magicka_definitions::schema::NumberKind::Int,
            )?;
            k.set("signed", s.signed)?;
            out.raw_set(i + 1, k)?;
        }
        Ok(Value::Table(out))
    })?;
    func(lua, &t, "Items", |lua, a| {
        let slot = read_number(&arg(&a, 0), "player")? as usize;
        let items = with_state(lua, |s| {
            Ok(s.snapshot
                .players
                .iter()
                .find(|p| p.slot == slot)
                .map(|p| p.items.clone())
                .unwrap_or_default())
        })?;
        to_lua(
            lua,
            &serde_json::to_value(items).map_err(|e| err(e.to_string()))?,
        )
    })?;
    func(lua, &t, "Apply", |lua, a| {
        let player = read_number(&arg(&a, 0), "player")? as usize;
        let target = read_string(&arg(&a, 1), "target")?.to_ascii_lowercase();
        let key = read_string(&arg(&a, 2), "key")?;
        let op = read_string(&arg(&a, 3), "op")?;
        let value = read_number(&arg(&a, 4), "value")?;
        let schema = magicka_definitions::schema::find(magicka_definitions::ITEM_FIELDS, &key)
            .filter(|s| s.upgradable)
            .ok_or_else(|| err(format!("{key} is not an upgradable number")))?;
        use magicka_definitions::schema::Change;
        let change = match op.as_str() {
            "add" => Change::Add(value),
            "percent" => Change::Percent(value),
            "multiply" => Change::Multiply(value),
            "set" => Change::Set(value),
            _ => return Err(err("op must be add, percent, multiply or set")),
        };
        with_state(lua, |s| {
            let owner = s.owners[s.owner()?].clone();
            s.next_condition += 1;
            let id = format!("{owner}#u{}", s.next_condition);
            s.issue(Command::Upgrade(Upgrade {
                id: id.clone(),
                owner,
                player,
                target,
                key: schema.key.to_string(),
                change,
            }))?;
            Ok(Value::String(lua.create_string(&id)?))
        })
    })?;
    func(lua, &t, "Remove", |lua, a| {
        let id = read_string(&arg(&a, 0), "upgrade id")?;
        with_state(lua, |s| s.issue(Command::RemoveUpgrade { id }))?;
        Ok(Value::Nil)
    })?;
    func(lua, &t, "List", |lua, a| {
        let player = read_number(&arg(&a, 0), "player")? as usize;
        let list: Vec<Upgrade> = with_state(lua, |s| {
            Ok(s.snapshot
                .upgrades
                .iter()
                .filter(|u| u.player == player)
                .cloned()
                .collect())
        })?;
        to_lua(
            lua,
            &serde_json::to_value(list).map_err(|e| err(e.to_string()))?,
        )
    })?;

    // Settings: a mod's own KV configuration, read through the mod
    // filesystem from authoritative paths (part of the rules hash).
    let t = namespace(lua, safe, "Settings")?;
    func(lua, &t, "Load", |lua, a| {
        let rel = read_string(&arg(&a, 0), "settings path")?;
        let path = magicka_mods::vfs::LogicalPath::new(&format!("scripts/{rel}"))
            .map_err(|e| err(e.to_string()))?;
        if !magicka_mods::is_authoritative(&path) {
            return Err(err(
                "settings live under scripts/npc, scripts/vscripts or scripts/levels",
            ));
        }
        let text = with_state(lua, |s| {
            s.set
                .vfs
                .read_string(&path)
                .ok_or_else(|| err(format!("{path} not found")))?
                .map_err(err)
        })?;
        let doc = magicka_mods::kv::parse(path.as_str(), &text).map_err(|e| err(e.to_string()))?;
        fn node(n: &magicka_mods::kv::KvNode) -> serde_json::Value {
            match &n.value {
                magicka_mods::kv::KvValue::Str(s) => serde_json::Value::String(s.clone()),
                magicka_mods::kv::KvValue::Block(children) => serde_json::Value::Object(
                    children.iter().map(|c| (c.key.clone(), node(c))).collect(),
                ),
            }
        }
        let root: serde_json::Map<String, serde_json::Value> =
            doc.nodes.iter().map(|n| (n.key.clone(), node(n))).collect();
        to_lua(lua, &serde_json::Value::Object(root))
    })?;

    // Localization
    let t = namespace(lua, safe, "Localization")?;
    func(lua, &t, "Get", |lua, a| {
        let key = read_string(&arg(&a, 0), "key")?;
        let language = match arg(&a, 1) {
            Value::Nil => "english".to_string(),
            v => read_string(&v, "language")?,
        };
        let text = with_state(lua, |s| {
            Ok(s.defs
                .localization
                .lookup(&language, &key)
                .map(str::to_string))
        })?;
        match text {
            Some(t) => Ok(Value::String(lua.create_string(t)?)),
            None => Ok(Value::Nil),
        }
    })?;

    // Random: the server's seeded gameplay RNG.
    let t = namespace(lua, safe, "Random")?;
    let r = random.clone();
    t.set(
        "Int",
        lua.create_function(move |_, a: Args| {
            let v = args(a);
            r.call::<Value>((arg(&v, 0), arg(&v, 1)))
        })?,
    )?;
    let r = random;
    t.set(
        "Float",
        lua.create_function(move |_, _: Args| r.call::<Value>(()))?,
    )?;
    Ok(())
}
