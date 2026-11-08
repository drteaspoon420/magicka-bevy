//! Player avatar: the shared Character state machine and animation
//! (`character_motion`), scripted event playback (AvatarAi), the
//! KeyboardMouseController avatar section (mouse movement, element keys,
//! Force/Area/Self cast buttons), conjuring into the spell queue, spell
//! casting through clip CastSpell actions, the Force spray effect, death and
//! fall damage. See research/play-session.md, research/avatar-events.md and
//! research/character-casting.md.
pub use crate::character_motion::{CharacterState as AvatarState, MOVE_RUN, MOVE_WALK};
use crate::{
    animation_controller::{AnimationSet, ANIMATION_IDLE},
    avatar_ai::AvatarAi,
    character::RuntimeCharacter,
    character_motion::{ActorAi, ActorFrame, CharacterFacts, CharacterHost, CharacterMotion},
    character_pose::character_orientation,
    play_content::CharacterRig,
    CharacterDefinition,
};
use magicka_content::skinned::{Mat, IDENTITY};
use magicka_sim::push_spell::{PushSpell, PushWorld};
use magicka_sim::{
    avatar_attack::{AttackContext, AttackEvent, AttackInput},
    avatar_cast_input::{CastInput, CastInputContext},
    avatar_input::{AvatarInput, ConjureContext, ConjureEvent},
    character_impact::ImpactContext,
    character_physics::{CharacterBody, GRAVITY},
    character_status::StatusContext,
    damage::{attack as A, CastType, Damage},
    damage_delivery::{AttackerKind, DamagePayload, DeliveryContext, NetworkRole},
    force_spray::{ForceCastFacts, ForceSpray, ForceSprayHost},
    impact_motion::{MotionContext, Vector},
    resistance::Resistance,
    self_beam::{SelfBeam, SelfBeamHost},
    self_lightning::{SelfLightning, SelfLightningHost},
    self_spray::{SelfSpray, SelfSprayHost},
    shield_self::{ShieldSelf, ShieldSelfPlan},
    spell_cast::{CastNotice, CastOwner, EffectKind, EffectLifecycle, EffectOwner, SpellCastHost},
    spells::{Direction, Elements as E, MagickTree, Spell, SpellType},
    spray_pulse::{Segment, SpraySource, SprayTarget, SprayWorld},
    status::Status,
    weapon_spells::{AvatarCastDispatch, WeaponSpellEvent, WeaponSpells},
};
use std::sync::Arc;
mod character_spells;
pub(crate) use character_spells::CharacterSpellHost;

/// KeyboardBindings element order: Water, Life, Shield, Cold, Lightning,
/// Arcane, Earth, Fire (default keys Q W E R A S D F).
pub const ELEMENT_KEYS: [E; 8] = [
    E::WATER,
    E::LIFE,
    E::SHIELD,
    E::COLD,
    E::LIGHTNING,
    E::ARCANE,
    E::EARTH,
    E::FIRE,
];

/// KeyboardMouseController.Invert(true) swaps the binding keys 0<->7,
/// 1<->4, 2<->3 and 5<->6: binding `i` then reads the key `INVERTED_KEYS[i]`
/// (research/confuse.md).
pub const INVERTED_KEYS: [usize; 8] = [7, 4, 3, 2, 1, 6, 5, 0];
/// Negating both axes of the native right-stick gesture.
pub const PAD_INVERTED_KEYS: [usize; 8] = [7, 2, 1, 4, 3, 6, 5, 0];

/// Avatar.Conjure* direction pairs fed to HandleCombo.
fn conjure_directions(element: E) -> [Direction; 2] {
    use Direction as D;
    match element {
        E::EARTH => [D::Down, D::Left],
        E::WATER => [D::Left, D::Up],
        E::COLD => [D::Right, D::Up],
        E::FIRE => [D::Right, D::Down],
        E::LIGHTNING => [D::Left, D::Down],
        E::ARCANE => [D::Up, D::Right],
        E::LIFE => [D::Up, D::Left],
        _ => [D::Down, D::Right],
    }
}

/// Input for one KeyboardMouseController avatar update.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerInput {
    /// Cursor in normalised device coordinates (x right, y up), when known.
    pub cursor_ndc: Option<[f32; 2]>,
    /// Rewrite gamepad adapter: independent world X/Z movement, including neutral.
    pub pad_movement: Option<[f32; 2]>,
    pub pad_attack: bool,
    pub pad_block: bool,
    /// KeyboardBindings.Move held (left mouse button by default).
    pub move_down: bool,
    /// KeyboardBindings.Cast held (right mouse button).
    pub cast_down: bool,
    /// KeyboardBindings.CastSelf held (middle mouse button).
    pub cast_self_down: bool,
    /// KeyboardBindings.Shift held (area casting).
    pub shift_down: bool,
    /// KeyboardBindings.Boost held (Space by default); processed on press edges.
    pub boost_down: bool,
    /// Element keys held, in `ELEMENT_KEYS` order.
    pub elements: [bool; 8],
    /// KeyboardBindings.PrevMagick pressed this frame (scroll wheel up).
    pub magick_prev: bool,
    /// KeyboardBindings.NextMagick pressed this frame (scroll wheel down).
    pub magick_next: bool,
    /// KeyboardBindings.Inventory pressed this frame (Tab).
    pub inventory: bool,
}

/// World facts for one KeyboardMouseController avatar update.
#[derive(Clone, Copy, Debug, Default)]
pub struct InputFrame {
    /// GetMousePick plane position, when the cursor is known.
    pub pick: Option<Vector>,
    /// ControlManager.IsInputLimited / player input lock.
    pub input_limited: bool,
    pub in_cutscene: bool,
    /// ElementManager.Update delta.
    pub dt: f32,
    pub play_time: f64,
    pub weapon_cast_disabled: bool,
    pub boost_target: Option<u16>,
    /// Controller consumed Move for a pickup interaction until release.
    pub move_locked: bool,
    /// SpellManager.IsMagickAllowed (Campaign): the playing players'
    /// UnlockedMagicks bits ORed together.
    pub magicks_allowed: u64,
    /// TutorialManager.IsCastTypeEnabled(Magick) is false.
    pub magick_cast_disabled: bool,
    pub elements_disabled: u32,
    pub force_cast_disabled: bool,
    pub area_cast_disabled: bool,
    pub self_cast_disabled: bool,
    pub push_disabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AvatarEvent {
    /// Rewrite extension: a registered (mod) Magick by registry index.
    CastRegisteredMagick(u16),
    /// Native NPC.ConjureSpell always presents the attempted element cue.
    NpcConjure(E),
    ItemReleased {
        slot: i32,
    },
    AnimationEffect {
        index: usize,
        bone: String,
        effect: String,
        retained: bool,
        first: bool,
    },
    AnimationEffectStopped(usize),
    /// Item.Update: the special recharged (`special_ready` at attach0).
    SpecialReady {
        slot: i32,
    },
    BoostShield {
        target: u16,
        amount: f32,
    },
    ItemSpecial {
        slot: i32,
        name: String,
    },
    HealFriendlies,
    SprayWeapon {
        position: Vector,
        direction: Vector,
        cast_source: Vector,
        timestamp: f64,
        plan: magicka_sim::spray_weapon::SprayWeaponPlan,
    },
    Weapon(WeaponSpellEvent),
    Attack(AttackEvent),
    PushStarted {
        cast_type: CastType,
        position: Vector,
        direction: Vector,
    },
    PushHit {
        target: u16,
    },
    FallDamage {
        amount: f32,
        result: u16,
    },
    SelfEffectStarted {
        index: usize,
        name: &'static str,
        position: Vector,
        direction: Vector,
    },
    SelfEffectsPosition {
        position: Vector,
        direction: Vector,
    },
    SelfEffectsStopped,
    /// ShieldSpell.CastForce/CastArea with Shield alone: a Shield entity
    /// (`shield_entity::pure_shield`), spawned by the scene.
    SpawnShield {
        position: Vector,
        radius: f32,
        direction: Vector,
        kind: magicka_sim::shield_entity::ShieldKind,
        hit_points: f32,
        color: Vector,
    },
    /// ShieldSpell.CastForce/CastArea with Arcane or Life and no physical
    /// element: two SpellMine placements the scene probes and spawns
    /// (`spell_mine::cast_placements`).
    SpawnMines {
        placements: [magicka_sim::spell_mine::MinePlacement; 2],
        spell: Spell,
    },
    /// ShieldSpell.AnimationEnd with mCastType = Weapon: the scene places a
    /// WALL shield, a mine line or a barrier line from the caster's current
    /// Position/Direction/Radius (`shield_weapon::animation_end`).
    ShieldWeapon {
        spell: Spell,
    },
    /// ShieldSpell.CastForce/CastArea with other elements: Barrier walls
    /// (`barrier::cast_placements`), spawned by the scene.
    SpawnBarriers {
        cast: Box<magicka_sim::barrier::BarrierCast>,
        spell: Spell,
    },
    /// SelfShield creation with Ice: a circle damage around the owner
    /// (Character.SelfShield ctor). Not yet delivered by the scene.
    ShieldCircleDamage {
        radius: f32,
        damage: Damage,
    },
    SelfCastApplied {
        result: u16,
        hit_points_before: f32,
        hit_points_after: f32,
    },
    KnockDown,
    KilledByKillPlane,
    Died,
    /// Conjure/queue/notice effects for HUD and audio.
    Conjure(ConjureEvent),
    /// ElementManager.Cooldown also notifies the keyboard HUD.
    ElementCooldown(E),
    CastNotice(CastNotice),
    /// Character.CastSpell reached Spell.Cast with this spell and type.
    SpellCast {
        spell: Spell,
        cast_type: CastType,
        /// Capture before ProjectileSpell.CastForce consumes/reset SpellPower.
        /// Its cue's Magnitude is this value times three.
        spell_power: f32,
    },
    /// SpraySpell.CastForce presentation start (sound, light, effects):
    /// visual effects start at CastSource facing the owner's Direction.
    SprayStarted {
        spell: Spell,
        range: f32,
        position: Vector,
        direction: Vector,
    },
    /// SpraySpell.CastUpdate (Force): the matrix every spray visual effect
    /// receives through EffectManager.UpdateOrientation.
    SprayOrientation(magicka_content::skinned::Mat),
    SprayDecalUpdate(crate::spray_decals::Frame),
    SprayStopped,
    /// ProjectileSpell.SpawnMissile request (the scene creates the missile).
    MissileLaunched(magicka_sim::projectile_launch::ProjectileLaunch),
    /// ProjectileSpell.CastUpdate: physical small-hit effect when a shield
    /// intersects the owner-to-CastSource segment before launch.
    ProjectileBlocked {
        element: E,
        position: Vector,
        direction: Vector,
    },
    /// A spell family whose effect is not reconstructed yet.
    SpellNotReconstructed(String),
    /// Character.CastSpell of a combined magick: the host runs its Effect.
    CastMagick(magicka_sim::spells::MagickType),
    /// LightningBolt.InitializeEffect: a bolt segment to present.
    LightningBolt(magicka_sim::lightning::BoltVisual),
    /// SpraySpell.CastArea: owned per-element cues, then the collection blast
    /// and its owner side effects (the scene delivers them in that order).
    /// UnderGroundAttack.Initialize (ProjectileSpell Weapon AnimationEnd).
    UndergroundAttack {
        position: Vector,
        velocity: [f32; 2],
        range: f32,
        damages: [Damage; 5],
        timestamp: f64,
    },
    /// IceBlade.OnRemove after the blade was killed (ProjectileSpell
    /// AnimationEnd/DeInitialize): the scene damages every damageable whose
    /// SegmentIntersect(Position → Position + Direction·range, 1) hits.
    IceBladeStrike {
        range: f32,
        damages: [Damage; 5],
        timestamp: f64,
        /// mOwner.GetOrientation() for `weapon_ice_death` (None if the blade
        /// was never posed).
        orientation: Option<magicka_content::skinned::Mat>,
    },
    /// IceBlade.Initialize: `weapon_ice_spawn` at the item orientation.
    IceBladeSpawned {
        orientation: magicka_content::skinned::Mat,
    },
    /// RailGunSpell.CastWeapon's immediate arc damage (and its ArcaneBlade
    /// on Equipment[0]).
    RailWeapon {
        spell: Spell,
        position: Vector,
        direction: Vector,
        range: f32,
        damages: [Damage; 5],
        timestamp: f64,
    },
    /// SpraySpell.CastUpdate Force block tail: Liquid.Freeze(position,
    /// direction·range, mAngle, dt, damages).
    SprayFreeze {
        origin: Vector,
        delta: Vector,
        spread: f32,
        multiplier: f32,
        damages: [Damage; 5],
    },
    SprayArea {
        spell: Spell,
        position: Vector,
        cast_source: Vector,
        timestamp: f64,
        plan: magicka_sim::spray_area::SprayAreaPlan,
    },
    GroundArea {
        position: Vector,
        radius: f32,
        damages: [magicka_sim::damage::Damage; 5],
        timestamp: f64,
    },
    BeamArea {
        position: Vector,
        radius: f32,
        damages: [magicka_sim::damage::Damage; 5],
        timestamp: f64,
        cue: (&'static str, f32),
    },
    /// A Force spray pulse damaged `target`.
    SprayHit {
        target: u16,
    },
}

/// MagickaElement cooldown (ElementManager): 0.125 s after each conjure.
#[derive(Clone, Copy, Debug)]
struct ElementCooldown {
    cooldown: f32,
    active: bool,
}
impl ElementCooldown {
    fn update(&mut self, dt: f32) {
        if self.active {
            self.cooldown -= dt;
            if self.cooldown <= 0.0 {
                self.active = false;
            }
        }
    }
}

/// The owner's CurrentSpell.
#[derive(Clone, Debug)]
pub enum SpellEffect {
    ForceSpray(ForceSpray),
    Push(PushSpell),
    SelfSpray(SelfSpray),
    SelfBeam(SelfBeam),
    /// LightningSpell (Self): damage at cast, lifecycle only afterwards.
    SelfLightning(SelfLightning),
    /// ShieldSpell (Self): owner shield/auras at cast, then mMinTTL.
    ShieldSelf(ShieldSelf),
    /// ShieldSpell.CastWeapon: the shared ShieldSpell CastUpdate plus
    /// mSpell/mHasCast for AnimationEnd.
    ShieldWeapon {
        shield: ShieldSelf,
        spell: Spell,
        has_cast: bool,
    },
    /// ProjectileSpell (Force): the launch controller.
    Projectile(magicka_sim::projectile_launch::ForceProjectile),
    /// LightningSpell (Force/Area).
    Lightning(magicka_sim::lightning::LightningSpell),
    /// RailGunSpell Force: one steered root Railgun.
    RailGun(magicka_sim::railgun::RailGunForce),
    /// RailGunSpell Area: the blast was issued at cast; mTTL = 0 ends the
    /// effect on its first CastUpdate.
    RailGunArea(EffectLifecycle),
    GroundArea(EffectLifecycle),
    SprayArea(magicka_sim::spray_area::SprayArea),
    SprayWeapon(magicka_sim::spray_weapon::SprayWeapon),
    /// Weapon casts without a dedicated update (RailGun/Projectile): the
    /// shared lifecycle plus the cast spell for AnimationEnd.
    Weapon {
        kind: EffectKind,
        spell: Spell,
        timestamp: f64,
        lifecycle: EffectLifecycle,
        /// ProjectileSpell.mIceBlade while alive.
        ice_blade: Option<IceBlade>,
    },
    /// Effect families that are not reconstructed: the shared lifecycle
    /// only, inactive at once so CastState can end.
    Unreconstructed {
        kind: EffectKind,
        lifecycle: EffectLifecycle,
    },
}
impl SpellEffect {
    pub fn active(&self) -> bool {
        match self {
            Self::Push(s) => s.lifecycle.active,
            Self::ForceSpray(s) => s.lifecycle.active,
            Self::SelfSpray(s) => s.lifecycle.active,
            Self::SprayArea(s) => s.lifecycle.active,
            Self::SprayWeapon(s) => s.lifecycle.active,
            Self::SelfBeam(s) => s.lifecycle.active,
            Self::SelfLightning(s) => s.lifecycle.active,
            Self::ShieldSelf(s) | Self::ShieldWeapon { shield: s, .. } => s.lifecycle.active,
            Self::Projectile(p) => p.lifecycle.active,
            Self::Lightning(l) => l.lifecycle.active,
            Self::RailGun(r) => r.lifecycle.active,
            Self::RailGunArea(l) | Self::GroundArea(l) => l.active,
            Self::Unreconstructed { lifecycle, .. } | Self::Weapon { lifecycle, .. } => {
                lifecycle.active
            }
        }
    }
}

/// IceBlade.Initialize arguments kept until the blade is killed: range
/// 1.25·Ice + 3.75 and Projectile/Weapon damage of the spell without Earth.
#[derive(Clone, Debug, PartialEq)]
pub struct IceBlade {
    pub range: f32,
    pub damages: [Damage; 5],
    pub timestamp: f64,
    /// mTrailVertices positions (hilt/tip pairs, newest first); None until
    /// the first update with the item's orientation.
    pub trail: Option<Box<[Vector; ICE_BLADE_TRAIL]>>,
    /// The item orientation of the latest update.
    pub orientation: Option<magicka_content::skinned::Mat>,
}

/// IceBlade.TRAIL_VERTEX_COUNT.
pub const ICE_BLADE_TRAIL: usize = 32;
/// Ice_Blade.xnb attach0 translation Y (IceBlade.sAttach).
pub const ICE_BLADE_ATTACH_Y: f32 = 2.595_292_3;

impl IceBlade {
    /// IceBlade.mScale: range / sAttach.Y.
    pub fn scale(&self) -> f32 {
        self.range / ICE_BLADE_ATTACH_Y
    }

    /// IceBlade.Initialize (first call: every pair at the unscaled hilt and
    /// attach point) and IceBlade.Update: pair 0 = scaled hilt/attach, then
    /// each later pair lerps toward the previous one by ε^dt (ε = 1e-45 as
    /// the float the IL loads).
    pub fn update_trail(&mut self, item: &magicka_content::skinned::Mat, dt: f32) {
        let at = |m: &magicka_content::skinned::Mat, y: f32| -> Vector {
            std::array::from_fn(|k| y * m[4 + k] + m[12 + k])
        };
        self.orientation = Some(*item);
        let trail = self.trail.get_or_insert_with(|| {
            let (hilt, tip) = (at(item, 0.0), at(item, ICE_BLADE_ATTACH_Y));
            Box::new(std::array::from_fn(|i| if i % 2 == 0 { hilt } else { tip }))
        });
        let scale = self.range / ICE_BLADE_ATTACH_Y;
        trail[0] = at(item, 0.0);
        trail[1] = at(item, ICE_BLADE_ATTACH_Y * scale);
        let amount = 1.401_298_464_324_817e-45f64.powf(dt as f64) as f32;
        for i in (2..ICE_BLADE_TRAIL).step_by(2) {
            for j in [i, i + 1] {
                let (a, b) = (trail[j - 2], trail[j]);
                trail[j] = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * amount);
            }
        }
    }

    /// The trail of the live blade, if any.
    pub fn trail_of(effect: Option<&SpellEffect>) -> Option<&[Vector; ICE_BLADE_TRAIL]> {
        match effect {
            Some(SpellEffect::Weapon {
                ice_blade: Some(blade),
                ..
            }) => blade.trail.as_deref(),
            _ => None,
        }
    }
    fn strike(self) -> AvatarEvent {
        AvatarEvent::IceBladeStrike {
            range: self.range,
            damages: self.damages,
            timestamp: self.timestamp,
            orientation: self.orientation,
        }
    }
}

/// Live item state: original definition, animation sweep and stored spells.
/// Staff abilities whose avatar cast follows a scene step (StaffCast).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaffCast {
    /// NorthPole after its teleport: three Cold, then an Area cast.
    NorthPole,
}

