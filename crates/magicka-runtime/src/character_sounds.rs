//! Character sound events driven by template data and simulation state:
//! template attached sounds (Character.Initialize/Die/Deinitialize), the
//! Footstep animation action, and the InternalDamage gore hit cue. See
//! research/audio-xact.md (2026-10-30) and evidence/character-sounds/.
use crate::scene_loading::custom_hash;
use magicka_sim::character_physics::CharacterBody;

/// One CharacterTemplate attached sound: `KeyValuePair<int, Banks>` of the
/// lowercased cue name's hash and the stored bank flag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachedSound {
    pub bank: u16,
    pub cue: i32,
    /// Lowercased cue name (logs and bank validation).
    pub name: String,
}

/// CharacterTemplate.Read keeps at most four attached sounds (the content
/// parser already drops later entries without reading them). The name is
/// lowercased before hashing; the bank is the unchecked Banks cast of the
/// stored Int32.
pub fn attached_sounds(
    template: &magicka_content::character::CharacterTemplate,
) -> Vec<AttachedSound> {
    template
        .attached_sounds
        .iter()
        .take(4)
        .map(|(name, bank)| {
            let name = name.to_lowercase();
            AttachedSound {
                bank: *bank as u16,
                cue: custom_hash(&name),
                name,
            }
        })
        .collect()
}

/// Footstep.sCueNames by CollisionMaterials index (Generic, Gravel, Grass,
/// Wood, Snow, Stone, Mud, Reflect, Water, Lava). Reflect reuses the generic
/// cue; lava has no footstep cue.
pub const FOOTSTEP_CUES: [Option<&str>; 10] = [
    Some("footstep_generic"),
    Some("footstep_gravel"),
    Some("footstep_grass"),
    Some("footstep_wood"),
    Some("footstep_snow"),
    Some("footstep_stone"),
    Some("footstep_mud"),
    Some("footstep_generic"),
    Some("footstep_water"),
    None,
];

/// Footstep.sEffectNames: the EffectManager effect started at the owner's
/// position raised by HeightOffset + WaterDepth (presentation; not emitted
/// yet).
pub const FOOTSTEP_EFFECTS: [Option<&str>; 10] = [
    None,
    Some("footstep_gravel"),
    Some("footstep_grass"),
    None,
    Some("footstep_snow"),
    None,
    Some("footstep_mud"),
    None,
    Some("footstep_water"),
    Some("footstep_lava"),
];

/// Footstep cue for a CharacterBody.GroundMaterial value (None for lava and
/// values outside the native table, which would throw natively).
pub fn footstep_cue(ground_material: u8) -> Option<&'static str> {
    FOOTSTEP_CUES
        .get(ground_material as usize)
        .copied()
        .flatten()
}

pub fn footstep_effect(ground_material: u8) -> Option<&'static str> {
    FOOTSTEP_EFFECTS
        .get(ground_material as usize)
        .copied()
        .flatten()
}

/// Footstep.FootstepVariables: the "Weight" (Body.Mass) and "WaterDepth"
/// (Character.WaterDepth) cue variables.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FootstepVariables {
    pub weight: f32,
    pub depth: f32,
}

impl FootstepVariables {
    pub const VARIABLE_WEIGHT: &'static str = "Weight";
    pub const VARIABLE_DEPTH: &'static str = "WaterDepth";
}

/// Footstep.InternalExecute on its first execution: nothing while airborne
/// (CharacterBody.IsTouchingGround false) or levitating; otherwise the
/// ground material's cue with the body's mass and water depth. Levitation is
/// the body's owner fact (Character.IsLevitating: timer > 0 and not dead).
pub fn footstep(body: &CharacterBody) -> Option<(&'static str, FootstepVariables)> {
    if !body.is_touching_ground() || body.owner.levitating {
        return None;
    }
    let cue = footstep_cue(body.ground_material)?;
    Some((
        cue,
        FootstepVariables {
            weight: body.mass,
            depth: body.owner.water_depth,
        },
    ))
}

/// Defines.SOUNDS_GORE by BloodType (regular, green, black, wood, insect,
/// none): every blood type shares one cue; `none` hashes the empty string,
/// which no bank defines.
pub const SOUNDS_GORE: [&str; 6] = [
    "misc_hit_blood",
    "misc_hit_blood",
    "misc_hit_blood",
    "misc_hit_blood",
    "misc_hit_blood",
    "",
];

/// The gore hit cue for a template BloodType (None for `none` and values
/// outside the table).
pub fn gore_hit_cue(blood: i32) -> Option<&'static str> {
    usize::try_from(blood)
        .ok()
        .and_then(|i| SOUNDS_GORE.get(i))
        .copied()
        .filter(|name| !name.is_empty())
}

