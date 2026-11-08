//! Mod world operations: Lua arguments (JSON) → `WorldOp` with resolved,
//! generation-checked entities. The routines are in
//! `scene_world::script_ops`.
use crate::scene_world::script_ops::{NativeKind, Weather, WorldOp, STATUSES};
use magicka_script::EntityRef;
use magicka_sim::damage::{attack, Damage};
use magicka_sim::spells::{Elements, Spell};
use serde_json::Value as Json;

type Resolve<'a> = &'a dyn Fn(EntityRef) -> Option<u16>;

struct Args<'a> {
    op: &'a str,
    values: &'a [Json],
    resolve: Resolve<'a>,
}

impl Args<'_> {
    fn at(&self, i: usize) -> &Json {
        self.values.get(i).unwrap_or(&Json::Null)
    }
    fn fail(&self, what: impl std::fmt::Display) -> String {
        format!("{}: {what}", self.op)
    }
    fn entity_of(&self, v: &Json, what: &str) -> Result<u16, String> {
        let handle = v.get("__entity").and_then(Json::as_u64);
        let generation = v.get("generation").and_then(Json::as_u64);
        let (Some(handle), Some(generation)) = (handle, generation) else {
            return Err(self.fail(format!("{what} must be an entity")));
        };
        (self.resolve)(EntityRef {
            handle: handle as u16,
            generation: generation as u32,
        })
        .ok_or_else(|| self.fail(format!("{what} is a stale entity")))
    }
    fn entity(&self, i: usize, what: &str) -> Result<u16, String> {
        self.entity_of(self.at(i), what)
    }
    fn opt_entity_of(&self, v: &Json, what: &str) -> Result<Option<u16>, String> {
        if v.is_null() {
            Ok(None)
        } else {
            self.entity_of(v, what).map(Some)
        }
    }
    fn number_of(&self, v: &Json, what: &str) -> Result<f32, String> {
        v.as_f64()
            .map(|n| n as f32)
            .filter(|n| n.is_finite())
            .ok_or_else(|| self.fail(format!("{what} must be a number")))
    }
    fn opt_number_of(&self, v: &Json, what: &str) -> Result<Option<f32>, String> {
        if v.is_null() {
            Ok(None)
        } else {
            self.number_of(v, what).map(Some)
        }
    }
    fn vec3_of(&self, v: &Json, what: &str) -> Result<[f32; 3], String> {
        let get = |k: &str, i: usize| {
            v.get(k)
                .or_else(|| v.get(i))
                .and_then(Json::as_f64)
                .map(|n| n as f32)
        };
        match (get("x", 0), get("y", 1), get("z", 2)) {
            (Some(x), Some(y), Some(z)) if [x, y, z].iter().all(|n| n.is_finite()) => Ok([x, y, z]),
            _ => Err(self.fail(format!("{what} must be a position {{x, y, z}}"))),
        }
    }
    fn opt_vec3_of(&self, v: &Json, what: &str) -> Result<Option<[f32; 3]>, String> {
        if v.is_null() {
            Ok(None)
        } else {
            self.vec3_of(v, what).map(Some)
        }
    }
    fn string_of(&self, v: &Json, what: &str) -> Result<String, String> {
        v.as_str()
            .map(str::to_string)
            .ok_or_else(|| self.fail(format!("{what} must be a string")))
    }
    /// Field `key` of the options table at `i` (Null when absent).
    fn opt(&self, i: usize, key: &str) -> &Json {
        self.at(i).get(key).unwrap_or(&Json::Null)
    }
    /// An entity or a position.
    fn place(&self, v: &Json, what: &str) -> Result<Place, String> {
        if v.get("__entity").is_some() {
            self.entity_of(v, what).map(Place::Entity)
        } else {
            self.vec3_of(v, what).map(Place::Position)
        }
    }
}

enum Place {
    Entity(u16),
    Position([f32; 3]),
}