pub struct AvatarItem {
    /// Item.Attached: detached equipment remains on the corpse as a reference.
    pub attached: bool,
    pub definition: magicka_content::item::Item,
    pub melee: crate::npc_combat::MeleeWeapon,
    pub spells: WeaponSpells,
    pub cooldown: f32,
    pub gun: crate::gun::GunState,
    pub ranged_missile: Option<u32>,
    pub special_cooldown: f32,
    /// Item.mAuras, executed with the holder as owner while attached.
    pub auras: Vec<crate::generic_aura::GenericAura>,
    /// The item's SpecialAbility instance identity (EpicBuff aura ID).
    pub special_buff_id: u32,
    /// World attach0/effect0/effect1 of the held item this frame.
    pub frame: Option<crate::item_effects::ItemFrame>,
    /// Definitions revision whose item overrides were applied (0: none).
    pub patched: u64,
    /// The definition before upgrade overlays (captured on the first
    /// upgrade; upgraded numbers are always recomputed from it).
    pub base_definition: Option<Box<magicka_content::item::Item>>,
}
impl AvatarItem {
    pub fn new(definition: magicka_content::item::Item) -> Self {
        let melee = crate::npc_combat::MeleeWeapon::new(
            definition.melee_range,
            definition.melee_multi_hit,
            Arc::new(definition.melee_conditions.0.clone()),
        );
        let auras = definition
            .auras
            .iter()
            .map(crate::generic_aura::GenericAura::from_content)
            .collect();
        Self {
            attached: true,
            definition,
            melee,
            spells: WeaponSpells::default(),
            cooldown: 0.0,
            gun: Default::default(),
            ranged_missile: None,
            special_cooldown: 0.0,
            auras,
            special_buff_id: crate::item_special::next_special_buff_id(),
            frame: None,
            patched: 0,
            base_definition: None,
        }
    }
    pub fn is_gun(&self) -> bool {
        matches!(self.definition.weapon_class, 14..=17)
    }
}

pub struct PlayerAvatar {
    /// SceneEntity handle.
    pub handle: u16,
    /// Died through Terminate (Versus counts its OnPlayerDeath twice).
    pub terminated: bool,
    pub slot: usize,
    /// Player.Playing; dedicated hosts retain a dormant slot-zero avatar.
    pub playing: bool,
    /// Index of the CharacterBody in the world's physics.
    pub body: usize,
    pub character: RuntimeCharacter,
    pub motion: CharacterMotion,
    pub ai: AvatarAi,
    pub move_abilities: u8,
    pub template_type: i32,
    /// Character type that trigger areas count this avatar as
    /// (`<present type="wizard">`). Every player avatar is a wizard whatever
    /// its robe template, so a robe such as Wizard_Tentacle (Template.ID
    /// "wizard_tentacle") still satisfies the campaign's player checks.
    /// INFERRED: authored scripts test type="wizard" for players and DLC robes
    /// work in the original; the native source of this is unverified.
    pub trigger_type: i32,
    pub faction: i32,
    pub rig: Option<CharacterRig>,
    /// Character.mStaffOrb (CastSource) and mWeaponTransform.
    pub staff_orb: Mat,
    /// Optional host camera cast transform. Native/top-down play leaves this
    /// unset. First-person hosts can supply their visible staff's muzzle and
    /// aim without tilting the character capsule or changing walking/AI.
    /// Presentation state: the host refreshes it rather than saving it.
    pub host_cast_source: Option<Mat>,
    /// The latest GetMousePick plane position (None without a cursor). A
    /// rewrite addition: target-only magicks aim at the character closest
    /// to it (research/custom-magicks.md).
    pub mouse_pick: Option<Vector>,
    /// Avatar.mPolymorphed: a Polymorph form ignores Boost, element keys,
    /// CastType changes and pickups (research/polymorph.md).
    pub polymorphed: bool,
    pub weapon_transform: Mat,
    /// Live and fading ArcaneBlades on Equipment[0] (SpellManager effects).
    pub arcane_blades: Vec<crate::arcane_blade::ArcaneBlade>,
    next_blade_id: u32,
    /// Element queue and directions (Player.SpellQueue / InputQueue).
    pub conjure: AvatarInput,
    pub cast: CastInput,
    /// Character.mSpell (prepared by CombineSpell).
    pub spell: Spell,
    pub current_spell: Option<SpellEffect>,
    /// SpellEffect's cast-time definition. Character.mSpell is cleared after
    /// dispatch and cannot supply CharacterCasting conditions afterwards.
    current_spell_definition: Option<(CastType, Spell)>,
    pub items: Vec<(i32, AvatarItem)>,
    pub melee_requests: Vec<crate::npc_combat::MeleeRequest>,
    pub missile_requests: Vec<(i32, Option<Vector>, bool)>,
    pub attack: AttackInput,
    pub boost: crate::shield_boost::ShieldBoost,
    pub focused: bool,
    magick_tree: Arc<MagickTree>,
    /// Rewrite extension: registered (mod) Magick combinations of this match
    /// by registry index, and the ones this player may combine.
    pub registered_magicks: Arc<Vec<(u16, Vec<E>)>>,
    pub registered_unlocked: Vec<u16>,
    /// Last InputFrame's IsMagickAllowed mask.
    magicks_allowed: u64,
    element_cooldowns: [ElementCooldown; 8],
    /// Physical element keys of the last update, in `ELEMENT_KEYS` order.
    elements_were_down: [bool; 8],
    cast_self_was_down: bool,
    /// A staff ability cast deferred to the next update (after the scene's
    /// part of the ability ran).
    pub pending_staff_cast: Option<StaffCast>,
    /// WardedExpulsion's 0.8 s SpellManager effect timer.
    pub warded_expulsion: Option<f32>,
    /// IceCacoon's frozen-time budget (SpellManager effect while Some).
    pub ice_cacoon: Option<f32>,
    /// RapidStrike's 2.5 s timer (SpellManager effect while Some).
    pub rapid_strike: Option<f32>,
    /// Character.FocusedAnimationSpeed while focused.
    pub focused_animation_speed: f32,
    /// The last input frame's (input limited, weapon cast disabled) gates.
    input_gates: (bool, bool),
    /// Controller.Invert (Confuse): element bindings swapped in pairs and the
    /// mouse direction negated.
    pub controls_inverted: bool,
    can_chant: bool,
    rng: u64,
    mouse_move: bool,
    pad_was_active: bool,
    pad_attack_was_down: bool,
    move_was_down: bool,
    /// Snapshot refreshed at every damage/status/owner-host boundary.
    equipment: [Resistance; 11],
    pub events: Vec<AvatarEvent>,
    /// Railgun kills from SpellEffect.Stop, applied by the scene's pool.
    pub rail_kills: Vec<magicka_sim::railgun::RailId>,
    /// PlayState.PlayTime at this frame's update (CastUpdate damage stamps).
    play_time: f64,
}

/// XNA Matrix.Forward of a row-major matrix.
fn forward(m: &Mat) -> Vector {
    [-m[8], -m[9], -m[10]]
}
pub(crate) fn translation(m: &Mat) -> Vector {
    [m[12], m[13], m[14]]
}

impl PlayerAvatar {
    /// Character.InternalDamage/AddStatusEffect read Equipment[1].Item.Resistance.
    /// Read the current staff so item swaps take effect before the next update.
    pub fn equipment_resistances(&self) -> [Resistance; 11] {
        self.items
            .iter()
            .find(|(slot, _)| *slot == 1)
            .map(|(_, item)| {
                crate::resistance_table(&item.definition.resistances)
                    .expect("staff resistance rows must have valid element indices")
            })
            .unwrap_or([Resistance::IDENTITY; 11])
    }

    fn refresh_equipment_resistances(&mut self) {
        self.equipment = self.equipment_resistances();
    }

    /// Native CharacterCasting reads the current effect's Spell and CastType,
    /// including the interval before the owner removes an inactive effect.
    /// Rewrite extension: SpellManager.CombineMagick for registered (mod)
    /// Magicks, after the native tree found none. The whole queue must be
    /// an unlocked registered combination.
    fn combine_registered_magick(&mut self) -> bool {
        let queue: Vec<E> = self.conjure.queue.iter().map(|s| s.element).collect();
        let Some(&(index, _)) = self
            .registered_magicks
            .iter()
            .find(|(i, combo)| *combo == queue && self.registered_unlocked.contains(i))
        else {
            return false;
        };
        self.conjure.queue.clear();
        self.conjure
            .queue
            .push(magicka_sim::spells::Spell::registered_magick(index));
        true
    }
    pub fn casting_spell(&self) -> Option<(CastType, Spell)> {
        self.current_spell.as_ref()?;
        self.current_spell_definition
    }

    pub fn liquid_status(
        &mut self,
        depth: f32,
        dt: f32,
        lava: bool,
        events: &mut Vec<magicka_sim::character_status::StatusEvent>,
    ) -> Result<
        magicka_sim::character_status::StatusResult,
        magicka_sim::character_status::StatusError,
    > {
        self.refresh_equipment_resistances();
        self.character
            .liquid_status(depth, dt, lava, Some(&self.equipment), events)
    }
    /// Character.AddStatusEffect with the avatar's equipment resistances.
    pub fn add_status_effect(
        &mut self,
        effect: magicka_sim::status::StatusEffect,
        events: &mut Vec<magicka_sim::character_status::StatusEvent>,
    ) -> Result<
        magicka_sim::character_status::StatusResult,
        magicka_sim::character_status::StatusError,
    > {
        self.refresh_equipment_resistances();
        self.character
            .add_status_effect(effect, Some(&self.equipment), events)
    }
    pub fn new(
        handle: u16,
        slot: usize,
        body: usize,
        definition: CharacterDefinition,
        animations: Arc<AnimationSet>,
        move_abilities: u8,
        template_type: i32,
    ) -> Self {
        // Avatar.Initialize: template faction | PLAYER0 << slot (team 0).
        // Rewrite slots beyond the original four (and the observer mirror
        // slot) shift as release builds always did; a debug build panicked.
        let faction = definition.faction | 0x100_i32.wrapping_shl(slot as u32);
        let mut avatar = Self {
            handle,
            slot,
            playing: true,
            body,
            character: RuntimeCharacter::new(definition),
            motion: CharacterMotion::new(animations),
            ai: AvatarAi::default(),
            move_abilities,
            template_type,
            trigger_type: crate::scene_loading::custom_hash("wizard"),
            faction,
            rig: None,
            staff_orb: IDENTITY,
            host_cast_source: None,
            mouse_pick: None,
            polymorphed: false,
            weapon_transform: IDENTITY,
            arcane_blades: Vec::new(),
            next_blade_id: 0,
            conjure: AvatarInput::default(),
            cast: CastInput::default(),
            spell: Spell::default(),
            current_spell: None,
            current_spell_definition: None,
            items: Vec::new(),
            melee_requests: Vec::new(),
            missile_requests: Vec::new(),
            attack: AttackInput::default(),
            boost: crate::shield_boost::ShieldBoost::default(),
            focused: false,
            magick_tree: Arc::new(MagickTree::default()),
            registered_magicks: Arc::new(Vec::new()),
            registered_unlocked: Vec::new(),
            magicks_allowed: 0,
            element_cooldowns: [ElementCooldown {
                cooldown: 0.0,
                active: true,
            }; 8],
            elements_were_down: [false; 8],
            controls_inverted: false,
            cast_self_was_down: false,
            pending_staff_cast: None,
            warded_expulsion: None,
            ice_cacoon: None,
            rapid_strike: None,
            focused_animation_speed: 1.0,
            input_gates: (false, false),
            can_chant: true,
            rng: 0x9E37_79B9_7F4A_7C15 ^ u64::from(handle),
            mouse_move: false,
            pad_was_active: false,
            pad_attack_was_down: false,
            move_was_down: false,
            equipment: [Resistance::IDENTITY; 11],
            events: Vec::new(),
            terminated: false,
            rail_kills: Vec::new(),
            play_time: 0.0,
        };
        // Character.Initialize: mDeadTimer = 20 + 20·random.
        avatar.motion.dead_timer = 20.0 + 20.0 * avatar.random() as f32;
        avatar
    }