/// Audio work a character requested from its own state, drained by the live
/// scene into AudioRequests at the owner's emitter.
#[derive(Clone, Debug, PartialEq)]
pub enum CharacterAudio {
    /// AudioManager.PlayCue(bank, cue, emitter) whose Cue is discarded.
    /// `magnitude` is SpellSoundVariables "Magnitude"; `variables` are other
    /// cue variables (Footstep Weight/WaterDepth) for presentations that can
    /// assign them.
    OneShot {
        bank: u16,
        cue: i32,
        magnitude: Option<f32>,
        variables: Vec<(&'static str, f32)>,
    },
    /// Character.Initialize: mAttachedSoundCues[slot] = PlayCue(bank, cue,
    /// emitter). Native never re-applies 3D to these cues afterwards.
    AttachedStart { slot: usize, bank: u16, cue: i32 },
    /// Character.Die / Deinitialize: mAttachedSoundCues[slot].Stop(AsAuthored)
    /// unless already stopping.
    AttachedStop { slot: usize },
}

/// Per-character sound state: the template attached sounds, whether their
/// cues are live, and the pending requests.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharacterSounds {
    pub attached: Vec<AttachedSound>,
    /// Slots whose attached cue was started and not yet stopped.
    pub attached_live: Vec<usize>,
    pub pending: Vec<CharacterAudio>,
}

impl CharacterSounds {
    /// Character.Initialize tail: start every template attached sound at the
    /// owner's emitter (entries without a cue name are skipped; native plays
    /// hash 0, which the cue dictionary does not contain).
    pub fn initialize(attached: Vec<AttachedSound>) -> Self {
        let mut sounds = Self {
            attached,
            attached_live: Vec::new(),
            pending: Vec::new(),
        };
        sounds.start_attached();
        sounds
    }

    pub fn start_attached(&mut self) {
        for (slot, sound) in self.attached.iter().enumerate() {
            if sound.name.is_empty() || self.attached_live.contains(&slot) {
                continue;
            }
            self.attached_live.push(slot);
            self.pending.push(CharacterAudio::AttachedStart {
                slot,
                bank: sound.bank,
                cue: sound.cue,
            });
        }
    }

    /// Character.Die / Deinitialize: stop the live attached cues once.
    pub fn stop_attached(&mut self) {
        for slot in std::mem::take(&mut self.attached_live) {
            self.pending.push(CharacterAudio::AttachedStop { slot });
        }
    }

    /// Per-frame: Die() stops the attached cues; a revived character
    /// (Initialize again) restarts them.
    pub fn update(&mut self, dead: bool) {
        if dead {
            self.stop_attached();
        } else if self.attached_live.len() < self.attached.len() {
            self.start_attached();
        }
    }

    /// Character.InternalDamage gore branch: PlayCue(Misc, SOUNDS_GORE[blood],
    /// Magnitude = damage amount) alongside the gore splash and blood decal.
    /// The caller has already applied the native guards (blood != none,
    /// BloodAndGore on, amount > 0, FeatureEffects, physical/arcane element,
    /// not frozen).
    pub fn gore_hit(&mut self, blood: i32, amount: f32) {
        if let Some(cue) = gore_hit_cue(blood) {
            self.pending.push(CharacterAudio::OneShot {
                bank: crate::audio::banks::MISC,
                cue: custom_hash(cue),
                magnitude: Some(amount),
                variables: Vec::new(),
            });
        }
    }

    /// Footstep action: PlayCue(Footsteps, cue, Weight/WaterDepth, emitter).
    pub fn footstep(&mut self, cue: &str, variables: FootstepVariables) {
        self.pending.push(CharacterAudio::OneShot {
            bank: crate::audio::banks::FOOTSTEPS,
            cue: custom_hash(cue),
            magnitude: None,
            variables: vec![
                (FootstepVariables::VARIABLE_WEIGHT, variables.weight),
                (FootstepVariables::VARIABLE_DEPTH, variables.depth),
            ],
        });
    }

    /// Every cue (bank, name) this state can emit for its template, for bank
    /// validation.
    pub fn attached_cue_names(&self) -> impl Iterator<Item = (u16, &str)> {
        self.attached.iter().map(|s| (s.bank, s.name.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footstep_tables_follow_collision_materials() {
        assert_eq!(footstep_cue(0), Some("footstep_generic"));
        assert_eq!(footstep_cue(7), Some("footstep_generic"));
        assert_eq!(footstep_cue(8), Some("footstep_water"));
        assert_eq!(footstep_cue(9), None);
        assert_eq!(footstep_cue(10), None);
        assert_eq!(footstep_effect(0), None);
        assert_eq!(footstep_effect(9), Some("footstep_lava"));
    }

    #[test]
    fn gore_cue_is_shared_by_blood_types_and_absent_for_none() {
        for blood in 0..5 {
            assert_eq!(gore_hit_cue(blood), Some("misc_hit_blood"));
        }
        assert_eq!(gore_hit_cue(5), None);
        assert_eq!(gore_hit_cue(-1), None);
        assert_eq!(gore_hit_cue(6), None);
    }
}