/// `{fire = 5, earth = 5}` or `{"fire", "fire"}` → a spell.
fn spell(args: &Args, v: &Json) -> Result<Spell, String> {
    let mut spell = Spell {
        element: Elements::NONE,
        mag: [0.0; 11],
        magick: Default::default(),
    };
    let mut add = |name: &str, magnitude: f32| -> Result<(), String> {
        let e = magicka_definitions::defs::element(name)
            .ok_or_else(|| args.fail(format!("unknown element {name:?}")))?;
        spell.element = Elements(spell.element.0 | e.0);
        spell.mag[e.index() as usize] += magnitude;
        Ok(())
    };
    match v {
        Json::Object(map) => {
            for (name, m) in map {
                add(name, args.number_of(m, "an element magnitude")?)?;
            }
        }
        Json::Array(list) => {
            for name in list {
                add(&args.string_of(name, "an element")?, 1.0)?;
            }
        }
        _ => return Err(args.fail("elements must be {name = magnitude} or a list of names")),
    }
    if spell.element == Elements::NONE {
        return Err(args.fail("a spell needs at least one element"));
    }
    Ok(spell)
}

fn attack_kind(args: &Args, name: &str) -> Result<u16, String> {
    let mut bits = 0;
    for part in name.split(['|', ',', ' ']).filter(|p| !p.is_empty()) {
        bits |= match part.to_ascii_lowercase().as_str() {
            "damage" => attack::DAMAGE,
            "knockdown" => attack::KNOCKDOWN,
            "pushed" | "push" => attack::PUSHED,
            "knockback" => attack::KNOCKBACK,
            "piercing" => attack::PIERCING,
            "armour_piercing" | "armor_piercing" => attack::ARMOUR_PIERCING,
            "status" => attack::STATUS,
            "entanglement" => attack::ENTANGLEMENT,
            "stun" => attack::STUN,
            "bleed" => attack::BLEED,
            other => return Err(args.fail(format!("unknown attack {other:?}"))),
        };
    }
    Ok(bits)
}