    /// Seeded xorshift in [0, 1); not the native System.Random sequence.
    fn random(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Avatar.MouseMoveStop.
    pub fn stop_mouse_move(&mut self) {
        self.mouse_move = false;
    }

    pub fn dead(&self) -> bool {
        self.character.combat.dead && !self.character.combat.cannot_die_without_explicit_kill
    }
    /// Refresh the Avatar override for shared character-only damage adapters.
    pub fn sync_immortality(&mut self, in_cutscene: bool) {
        self.character.avatar_in_event = self.ai.controls_avatar();
        self.character.avatar_in_cutscene = in_cutscene;
    }

    /// Character.CurrentSpell.Stop (Nullify); true when a spell was active.
    pub fn stop_current_spell(&mut self) -> bool {
        let Some(effect) = self.current_spell.as_mut() else {
            return false;
        };
        stop_spell_effect(effect, &mut self.events, &mut self.rail_kills);
        true
    }

    pub fn go_to_animation(&mut self, animation: u16, time: f32) {
        self.motion.go_to_animation(animation, time);
    }

    /// VersusRuleset.MovePlayer's reset after the body move: ResetSpell,
    /// SpellQueue and icons cleared, CastType None, IdleState.
    pub fn reset_to_idle(&mut self, body: &mut CharacterBody) {
        self.stop_current_spell();
        self.current_spell = None;
        self.conjure.queue.clear();
        let facts = self.facts(false, body.is_touching_ground());
        let (motion, mut host) = self.split();
        motion.change_state(body, &facts, AvatarState::Idle, &mut host);
    }

    fn facts(&mut self, in_cutscene: bool, on_ground: bool) -> CharacterFacts {
        CharacterFacts {
            undie: self.character.undie_timer.is_some(),
            hit_points: self.character.combat.health.hit_points,
            cannot_die_without_explicit_kill: self
                .character
                .combat
                .cannot_die_without_explicit_kill,
            dead: self.character.combat.dead,
            overkilled: self.character.combat.overkilled,
            avatar: true,
            cast_type: self.cast.cast_type,
            spell_type: self.spell.spell_type(),
            magick_animation: (self.spell.element == E::ALL)
                .then(|| magicka_sim::magicks::animation(self.spell.magick))
                .flatten()
                .and_then(crate::character_motion::animation_index),
            charge_unlocked: self.cast.charge_unlocked,
            in_cutscene,
            frozen: self
                .character
                .combat
                .statuses
                .has(magicka_sim::status::Status::Frozen),
            random: self.random(),
            weapon_spell: self
                .items
                .iter()
                .find(|(slot, _)| *slot == 0)
                .filter(|(_, item)| item.spells.charged() && self.conjure.queue.is_empty())
                .map(|(_, item)| item.spells.peek().spell_type()),
            fear: self.character.fear_source,
            // Avatar PanicState entry stays fear-only: the status panic and
            // Ride stumble branches are wired for NPCs (research/npc-agents.md).
            panicking: false,
            stumbling: false,
            max_panic: self.character.status_frame.max_panic,
            stunned: self.character.combat.stun_timer > 0.0 && !self.character.combat.dead,
            knocked_down: self.character.combat.motion.knocked_down,
            on_ground,
            grip: {
                let mut grip = self.character.grip.facts();
                grip.entangled = self.character.entanglement.entangled();
                grip
            },
        }
    }

    /// SpawnAnimation, then ChangeState(RessurectionState.Instance).
    pub fn begin_resurrection(&mut self, body: &mut CharacterBody, animation: u16) {
        let facts = self.facts(false, body.is_touching_ground());
        let (motion, mut host) = self.split();
        motion.begin_resurrection(body, &facts, animation, &mut host);
    }

    /// Avatar.Events setter for script AvatarMove: a null/empty list changes
    /// to IdleState and plays idle with a 0.05 s blend.
    pub fn set_events(
        &mut self,
        body: &mut CharacterBody,
        events: Option<Vec<crate::ai_event::AiEvent>>,
    ) {
        if self.ai.set_events(events) {
            let facts = self.facts(false, body.is_touching_ground());
            let (motion, mut host) = self.split();
            motion.change_state(body, &facts, AvatarState::Idle, &mut host);
            self.go_to_animation(ANIMATION_IDLE, 0.05);
        }
        self.character.avatar_in_event = self.ai.controls_avatar();
    }

    /// Avatar.UpdateMouseDirection (fear gating; no charm/panic/stumble).
    fn update_mouse_direction(&mut self, body: &mut CharacterBody, mut value: [f32; 2]) {
        if self.ai.controls_avatar() {
            return;
        }
        if value[0].is_nan() || value[1].is_nan() {
            value = [0.0, 0.0];
        }
        let feared = self.character.fear.is_feared(self.character.combat.dead);
        let num = value[0] * value[0] + value[1] * value[1];
        if num < 1e-6 && !feared {
            body.set_movement([0.0; 3]);
            let forward = body.orientation.forward();
            body.set_desired_direction(forward);
            return;
        }
        let mut direction = [value[0], 0.0, value[1]];
        if num > 1e-6 {
            let inv = 1.0 / num.sqrt();
            direction = direction.map(|v| v * inv);
        }
        if !self.mouse_move {
            if !feared && num > 0.1 {
                body.set_movement(direction);
            }
            direction = [0.0; 3];
        }
        if !feared {
            body.set_movement(direction);
        }
    }

    /// Character.CastType setter: a change to Force/Area/Self combines the
    /// queue into the prepared spell (Avatar.CombineSpell).
    fn after_cast_type_change(&mut self, before: CastType) {
        // Avatar.CastType setter: ignored while polymorphed.
        if self.polymorphed {
            self.cast.cast_type = before;
            return;
        }
        let now = self.cast.cast_type;
        if now != before && now != CastType::Weapon && now != CastType::Blast {
            let mut events = Vec::new();
            self.spell = self.conjure.combine_avatar_spell(&mut events);
        }
    }

    fn cast_context(
        &self,
        body: &CharacterBody,
        input_limited: bool,
        cast_disabled: bool,
        push_disabled: bool,
    ) -> CastInputContext {
        CastInputContext {
            networked: false,
            staff_equipped: self.rig.as_ref().is_some_and(|r| r.spell_source.is_some()),
            cast_enabled: !cast_disabled,
            push_enabled: !push_disabled,
            queue_nonempty: !self.conjure.queue.is_empty(),
            input_limited,
            current_spell: self.current_spell.is_some(),
            frozen: false,
            flying: false,
            pushed: body.is_pushed(),
            knocked_down: false,
            casting: self.motion.state == AvatarState::Cast,
            charging: self.motion.state == AvatarState::Charge,
            item_special: self
                .items
                .iter()
                .any(|(slot, item)| *slot == 1 && item.definition.special_ability.is_some()),
            source_item_index: 1,
        }
    }

    /// Avatar.HandleCombo for one element key.
    fn conjure_element(
        &mut self,
        element: E,
        position: Vector,
        facing: Vector,
        play_time: f64,
        elements_disabled: u32,
    ) {
        self.refresh_equipment_resistances();
        let ctx = ConjureContext {
            in_event: self.ai.controls_avatar(),
            polymorphed: false,
            charging: self.motion.state == AvatarState::Charge,
            networked: false,
            status_mask: self.character.combat.statuses.current_mask,
            wet_lightning_passive: self.character.wet_lightning_passive,
            can_chant: self.can_chant,
        };
        let tree = self.magick_tree.clone();
        let allowed = self.magicks_allowed;
        for direction in conjure_directions(element) {
            let mut events = Vec::new();
            let mut hazard = None;
            let result = self.conjure.direction(
                direction,
                ctx,
                &tree,
                (
                    |e: E| e.0 & !elements_disabled != 0,
                    |m| magicka_sim::magicks::allowed(allowed, m),
                ),
                |damages, _| hazard = Some(damages),
                &mut events,
            );
            self.events
                .extend(events.into_iter().map(AvatarEvent::Conjure));
            if let Some(damages) = hazard {
                // Wet lightning / greased fire: the caster's own Damage route.
                deliver_to_character(
                    &mut self.character,
                    &self.equipment,
                    true,
                    self.handle,
                    position,
                    facing,
                    DamagePayload::Collection(damages),
                    Some(self.handle),
                    position,
                    play_time,
                );
            }
            if result.is_err() {
                break;
            }
        }
    }

    pub(crate) fn action_pressed(&self, down: bool) -> bool {
        down && !self.move_was_down
    }

    /// KeyboardMouseController avatar section, after the frame update. The
    /// press frame selects targets instead of calling MouseMove.
    pub fn handle_input(
        &mut self,
        body: &mut CharacterBody,
        input: PlayerInput,
        frame: InputFrame,
    ) {
        let InputFrame {
            pick,
            input_limited,
            in_cutscene,
            dt,
            play_time,
            weapon_cast_disabled,
            boost_target,
            move_locked,
            magicks_allowed,
            magick_cast_disabled,
            elements_disabled,
            force_cast_disabled,
            area_cast_disabled,
            self_cast_disabled,
            push_disabled,
        } = frame;
        self.mouse_pick = pick;
        self.magicks_allowed = magicks_allowed;
        if self.dead() {
            return;
        }
        self.boost.target = boost_target;
        let boost_pressed = input.boost_down && !self.boost.was_down;
        self.boost.was_down = input.boost_down;
        let pressed = input.move_down && !self.move_was_down;
        let released = !input.move_down && self.move_was_down;
        self.move_was_down = input.move_down;
        let attack_pressed = if input.pad_movement.is_some() {
            input.pad_attack && !self.pad_attack_was_down
        } else {
            pressed
        };
        let attack_released = if input.pad_movement.is_some() {
            !input.pad_attack && self.pad_attack_was_down
        } else {
            released
        };
        self.pad_attack_was_down = input.pad_attack;
        if input.shift_down {
            self.mouse_move = false;
        }
        if self.pad_was_active
            && input.pad_movement.is_none()
            && !self.character.fear.is_feared(self.character.combat.dead)
            && self.character.hypnotized.is_none()
            && !self.ai.controls_avatar()
        {
            // Source changes can happen while paused, when no input frame runs.
            body.set_movement([0.0; 3]);
        }
        self.pad_was_active = input.pad_movement.is_some();
        if let Some(direction) = self.character.hypnotized {
            // UpdateMouseDirection/UpdatePadDirection return while
            // hypnotized; the Hypnotize movement is held.
            body.set_movement(direction);
        } else if input_limited {
            if !self.ai.controls_avatar() {
                body.set_movement([0.0; 3]);
            }
        } else if let Some(movement) = input.pad_movement {
            // Independent sticks must not turn walking into a click or attack.
            self.mouse_move = false;
            if !in_cutscene
                && !move_locked
                && !self.ai.controls_avatar()
                && !self.character.fear.is_feared(self.character.combat.dead)
            {
                let sign = if self.controls_inverted { -1.0 } else { 1.0 };
                let movement = movement.map(|v| if v.is_finite() { v * sign } else { 0.0 });
                let length = movement[0].hypot(movement[1]).max(1.0);
                body.set_movement([movement[0] / length, 0.0, movement[1] / length]);
                if let Some(pick) = pick {
                    let dx = (pick[0] - body.position[0]) * sign;
                    let dz = (pick[2] - body.position[2]) * sign;
                    let length = dx.hypot(dz);
                    if length.is_finite() && length > 1e-5 {
                        body.set_desired_direction([dx / length, 0.0, dz / length]);
                    }
                }
            } else if !self.ai.controls_avatar()
                && !self.character.fear.is_feared(self.character.combat.dead)
            {
                body.set_movement([0.0; 3]);
            }
        } else if let Some(pick) = pick {
            let p = body.position;
            let mut value = [pick[0] - p[0], pick[2] - p[2]];
            if self.controls_inverted {
                value = [-value[0], -value[1]];
            }
            self.update_mouse_direction(body, value);
        }
        if !input.move_down || move_locked {
            self.mouse_move = false;
            if move_locked && self.character.hypnotized.is_none() {
                body.set_movement([0.0; 3]);
            }
        } else if !pressed && !input.shift_down && input.pad_movement.is_none() {
            self.mouse_move = true;
        }
        if input_limited || in_cutscene {
            self.elements_were_down = input.elements;
            self.cast_self_was_down = input.cast_self_down;
            return;
        }
        // XInputController.Update assigns Avatar.IsBlocking from the held
        // Block binding every frame (inside the input-limited gate); the
        // Avatar setter ignores it while polymorphed. The NPC one-second
        // Block timer is never ticked for avatars, so it must not be used.
        if input.pad_movement.is_some() && !self.polymorphed {
            self.motion.blocking = input.pad_block;
        }
        // ElementManager.Update runs with the controller outside cutscenes.
        for c in &mut self.element_cooldowns {
            c.update(dt);
        }
        let ctx = self.cast_context(
            body,
            input_limited,
            if input.shift_down {
                area_cast_disabled
            } else {
                force_cast_disabled
            },
            push_disabled,
        );
        // Avatar.Boost: CombineMagick (unlocked, tutorial-enabled) or an
        // already combined magick takes priority over shield boosting.
        // Avatar.Boost returns in ChargeState or while polymorphed.
        let mut boost_pressed =
            boost_pressed && self.motion.state != AvatarState::Charge && !self.polymorphed;
        if boost_pressed
            && !magick_cast_disabled
            && (self
                .magick_tree
                .combine_magick(&mut self.conjure.queue, |m| {
                    magicka_sim::magicks::allowed(magicks_allowed, m)
                })
                || self.combine_registered_magick()
                || self
                    .conjure
                    .queue
                    .first()
                    .is_some_and(|s| s.element == E::ALL))
        {
            boost_pressed = false;
            let before = self.cast.cast_type;
            self.cast.cast_type = CastType::Magick;
            self.after_cast_type_change(before);
            let instant = magicka_sim::magicks::is_instant(self.spell.magick);
            let frozen = self
                .character
                .combat
                .statuses
                .has(magicka_sim::status::Status::Frozen);
            // FlyingState/KnockDownState are not reconstructed states here.
            if instant && frozen {
                let (_, mut host) = self.split();
                host.cast_spell(true, Some(""));
            }
        }
        // Avatar.Boost: else, while entangled or gripped, BreakFree (the
        // counter grows by BreakFreeStrength, DamageEntanglement(0.5,
        // Earth)) and play spec_entangled_attack unless casting.
        if boost_pressed
            && (self.character.grip.gripper.is_some() || self.character.entanglement.entangled())
        {
            boost_pressed = false;
            self.character.grip.break_free_counter +=
                self.character.definition.break_free_strength as i32;
            let c = &mut self.character;
            c.entanglement.decrease(
                0.5,
                magicka_sim::spells::Elements::EARTH,
                &mut c.entangle_events,
            );
            if self.motion.state != AvatarState::Cast {
                self.go_to_animation(
                    crate::character_motion::ANIMATION_SPEC_ENTANGLED_ATTACK,
                    0.1,
                );
            }
        }
        if boost_pressed
            && (boost_target.is_some()
                || (self.character.combat.health.shield.active()
                    && !self.character.combat.health.shield.solid()))
        {
            let staff = self.items.iter().find(|(slot, _)| *slot == 1);
            let passive = staff.is_some_and(|(_, item)| item.definition.passive_ability == 1);
            let multipliers = staff
                .into_iter()
                .flat_map(|(_, item)| &item.definition.auras)
                .filter_map(|a| match a.kind {
                    magicka_content::common::Aura::Boost { magnitude } => Some(magnitude),
                    _ => None,
                })
                .chain(self.character.buffs.general.boost_amounts())
                .collect::<Vec<_>>();
            self.boost.press(
                self.motion.state == AvatarState::Boost,
                passive,
                multipliers,
            );
        }
        let before = self.cast.cast_type;
        let mut cast_events = Vec::new();
        if input.shift_down {
            self.cast.force_released(false, &mut cast_events);
            if input.cast_down {
                self.cast.area_pressed(ctx, &mut cast_events);
            } else {
                self.cast.area_released(false, &mut cast_events);
            }
        } else {
            self.cast.area_released(false, &mut cast_events);
            if input.cast_down {
                self.cast.force_pressed(ctx, &mut cast_events);
            } else {
                self.cast.force_released(false, &mut cast_events);
            }
        }
        if input.pad_movement.is_some() || input.shift_down {
            let weapon = self
                .items
                .iter()
                .find(|(slot, _)| *slot == 0)
                .map(|(_, item)| item);
            let gun = weapon.is_some_and(AvatarItem::is_gun);
            let charged = weapon.is_some_and(|w| w.spells.charged());
            self.input_gates = (input_limited, weapon_cast_disabled);
            if attack_pressed
                || ((if input.pad_movement.is_some() {
                    input.pad_attack
                } else {
                    input.move_down
                }) && (gun || charged))
            {
                self.press_attack(input_limited, weapon_cast_disabled);
            } else if attack_released {
                self.cast.attack_released(gun, &mut cast_events);
            }
        } else if self.motion.attacking {
            let gun = self
                .items
                .iter()
                .find(|(slot, _)| *slot == 0)
                .is_some_and(|(_, w)| w.is_gun());
            self.cast.attack_released(gun, &mut cast_events);
        }
        if cast_events
            .contains(&magicka_sim::avatar_cast_input::CastInputEvent::ClearNextAttackAnimation)
        {
            self.motion.next_attack_animation = crate::animation_controller::ANIMATION_NONE;
        }
        self.after_cast_type_change(before);
        let before = self.cast.cast_type;
        if input.cast_self_down && !self.cast_self_was_down {
            let ctx = self.cast_context(body, input_limited, self_cast_disabled, push_disabled);
            self.cast.special(ctx, |_, _| {}, &mut cast_events);
        } else if !input.cast_self_down && self.cast_self_was_down {
            self.cast.special_released();
        }
        self.cast_self_was_down = input.cast_self_down;
        self.after_cast_type_change(before);
        for event in &cast_events {
            if let magicka_sim::avatar_cast_input::CastInputEvent::ExecuteItemSpecial(slot) = event
            {
                self.execute_item_special(*slot, body, play_time);
            }
        }
        for (i, &element) in ELEMENT_KEYS.iter().enumerate() {
            let key = if self.controls_inverted {
                if input.pad_movement.is_some() {
                    PAD_INVERTED_KEYS[i]
                } else {
                    INVERTED_KEYS[i]
                }
            } else {
                i
            };
            let down = input.elements[key];
            // KeyboardMouseController skips the element keys when polymorphed.
            if down
                && !self.polymorphed
                && !self.elements_were_down[key]
                && !self.element_cooldowns[i].active
            {
                self.conjure_element(
                    element,
                    body.position,
                    body.direction(),
                    play_time,
                    elements_disabled,
                );
                self.element_cooldowns[i] = ElementCooldown {
                    cooldown: 0.125,
                    active: true,
                };
                self.events.push(AvatarEvent::ElementCooldown(element));
            }
        }
        self.elements_were_down = input.elements;
    }

    /// Avatar.Attack for the held weapon (input press, RapidStrike slashes).
    fn press_attack(&mut self, input_limited: bool, weapon_cast_disabled: bool) {
        let Some(weapon) = self
            .items
            .iter()
            .find(|(slot, _)| *slot == 0)
            .map(|(_, item)| item)
        else {
            return;
        };
        let gun = weapon.is_gun();
        let charged = weapon.spells.charged();
        let ctx = AttackContext {
            current_spell: self.current_spell.is_some(),
            casting: self.motion.state == AvatarState::Cast,
            charging: self.motion.state == AvatarState::Charge,
            input_limited,
            item_cooling_down: weapon.cooldown > 0.0,
            queued_elements: self
                .conjure
                .queue
                .first()
                .is_some_and(|s| s.element != E::ALL),
            weapon_charged: charged,
            focused: self.focused,
            weapon_cast_enabled: !weapon_cast_disabled,
            attacking: self.motion.attacking,
            animation_time: self.motion.controller.time,
            animation_duration: self.motion.controller.clip_duration(),
            weapon_class: i32::from(weapon.definition.weapon_class),
            wielding_gun: gun,
            ..Default::default()
        };
        let rng = &mut self.rng;
        let mut events = Vec::new();
        self.attack.press(
            &mut self.cast,
            ctx,
            || {
                *rng ^= *rng << 13;
                *rng ^= *rng >> 7;
                *rng ^= *rng << 17;
                (*rng >> 11) as f64 / (1u64 << 53) as f64
            },
            &mut events,
        );
        for event in events {
            if let AttackEvent::Attack {
                animation,
                wielding_gun,
            } = event
            {
                self.motion.attack(animation as u16, wielding_gun);
            }
            self.events.push(AvatarEvent::Attack(event));
        }
    }

    fn execute_item_special(&mut self, slot: i32, body: &CharacterBody, play_time: f64) {
        let Some((_, item)) = self.items.iter_mut().find(|(index, _)| *index == slot) else {
            return;
        };
        let Some((recharge, ability)) = item.definition.special_ability.clone() else {
            return;
        };
        // Native bgt.un rejects both positive and unordered cooldowns.
        if item.special_cooldown.is_nan() || item.special_cooldown > 0.0 {
            if ability.type_name.eq_ignore_ascii_case("IceCacoon") {
                // IceCacoon.IceBreak: end the effect on its next update.
                if let Some(timer) = self.ice_cacoon.as_mut() {
                    if *timer > 0.0 {
                        *timer = 0.0;
                    }
                }
            }
            return;
        }
        let animation = if ability.animation.is_empty() {
            // SpecialAbility.Read initializes the animation to cast_self (58).
            Some(58)
        } else {
            crate::dialog_enums::ANIMATIONS
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(&ability.animation))
                .map(|(_, animation)| *animation as u16)
        };
        if let Some(animation) = animation {
            self.motion.attack(animation, false);
        }
        self.events.push(AvatarEvent::ItemSpecial {
            slot,
            name: ability.type_name.clone(),
        });
        if ability.type_name.eq_ignore_ascii_case("invisibility") {
            self.character.set_invisible(15.0);
            self.character.just_cast_invisible = true;
            if let Some((_, item)) = self.items.iter_mut().find(|(index, _)| *index == slot) {
                item.special_cooldown = recharge;
            }
            return;
        }
        let local = ["IceCacoon", "RapidStrike", "RandomMine"]
            .iter()
            .find(|n| ability.type_name.eq_ignore_ascii_case(n));
        if let Some(&name) = local {
            match name {
                "IceCacoon" => self.ice_cacoon_execute(),
                "RapidStrike" => self.rapid_strike_execute(),
                _ => self.random_mine_execute(body, play_time),
            }
            if let Some((_, item)) = self.items.iter_mut().find(|(index, _)| *index == slot) {
                item.special_cooldown = recharge;
            }
            return;
        }
        if ability.type_name.eq_ignore_ascii_case("WardedExpulsion") {
            self.warded_expulsion(body, play_time);
            if let Some((_, item)) = self.items.iter_mut().find(|(index, _)| *index == slot) {
                item.special_cooldown = recharge;
            }
            return;
        }
        if !ability.type_name.eq_ignore_ascii_case("HealFriendlies") {
            // The scene executes the ability (SceneWorld::execute_special).
            if let Some((_, item)) = self.items.iter_mut().find(|(index, _)| *index == slot) {
                item.special_cooldown = recharge;
            }
            return;
        }
        let mut spell = Spell::default_for(E::LIFE);
        spell.mag[6] = 2.0;
        let mut queue_events = Vec::new();
        magicka_sim::spells::try_add_to_queue(&mut self.conjure.queue, 5, spell, &mut queue_events);
        let before = self.cast.cast_type;
        self.cast.cast_type = CastType::SelfCast;
        self.after_cast_type_change(before);
        let source = self.spray_source(body);
        let (_, mut host) = self.split();
        host.source = Some(source);
        host.play_time = play_time;
        host.cast_spell(true, Some(""));
        self.events.push(AvatarEvent::HealFriendlies);
        if let Some((_, item)) = self.items.iter_mut().find(|(index, _)| *index == slot) {
            item.special_cooldown = recharge;
        }
    }

