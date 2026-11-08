//! Host-scripted world state that clients present but never simulate
//! (rewrite extension, protocol 10). Clients run no scene scripts, so every
//! state a host trigger creates arrives here: animated level parts, spawned
//! pickups, tutorial hints and gating, and spell presentation. Protocol 14
//! adds the per-wizard presentation the viewer reads straight from the
//! avatar (self-shields, aura halos, weapon trails) and one-shot pulses.
use magicka_sim::{
    barrier::Barrier, lightning::BoltVisual, railgun::RailRender, shield_entity::ShieldEntity,
    spell_mine::SpellMine,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldPresentation {
    /// AnimatedLevelPart playback of the scene, pre-order (the same list a
    /// scene change saves).
    pub animations: Vec<PartPlaybackState>,
    pub pickups: Vec<PickupState>,
    pub tutorial: TutorialState,
    pub gating: GatingState,
    pub spells: SpellPresentation,
    /// Protocol 14: per-wizard draw inputs, only for wizards with something
    /// to draw (server slots).
    pub avatars: Vec<AvatarPresentation>,
    /// Protocol 14: recent one-shot per-wizard events, kept briefly for
    /// loss; each has a unique serial so clients draw it once.
    pub pulses: Vec<AvatarPulse>,
    /// Protocol 17: ground decals (scorch, frost, water, blood) changed
    /// recently, kept briefly for loss.
    pub decals: Vec<DecalState>,
    /// Protocol 17: the floating damage numbers showing now.
    pub numbers: Vec<NumberState>,
}

/// Decals in one frame; the most recently changed are kept.
pub const MAX_DECALS: usize = 64;
pub const MAX_NUMBERS: usize = 128;

/// One DecalManager projection: its slot, and what it looks like now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecalState {
    pub index: u16,
    pub generation: u64,
    pub transform: [f32; 16],
    /// Atlas column, row, remaining lifetime.
    pub args: [f32; 3],
    pub color: [f32; 4],
    pub animated_owner: Option<u32>,
}

/// One floating number as it is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct NumberState {
    pub position: [f32; 3],
    pub scale: [f32; 2],
    pub value: f32,
    pub is_xp: bool,
    pub color: [f32; 4],
}

/// IceBlade.TRAIL_VERTEX_COUNT and ArcaneBlade's strip: hilt/tip pairs.
pub const TRAIL_VERTICES: usize = 32;
pub const MAX_AVATAR_BLADES: usize = 8;
pub const MAX_PULSES: usize = 256;

/// Character.SelfShield as the viewer draws it: the pure Shield projection
/// (noise offsets, damage flash) or the Earth/Ice armour overlays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SelfShieldState {
    /// SelfShieldType: 1 Shield, 2 Earth, 3 Ice, 4 IcedEarth.
    pub kind: u8,
    pub health: f32,
    pub max_health: f32,
    pub magnitude: f32,
    pub ice_magnitude: f32,
    pub earth_magnitude: f32,
    pub noise_offsets: [[f32; 2]; 3],
    pub time_since_damage: f32,
    pub time_sensitive: bool,
}

/// ArcaneBlade.RenderData inputs: the strip and the spell colour.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArcaneBladeState {
    /// Avatar-local identity (draw entities key on owner + id).
    pub id: u32,
    pub color: [f32; 3],
    /// `TRAIL_VERTICES` positions.
    pub vertices: Vec<[f32; 3]>,
}

/// What a client needs to run the viewer's per-avatar draw paths for a
/// wizard it does not simulate.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AvatarPresentation {
    /// Server slot.
    pub slot: usize,
    pub shield: Option<SelfShieldState>,
    /// HaloAuraRenderData tint while auras (Boost/buffs) are drawn.
    pub halo: Option<[f32; 4]>,
    /// IceBlade.TrailRenderData positions (`TRAIL_VERTICES`) while swinging.
    pub ice_blade_trail: Option<Vec<[f32; 3]>>,
    pub arcane_blades: Vec<ArcaneBladeState>,
}