/// Parse one queued operation.
pub fn parse(op: &str, values: &[Json], resolve: Resolve) -> Result<WorldOp, String> {
    let a = Args {
        op,
        values,
        resolve,
    };
    let (ns, name) = op.split_once('.').unwrap_or(("", op));
    Ok(match (ns, name) {
        ("Spells", "Hit") => {
            let d = a.at(1);
            let element = match d.get("element") {
                None | Some(Json::Null) => Elements::NONE,
                Some(v) => {
                    let n = a.string_of(v, "element")?;
                    magicka_definitions::defs::element(&n)
                        .ok_or_else(|| a.fail(format!("unknown element {n:?}")))?
                }
            };
            let attack = match d.get("attack") {
                None | Some(Json::Null) => attack::DAMAGE,
                Some(v) => attack_kind(&a, &a.string_of(v, "attack")?)?,
            };
            WorldOp::Hit {
                target: a.entity(0, "target")?,
                damage: Damage {
                    attack,
                    element: element.0,
                    amount: a.number_of(d.get("amount").unwrap_or(&Json::Null), "amount")?,
                    magnitude: a
                        .opt_number_of(d.get("magnitude").unwrap_or(&Json::Null), "magnitude")?
                        .unwrap_or(1.0),
                },
                source: a.opt_entity_of(a.opt(2, "source"), "source")?,
                from: a.opt_vec3_of(a.opt(2, "from"), "from")?,
            }
        }
        ("Spells", "Status") => {
            let name = a.string_of(a.at(1), "status")?.to_ascii_lowercase();
            WorldOp::Status {
                target: a.entity(0, "target")?,
                status: STATUSES
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, s)| *s)
                    .ok_or_else(|| a.fail(format!("unknown status {name:?}")))?,
                dps: a.opt_number_of(a.opt(2, "dps"), "dps")?.unwrap_or(0.0),
                magnitude: a
                    .opt_number_of(a.opt(2, "magnitude"), "magnitude")?
                    .unwrap_or(1.0),
            }
        }
        ("Spells", "Projectile") => WorldOp::Projectile {
            owner: a.opt_entity_of(a.at(0), "owner")?,
            spell: spell(&a, a.opt(1, "elements"))?,
            position: a.vec3_of(a.opt(1, "position"), "position")?,
            velocity: a.vec3_of(a.opt(1, "velocity"), "velocity")?,
            splash: a
                .opt_number_of(a.opt(1, "splash"), "splash")?
                .unwrap_or(0.0),
        },
        ("Spells", "Lightning") => WorldOp::Lightning {
            owner: a.entity(0, "owner")?,
            target: a.opt_entity_of(a.opt(1, "target"), "target")?,
            from: a.opt_vec3_of(a.opt(1, "from"), "from")?,
            direction: a.opt_vec3_of(a.opt(1, "direction"), "direction")?,
            damage: a
                .opt_number_of(a.opt(1, "damage"), "damage")?
                .unwrap_or(850.0),
            range: a.opt_number_of(a.opt(1, "range"), "range")?.unwrap_or(15.0),
            hit_owner: a.opt(1, "hit_owner").as_bool().unwrap_or(false),
        },
        ("Spells", "Thunderbolt") => WorldOp::Thunderbolt {
            owner: a.opt_entity_of(a.at(0), "owner")?,
            position: a.vec3_of(a.at(1), "position")?,
            direction: a.vec3_of(a.at(2), "direction")?,
        },
        ("Spells", "Strike") => WorldOp::Strike {
            owner: a.opt_entity_of(a.at(0), "owner")?,
            position: a.vec3_of(a.at(1), "position")?,
        },
        ("Spells", "Push") => WorldOp::Push {
            center: a.vec3_of(a.at(0), "center")?,
            owner: a.opt_entity_of(a.opt(1, "owner"), "owner")?,
            pull: a.opt(1, "pull").as_bool().unwrap_or(false),
        },
        ("Spells", "GreaseField") => WorldOp::GreaseField {
            position: a.vec3_of(a.at(0), "position")?,
            owner: a.opt_entity_of(a.at(1), "owner")?,
        },
        ("Entities", "SetInvisible") => WorldOp::SetInvisible {
            target: a.entity(0, "target")?,
            seconds: a.number_of(a.at(1), "seconds")?,
        },
        ("Entities", "SetEthereal") => WorldOp::SetEthereal {
            target: a.entity(0, "target")?,
            ethereal: a
                .at(1)
                .as_bool()
                .ok_or_else(|| a.fail("ethereal must be a boolean"))?,
        },
        ("Entities", "Levitate") => WorldOp::Levitate {
            target: a.entity(0, "target")?,
        },
        ("Entities", "Haste") => WorldOp::Haste {
            target: a.entity(0, "target")?,
            seconds: a.opt_number_of(a.at(1), "seconds")?,
        },
        ("Entities", "Resize") => WorldOp::Resize {
            target: a.entity(0, "target")?,
            grow: match a.string_of(a.at(1), "size")?.to_ascii_lowercase().as_str() {
                "grow" => true,
                "shrink" => false,
                other => return Err(a.fail(format!("size {other:?} (grow or shrink)"))),
            },
        },
        ("Entities", "Charm") => WorldOp::Charm {
            target: a.entity(0, "target")?,
            owner: a.entity(1, "owner")?,
            seconds: a
                .opt_number_of(a.at(2), "seconds")?
                .unwrap_or(crate::scene_world::CHARM_TIME),
        },
        ("Entities", "Fear") => WorldOp::Fear {
            target: a.entity(0, "target")?,
            source: match a.place(a.at(1), "source")? {
                Place::Entity(h) => magicka_sim::fear::FearSource::Character(h),
                Place::Position(p) => magicka_sim::fear::FearSource::Position(p),
            },
        },
        ("Entities", "Confuse") => WorldOp::Confuse {
            target: a.entity(0, "target")?,
            from: match a.place(a.at(1), "from")? {
                Place::Entity(h) => Err(h),
                Place::Position(p) => Ok(p),
            },
        },
        ("Entities", "Polymorph") => WorldOp::Polymorph {
            target: a.entity(0, "target")?,
        },
        ("Entities", "Terminate") => WorldOp::Terminate {
            target: a.entity(0, "target")?,
        },
        ("Entities", "Revive") => WorldOp::Revive {
            position: a.vec3_of(a.at(0), "position")?,
            from: a.opt_vec3_of(a.opt(1, "from"), "from")?,
            health: a
                .opt_number_of(a.opt(1, "health"), "health")?
                .unwrap_or(magicka_sim::magicks::REVIVE_CASTER_HEALTH),
            slot: a.opt(1, "slot").as_u64().map(|s| s as usize),
        },
        ("Entities", "Blink") => WorldOp::Blink {
            target: a.entity(0, "target")?,
            position: a.vec3_of(a.at(1), "position")?,
        },
        ("Entities", "Summon") => WorldOp::Summon {
            unit: a.string_of(a.at(0), "unit")?,
            position: a.vec3_of(a.at(1), "position")?,
            owner: a.opt_entity_of(a.opt(2, "owner"), "owner")?,
            undead: a.opt(2, "undead").as_bool().unwrap_or(false),
        },
        ("Entities", "SpawnNative") => {
            let kind = a.string_of(a.at(0), "kind")?;
            WorldOp::SpawnNative {
                kind: NativeKind::parse(&kind).ok_or_else(|| {
                    let kinds: Vec<_> = NativeKind::ALL.iter().map(|(n, _)| *n).collect();
                    a.fail(format!(
                        "unknown kind {kind:?} (kinds: {})",
                        kinds.join(", ")
                    ))
                })?,
                owner: a.entity(1, "owner")?,
            }
        }
        ("World", "SetLight") => WorldOp::SetLight {
            intensity: a.number_of(a.at(0), "intensity")?,
        },
        ("World", "Flash") => WorldOp::Flash {
            seconds: a.number_of(a.at(0), "seconds")?,
        },
        ("World", "Shake") => WorldOp::Shake {
            magnitude: a.number_of(a.at(0), "magnitude")?,
            seconds: a.number_of(a.at(1), "seconds")?,
        },
        ("World", "Cue") => WorldOp::Cue {
            name: a.string_of(a.at(0), "cue")?,
            position: a.opt_vec3_of(a.opt(1, "position"), "position")?,
            bank: a
                .opt_number_of(a.opt(1, "bank"), "bank")?
                .map_or(16, |b| b as u16),
        },
        ("World", "Effect") => WorldOp::Effect {
            name: a.string_of(a.at(0), "effect")?,
            position: a.vec3_of(a.at(1), "position")?,
            direction: a
                .opt_vec3_of(a.at(2), "direction")?
                .unwrap_or([0.0, 0.0, 1.0]),
        },
        ("World", "Weather") => WorldOp::Weather {
            kind: match a
                .string_of(a.at(0), "weather")?
                .to_ascii_lowercase()
                .as_str()
            {
                "rain" => Weather::Rain,
                "blizzard" => Weather::Blizzard,
                other => return Err(a.fail(format!("unknown weather {other:?}"))),
            },
            owner: a.opt_entity_of(a.opt(1, "owner"), "owner")?,
        },
        ("World", "TimeWarp") => WorldOp::TimeWarp {
            owner: a.opt_entity_of(a.at(0), "owner")?,
        },
        ("World", "Nullify") => WorldOp::Nullify,
        ("World", "Corporealize") => WorldOp::Corporealize {
            position: a.vec3_of(a.at(0), "position")?,
        },
        _ => return Err(format!("unknown world operation {op}")),
    })
}