    pub fn apply_fall_damage(
        &mut self,
        body: &mut CharacterBody,
        amount: f32,
        knockdown: bool,
        play_time: f64,
    ) {
        self.refresh_equipment_resistances();
        self.character.avatar_in_event = self.ai.controls_avatar();
        let mut attack = A::DAMAGE;
        if knockdown {
            attack |= A::KNOCKDOWN;
        }
        let damage = Damage {
            attack,
            element: E::EARTH.0,
            amount,
            magnitude: 1.0,
        };
        let result = deliver_to_body(
            &mut self.character,
            body,
            &self.equipment,
            true,
            self.handle,
            DamagePayload::Single(damage),
            None,
            body.position,
            play_time,
        );
        self.events.push(AvatarEvent::FallDamage { amount, result });
        body.owner.hit_points = self.character.combat.health.hit_points;
    }

    /// Character.Damage from another entity through the public route.
    pub fn receive_damage(
        &mut self,
        body: &mut CharacterBody,
        damage: Damage,
        attacker: u16,
        attack_position: Vector,
        play_time: f64,
    ) -> u16 {
        self.receive_payload(
            body,
            DamagePayload::Single(damage),
            Some(attacker),
            attack_position,
            play_time,
        )
    }

    /// IDamageable.Damage with any payload (DamageCollection5 for beams and
    /// blasts), through the public route with this avatar's equipment.
    pub fn receive_payload(
        &mut self,
        body: &mut CharacterBody,
        payload: DamagePayload,
        attacker: Option<u16>,
        attack_position: Vector,
        timestamp: f64,
    ) -> u16 {
        self.refresh_equipment_resistances();
        self.character.avatar_in_event = self.ai.controls_avatar();
        self.character.status_current_spell = self.current_spell.is_some();
        let result = deliver_to_body(
            &mut self.character,
            body,
            &self.equipment,
            true,
            self.handle,
            payload,
            attacker,
            attack_position,
            timestamp,
        );
        body.owner.hit_points = self.character.combat.health.hit_points;
        self.apply_status_spell_stop();
        result
    }

    pub(crate) fn apply_status_spell_stop(&mut self) {
        apply_status_spell_stop(
            &mut self.character,
            &mut self.current_spell,
            &mut self.events,
            &mut self.rail_kills,
        );
    }

    /// Character.Kill (no Frozen overkill): hit points to zero; DeadState
    /// and Die() follow through the state machine.
    pub fn kill(&mut self) {
        if !self.character.combat.cannot_die_without_explicit_kill {
            self.character.combat.health.hit_points = 0.0;
        }
    }

    /// RailGunSpell.CastWeapon starts an ArcaneBlade (range TotalMagnitude·
    /// 0.5 + 1.5); DeInitialize (the weapon effect no longer active) kills
    /// it. Blades update with Equipment[0]'s transform and are removed once
    /// faded.
    fn update_arcane_blades(&mut self, item: Option<Mat>, dt: f32) {
        use crate::arcane_blade::ArcaneBlade;
        for event in &self.events {
            if let AvatarEvent::RailWeapon { spell, .. } = event {
                self.next_blade_id += 1;
                self.arcane_blades.push(ArcaneBlade::new(
                    self.next_blade_id,
                    spell.element,
                    spell.total_magnitude() * 0.5 + 1.5,
                ));
            }
        }
        let weapon_live = matches!(
            &self.current_spell,
            Some(SpellEffect::Weapon {
                kind: EffectKind::RailGun,
                lifecycle,
                ..
            }) if lifecycle.active
        );
        let newest = self.arcane_blades.len().checked_sub(1);
        for (i, blade) in self.arcane_blades.iter_mut().enumerate() {
            if !(weapon_live && Some(i) == newest) {
                blade.kill();
            }
        }
        if let Some(item) = item {
            for blade in &mut self.arcane_blades {
                blade.update(&item, dt);
            }
        }
        self.arcane_blades.retain(|b| !b.is_dead());
    }

    /// The live IceBlade's trail vertex positions.
    pub fn ice_blade_trail(&self) -> Option<&[Vector; ICE_BLADE_TRAIL]> {
        IceBlade::trail_of(self.current_spell.as_ref())
    }

    /// The live IceBlade's range (ProjectileSpell.mIceBlade on Equipment[0]).
    pub fn ice_blade_range(&self) -> Option<f32> {
        match &self.current_spell {
            Some(SpellEffect::Weapon {
                ice_blade: Some(blade),
                ..
            }) => Some(blade.range),
            _ => None,
        }
    }

    /// Killed by the kill plane: Terminate (dead immediately).
    pub fn terminate(&mut self) {
        if !self.dead() {
            self.terminated = true;
            self.character.combat.health.hit_points = 0.0;
            self.character.combat.dead = true;
            self.character.buffs.general.clear();
            self.motion.dead_timer = -100.0;
            self.current_spell = None;
            self.events.push(AvatarEvent::Died);
        }
    }

    fn split(&mut self) -> (&mut CharacterMotion, AvatarCharacterHost<'_>) {
        self.refresh_equipment_resistances();
        (
            &mut self.motion,
            AvatarCharacterHost {
                handle: self.handle,
                equipment: &self.equipment,
                character: &mut self.character,
                cast: &mut self.cast,
                conjure: &mut self.conjure,
                spell: &mut self.spell,
                current_spell: &mut self.current_spell,
                current_spell_definition: &mut self.current_spell_definition,
                events: &mut self.events,
                rail_kills: &mut self.rail_kills,
                items: &mut self.items,
                melee_requests: &mut self.melee_requests,
                missile_requests: &mut self.missile_requests,
                source: None,
                play_time: 0.0,
                boost: &mut self.boost,
            },
        )
    }

    /// Avatar.Update AI branch, then Character.Update up to the state
    /// machine (phase 1); the scene then runs `cast_update` and `finish`.
    pub fn update(&mut self, mut host: ActorFrame<'_>, dt: f32) {
        if self.boost.cooldown > 0.0 {
            self.boost.cooldown -= dt;
        }
        self.character.status_current_spell = self.current_spell.is_some();
        self.apply_status_spell_stop();
        for (slot, item) in &mut self.items {
            item.cooldown -= dt;
            let was_cooling = item.special_cooldown > 0.0;
            item.special_cooldown -= dt;
            if was_cooling && item.special_cooldown <= 0.0 {
                self.events.push(AvatarEvent::SpecialReady { slot: *slot });
            }
        }
        self.sync_immortality(host.in_cutscene);
        self.update_staff_casts(host.body, host.play_time, dt);
        self.character.update_contact_timers(dt);
        let killed = {
            let mut ai = ActorAi {
                motion: &mut self.motion,
                frame: &mut host,
                killed: None,
                combat: None,
                group: &[],
                leader: None,
            };
            self.ai.update(&mut ai, dt);
            ai.killed
        };
        if killed.is_some() {
            self.kill();
        }
        self.cast
            .clear_completed_button(self.current_spell.is_some(), false, &mut Vec::new());
        self.sync_immortality(host.in_cutscene);
        let facts = self.facts(host.in_cutscene, host.body.is_touching_ground());
        let source = self.spray_source(host.body);
        let play_time = host.play_time;
        self.play_time = play_time;
        let (motion, mut character_host) = self.split();
        character_host.source = Some(source);
        character_host.play_time = play_time;
        // Character.Update: BlockItem = -1 before the animation actions.
        character_host.character.block_item = -1;
        let was_dead = character_host.character.combat.dead;
        motion.update(host.body, &facts, dt, &mut character_host);
        if !was_dead && self.character.combat.dead {
            // Character.Die's undie branch (mUndying read before the clear).
            self.character.arm_undie(self.motion.drowning);
        }
        self.update_shield_and_auras(host.body, dt);
        self.update_item_passives(host.body);
        // Character.Update: AnimationController.Speed = FocusedAnimationSpeed
        // while focused (applies to the next animation step).
        if self.focused {
            self.motion.controller.speed = self.focused_animation_speed;
        }
        self.character.tick_stun(dt);
    }

