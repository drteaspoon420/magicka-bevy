//! `Magicka.ContentReaders.CharacterTemplateReader` ->
//! `Entities.CharacterTemplate.Read` (`Data/Characters/**`).

use crate::common::*;

/// Number of animation-set slots (`new AnimationClipAction[27][]`).
pub const ANIMATION_SETS: usize = 27;

/// The 27 animation sets shared by `CharacterTemplate.Read` and
/// `PhysicsEntityTemplate.Read`: per set an Int32 count (non-positive = no
/// set) of `animation:string` + `AnimationClipAction`.
pub fn read_animation_sets(r: &mut Reader) -> Result<Vec<Vec<AnimationClipAction>>> {
    let mut sets = Vec::with_capacity(ANIMATION_SETS);
    for _ in 0..ANIMATION_SETS {
        let n = r.i32()?.max(0);
        let mut set = Vec::new();
        for _ in 0..n {
            let animation = r.string()?;
            let clip = r.string()?;
            let speed = r.f32()?;
            let blend_time = r.f32()?;
            let looping = r.bool()?;
            let k = r.count()?;
            let actions = (0..k)
                .map(|_| AnimationAction::read(r))
                .collect::<Result<_>>()?;
            set.push(AnimationClipAction {
                animation,
                clip,
                speed,
                blend_time,
                looping,
                actions,
            });
        }
        sets.push(set);
    }
    Ok(sets)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gib {
    pub model: Option<String>,
    pub mass: f32,
    pub scale: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JointLight {
    pub joint: String,
    pub light: PointLight,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelProperties {
    pub model: Option<String>,
    pub scale: f32,
    pub tint: [f32; 3],
}

/// `AnimationClipAction(Animations, ContentReader, clips, skeleton)`.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationClipAction {
    /// `Animations` enum name (parsed case-insensitively by the game).
    pub animation: String,
    /// Clip key in the skinned model's animation dictionary.
    pub clip: String,
    pub speed: f32,
    pub blend_time: f32,
    pub looping: bool,
    pub actions: Vec<AnimationAction>,
}

/// `AnimationActions.AnimationAction.Read`: type name, start/end time, body.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationAction {
    pub start_time: f32,
    pub end_time: f32,
    pub kind: ActionKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ActionKind {
    Block {
        weapon: i32,
    },
    BreakFree {
        magnitude: f32,
        weapon: i32,
    },
    CameraShake {
        unused: String,
        duration: f32,
        magnitude: f32,
    },
    CastSpell {
        from_staff: bool,
        source: Option<String>,
    },
    Crouch {
        radius: f32,
        length: f32,
    },
    DamageGrip {
        damage_to_owner: bool,
        damages: Vec<Damage>,
    },
    DealDamage {
        weapon: i32,
        target: u8,
    },
    DetachItem {
        item: i32,
        velocity: [f32; 3],
    },
    Ethereal {
        ethereal: bool,
        alpha: f32,
        speed: f32,
    },
    Footstep,
    Grip {
        grip_type: u8,
        radius: f32,
        break_free_tolerance: f32,
        grip_attach: String,
        target_attach: String,
        finish_on_grip: bool,
    },
    Gunfire {
        weapon: i32,
        accuracy: f32,
    },
    Immortal {
        collide: bool,
    },
    Invisible {
        no_effect: bool,
    },
    Jump {
        elevation_deg: f32,
        min_range: Option<f32>,
        max_range: Option<f32>,
    },
    Move {
        velocity: [f32; 3],
    },
    OverkillGrip,
    PlayEffect {
        bone: String,
        attach: bool,
        effect: String,
        unused: f32,
    },
    PlaySound {
        sound: String,
        bank: i32,
    },
    ReleaseGrip,
    RemoveStatus {
        status: String,
    },
    SetItemAttach {
        item: i32,
        bone: String,
    },
    SpawnMissile {
        weapon: i32,
        velocity: [f32; 3],
        item_aligned: bool,
    },
    SpecialAbility {
        weapon: i32,
        ability: Option<SpecialAbility>,
    },
    Suicide {
        overkill: bool,
    },
    ThrowGrip,
    Tongue {
        max_length: f32,
    },
    WeaponVisibility {
        weapon: i32,
        visible: bool,
    },
}

impl AnimationAction {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let name = r.string()?;
        let start_time = r.f32()?;
        let end_time = r.f32()?;
        // Type lookup is a case-insensitive match on the subclass name.
        let kind = match name.to_ascii_lowercase().as_str() {
            "block" => ActionKind::Block { weapon: r.i32()? },
            "breakfree" => ActionKind::BreakFree {
                magnitude: r.f32()?,
                weapon: r.i32()?,
            },
            "camerashake" => ActionKind::CameraShake {
                unused: r.string()?,
                duration: r.f32()?,
                magnitude: r.f32()?,
            },
            "castspell" => {
                let from_staff = r.bool()?;
                let source = if from_staff { None } else { Some(r.string()?) };
                ActionKind::CastSpell { from_staff, source }
            }
            "crouch" => ActionKind::Crouch {
                radius: r.f32()?,
                length: r.f32()?,
            },
            "damagegrip" => {
                let damage_to_owner = r.bool()?;
                let n = r.count()?;
                if n > 5 {
                    return invalid("DamageGrip: too many damages (max 5)");
                }
                let damages = (0..n).map(|_| Damage::read(r)).collect::<Result<_>>()?;
                ActionKind::DamageGrip {
                    damage_to_owner,
                    damages,
                }
            }
            "dealdamage" => ActionKind::DealDamage {
                weapon: r.i32()?,
                target: r.u8()?,
            },
            "detachitem" => ActionKind::DetachItem {
                item: r.i32()?,
                velocity: r.vec3()?,
            },
            "ethereal" => ActionKind::Ethereal {
                ethereal: r.bool()?,
                alpha: r.f32()?,
                speed: r.f32()?,
            },
            "footstep" => ActionKind::Footstep,
            "grip" => ActionKind::Grip {
                grip_type: r.u8()?,
                radius: r.f32()?,
                break_free_tolerance: r.f32()?,
                grip_attach: r.string()?,
                target_attach: r.string()?,
                finish_on_grip: r.bool()?,
            },
            "gunfire" => ActionKind::Gunfire {
                weapon: r.i32()?,
                accuracy: r.f32()?,
            },
            "immortal" => ActionKind::Immortal { collide: r.bool()? },
            "invisible" => ActionKind::Invisible {
                no_effect: r.bool()?,
            },
            "jump" => {
                let elevation_deg = r.f32()?;
                let min_range = if r.bool()? { Some(r.f32()?) } else { None };
                let max_range = if r.bool()? { Some(r.f32()?) } else { None };
                ActionKind::Jump {
                    elevation_deg,
                    min_range,
                    max_range,
                }
            }
            "move" => ActionKind::Move {
                velocity: r.vec3()?,
            },
            "overkillgrip" => ActionKind::OverkillGrip,
            "playeffect" => ActionKind::PlayEffect {
                bone: r.string()?,
                attach: r.bool()?,
                effect: r.string()?,
                unused: r.f32()?,
            },
            "playsound" => ActionKind::PlaySound {
                sound: r.string()?,
                bank: r.i32()?,
            },
            "releasegrip" => ActionKind::ReleaseGrip,
            "removestatus" => ActionKind::RemoveStatus {
                status: r.string()?,
            },
            "setitemattach" => ActionKind::SetItemAttach {
                item: r.i32()?,
                bone: r.string()?,
            },
            "spawnmissile" => ActionKind::SpawnMissile {
                weapon: r.i32()?,
                velocity: r.vec3()?,
                item_aligned: r.bool()?,
            },
            "specialability" => {
                let weapon = r.i32()?;
                let ability = if weapon < 0 {
                    Some(SpecialAbility::read(r)?)
                } else {
                    None
                };
                ActionKind::SpecialAbility { weapon, ability }
            }
            "suicide" => ActionKind::Suicide {
                overkill: r.bool()?,
            },
            "throwgrip" => ActionKind::ThrowGrip,
            "tongue" => ActionKind::Tongue {
                max_length: r.f32()?,
            },
            "weaponvisibility" => ActionKind::WeaponVisibility {
                weapon: r.i32()?,
                visible: r.bool()?,
            },
            _ => return invalid(format!("Invalid AnimationAction \"{name}\"")),
        };
        Ok(AnimationAction {
            start_time,
            end_time,
            kind,
        })
    }
}

/// `Abilities.Ability.Read`: type name, base fields, subclass body.
#[derive(Debug, Clone, PartialEq)]
pub struct Ability {
    pub cooldown: f32,
    pub target: u8,
    /// Fuzzy-logic expression source (`Expression.Read`), if present.
    pub fuzzy_expression: Option<String>,
    pub animation_keys: Vec<String>,
    pub kind: AbilityKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AbilityKind {
    Block {
        arc: f32,
        shield: i32,
    },
    CastSpell {
        min_range: f32,
        max_range: f32,
        arc: f32,
        chant_speed: f32,
        power: f32,
        cast_type: i32,
        elements: Vec<i32>,
    },
    ConfuseGrip,
    DamageGrip,
    Dash {
        min_range: f32,
        max_range: f32,
        arc: f32,
        vector: [f32; 3],
    },
    ElementSteal {
        range: f32,
        angle: f32,
    },
    GripCharacterFromBehind {
        max_range: f32,
        min_range: f32,
        angle: f32,
        max_weight: f32,
    },
    Jump {
        max_range: f32,
        min_range: f32,
        angle: f32,
        elevation: f32,
    },
    Melee {
        min_range: f32,
        max_range: f32,
        arc: f32,
        weapons: Vec<i32>,
        rotate: bool,
    },
    PickUpCharacter {
        max_range: f32,
        min_range: f32,
        angle: f32,
        max_weight: f32,
        drop_animation: String,
    },
    /// `arc` is read then overwritten with PI by the original.
    Ranged {
        min_range: f32,
        max_range: f32,
        elevation: f32,
        arc_as_stored: f32,
        accuracy: f32,
        weapons: Vec<i32>,
    },
    RemoveStatus,
    SpecialAbilityAbility {
        max_range: f32,
        min_range: f32,
        angle: f32,
        weapon: i32,
    },
    /// Only `min(count, 4)` damages are read (preserved quirk).
    ThrowGrip {
        max_range: f32,
        min_range: f32,
        elevation: f32,
        damages: Vec<Damage>,
    },
    ZombieGrip {
        max_range: f32,
        min_range: f32,
        angle: f32,
        max_weight: f32,
        drop_animation: String,
    },
}

impl Ability {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let name = r.string()?;
        let cooldown = r.f32()?;
        let target = r.u8()?;
        let fuzzy_expression = if r.bool()? { Some(r.string()?) } else { None };
        let animation_keys = r.strings()?;
        let kind = match name.to_ascii_lowercase().as_str() {
            "block" => AbilityKind::Block {
                arc: r.f32()?,
                shield: r.i32()?,
            },
            "castspell" => {
                let (min_range, max_range, arc, chant_speed, power) =
                    (r.f32()?, r.f32()?, r.f32()?, r.f32()?, r.f32()?);
                let cast_type = r.i32()?;
                let n = r.count()?;
                let elements = (0..n).map(|_| r.i32()).collect::<Result<_>>()?;
                AbilityKind::CastSpell {
                    min_range,
                    max_range,
                    arc,
                    chant_speed,
                    power,
                    cast_type,
                    elements,
                }
            }
            "confusegrip" => AbilityKind::ConfuseGrip,
            "damagegrip" => AbilityKind::DamageGrip,
            "dash" => AbilityKind::Dash {
                min_range: r.f32()?,
                max_range: r.f32()?,
                arc: r.f32()?,
                vector: r.vec3()?,
            },
            "elementsteal" => AbilityKind::ElementSteal {
                range: r.f32()?,
                angle: r.f32()?,
            },
            "gripcharacterfrombehind" => AbilityKind::GripCharacterFromBehind {
                max_range: r.f32()?,
                min_range: r.f32()?,
                angle: r.f32()?,
                max_weight: r.f32()?,
            },
            "jump" => AbilityKind::Jump {
                max_range: r.f32()?,
                min_range: r.f32()?,
                angle: r.f32()?,
                elevation: r.f32()?,
            },
            "melee" => {
                let (min_range, max_range, arc) = (r.f32()?, r.f32()?, r.f32()?);
                let n = r.count()?;
                let weapons = (0..n).map(|_| r.i32()).collect::<Result<_>>()?;
                AbilityKind::Melee {
                    min_range,
                    max_range,
                    arc,
                    weapons,
                    rotate: r.bool()?,
                }
            }
            "pickupcharacter" => AbilityKind::PickUpCharacter {
                max_range: r.f32()?,
                min_range: r.f32()?,
                angle: r.f32()?,
                max_weight: r.f32()?,
                drop_animation: r.string()?,
            },
            "ranged" => {
                let (min_range, max_range, elevation, arc_as_stored, accuracy) =
                    (r.f32()?, r.f32()?, r.f32()?, r.f32()?, r.f32()?);
                let n = r.count()?;
                let weapons = (0..n).map(|_| r.i32()).collect::<Result<_>>()?;
                AbilityKind::Ranged {
                    min_range,
                    max_range,
                    elevation,
                    arc_as_stored,
                    accuracy,
                    weapons,
                }
            }
            "removestatus" => AbilityKind::RemoveStatus,
            "specialabilityability" => AbilityKind::SpecialAbilityAbility {
                max_range: r.f32()?,
                min_range: r.f32()?,
                angle: r.f32()?,
                weapon: r.i32()?,
            },
            "throwgrip" => {
                let (max_range, min_range, elevation) = (r.f32()?, r.f32()?, r.f32()?);
                let n = r.i32()?.clamp(0, 4) as usize;
                let damages = (0..n)
                    .map(|_| Damage::read_int_amount(r))
                    .collect::<Result<_>>()?;
                AbilityKind::ThrowGrip {
                    max_range,
                    min_range,
                    elevation,
                    damages,
                }
            }
            "zombiegrip" => AbilityKind::ZombieGrip {
                max_range: r.f32()?,
                min_range: r.f32()?,
                angle: r.f32()?,
                max_weight: r.f32()?,
                drop_animation: r.string()?,
            },
            _ => return invalid(format!("unknown Ability type \"{name}\"")),
        };
        Ok(Ability {
            cooldown,
            target,
            fuzzy_expression,
            animation_keys,
            kind,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CharacterTemplate {
    pub id: String,
    pub display_id: String,
    pub faction: i32,
    pub blood: i32,
    pub is_ethereal: bool,
    pub looks_ethereal: bool,
    pub fearless: bool,
    pub uncharmable: bool,
    pub nonslippery: bool,
    pub has_fairy: bool,
    pub can_see_invisible: bool,
    /// Declared count; only the first 4 entries are read (see `sounds`).
    pub attached_sound_count: i32,
    pub attached_sounds: Vec<(String, i32)>,
    pub gibs: Vec<Gib>,
    pub lights: Vec<JointLight>,
    pub max_hitpoints: f32,
    /// As stored; the game clamps values <= 0 to 1.
    pub number_of_health_bars: i32,
    pub undying: bool,
    pub undie_time: f32,
    pub undie_hitpoints: f32,
    pub hit_tolerance: i32,
    pub knockdown_tolerance: f32,
    pub score_value: i32,
    pub experience_value: i32,
    pub reward_on_kill: bool,
    pub reward_on_overkill: bool,
    pub regeneration: i32,
    pub max_panic: f32,
    pub zap_modifier: f32,
    /// As stored; the game uses `max(length, 0.01)`.
    pub length: f32,
    pub radius: f32,
    pub mass: f32,
    pub speed: f32,
    pub turn_speed: f32,
    pub bleed_rate: f32,
    pub stun_time: f32,
    pub summon_element_bank: i32,
    pub summon_element_cue: String,
    /// Sparse overrides as stored. Non-FRIENDLY runtime tables scale nonnegative
    /// values by `1 - (playerCount - 1) * 0.00666`; negative values divide by it.
    pub resistances: Vec<Resistance>,
    pub models: Vec<ModelProperties>,
    pub skeleton_model: Option<String>,
    pub attached_effects: Vec<(String, String)>,
    /// 27 animation sets; each is the list of clip actions defined for it.
    pub animation_sets: Vec<Vec<AnimationClipAction>>,
    pub equipment: Vec<(i32, Attachment)>,
    pub event_conditions: ConditionCollection,
    pub alert_radius: f32,
    pub group_chase: f32,
    pub group_separation: f32,
    pub group_cohesion: f32,
    pub group_alignment: f32,
    pub group_wander: f32,
    pub friendly_avoidance: f32,
    pub enemy_avoidance: f32,
    pub sight_avoidance: f32,
    pub danger_avoidance: f32,
    pub anger_weight: f32,
    pub distance_weight: f32,
    pub health_weight: f32,
    pub flocking: bool,
    pub break_free_strength: f32,
    pub abilities: Vec<Ability>,
    pub move_animations: Vec<(u8, Vec<String>)>,
    pub buffs: Vec<BuffStorage>,
    pub auras: Vec<AuraStorage>,
}

impl CharacterTemplate {
    pub fn read(r: &mut Reader) -> Result<Self> {
        let id = r.string()?;
        let display_id = r.string()?;
        let faction = r.i32()?;
        let blood = r.i32()?;
        let flags: Vec<bool> = (0..7).map(|_| r.bool()).collect::<Result<_>>()?;
        let attached_sound_count = r.i32()?;
        let mut attached_sounds = Vec::new();
        // Quirk: entries beyond the 4th are skipped *without reading*.
        for i in 0..attached_sound_count.max(0) {
            if i < 4 {
                attached_sounds.push((r.string()?, r.i32()?));
            }
        }
        let n = r.count()?;
        let gibs = (0..n)
            .map(|_| {
                Ok(Gib {
                    model: r.external()?,
                    mass: r.f32()?,
                    scale: r.f32()?,
                })
            })
            .collect::<Result<_>>()?;
        let n = r.count()?;
        if n > 4 {
            return invalid("Character may Only have Four Lights!");
        }
        let lights = (0..n)
            .map(|_| {
                Ok(JointLight {
                    joint: r.string()?,
                    light: PointLight::read(r)?,
                })
            })
            .collect::<Result<_>>()?;
        let max_hitpoints = r.f32()?;
        let number_of_health_bars = r.i32()?;
        let undying = r.bool()?;
        let undie_time = r.f32()?;
        let undie_hitpoints = r.f32()?;
        let hit_tolerance = r.i32()?;
        let knockdown_tolerance = r.f32()?;
        let score_value = r.i32()?;
        let experience_value = r.i32()?;
        let reward_on_kill = r.bool()?;
        let reward_on_overkill = r.bool()?;
        let regeneration = r.i32()?;
        let max_panic = r.f32()?;
        let zap_modifier = r.f32()?;
        let length = r.f32()?;
        let radius = r.f32()?;
        let mass = r.f32()?;
        let speed = r.f32()?;
        let turn_speed = r.f32()?;
        let bleed_rate = r.f32()?;
        let stun_time = r.f32()?;
        let summon_element_bank = r.i32()?;
        let summon_element_cue = r.string()?;
        let n = r.count()?;
        let resistances = (0..n).map(|_| Resistance::read(r)).collect::<Result<_>>()?;
        let n = r.count()?;
        let models = (0..n)
            .map(|_| {
                Ok(ModelProperties {
                    model: r.external()?,
                    scale: r.f32()?,
                    tint: r.vec3()?,
                })
            })
            .collect::<Result<_>>()?;
        let skeleton_model = r.external()?;
        let n = r.count()?;
        let attached_effects = (0..n)
            .map(|_| Ok((r.string()?, r.string()?)))
            .collect::<Result<_>>()?;
        let animation_sets = read_animation_sets(r)?;
        let n = r.count()?;
        let mut equipment: Vec<(i32, Attachment)> = Vec::with_capacity(n);
        for _ in 0..n {
            let slot = r.i32()?;
            let attachment = Attachment::read(r)?;
            // CharacterTemplate reads into mEquipment[slot]: a repeated slot
            // replaces the earlier entry (Firedrake lists spider fangs, then
            // firedrake_puke, both in slot 0).
            if let Some(entry) = equipment.iter_mut().find(|(s, _)| *s == slot) {
                entry.1 = attachment;
            } else {
                equipment.push((slot, attachment));
            }
        }
        let event_conditions = ConditionCollection::read(r)?;
        Ok(CharacterTemplate {
            id,
            display_id,
            faction,
            blood,
            is_ethereal: flags[0],
            looks_ethereal: flags[1],
            fearless: flags[2],
            uncharmable: flags[3],
            nonslippery: flags[4],
            has_fairy: flags[5],
            can_see_invisible: flags[6],
            attached_sound_count,
            attached_sounds,
            gibs,
            lights,
            max_hitpoints,
            number_of_health_bars,
            undying,
            undie_time,
            undie_hitpoints,
            hit_tolerance,
            knockdown_tolerance,
            score_value,
            experience_value,
            reward_on_kill,
            reward_on_overkill,
            regeneration,
            max_panic,
            zap_modifier,
            length,
            radius,
            mass,
            speed,
            turn_speed,
            bleed_rate,
            stun_time,
            summon_element_bank,
            summon_element_cue,
            resistances,
            models,
            skeleton_model,
            attached_effects,
            animation_sets,
            equipment,
            event_conditions,
            alert_radius: r.f32()?,
            group_chase: r.f32()?,
            group_separation: r.f32()?,
            group_cohesion: r.f32()?,
            group_alignment: r.f32()?,
            group_wander: r.f32()?,
            friendly_avoidance: r.f32()?,
            enemy_avoidance: r.f32()?,
            sight_avoidance: r.f32()?,
            danger_avoidance: r.f32()?,
            anger_weight: r.f32()?,
            distance_weight: r.f32()?,
            health_weight: r.f32()?,
            flocking: r.bool()?,
            break_free_strength: r.f32()?,
            abilities: {
                let n = r.count()?;
                (0..n).map(|_| Ability::read(r)).collect::<Result<_>>()?
            },
            move_animations: {
                let n = r.count()?;
                (0..n)
                    .map(|_| Ok((r.u8()?, r.strings()?)))
                    .collect::<Result<_>>()?
            },
            buffs: {
                let n = r.count()?;
                (0..n)
                    .map(|_| BuffStorage::read(r))
                    .collect::<Result<_>>()?
            },
            auras: {
                let n = r.count()?;
                (0..n)
                    .map(|_| AuraStorage::read(r))
                    .collect::<Result<_>>()?
            },
        })
    }
}