impl AvatarPresentation {
    /// Nothing to draw: the host omits the record.
    pub fn is_empty(&self) -> bool {
        self.shield.is_none()
            && self.halo.is_none()
            && self.ice_blade_trail.is_none()
            && self.arcane_blades.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PulseKind {
    /// LightningZapRenderData: mZapTimer armed (flash + body highlight).
    Zap,
}

/// A one-shot per-wizard presentation event.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AvatarPulse {
    /// Unique per host session, increasing.
    pub serial: u32,
    /// Server slot.
    pub slot: usize,
    pub kind: PulseKind,
    /// Host clock (`GameFrame::time`) when it fired.
    pub time: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PartPlaybackState {
    pub time: f32,
    pub start: f32,
    pub end: f32,
    pub speed: f32,
    pub looping: bool,
    pub playing: bool,
    pub resting_timer: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PickupKind {
    /// A magick tome (MagickType value).
    Book { magick: u8 },
    /// A wizard item by template name.
    Item { name: String },
}

/// A live book or item pickup; its pose travels as a physics body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PickupState {
    pub handle: u16,
    pub generation: u64,
    pub kind: PickupKind,
    pub transform: [f32; 16],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HintState {
    pub id: i32,
    pub text: String,
    pub controller_text: Option<String>,
    pub animation: i32,
    pub position: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DialogState {
    /// Host-side identity of this dialog: each machine dismisses its own copy.
    pub serial: u32,
    pub text: String,
    pub element: u32,
    pub magick: i32,
    pub scale: Option<f32>,
    pub size: Option<[f32; 2]>,
}

/// TutorialManager presentation targets. Clients run the fades themselves.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TutorialState {
    /// The hint the host shows or is fading in.
    pub hint: Option<HintState>,
    pub dialog: Option<DialogState>,
    /// EnableDark without a matching DisableDark.
    pub dark: bool,
}

/// Tutorial input gating and SpellManager.IsMagickAllowed, for client HUDs.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GatingState {
    pub elements_disabled: i32,
    pub cast_types_disabled: Vec<i32>,
    pub push_disabled: bool,
    pub magicks_allowed: u64,
    /// PlayState.UIEnabled (false during the host's cutscenes).
    pub ui_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoltState {
    pub serial: u32,
    pub ttl: f32,
    pub visual: BoltVisual,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArcaneBlastState {
    pub position: [f32; 3],
    pub max_radius: f32,
    pub elements: u32,
    pub color: [f32; 3],
    pub ttl: f32,
    pub rotation: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MissileState {
    pub id: u32,
    /// Content-relative model file without extension (glTF export name).
    pub model: String,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
}

/// The draw inputs of live spells (beams, bolts, shields, walls, mines,
/// blasts and missiles). Particle sprays travel as effects.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpellPresentation {
    pub beams: Vec<RailRender>,
    pub bolts: Vec<BoltState>,
    pub shields: Vec<(u16, ShieldEntity)>,
    pub barriers: Vec<(usize, Barrier)>,
    pub mines: Vec<(usize, SpellMine)>,
    pub arcane_blasts: Vec<ArcaneBlastState>,
    pub missiles: Vec<MissileState>,
    /// Protocol 17: the ground lights of blasts, as drawn now.
    pub lights: Vec<LightState>,
}

pub const MAX_LIGHTS: usize = 64;

/// One DynamicLight as the viewer draws it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LightState {
    pub position: [f32; 3],
    pub diffuse: [f32; 3],
    pub ambient: [f32; 3],
    /// None for point lights; Some(end) for a capsule starting at position.
    pub capsule_end: Option<[f32; 3]>,
    pub intensity: f32,
    pub radius: f32,
    pub specular: f32,
}

fn text(s: &str, limit: usize) -> bool {
    s.len() <= limit
}
/// A name clients look up under their own content folder: relative, with no
/// way out of it. (Joining an absolute path replaces the folder entirely.)
fn content_name(s: &str, limit: usize) -> bool {
    !s.is_empty()
        && s.len() <= limit
        && !s.starts_with(['/', '\\'])
        && !s.contains(['\0', ':'])
        && !s.split(['/', '\\']).any(|part| part == "..")
}

impl WorldPresentation {
    /// Drop what does not fit in a frame. The game does not bound most of these
    /// lists: a player casting enough Arcane blasts could otherwise make every
    /// frame fail validation and end the game for all clients. These are what
    /// clients draw, so the cost of trimming is some missing visuals.
    pub fn trim_to_limits(&mut self) {
        self.animations.truncate(1024);
        self.pickups.truncate(1024);
        self.avatars.truncate(crate::lobby::PLAYER_SLOT_CAP);
        self.pulses.truncate(MAX_PULSES);
        self.decals.truncate(MAX_DECALS);
        self.numbers.truncate(MAX_NUMBERS);
        let spells = &mut self.spells;
        spells.beams.truncate(256);
        spells.bolts.truncate(256);
        spells.shields.truncate(256);
        spells.barriers.truncate(magicka_sim::barrier::POOL_SIZE);
        spells.mines.truncate(magicka_sim::spell_mine::POOL_SIZE);
        spells.arcane_blasts.truncate(64);
        spells.missiles.truncate(512);
        spells.lights.truncate(MAX_LIGHTS);
    }
    pub fn valid(&self) -> bool {
        let finite = |v: &[f32]| v.iter().all(|x| x.is_finite());
        let spells = &self.spells;
        self.animations.len() <= 1024
            && self
                .animations
                .iter()
                .all(|a| finite(&[a.time, a.start, a.end, a.speed, a.resting_timer]))
            && self.pickups.len() <= 1024
            && self.pickups.iter().all(|p| {
                finite(&p.transform)
                    && match &p.kind {
                        PickupKind::Book { .. } => true,
                        PickupKind::Item { name } => content_name(name, 128),
                    }
            })
            && self.tutorial.hint.as_ref().is_none_or(|h| {
                text(&h.text, 4096) && h.controller_text.as_deref().is_none_or(|t| text(t, 4096))
            })
            && self
                .tutorial
                .dialog
                .as_ref()
                .is_none_or(|d| text(&d.text, 4096))
            && self.gating.cast_types_disabled.len() <= 16
            && spells.beams.len() <= 256
            && spells.bolts.len() <= 256
            && spells.shields.len() <= 256
            && spells.barriers.len() <= magicka_sim::barrier::POOL_SIZE
            && spells.mines.len() <= magicka_sim::spell_mine::POOL_SIZE
            && spells.arcane_blasts.len() <= 64
            && spells.missiles.len() <= 512
            && spells
                .missiles
                .iter()
                .all(|m| content_name(&m.model, 256) && finite(&m.position) && finite(&m.velocity))
            && spells
                .shields
                .iter()
                .all(|(_, s)| finite(&s.position) && s.radius.is_finite())
            && spells
                .beams
                .iter()
                .all(|b| finite(&b.position) && finite(&b.direction))
            && self.avatars_valid()
            && self.pulses_valid()
            && spells.lights.len() <= MAX_LIGHTS
            && spells.lights.iter().all(|l| {
                finite(&l.position)
                    && finite(&l.diffuse)
                    && finite(&l.ambient)
                    && l.capsule_end.as_ref().is_none_or(|end| finite(end))
                    && finite(&[l.intensity, l.radius, l.specular])
            })
            && self.decals.len() <= MAX_DECALS
            && self
                .decals
                .iter()
                .all(|d| finite(&d.transform) && finite(&d.args) && finite(&d.color))
            && self.numbers.len() <= MAX_NUMBERS
            && self.numbers.iter().all(|n| {
                finite(&n.position) && finite(&n.scale) && n.value.is_finite() && finite(&n.color)
            })
    }

    fn avatars_valid(&self) -> bool {
        let finite = |v: &[f32]| v.iter().all(|x| x.is_finite());
        let mut slots = std::collections::BTreeSet::new();
        self.avatars.len() <= crate::lobby::PLAYER_SLOT_CAP
            && self.avatars.iter().all(|a| {
                slots.insert(a.slot)
                    && a.slot < crate::lobby::PLAYER_SLOT_CAP
                    && a.shield.as_ref().is_none_or(|s| {
                        (1..=4).contains(&s.kind)
                            && finite(&[
                                s.health,
                                s.max_health,
                                s.magnitude,
                                s.ice_magnitude,
                                s.earth_magnitude,
                                s.time_since_damage,
                            ])
                            && s.noise_offsets.iter().all(|o| finite(o))
                    })
                    && a.halo.as_ref().is_none_or(|h| finite(h))
                    && a.ice_blade_trail
                        .as_ref()
                        .is_none_or(|t| t.len() == TRAIL_VERTICES && t.iter().all(|v| finite(v)))
                    && a.arcane_blades.len() <= MAX_AVATAR_BLADES
                    && a.arcane_blades.iter().all(|b| {
                        finite(&b.color)
                            && b.vertices.len() == TRAIL_VERTICES
                            && b.vertices.iter().all(|v| finite(v))
                    })
            })
    }

    fn pulses_valid(&self) -> bool {
        let mut serials = std::collections::BTreeSet::new();
        self.pulses.len() <= MAX_PULSES
            && self.pulses.iter().all(|p| {
                serials.insert(p.serial)
                    && p.slot < crate::lobby::PLAYER_SLOT_CAP
                    && p.time.is_finite()
                    && p.time >= 0.0
            })
    }
}