    fn spell_host(&mut self, body: &CharacterBody, play_time: f64) -> CharacterSpellHost<'_> {
        self.refresh_equipment_resistances();
        let source = self.spray_source(body);
        CharacterSpellHost {
            owner_is_character: true,
            avatar: true,
            handle: self.handle,
            equipment: &self.equipment,
            character: &mut self.character,
            current_spell: &mut self.current_spell,
            current_spell_definition: &mut self.current_spell_definition,
            events: &mut self.events,
            rail_kills: &mut self.rail_kills,
            source: Some(source),
            play_time,
        }
    }

    /// Deferred NorthPole cast and WardedExpulsion's SpellManager effect.
    fn update_staff_casts(&mut self, body: &CharacterBody, play_time: f64, dt: f32) {
        if let Some(StaffCast::NorthPole) = self.pending_staff_cast.take() {
            // NorthPole.Execute: ConjureCold ×3, CastType = Area,
            // CastSpell(fromStaff: true, null).
            for _ in 0..3 {
                self.conjure_element(E::COLD, body.position, body.direction(), play_time, 0);
            }
            self.cast.cast_type = CastType::Area;
            self.after_cast_type_change(CastType::Blast);
            let source = self.spray_source(body);
            let (_, mut host) = self.split();
            host.source = Some(source);
            host.play_time = play_time;
            host.cast_spell(true, None);
        }
        self.ice_cacoon_update(body, play_time, dt);
        self.rapid_strike_update(dt);
        if let Some(timer) = self.warded_expulsion.as_mut() {
            *timer -= dt;
            if *timer <= 0.0 {
                self.warded_expulsion = None;
                // WardedExpulsion.OnRemove: Shield-1 Spell.Cast(Area) and a
                // PushSpell.CastArea with the same spell. Neither becomes the
                // owner's current spell.
                let mut spell = Spell::default_for(E::SHIELD);
                spell.mag = [0.0; 11];
                spell.mag[E::SHIELD.index() as usize] = 1.0;
                let current = self.current_spell.take();
                let definition = self.current_spell_definition.take();
                let mut host = self.spell_host(body, play_time);
                host.cast(spell, CastType::Area, true);
                host.cast_effect(EffectKind::Push, spell, CastType::Area, true);
                self.current_spell = current;
                self.current_spell_definition = definition;
            }
        }
    }

    fn status(&mut self, kind: Status, dps: f32, magnitude: f32) {
        let mut events = Vec::new();
        let effect = magicka_sim::status::StatusEffect::new(kind, dps, magnitude, false);
        let _ = self.add_status_effect(effect, &mut events);
    }

    /// IceCacoon.Execute: Wet (twice, drying reset, while Burning, Greased
    /// or Poisoned), Cold 300, DryTimer 4 and a 4 s frozen budget.
    fn ice_cacoon_execute(&mut self) {
        self.ice_cacoon = Some(4.0);
        let statuses = &self.character.combat.statuses;
        if statuses.has(Status::Burning)
            || statuses.has(Status::Greased)
            || statuses.has(Status::Poisoned)
        {
            self.status(Status::Wet, 1.0, 1.0);
            self.character.combat.statuses.dry_timer = 0.0;
        }
        self.status(Status::Wet, 1.0, 1.0);
        self.status(Status::Cold, 300.0, 1.0);
        self.character.combat.statuses.dry_timer = 4.0;
    }

    /// IceCacoon.Update/OnRemove: while Frozen the budget runs down and the
    /// owner takes 300/s Cold damage; unfrozen (or IceBreak) it ends with
    /// Burning if still Frozen, a Cold-5 SpraySpell Area from the staff and
    /// an Area CastSpell of the queue.
    fn ice_cacoon_update(&mut self, body: &CharacterBody, play_time: f64, dt: f32) {
        self.refresh_equipment_resistances();
        let Some(mut timer) = self.ice_cacoon else {
            return;
        };
        if self.character.combat.statuses.has(Status::Frozen) {
            timer -= dt;
            let damage = Damage {
                attack: magicka_sim::damage::attack::DAMAGE,
                element: E::COLD.0,
                amount: 300.0 * dt,
                magnitude: 1.0,
            };
            deliver_to_character(
                &mut self.character,
                &self.equipment,
                true,
                self.handle,
                body.position,
                body.direction(),
                DamagePayload::Single(damage),
                Some(self.handle),
                body.position,
                0.0,
            );
        } else {
            timer = 0.0;
        }
        if timer > 0.0 {
            self.ice_cacoon = Some(timer);
            return;
        }
        self.ice_cacoon = None;
        self.character.combat.statuses.dry_timer = 0.0;
        if self.character.combat.statuses.has(Status::Frozen) {
            self.status(Status::Burning, 1.0, 1.0);
        }
        let mut spell = Spell::default_for(E::COLD);
        spell.mag = [0.0; 11];
        spell.mag[E::COLD.index() as usize] = 5.0;
        let current = self.current_spell.take();
        let definition = self.current_spell_definition.take();
        self.spell_host(body, play_time).cast_effect(
            EffectKind::Spray,
            spell,
            CastType::Area,
            true,
        );
        self.current_spell = current;
        self.current_spell_definition = definition;
        let before = self.cast.cast_type;
        self.cast.cast_type = CastType::Area;
        self.after_cast_type_change(before);
        let source = self.spray_source(body);
        let (_, mut host) = self.split();
        host.source = Some(source);
        host.play_time = play_time;
        host.cast_spell(true, Some(""));
    }

    /// RapidStrike.Execute: melee weapons only; the owner focuses (slow
    /// animation, then five quick slashes) for 2.5 s.
    fn rapid_strike_execute(&mut self) {
        let melee = self
            .items
            .iter()
            .find(|(slot, _)| *slot == 0)
            .is_some_and(|(_, w)| !w.is_gun() && !(10..=12).contains(&w.definition.weapon_class));
        if melee {
            self.focused = true;
            self.focused_animation_speed = 0.3;
            self.rapid_strike = Some(2.5);
        }
    }

    /// RapidStrike.Update/OnRemove: gripped owners stop; in the last
    /// 0.5 s each crossing of 0.4, 0.3, 0.2, 0.1 and 0 s attacks at 5.1×.
    fn rapid_strike_update(&mut self, dt: f32) {
        let Some(before) = self.rapid_strike else {
            return;
        };
        let timer = if self.character.grip.gripper.is_some() {
            0.0
        } else {
            before - dt
        };
        if timer <= 0.5 && before > 0.0 && self.character.grip.gripper.is_none() {
            let slash = (0..5)
                .map(|i| i as f32 * 0.1)
                .find(|&t| timer <= t)
                .is_some_and(|t| before > t);
            if slash {
                self.focused_animation_speed = 5.1;
                let (limited, disabled) = self.input_gates;
                self.press_attack(limited, disabled);
            }
        }
        if timer <= 0.0 {
            self.rapid_strike = None;
            self.focused = false;
            self.focused_animation_speed = 1.0;
        } else {
            self.rapid_strike = Some(timer);
        }
    }

    /// RandomMine.Execute (local avatar): Shield plus one of seven element
    /// sets, then a Force CastSpell from the staff.
    fn random_mine_execute(&mut self, body: &CharacterBody, play_time: f64) {
        let (position, facing) = (body.position, body.direction());
        let conjure =
            |avatar: &mut Self, e: E| avatar.conjure_element(e, position, facing, play_time, 0);
        conjure(self, E::SHIELD);
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        let pick = ((self.rng >> 11) as f64 / (1u64 << 53) as f64 * 7.0) as u32;
        let set: &[E] = match pick.min(6) {
            0 => &[E::ARCANE],
            1 => &[E::ARCANE, E::COLD],
            2 => &[E::ARCANE, E::WATER],
            3 => &[E::ARCANE, E::FIRE],
            4 => &[E::ARCANE, E::LIGHTNING],
            5 => &[E::ARCANE, E::FIRE, E::WATER],
            _ => &[E::LIFE],
        };
        for &e in set {
            conjure(self, e);
        }
        let before = self.cast.cast_type;
        self.cast.cast_type = CastType::Force;
        self.after_cast_type_change(before);
        let source = self.spray_source(body);
        let (_, mut host) = self.split();
        host.source = Some(source);
        host.play_time = play_time;
        host.cast_spell(true, None);
    }

    /// WardedExpulsion.Execute: an Area CastSpell from the staff with
    /// AdditionalPushRadius 10 and SpellPower 1, then a 0.8 s effect.
    fn warded_expulsion(&mut self, body: &CharacterBody, play_time: f64) {
        self.character.additional_push_radius = 10.0;
        let before = self.cast.cast_type;
        self.cast.cast_type = CastType::Area;
        self.after_cast_type_change(before);
        self.character.spell_power = 1.0;
        let source = self.spray_source(body);
        let (_, mut host) = self.split();
        host.source = Some(source);
        host.play_time = play_time;
        host.cast_spell(true, Some(""));
        self.character.additional_push_radius = 0.0;
        self.warded_expulsion = Some(0.8);
    }

    /// Item.Update's passive abilities that act on the holder: MoveSpeed
    /// (SpeedMultiplier *= Variable after Character.Update's reset) and the
    /// WetLightning flag Character.HasPassiveAbility reports.
    fn update_item_passives(&mut self, body: &mut CharacterBody) {
        const WET_LIGHTNING: u8 = 10;
        const MOVE_SPEED: u8 = 11;
        let attached = || self.items.iter().filter(|(_, i)| i.attached);
        self.character.wet_lightning_passive =
            attached().any(|(_, i)| i.definition.passive_ability == WET_LIGHTNING);
        for (_, item) in attached() {
            if item.definition.passive_ability == MOVE_SPEED {
                body.set_speed_multiplier(body.speed_multiplier() * item.definition.passive_value);
            }
        }
    }

    /// Character.Update: `if (mSelfShield.Active) mSelfShield.Update(this,
    /// dt)` after SpeedMultiplier = 1, then the owner-side shield events
    /// (mass, aura clearing). Aura TTLs count down here (AuraManager).
    fn update_shield_and_auras(&mut self, body: &mut CharacterBody, dt: f32) {
        let mut events = std::mem::take(&mut self.character.shield_events);
        let shield = &mut self.character.combat.health.shield;
        if shield.active() {
            let mut speed = body.speed_multiplier();
            shield.update(
                dt,
                &mut speed,
                self.character.definition.mass,
                false,
                &mut events,
            );
            body.set_speed_multiplier(speed);
        }
        for event in events {
            use magicka_sim::self_shield::ShieldEvent as S;
            match event {
                S::SetMass(mass) => body.mass = mass,
                S::ClearAura => self.character.auras.clear(),
                S::CircleDamage { radius, damage } => self
                    .events
                    .push(AvatarEvent::ShieldCircleDamage { radius, damage }),
                _ => {}
            }
        }
        for aura in &mut self.character.auras {
            aura.ttl -= dt;
        }
        self.character.auras.retain(|a| a.ttl > 0.0);
    }

    /// The owner transforms SpraySpell reads (previous frame's staff orb).
    pub fn spray_source(&self, body: &CharacterBody) -> SpraySource {
        let orb = self.host_cast_source.as_ref().unwrap_or(&self.staff_orb);
        SpraySource {
            owner_handle: self.handle,
            owner_position: body.position,
            owner_direction: body.direction(),
            cast_position: translation(orb),
            cast_direction: forward(orb),
            weapon_position: translation(&self.weapon_transform),
            weapon_direction: forward(&self.weapon_transform),
        }
    }

    /// Character.Update: mCurrentSpell.CastUpdate and turn speed.
    pub fn cast_update<
        W: SprayWorld
            + PushWorld
            + magicka_sim::lightning::LightningScene
            + magicka_sim::railgun::RailControl,
    >(
        &mut self,
        body: &mut CharacterBody,
        dt: f32,
        world: &mut W,
    ) {
        self.refresh_equipment_resistances();
        let mut source = self.spray_source(body);
        if self.host_cast_source.is_some()
            && matches!(&self.current_spell, Some(SpellEffect::Projectile(p))
                if p.lifecycle.cast_type == CastType::Force)
        {
            // Only aimed force projectiles use camera pitch. Self-cast rocks
            // and ice retain their native overhead/falling launch geometry.
            source.owner_direction = source.cast_direction;
        }
        CharacterSpellHost {
            owner_is_character: true,
            avatar: true,
            handle: self.handle,
            equipment: &self.equipment,
            character: &mut self.character,
            current_spell: &mut self.current_spell,
            current_spell_definition: &mut self.current_spell_definition,
            events: &mut self.events,
            rail_kills: &mut self.rail_kills,
            source: Some(source),
            play_time: self.play_time,
        }
        .update(
            body,
            dt,
            world,
            self.cast.cast_type,
            self.cast.pressed == CastType::Blast,
            self.motion.controller.has_finished(),
            &mut self.rng,
        );
    }

    /// Character.Update tail: staff orb from this frame's pose, dead timer,
    /// move animation speed; Avatar.Update chant flag and queue slowdown.
    pub fn finish(&mut self, body: &mut CharacterBody, in_cutscene: bool, dt: f32) {
        let mut blade_item = None;
        if let Some(rig) = &self.rig {
            let height_offset = -0.5 * body.length - body.radius;
            let parent = character_orientation(
                &self
                    .character
                    .scaled_static_transform(&rig.static_transform),
                &body.orientation,
                body.position,
                height_offset,
            );
            let skinned = rig.skeleton.skinned_for(&self.motion.controller, &parent);
            for (slot, item) in &mut self.items {
                if !item.attached {
                    item.frame = None;
                    continue;
                }
                if let Some(a) = rig.attachments.iter().find(|a| a.slot == *slot) {
                    item.melee.track(
                        translation(&a.attach_absolute(&skinned)),
                        translation(&a.item_transform(&skinned)),
                    );
                    item.frame = Some(crate::item_effects::ItemFrame::new(a, &skinned));
                }
            }
            if let Some(i) = rig.spell_source {
                self.staff_orb = rig.attachments[i].attach_absolute(&skinned);
            }
            blade_item = rig
                .attachments
                .iter()
                .find(|a| a.slot == 0)
                .map(|a| a.item_transform(&skinned));
            if let Some(weapon) = rig.attachments.iter().find(|a| a.slot == 0) {
                self.weapon_transform = weapon.attach_absolute(&skinned);
                if let Some(SpellEffect::Weapon {
                    ice_blade: Some(blade),
                    ..
                }) = self.current_spell.as_mut()
                {
                    let item = weapon.item_transform(&skinned);
                    if blade.trail.is_none() {
                        self.events
                            .push(AvatarEvent::IceBladeSpawned { orientation: item });
                    }
                    blade.update_trail(&item, dt);
                }
            }
        }
        self.update_arcane_blades(blade_item, dt);
        let facts = self.facts(in_cutscene, body.is_touching_ground());
        self.motion.finish(body, &facts, dt);
        self.can_chant = matches!(
            self.motion.state,
            AvatarState::Idle | AvatarState::Move | AvatarState::Cast
        );
        let queued = self.conjure.queue.len() as f32;
        if queued > 0.0 {
            let m = body.speed_multiplier() * (1.0 - queued / 5.0 * 0.6);
            body.set_speed_multiplier(m);
        }
        body.owner.hit_points = self.character.combat.health.hit_points;
    }
}

/// Avatar CharacterHost: Character.CastSpell → Spell.Cast, CastState exit,
/// Die().
struct AvatarCharacterHost<'a> {
    boost: &'a mut crate::shield_boost::ShieldBoost,
    handle: u16,
    equipment: &'a [Resistance; 11],
    character: &'a mut RuntimeCharacter,
    cast: &'a mut CastInput,
    conjure: &'a mut AvatarInput,
    spell: &'a mut Spell,
    current_spell: &'a mut Option<SpellEffect>,
    current_spell_definition: &'a mut Option<(CastType, Spell)>,
    events: &'a mut Vec<AvatarEvent>,
    rail_kills: &'a mut Vec<magicka_sim::railgun::RailId>,
    items: &'a mut Vec<(i32, AvatarItem)>,
    melee_requests: &'a mut Vec<crate::npc_combat::MeleeRequest>,
    missile_requests: &'a mut Vec<(i32, Option<Vector>, bool)>,
    source: Option<SpraySource>,
    play_time: f64,
}

impl CharacterHost for AvatarCharacterHost<'_> {
    fn execute_gun(&mut self, weapon: i32, accuracy: f32) {
        if let Some((_, item)) = self
            .items
            .iter_mut()
            .find(|(s, i)| *s == weapon && i.attached)
        {
            if !item.definition.gun_conditions.0.is_empty() {
                item.melee.clear_hitlist();
                item.gun.execute(
                    item.definition.gun_clip,
                    item.definition.gun_accuracy * accuracy,
                );
                item.cooldown = item.definition.cooldown_time;
            }
        }
    }
    fn stop_gun(&mut self, weapon: i32) {
        if let Some((_, item)) = self.items.iter_mut().find(|(s, _)| *s == weapon) {
            item.gun.stop(true, true, item.definition.weapon_class);
        }
    }
    fn stop_all_guns(&mut self) {
        for (_, item) in self.items.iter_mut() {
            item.gun.stop(true, false, item.definition.weapon_class);
        }
    }

    fn spawn_missile(&mut self, weapon: i32, velocity: Option<Vector>, item_aligned: bool) {
        self.missile_requests.push((weapon, velocity, item_aligned));
    }

    fn block_action(&mut self, weapon: i32) {
        self.character.block_item = weapon;
        self.character.block_values = self
            .items
            .iter()
            .map(|(slot, item)| (*slot, item.definition.block_value))
            .collect();
    }
    fn kill_block_action(&mut self) {
        self.character.block_item = -1;
    }
    fn remove_status(&mut self, mask: u16) {
        self.character.stop_status_effects(mask, true);
    }
    fn play_animation_sound(&mut self, bank: u16, sound: &str) {
        self.character
            .pending_animation_audio
            .push((bank, crate::scene_loading::custom_hash(sound)));
    }
    fn clear_knocked_down(&mut self) {
        self.character.combat.motion.knocked_down = false;
    }
    fn play_animation_effect(
        &mut self,
        index: usize,
        action: &magicka_content::character::AnimationAction,
        first: bool,
    ) {
        if let magicka_content::character::ActionKind::PlayEffect { bone, effect, .. } =
            &action.kind
        {
            self.events.push(AvatarEvent::AnimationEffect {
                index,
                bone: bone.clone(),
                effect: effect.to_lowercase(),
                retained: action.start_time < action.end_time,
                first,
            });
        }
    }
    fn stop_animation_effect(&mut self, index: usize) {
        self.events.push(AvatarEvent::AnimationEffectStopped(index));
    }
    fn wants_boost(&mut self) -> bool {
        if self.boost.count <= 0 {
            return false;
        }
        if self.boost.target.is_some() || self.character.combat.health.shield.active() {
            true
        } else {
            self.boost.count = 0;
            false
        }
    }
    fn update_boost(&mut self) -> bool {
        let shield = &mut self.character.combat.health.shield;
        if shield.active() && !shield.solid() {
            shield.heal(50.0 * self.boost.count as f32);
            self.boost.count = 0;
        } else if let Some(target) = self.boost.target {
            self.events.push(AvatarEvent::BoostShield {
                target,
                amount: 100.0 * self.boost.count as f32,
            });
            self.boost.count = 0;
        } else {
            return false;
        }
        self.boost.cooldown > 0.0
    }
    fn immortal_action(&mut self, collide: bool) {
        self.character.execute_immortal_action(collide);
    }
    fn kill_immortal_action(&mut self) {
        self.character.kill_immortal_action();
    }
    fn begin_charge(&mut self) {
        self.character.ethereal(false, 1.0, 1.0);
        self.character.set_invisible(0.0);
        self.character.spell_power = 0.;
        self.cast.charge_unlocked = false;
    }
    fn ethereal_action(&mut self, ethereal: bool, alpha: f32, speed: f32) {
        self.character
            .execute_ethereal_action(ethereal, alpha, speed);
    }
    fn entered_grip_attack_state(&mut self) {
        self.character.ethereal(false, 1.0, 1.0);
        self.character.set_invisible(0.0);
    }
    fn entered_attack_state(&mut self) {
        self.character.ethereal(false, 1.0, 1.0);
        self.character.enter_attack_visibility();
    }
    fn entered_cast_state(&mut self) {
        self.character.ethereal(false, 1.0, 1.0);
        self.character.set_invisible(0.0);
    }
    fn entered_panic_state(&mut self) {
        self.character.set_invisible(0.0);
    }
    fn set_spell_power(&mut self, power: f32) {
        self.character.spell_power = power;
    }
    fn cast_button_held(&self, kind: CastType) -> bool {
        self.cast.pressed == kind
    }
    fn end_charge(&mut self) {
        if self.spell.element != E::NONE {
            self.cast_spell(true, Some(""));
        }
    }

    fn cast_spell(&mut self, from_staff: bool, _source: Option<&str>) {
        let mut released = None;
        if self.cast.cast_type == CastType::Weapon && self.spell.element != E::ALL {
            let Some((_, item)) = self.items.iter_mut().find(|(slot, _)| *slot == 0) else {
                return;
            };
            self.conjure.prepared = *self.spell;
            let mut events = Vec::new();
            let dispatch = self.conjure.dispatch_weapon_cast(
                CastType::Weapon,
                false,
                &mut item.spells,
                &mut events,
            );
            self.events
                .extend(events.into_iter().map(AvatarEvent::Weapon));
            match dispatch {
                AvatarCastDispatch::Enchanted => return,
                AvatarCastDispatch::Release(spell) => released = Some(spell),
                AvatarCastDispatch::Base => {}
            }
        }
        if released.is_none() && self.current_spell.is_some() {
            return;
        }
        if self.spell.element == E::ALL {
            // Character.CastSpell: Magick.Effect.Execute, CastType = None
            // (the setter does not combine for None).
            self.cast.cast_type = CastType::Blast;
            match self.spell.registered_index() {
                Some(index) => self.events.push(AvatarEvent::CastRegisteredMagick(index)),
                None => self.events.push(AvatarEvent::CastMagick(self.spell.magick)),
            }
            *self.spell = Spell::default();
            return;
        }
        let spell = released.unwrap_or(*self.spell);
        let cast_type = self.cast.cast_type;
        CharacterSpellHost {
            owner_is_character: true,
            avatar: true,
            handle: self.handle,
            equipment: self.equipment,
            character: self.character,
            current_spell: self.current_spell,
            current_spell_definition: self.current_spell_definition,
            events: self.events,
            rail_kills: self.rail_kills,
            source: self.source,
            play_time: self.play_time,
        }
        .cast(spell, cast_type, from_staff);
        // Character.CastSpell: a self cast while entangled hits the web.
        if cast_type == CastType::SelfCast && self.character.entanglement.entangled() {
            let c = &mut *self.character;
            c.entanglement
                .decrease(0.0, spell.element, &mut c.entangle_events);
        }
        if released.is_none() {
            *self.spell = Spell::default();
        }
    }
    fn stop_execute(&mut self, weapon: i32) {
        self.melee_requests
            .push(crate::npc_combat::MeleeRequest::Stop(weapon));
    }
    fn deal_damage(&mut self, weapon: i32, targets: u8, first: bool) {
        if let Some((_, item)) = self.items.iter_mut().find(|(slot, _)| *slot == weapon) {
            if first {
                item.melee.clear_hitlist();
            }
            self.melee_requests
                .push(crate::npc_combat::MeleeRequest::Execute {
                    weapon,
                    targets,
                    first,
                });
        }
    }
    /// SpellEffect.AnimationEnd (CastSpell action Kill). ProjectileSpell's
    /// Weapon branch launches an UnderGroundAttack for Earth: the spell
    /// without Ice, from WeaponSource at 40 × Direction.xz, 15 m.
    fn spell_animation_end(&mut self) {
        CharacterSpellHost {
            owner_is_character: true,
            avatar: true,
            handle: self.handle,
            equipment: self.equipment,
            character: self.character,
            current_spell: self.current_spell,
            current_spell_definition: self.current_spell_definition,
            events: self.events,
            rail_kills: self.rail_kills,
            source: self.source,
            play_time: self.play_time,
        }
        .animation_end();
    }
    fn current_spell(&self) -> Option<bool> {
        self.current_spell.as_ref().map(SpellEffect::active)
    }
    fn end_cast(&mut self) {
        if let Some(mut effect) = self.current_spell.take() {
            stop_spell_effect(&mut effect, self.events, self.rail_kills);
        }
        self.cast.cast_type = CastType::Blast;
    }
    fn entered_dead_state(&mut self) {
        for (slot, item) in self.items.iter_mut() {
            if item.attached {
                item.attached = false;
                item.frame = None;
                self.events.push(AvatarEvent::ItemReleased { slot: *slot });
            }
        }
    }
    fn die(&mut self) {
        let combat = &mut self.character.combat;
        if combat.dead {
            return;
        }
        combat.dead = true;
        combat.death_status_mask = combat.statuses.current_mask;
        self.character.collision_hits.clear();
        // Character.Die: mBuffs.Clear() (general part).
        self.character.buffs.general.clear();
        if let Some(effect) = self.current_spell.take() {
            match effect {
                SpellEffect::ForceSpray(s) if s.lifecycle.active => {
                    self.events.push(AvatarEvent::SprayStopped)
                }
                SpellEffect::SelfSpray(s) if s.lifecycle.active => {
                    self.events.push(AvatarEvent::SelfEffectsStopped)
                }
                SpellEffect::SprayWeapon(s) if s.lifecycle.active => {
                    self.events.push(AvatarEvent::SelfEffectsStopped)
                }
                SpellEffect::SelfBeam(s) if s.lifecycle.active => {
                    self.events.push(AvatarEvent::SelfEffectsStopped)
                }
                SpellEffect::SelfLightning(s) if s.lifecycle.active => {
                    self.events.push(AvatarEvent::SelfEffectsStopped)
                }
                _ => {}
            }
        }
        self.conjure.queue.clear();
        self.events.push(AvatarEvent::Died);
    }
}

/// SpellCastHost at the CastSpell action: creates the effect. SpraySpell
/// CastForce performs no world queries, so the SprayWorld part is inert.
struct CastPhase<'a> {
    /// Character.GetSpellTTLModifier / GetSpellRangeModifier (mul, add).
    ttl_modifier: (f32, f32),
    range_modifier: (f32, f32),
    current: Option<SpellEffect>,
    events: &'a mut Vec<AvatarEvent>,
    source: SpraySource,
    facts: ForceCastFacts,
    power: &'a mut f32,
    additional_radius: f32,
    owner_damage: Vec<(DamagePayload, f64, Vector)>,
    /// ShieldSpell.CastSelf owner operations (with the cast spell).
    owner_shield: Option<(Spell, ShieldSelfPlan)>,
}
impl SpellCastHost for CastPhase<'_> {
    type Handle = EffectKind;
    fn notice(&mut self, notice: CastNotice) {
        self.events.push(AvatarEvent::CastNotice(notice));
    }
    fn acquire(&mut self, kind: EffectKind) -> EffectKind {
        kind
    }
    fn set_current(&mut self, _: EffectKind) {}
    fn cast(&mut self, kind: EffectKind, spell: Spell, cast_type: CastType, from_staff: bool) {
        if kind == EffectKind::RailGun && cast_type == CastType::Weapon {
            // RailGunSpell.CastWeapon: Beam/Weapon damage over an arc of
            // TotalMagnitude·0.5 + 1.5 around the owner, then the effect
            // lives while the owner's CastType stays Weapon.
            let mut damages = [Damage::default(); 5];
            match magicka_sim::damage::calculate_damage(&spell, SpellType::Beam, CastType::Weapon) {
                Ok(list) => {
                    for (slot, d) in damages.iter_mut().zip(list) {
                        *slot = d;
                    }
                }
                Err(e) => {
                    self.events.push(AvatarEvent::SpellNotReconstructed(format!(
                        "weapon beam damage: {e:?}"
                    )));
                    return;
                }
            }
            self.events.push(AvatarEvent::RailWeapon {
                spell,
                position: self.source.owner_position,
                direction: self.source.owner_direction,
                range: spell.total_magnitude() * 0.5 + 1.5,
                damages,
                timestamp: self.facts.timestamp,
            });
            if let Some(lifecycle) =
                EffectLifecycle::begin(cast_type, from_staff, self.facts.timestamp)
            {
                self.current = Some(SpellEffect::Weapon {
                    kind,
                    spell,
                    timestamp: self.facts.timestamp,
                    lifecycle,
                    ice_blade: None,
                });
            }
            return;
        }
        if kind == EffectKind::Projectile && cast_type == CastType::Weapon {
            // ProjectileSpell.CastWeapon: Ice grows an IceBlade on
            // Equipment[0] now; the Earth part acts at AnimationEnd
            // (UnderGroundAttack).
            let mut ice_blade = None;
            if spell.get(E::ICE) > 0.0 {
                let mut ice = spell;
                if let Some(i) = E::BY_INDEX.iter().position(|x| *x == E::EARTH) {
                    ice.mag[i] = 0.0;
                }
                ice.element = E(ice.element.0 & !E::EARTH.0);
                match magicka_sim::damage::calculate_damage(
                    &ice,
                    SpellType::Projectile,
                    CastType::Weapon,
                ) {
                    Ok(list) => {
                        let mut damages = [Damage::default(); 5];
                        for (slot, d) in damages.iter_mut().zip(list) {
                            *slot = d;
                        }
                        ice_blade = Some(IceBlade {
                            range: 1.25 * ice.get(E::ICE) + 3.75,
                            damages,
                            timestamp: self.facts.timestamp,
                            trail: None,
                            orientation: None,
                        });
                    }
                    Err(e) => self.events.push(AvatarEvent::SpellNotReconstructed(format!(
                        "ice blade damage: {e:?}"
                    ))),
                }
            }
            if let Some(lifecycle) =
                EffectLifecycle::begin(cast_type, from_staff, self.facts.timestamp)
            {
                self.current = Some(SpellEffect::Weapon {
                    kind,
                    spell,
                    timestamp: self.facts.timestamp,
                    lifecycle,
                    ice_blade,
                });
            }
            return;
        }
        if kind == EffectKind::Spray && cast_type == CastType::Weapon {
            match magicka_sim::spray_weapon::SprayWeaponPlan::new(&spell, self.facts.greased) {
                Ok(plan) => {
                    for &index in &plan.effect_indices {
                        self.events.push(AvatarEvent::SelfEffectStarted {
                            index,
                            name: magicka_sim::spray_weapon::SWORD_EFFECTS[index],
                            position: self.source.weapon_position,
                            direction: self.source.owner_direction,
                        });
                    }
                    self.events.push(AvatarEvent::SprayWeapon {
                        position: self.source.weapon_position,
                        direction: self.source.owner_direction,
                        cast_source: self.source.cast_position,
                        timestamp: self.facts.timestamp,
                        plan,
                    });
                    self.current = Some(SpellEffect::SprayWeapon(
                        magicka_sim::spray_weapon::SprayWeapon::begin(
                            from_staff,
                            self.facts.timestamp,
                        ),
                    ));
                }
                Err(e) => self.events.push(AvatarEvent::SpellNotReconstructed(format!(
                    "weapon spray damage: {e:?}"
                ))),
            }
            return;
        }
        if kind == EffectKind::Push && cast_type == CastType::Weapon {
            // No push, effect or cue: see PushSpell::cast_weapon.
            self.current = Some(SpellEffect::Push(PushSpell::cast_weapon(
                from_staff,
                self.facts.timestamp,
                self.source.owner_direction,
            )));
            return;
        }
        if kind == EffectKind::Push && matches!(cast_type, CastType::Force | CastType::Area) {
            let from_staff = from_staff && (cast_type != CastType::Force || self.facts.panic_cast);
            if let Some(push) = PushSpell::cast(
                cast_type,
                from_staff,
                self.facts.timestamp,
                self.power,
                self.additional_radius,
                self.source.owner_direction,
            ) {
                self.events.push(AvatarEvent::PushStarted {
                    cast_type,
                    position: if cast_type == CastType::Area {
                        self.source.owner_position
                    } else {
                        self.source.cast_position
                    },
                    direction: self.source.owner_direction,
                });
                self.current = Some(SpellEffect::Push(push));
            }
            return;
        }
        if kind == EffectKind::Projectile && cast_type == CastType::Area {
            match magicka_sim::damage::calculate_damage(
                &spell,
                magicka_sim::spells::SpellType::Projectile,
                CastType::Area,
            ) {
                Ok(damages) => {
                    let mut slots = [magicka_sim::damage::Damage::default(); 5];
                    for (slot, damage) in slots.iter_mut().zip(damages) {
                        *slot = damage;
                    }
                    self.events.push(AvatarEvent::GroundArea {
                        position: self.source.owner_position,
                        radius: spell.blast_size() * 10.0,
                        damages: slots,
                        timestamp: self.facts.timestamp,
                    });
                    self.current = Some(SpellEffect::GroundArea(
                        EffectLifecycle::begin(CastType::Area, from_staff, self.facts.timestamp)
                            .unwrap(),
                    ));
                }
                Err(e) => self.events.push(AvatarEvent::SpellNotReconstructed(format!(
                    "ground area damage: {e:?}"
                ))),
            }
            return;
        }
        if kind == EffectKind::Projectile
            && matches!(cast_type, CastType::Force | CastType::SelfCast)
        {
            let result = if cast_type == CastType::SelfCast {
                magicka_sim::projectile_launch::ForceProjectile::cast_self(
                    spell,
                    self.power,
                    from_staff,
                    self.facts.timestamp,
                )
            } else {
                magicka_sim::projectile_launch::ForceProjectile::cast(
                    spell,
                    self.power,
                    true,
                    from_staff,
                    self.facts.timestamp,
                )
            };
            match result {
                Ok(p) => self.current = Some(SpellEffect::Projectile(p)),
                Err(e) => self.events.push(AvatarEvent::SpellNotReconstructed(format!(
                    "projectile: {e:?}"
                ))),
            }
            return;
        }
        if kind == EffectKind::Lightning
            && matches!(
                cast_type,
                CastType::Force | CastType::Area | CastType::Weapon
            )
        {
            // CastForce keeps mFromStaff only in PanicCastState; CastWeapon
            // is CastForce with mCastType = Weapon (bolts from WeaponSource).
            let result = if cast_type == CastType::Force {
                magicka_sim::lightning::LightningSpell::cast_force(
                    spell,
                    from_staff && self.facts.panic_cast,
                    self.facts.timestamp,
                )
            } else if cast_type == CastType::Weapon {
                magicka_sim::lightning::LightningSpell::cast_weapon(
                    spell,
                    from_staff && self.facts.panic_cast,
                    self.facts.timestamp,
                )
            } else {
                magicka_sim::lightning::LightningSpell::cast_area(spell, self.facts.timestamp)
            };
            match result {
                Ok(mut l) => {
                    // LightningSpell.Cast*: GetSpellRangeModifier(ref mRange).
                    l.range = l.range * self.range_modifier.0 + self.range_modifier.1;
                    self.current = Some(SpellEffect::Lightning(l));
                }
                Err(e) => self.events.push(AvatarEvent::SpellNotReconstructed(format!(
                    "lightning damage: {e:?}"
                ))),
            }
            return;
        }
        if kind == EffectKind::RailGun && cast_type == CastType::Force {
            // RailGunSpell.CastForce assigns mFromStaff itself.
            match magicka_sim::railgun::RailGunForce::cast(
                spell,
                from_staff,
                self.facts.timestamp,
                self.ttl_modifier,
            ) {
                Ok(r) => self.current = Some(SpellEffect::RailGun(r)),
                Err(e) => self.events.push(AvatarEvent::SpellNotReconstructed(format!(
                    "beam damage: {e:?}"
                ))),
            }
            return;
        }
        if kind == EffectKind::RailGun && cast_type == CastType::Area {
            match magicka_sim::railgun::area_blast(&spell) {
                Ok((radius, damages)) => {
                    self.events.push(AvatarEvent::BeamArea {
                        position: self.source.owner_position,
                        radius,
                        damages,
                        timestamp: self.facts.timestamp,
                        cue: magicka_sim::railgun::area_cue(&spell),
                    });
                    self.current = Some(SpellEffect::RailGunArea(
                        EffectLifecycle::begin(CastType::Area, from_staff, self.facts.timestamp)
                            .unwrap(),
                    ));
                }
                Err(e) => self.events.push(AvatarEvent::SpellNotReconstructed(format!(
                    "beam area damage: {e:?}"
                ))),
            }
            return;
        }
        if kind == EffectKind::Shield && cast_type == CastType::Weapon {
            // ShieldSpell.CastWeapon: mSpell, mHasCast = false, base
            // CastWeapon; the entity is placed at AnimationEnd.
            self.current = Some(SpellEffect::ShieldWeapon {
                shield: ShieldSelf::begin(cast_type, from_staff, self.facts.timestamp),
                spell,
                has_cast: false,
            });
            return;
        }
        if kind == EffectKind::Shield
            && matches!(cast_type, CastType::Force | CastType::Area)
            && spell.element.0 & !E::SHIELD.0 == 0
        {
            let (position, direction) = (self.source.owner_position, self.source.owner_direction);
            let (position, radius, shield_kind, hit_points, color) =
                magicka_sim::shield_entity::pure_shield(
                    cast_type == CastType::Area,
                    position,
                    direction,
                    spell.get(E::SHIELD),
                    spell.get(E::ARCANE),
                    spell.get(E::LIFE),
                );
            self.events.push(AvatarEvent::SpawnShield {
                position,
                radius,
                direction,
                kind: shield_kind,
                hit_points,
                color,
            });
            self.current = Some(SpellEffect::ShieldSelf(ShieldSelf::begin(
                cast_type,
                from_staff,
                self.facts.timestamp,
            )));
            return;
        }
        if kind == EffectKind::Shield
            && matches!(cast_type, CastType::Force | CastType::Area)
            && magicka_sim::spell_mine::is_mine_spell(&spell)
        {
            self.events.push(AvatarEvent::SpawnMines {
                placements: magicka_sim::spell_mine::cast_placements(
                    cast_type == CastType::Area,
                    self.source.owner_position,
                    self.source.owner_direction,
                ),
                spell,
            });
            self.current = Some(SpellEffect::ShieldSelf(ShieldSelf::begin(
                cast_type,
                from_staff,
                self.facts.timestamp,
            )));
            return;
        }
        if kind == EffectKind::Shield
            && matches!(cast_type, CastType::Force | CastType::Area)
            && magicka_sim::barrier::is_barrier_spell(&spell)
        {
            match magicka_sim::barrier::cast_placements(
                cast_type == CastType::Area,
                self.source.owner_position,
                self.source.owner_direction,
                &spell,
            ) {
                Ok(cast) => {
                    self.events.push(AvatarEvent::SpawnBarriers {
                        cast: Box::new(cast),
                        spell,
                    });
                    self.current = Some(SpellEffect::ShieldSelf(ShieldSelf::begin(
                        cast_type,
                        from_staff,
                        self.facts.timestamp,
                    )));
                }
                Err(e) => self.events.push(AvatarEvent::SpellNotReconstructed(format!(
                    "barrier damage: {e:?}"
                ))),
            }
            return;
        }
        if kind == EffectKind::Spray && cast_type == CastType::Area {
            match magicka_sim::spray_area::plan(&spell, self.facts.greased) {
                Ok(plan) => {
                    self.events.push(AvatarEvent::SprayArea {
                        spell,
                        position: self.source.owner_position,
                        cast_source: self.source.cast_position,
                        timestamp: self.facts.timestamp,
                        plan,
                    });
                    // Live effects currently allocate fresh instances. Shared
                    // SpraySpell pool history remains a separate integration gap.
                    self.current = Some(SpellEffect::SprayArea(
                        magicka_sim::spray_area::SprayArea::begin(
                            from_staff,
                            self.facts.timestamp,
                            0.0,
                        ),
                    ));
                }
                Err(e) => self.events.push(AvatarEvent::SpellNotReconstructed(format!(
                    "spray area: {e:?}"
                ))),
            }
            return;
        }
        if kind == EffectKind::Spray && cast_type == CastType::Force {
            let source = self.source;
            let mut facts = self.facts;
            facts.from_staff = from_staff;
            match ForceSpray::cast(spell, facts, source, self) {
                Ok(spray) => self.current = Some(SpellEffect::ForceSpray(spray)),
                Err(e) => self.events.push(AvatarEvent::SpellNotReconstructed(format!(
                    "spray damage: {e:?}"
                ))),
            }
            return;
        }
        if cast_type == CastType::SelfCast {
            let result = match kind {
                EffectKind::Spray => SelfSpray::cast(spell, from_staff, self.facts.timestamp, self)
                    .map(SpellEffect::SelfSpray),
                EffectKind::RailGun => {
                    SelfBeam::cast(spell, from_staff, self.facts.timestamp, self)
                        .map(SpellEffect::SelfBeam)
                }
                EffectKind::Lightning => {
                    SelfLightning::cast(spell, from_staff, self.facts.timestamp, self)
                        .map(SpellEffect::SelfLightning)
                }
                EffectKind::Shield => {
                    self.owner_shield = Some((spell, magicka_sim::shield_self::plan(&spell)));
                    Ok(SpellEffect::ShieldSelf(ShieldSelf::cast(
                        from_staff,
                        self.facts.timestamp,
                    )))
                }
                _ => {
                    self.unsupported(kind, cast_type, from_staff);
                    return;
                }
            };
            match result {
                Ok(effect) => self.current = Some(effect),
                Err(e) => self.events.push(AvatarEvent::SpellNotReconstructed(format!(
                    "self-cast damage: {e:?}"
                ))),
            }
            return;
        }
        self.unsupported(kind, cast_type, from_staff);
    }
}
impl CastPhase<'_> {
    fn unsupported(&mut self, kind: EffectKind, cast_type: CastType, from_staff: bool) {
        self.events.push(AvatarEvent::SpellNotReconstructed(format!(
            "{kind:?} {cast_type:?}"
        )));
        if let Some(mut lifecycle) =
            EffectLifecycle::begin(cast_type, from_staff, self.facts.timestamp)
        {
            lifecycle.active = false;
            self.current = Some(SpellEffect::Unreconstructed { kind, lifecycle });
        }
    }
}
impl SprayWorld for CastPhase<'_> {
    fn scene_intersection(&mut self, _: Segment) -> Option<f32> {
        None
    }
    fn shields(&mut self) -> Vec<u16> {
        Vec::new()
    }
    fn shield_intersection(&mut self, _: u16, _: Segment) -> Option<f32> {
        None
    }
    fn targets(&mut self, _: Vector, _: f32) -> Vec<SprayTarget> {
        Vec::new()
    }
    fn arc_intersects(&mut self, _: u16, _: Vector, _: Vector, _: f32, _: f32, _: f32) -> bool {
        false
    }
    fn damage(&mut self, _: u16, _: u16, _: [Damage; 5], _: f64, _: Vector) {}
    fn electrocute(&mut self, _: u16, _: u16, _: f32) {}
}
impl ForceSprayHost for CastPhase<'_> {
    fn initialize_force(&mut self, spell: Spell, range: f32, source: SpraySource) {
        self.events.push(spray_started(spell, range, source));
    }
    fn damage_owner_single(&mut self, damage: Damage, timestamp: f64, source: SpraySource) {
        self.owner_damage.push((
            DamagePayload::Single(damage),
            timestamp,
            source.cast_position,
        ));
    }
    fn rumble(&mut self, _: f32, _: f32) {}
    fn update_force_presentation(
        &mut self,
        _: f32,
        _: f32,
        _: bool,
        _: SpraySource,
        _: [Damage; 5],
    ) {
    }
    fn deinitialize(&mut self, _: &mut EffectLifecycle) {}
}

fn spray_started(spell: Spell, range: f32, source: SpraySource) -> AvatarEvent {
    AvatarEvent::SprayStarted {
        spell,
        range,
        position: source.cast_position,
        direction: source.owner_direction,
    }
}

/// ForceSprayHost over a live SprayWorld for CastUpdate.
struct LiveSprayHost<'a, W> {
    world: &'a mut W,
    events: &'a mut Vec<AvatarEvent>,
}
impl<W: SprayWorld> SprayWorld for LiveSprayHost<'_, W> {
    fn scene_intersection(&mut self, segment: Segment) -> Option<f32> {
        self.world.scene_intersection(segment)
    }
    fn shields(&mut self) -> Vec<u16> {
        self.world.shields()
    }
    fn shield_intersection(&mut self, handle: u16, segment: Segment) -> Option<f32> {
        self.world.shield_intersection(handle, segment)
    }
    fn targets(&mut self, center: Vector, range: f32) -> Vec<SprayTarget> {
        self.world.targets(center, range)
    }
    fn arc_intersects(
        &mut self,
        target: u16,
        origin: Vector,
        direction: Vector,
        range: f32,
        angle: f32,
        height: f32,
    ) -> bool {
        self.world
            .arc_intersects(target, origin, direction, range, angle, height)
    }
    fn damage(
        &mut self,
        target: u16,
        attacker: u16,
        damages: [Damage; 5],
        timestamp: f64,
        position: Vector,
    ) {
        self.events.push(AvatarEvent::SprayHit { target });
        self.world
            .damage(target, attacker, damages, timestamp, position);
    }
    fn electrocute(&mut self, target: u16, attacker: u16, multiplier: f32) {
        self.world.electrocute(target, attacker, multiplier);
    }
}
impl<W: SprayWorld> ForceSprayHost for LiveSprayHost<'_, W> {
    fn initialize_force(&mut self, spell: Spell, range: f32, source: SpraySource) {
        self.events.push(spray_started(spell, range, source));
    }
    fn damage_owner_single(&mut self, _: Damage, _: f64, _: SpraySource) {}
    fn rumble(&mut self, _: f32, _: f32) {}
    /// SpraySpell.CastUpdate, Force branch presentation; clipped scale is
    /// shared by visual orientation and retained ground mark placement.
    fn update_force_presentation(
        &mut self,
        dt: f32,
        range: f32,
        from_staff: bool,
        source: SpraySource,
        damages: [Damage; 5],
    ) {
        // Direction from the top of CastUpdate (Force: staff forward when
        // casting from the staff, otherwise the owner's direction); the
        // Force block then takes the position from WeaponSource when
        // from_staff and CastSource otherwise, as the original does.
        let (direction, position) = if from_staff {
            (source.cast_direction, source.weapon_position)
        } else {
            (source.owner_direction, source.cast_position)
        };
        let mut m = crate::particles::create_world(position, direction, [0.0, 1.0, 0.0]);
        // mAngle (π/10) <= 0.5: scale the visual by the obstructed range.
        let mut scale = range * 0.1;
        let mut segment = Segment {
            origin: position,
            delta: direction.map(|d| d * range),
        };
        if let Some(fraction) = self.world.scene_intersection(segment) {
            scale *= fraction;
            segment.delta = segment.delta.map(|d| d * fraction);
        }
        for shield in self.world.shields() {
            if let Some(fraction) = self.world.shield_intersection(shield, segment) {
                scale *= fraction;
                segment.delta = segment.delta.map(|d| d * fraction);
            }
        }
        // MagickaMath.UniformMatrixScale: the 3×3 part only.
        for row in 0..3 {
            for column in 0..3 {
                m[row * 4 + column] *= scale;
            }
        }
        self.events.push(AvatarEvent::SprayOrientation(m));
        self.events
            .push(AvatarEvent::SprayDecalUpdate(crate::spray_decals::Frame {
                dt,
                scale,
                direction,
                position,
                cast_position: source.cast_position,
            }));
        // Liquid.Freeze over the full (unobstructed) range, magnitude dt.
        self.events.push(AvatarEvent::SprayFreeze {
            origin: position,
            delta: direction.map(|d| d * range),
            spread: std::f32::consts::PI / 10.0,
            multiplier: dt,
            damages,
        });
    }
    fn deinitialize(&mut self, lifecycle: &mut EffectLifecycle) {
        if lifecycle.active {
            self.events.push(AvatarEvent::SprayStopped);
        }
        lifecycle.active = false;
    }
}

/// ShieldSpell.CastSelf owner operations: Character.AddSelfShield (which
/// first expires self-casted auras; a pure Shield toggles an existing pure
/// Shield off) and/or RemoveSelfShield, ClearAura and the immunity auras.
fn apply_shield_self(character: &mut RuntimeCharacter, spell: &Spell, plan: &ShieldSelfPlan) {
    let mass = character.definition.mass;
    if plan.add_self_shield {
        character.add_self_shield(spell);
    }
    if let Some(auras) = &plan.auras {
        if auras.remove_self_shield && character.combat.health.shield.active() {
            character
                .combat
                .health
                .shield
                .remove(mass, false, &mut character.shield_events);
        }
        character.auras.clear();
        for &element in &auras.immunities {
            character.auras.push(crate::character::ActiveAura {
                resistance: Resistance {
                    against: element,
                    multiplier: 0.0,
                    modifier: 0.0,
                    status_resistance: false,
                },
                ttl: auras.ttl,
                start_ttl: auras.ttl,
                self_casted: true,
                self_only: false,
                visual: crate::buffs::AuraVisual::new(
                    auras.radius,
                    crate::buffs::immunity_color(element),
                ),
            });
        }
    }
}

/// Character.Damage through the public delivery route, including live blocking
/// equipment and resistance buffs. Returns the DamageResult flags.
#[allow(clippy::too_many_arguments)]
pub fn deliver_to_character(
    character: &mut RuntimeCharacter,
    equipment: &[Resistance; 11],
    avatar: bool,
    target: u16,
    position: Vector,
    facing: Vector,
    payload: DamagePayload,
    attacker: Option<u16>,
    attack_position: Vector,
    play_time: f64,
) -> u16 {
    let motion = MotionContext {
        position,
        mass: character.definition.mass,
        gravity_y: GRAVITY[1],
        ..Default::default()
    };
    deliver_with_motion(
        character,
        equipment,
        avatar,
        target,
        position,
        facing,
        payload,
        attacker,
        attack_position,
        play_time,
        motion,
    )
}

/// Public damage with synchronous Character.AddImpulseVelocity forwarding to
/// its live physics body. Pending numerical impulses are consumed exactly once.
#[allow(clippy::too_many_arguments)]
pub fn deliver_to_body(
    character: &mut RuntimeCharacter,
    body: &mut CharacterBody,
    equipment: &[Resistance; 11],
    avatar: bool,
    target: u16,
    payload: DamagePayload,
    attacker: Option<u16>,
    attack_position: Vector,
    play_time: f64,
) -> u16 {
    character.combat.motion.velocity = body.velocity;
    let motion = MotionContext {
        position: body.position,
        mass: body.mass,
        gravity_y: GRAVITY[1],
        ethereal: character.ethereal,
        entangled: body.owner.entangled,
        dead: character.combat.dead,
        ..Default::default()
    };
    let result = deliver_with_motion(
        character,
        equipment,
        avatar,
        target,
        body.position,
        body.direction(),
        payload,
        attacker,
        attack_position,
        play_time,
        motion,
    );
    body.velocity = character.combat.motion.velocity;
    if character.combat.motion.is_pushed {
        body.add_impulse_velocity(std::mem::take(&mut character.combat.motion.impulse));
        character.combat.motion.is_pushed = false;
        character.combat.motion.leaving_ground = false;
    }
    body.owner.hit_points = character.combat.health.hit_points;
    result
}

#[allow(clippy::too_many_arguments)]
fn deliver_with_motion(
    character: &mut RuntimeCharacter,
    equipment: &[Resistance; 11],
    avatar: bool,
    target: u16,
    position: Vector,
    facing: Vector,
    payload: DamagePayload,
    attacker: Option<u16>,
    attack_position: Vector,
    play_time: f64,
    motion: MotionContext,
) -> u16 {
    // Resistance buffs from auras on this character (mBuffs, list order).
    let buffs: Vec<Resistance> = character.buffs.resistances();
    let block_values: Vec<i32> = character.block_values.values().copied().collect();
    let definition = &character.definition;
    let mut impact = ImpactContext {
        status: StatusContext {
            immortal: character.is_immortal(),
            ethereal: character.ethereal,
            can_have_status: definition.can_have_status,
            avatar,
            current_spell: character.status_current_spell,
            base: &definition.resistances,
            equipment: avatar.then_some(equipment),
            buffs: &buffs,
            cues: character.status_cues,
        },
        motion,
        attack_position,
        forward: facing,
        block_values: &block_values,
        block_item: character.block_item,
        features: 15,
        network_client: false,
        attacker: attacker.map(u64::from),
        attacker_is_self: attacker == Some(target),
        attacker_avatar: false,
        attacker_local_avatar: false,
        attacker_is_gripped: false,
        grip_attack_state: false,
        hit_tolerance: definition.hit_tolerance,
        resurrection_state: false,
        stun_time: definition.stun_time,
        bleed_rate: definition.bleed_rate,
        blood_and_gore: crate::global_settings::blood_and_gore(),
        has_blood: definition.blood != crate::gibs::BLOOD_NONE,
        bloat_killed: false,
        has_damage_trigger: false,
    };
    let delivery = DeliveryContext {
        network: NetworkRole::Offline,
        attacker_kind: if attacker.is_some() {
            AttackerKind::Character
        } else {
            AttackerKind::Other
        },
        target_handle: u64::from(target),
        attacker_handle: attacker.map(u64::from),
        time_stamp: play_time,
        target_position: position,
        attack_position,
    };
    let mut events = Vec::new();
    let before = character.combat.health.hit_points;
    let result = character
        .combat
        .deliver(payload, delivery, &[], &mut impact, &mut events)
        .unwrap_or(0);
    character
        .block_effects
        .extend(events.iter().filter_map(|event| {
            if matches!(
                event,
                magicka_sim::damage_delivery::DeliveryEvent::Impact(
                    magicka_sim::character_impact::ImpactEvent::BlockEffect
                )
            ) {
                Some((
                    attack_position,
                    std::array::from_fn(|i| attack_position[i] - position[i]),
                ))
            } else {
                None
            }
        }));
    // Character.Damage: damage from the gripped character accumulates on the
    // gripper and returns a GripAttackState to GrippingState (INFERRED: the
    // HP loss stands in for the per-slot amount).
    if attacker.is_some() && attacker == character.grip.gripped {
        let dealt = before - character.combat.health.hit_points;
        if dealt > 0.0 {
            character.grip.damage_accumulation += dealt;
            character.grip.grip_attack_interrupted = true;
        }
    }
    for event in &events {
        match event {
            magicka_sim::damage_delivery::DeliveryEvent::Impact(
                magicka_sim::character_impact::ImpactEvent::Entangle(magnitude),
            ) => {
                let burning = character
                    .combat
                    .statuses
                    .has(magicka_sim::status::Status::Burning);
                let immortal = character.is_immortal();
                character
                    .entanglement
                    .entangle(*magnitude, immortal, burning);
            }
            magicka_sim::damage_delivery::DeliveryEvent::Impact(
                magicka_sim::character_impact::ImpactEvent::DecreaseEntanglement {
                    magnitude,
                    element,
                },
            ) => character.entanglement.decrease(
                *magnitude,
                *element,
                &mut character.entangle_events,
            ),
            magicka_sim::damage_delivery::DeliveryEvent::Impact(
                magicka_sim::character_impact::ImpactEvent::StopHypnotize,
            ) => character.stop_hypnotize(),
            magicka_sim::damage_delivery::DeliveryEvent::Impact(
                magicka_sim::character_impact::ImpactEvent::Statistics { damage, result },
            ) => character
                .statistics_events
                .push(crate::character::StatisticsDamage {
                    attacker,
                    amount: damage.amount,
                    magnitude: damage.magnitude,
                    result: *result,
                }),
            _ => {}
        }
    }
    character.retain_status_events(events.iter().filter_map(|e| {
        if let magicka_sim::damage_delivery::DeliveryEvent::Impact(
            magicka_sim::character_impact::ImpactEvent::Status(event),
        ) = e
        {
            Some(*event)
        } else {
            None
        }
    }));
    character
        .damage_numbers
        .extend(events.iter().filter_map(|e| match e {
            magicka_sim::damage_delivery::DeliveryEvent::Impact(
                magicka_sim::character_impact::ImpactEvent::Notify { amount, .. },
            ) => Some(crate::floating_text::NumberRequest::Direct { amount: *amount }),
            _ => None,
        }));
    for event in &events {
        if let magicka_sim::damage_delivery::DeliveryEvent::Impact(
            magicka_sim::character_impact::ImpactEvent::Gore {
                amount, magnitude, ..
            },
        ) = event
        {
            character.blood_decals.push(crate::decals::BloodDamage {
                position,
                attack_position,
                radius: character.definition.radius,
                blood: character.definition.blood,
                amount: *amount,
                magnitude: *magnitude,
            });
            // Same branch: PlayCue(Misc, SOUNDS_GORE[blood], Magnitude = Amount).
            character
                .sounds
                .gore_hit(character.definition.blood, *amount);
        }
    }
    if events.iter().any(|e| {
        matches!(
            e,
            magicka_sim::damage_delivery::DeliveryEvent::Impact(
                magicka_sim::character_impact::ImpactEvent::OverkillLifecycle
            )
        )
    }) {
        character.collision_hits.clear();
    }
    result
}

fn stop_spell_effect(
    effect: &mut SpellEffect,
    events: &mut Vec<AvatarEvent>,
    rail_kills: &mut Vec<magicka_sim::railgun::RailId>,
) {
    match effect {
        SpellEffect::Push(push) => push.lifecycle.stop(|_| {}),
        SpellEffect::ForceSpray(spray) => {
            if spray.lifecycle.active {
                events.push(AvatarEvent::SprayStopped);
                spray.pulse.hits.clear();
            }
            spray.lifecycle.stop(|_| {});
        }
        SpellEffect::SprayArea(s) => s.lifecycle.stop(|l| l.active = false),
        SpellEffect::SprayWeapon(s) => s.lifecycle.stop(|l| {
            if l.active {
                events.push(AvatarEvent::SelfEffectsStopped);
            }
            l.active = false;
        }),
        SpellEffect::SelfSpray(s) => s.lifecycle.stop(|l| {
            if l.active {
                events.push(AvatarEvent::SelfEffectsStopped);
            }
        }),
        SpellEffect::SelfBeam(s) => s.lifecycle.stop(|l| {
            if l.active {
                events.push(AvatarEvent::SelfEffectsStopped);
            }
        }),
        SpellEffect::SelfLightning(s) => s.lifecycle.stop(|l| {
            if l.active {
                events.push(AvatarEvent::SelfEffectsStopped);
            }
        }),
        // ShieldSpell.DeInitialize (mHasCast): Active = false.
        SpellEffect::ShieldSelf(s) => s.lifecycle.active = false,
        // ShieldSpell.DeInitialize: AnimationEnd first unless mHasCast.
        SpellEffect::ShieldWeapon {
            shield,
            spell,
            has_cast,
        } => {
            if shield.lifecycle.active && !*has_cast {
                *has_cast = true;
                events.push(AvatarEvent::ShieldWeapon { spell: *spell });
            }
            shield.lifecycle.active = false;
        }
        // SpellEffect.Stop: DeInitialize, then Active = false.
        SpellEffect::Projectile(p) => {
            p.deinitialize();
            p.lifecycle.active = false;
        }
        SpellEffect::Lightning(l) => l.deinitialize(),
        // DeInitialize: Railgun.Kill (queued for the scene's pool).
        SpellEffect::RailGun(r) => {
            if r.lifecycle.active {
                if let Some(id) = r.rail.take() {
                    rail_kills.push(id);
                }
            }
            r.lifecycle.active = false;
        }
        SpellEffect::RailGunArea(l) | SpellEffect::GroundArea(l) => l.active = false,
        SpellEffect::Unreconstructed { lifecycle, .. } => lifecycle.stop(|_| {}),
        SpellEffect::Weapon {
            lifecycle,
            ice_blade,
            ..
        } => {
            if let Some(blade) = ice_blade.take() {
                events.push(blade.strike());
            }
            lifecycle.stop(|_| {})
        }
    }
}

fn apply_status_spell_stop(
    character: &mut RuntimeCharacter,
    current: &mut Option<SpellEffect>,
    events: &mut Vec<AvatarEvent>,
    rail_kills: &mut Vec<magicka_sim::railgun::RailId>,
) {
    let mut stop = false;
    character.pending_status_audio.retain(|event| {
        if matches!(
            event,
            magicka_sim::character_status::StatusEvent::StopCurrentSpell
        ) {
            stop = true;
            false
        } else {
            true
        }
    });
    if stop {
        if let Some(effect) = current.as_mut() {
            stop_spell_effect(effect, events, rail_kills);
        }
    }
    character.status_current_spell = current.is_some();
}

/// World access for one avatar update.
pub type AvatarFrame<'a> = ActorFrame<'a>;

/// SpellType of an element queue once combined (HUD hint).
pub fn queue_spell_type(queue: &[Spell]) -> SpellType {
    magicka_sim::spells::combine(queue).spell_type()
}

impl SelfSprayHost for CastPhase<'_> {
    fn electrocute_self(&mut self, _: f32) {} // Character.Electrocute is empty.
    fn start_effect(&mut self, index: usize, name: &'static str) {
        self.events.push(AvatarEvent::SelfEffectStarted {
            index,
            name,
            position: self.source.owner_position,
            direction: self.source.owner_direction,
        });
    }
    fn damage_self(&mut self, damages: [Damage; 5], timestamp: f64) {
        self.owner_damage.push((
            DamagePayload::Collection(damages),
            timestamp,
            self.source.owner_position,
        ));
    }
}
impl SelfLightningHost for CastPhase<'_> {
    fn start_effect(&mut self, index: usize, name: &'static str) {
        SelfSprayHost::start_effect(self, index, name);
    }
    fn damage_self(&mut self, damages: [Damage; 5], timestamp: f64) {
        SelfSprayHost::damage_self(self, damages, timestamp);
    }
}
impl SelfBeamHost for CastPhase<'_> {
    fn start_effect(&mut self, index: usize, name: &'static str) {
        SelfSprayHost::start_effect(self, index, name);
    }
    fn damage_self(&mut self, _: [Damage; 5], _: f64) {
        unreachable!("beam damage is deferred until Update")
    }
    fn update_effects(&mut self) {
        unreachable!("Cast phase does not update beam")
    }
    fn deinitialize(&mut self, _: &mut EffectLifecycle) {
        unreachable!("Cast phase does not stop beam")
    }
}
struct LiveSelfBeamHost<'a> {
    character: &'a mut RuntimeCharacter,
    body: &'a mut CharacterBody,
    equipment: &'a [Resistance; 11],
    avatar: bool,
    handle: u16,
    events: &'a mut Vec<AvatarEvent>,
}
impl SelfBeamHost for LiveSelfBeamHost<'_> {
    fn start_effect(&mut self, _: usize, _: &'static str) {
        unreachable!("Update phase does not start beam")
    }
    fn damage_self(&mut self, damages: [Damage; 5], timestamp: f64) {
        let position = self.body.position;
        let hit_points_before = self.character.combat.health.hit_points;
        let payload =
            DamagePayload::Collection(damages).with_boosts(&self.character.buffs.damage_boosts());
        let result = deliver_to_body(
            self.character,
            self.body,
            self.equipment,
            self.avatar,
            self.handle,
            payload,
            Some(self.handle),
            position,
            timestamp,
        );
        self.events.push(AvatarEvent::SelfCastApplied {
            result,
            hit_points_before,
            hit_points_after: self.character.combat.health.hit_points,
        });
    }
    fn update_effects(&mut self) {
        self.events.push(AvatarEvent::SelfEffectsPosition {
            position: self.body.position,
            direction: self.body.direction(),
        });
    }
    fn deinitialize(&mut self, lifecycle: &mut EffectLifecycle) {
        if lifecycle.active {
            self.events.push(AvatarEvent::SelfEffectsStopped);
        }
        lifecycle.active = false;
    }
}

/// ProjectileLaunchHost over the live scene, spawning missiles as events.
struct LaunchHost<'a, W> {
    world: &'a mut W,
    events: &'a mut Vec<AvatarEvent>,
    rng: &'a mut u64,
}
impl<W: SprayWorld> magicka_sim::projectile_launch::ProjectileLaunchHost for LaunchHost<'_, W> {
    fn shields(&mut self) -> Vec<u16> {
        self.world.shields()
    }
    fn shield_intersection(&mut self, shield: u16, segment: Segment, _: f32) -> Option<Vector> {
        // Shield.SegmentIntersect accepts a radius but never reads it; it
        // delegates to the same primitive intersection used by sprays.
        self.world
            .shield_intersection(shield, segment)
            .map(|t| std::array::from_fn(|i| segment.origin[i] + segment.delta[i] * t))
    }
    fn blocked_effect(&mut self, element: E, position: Vector, direction: Vector) {
        self.events.push(AvatarEvent::ProjectileBlocked {
            element,
            position,
            direction,
        });
    }
    /// System.Random.NextDouble stand-in (time-seeded in the original).
    fn next_double(&mut self) -> f64 {
        *self.rng ^= *self.rng << 13;
        *self.rng ^= *self.rng >> 7;
        *self.rng ^= *self.rng << 17;
        (*self.rng >> 11) as f64 / (1u64 << 53) as f64
    }
    fn spawn(&mut self, launch: magicka_sim::projectile_launch::ProjectileLaunch) {
        self.events.push(AvatarEvent::MissileLaunched(launch));
    }
}

/// LightningWorld for an avatar: the live scene plus bolt events and the
/// random stand-in.
struct LightningHost<'a, W> {
    world: &'a mut W,
    events: &'a mut Vec<AvatarEvent>,
    rng: &'a mut u64,
}
impl<W: magicka_sim::lightning::LightningScene> magicka_sim::lightning::LightningScene
    for LightningHost<'_, W>
{
    fn lightning_targets(
        &mut self,
        center: Vector,
        range: f32,
    ) -> Vec<magicka_sim::lightning::LightningTarget> {
        self.world.lightning_targets(center, range)
    }
    fn lightning_shielded(&mut self, segment: Segment) -> bool {
        self.world.lightning_shielded(segment)
    }
    fn lightning_scene(&mut self, segment: Segment) -> Option<f32> {
        self.world.lightning_scene(segment)
    }
    fn lightning_damage(
        &mut self,
        target: u16,
        attacker: u16,
        damages: [Damage; 5],
        timestamp: f64,
        position: Vector,
    ) {
        self.world
            .lightning_damage(target, attacker, damages, timestamp, position);
    }
}
impl<W: magicka_sim::lightning::LightningScene> magicka_sim::lightning::LightningWorld
    for LightningHost<'_, W>
{
    fn lightning_bolt(&mut self, bolt: magicka_sim::lightning::BoltVisual) {
        self.events.push(AvatarEvent::LightningBolt(bolt));
    }
    /// MagickaMath.Random.NextDouble stand-in.
    fn next_double(&mut self) -> f64 {
        *self.rng ^= *self.rng << 13;
        *self.rng ^= *self.rng >> 7;
        *self.rng ^= *self.rng << 17;
        (*self.rng >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[cfg(test)]
mod ice_blade_tests {
    use super::*;

    fn item_at(x: f32) -> magicka_content::skinned::Mat {
        let mut m = magicka_content::skinned::IDENTITY;
        m[12] = x;
        m
    }

    #[test]
    fn trail_starts_unscaled_then_tracks_the_scaled_blade_and_lags_behind() {
        let mut blade = IceBlade {
            range: 5.0,
            damages: [Damage::default(); 5],
            timestamp: 0.0,
            trail: None,
            orientation: None,
        };
        blade.update_trail(&item_at(0.0), 1.0 / 60.0);
        let trail = *blade.trail.clone().unwrap();
        // Pair 0: hilt and the tip at range (attach0.Y × range/attach0.Y).
        assert_eq!(trail[0], [0.0; 3]);
        assert!((trail[1][1] - 5.0).abs() < 1e-5);
        // Older pairs were initialised at the unscaled attach point and lerp
        // toward the newer pair by ε^dt each update.
        assert!(trail[3][1] > ICE_BLADE_ATTACH_Y && trail[3][1] < 5.0);
        // Moving the sword leaves older pairs behind, ordered by age.
        for _ in 0..3 {
            blade.update_trail(&item_at(1.0), 1.0 / 60.0);
        }
        let trail = *blade.trail.clone().unwrap();
        assert_eq!(trail[0][0], 1.0);
        let xs: Vec<f32> = (0..ICE_BLADE_TRAIL)
            .step_by(2)
            .map(|i| trail[i][0])
            .collect();
        assert!(xs.windows(2).all(|w| w[0] >= w[1]), "{xs:?}");
        assert!(xs[15] < 1.0 && xs[15] >= 0.0, "{xs:?}");
        // ε^(1/60) ≈ 0.179: pair 1 after one step from 0 → 1 sits at 0.821.
        let amount = 1.401_298_464_324_817e-45f64.powf(1.0 / 60.0) as f32;
        assert!((amount - 0.1789).abs() < 1e-3, "{amount}");
    }
}
