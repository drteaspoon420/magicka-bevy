//! Character auras and buffs (research/aura-buffs.md): ActiveAura.Execute
//! hands its BuffStorage to every non-ethereal character in range every
//! 0.25 s (TTL 0.5, refreshed by UniqueID), Character.Update draws each
//! buff as a rotating ground decal under its character and each aura as a
//! ring of six decals. Only the Defensive category (self-cast Shield
//! immunities) has visuals in the rewrite. Size magicks also use invisible
//! damage/health buffs with the same ID refresh and expiry rules.
use crate::decals::{Decals, Reference, Request};
use magicka_sim::resistance::Resistance;
use std::f32::consts::{FRAC_PI_3, PI, TAU};
use std::sync::atomic::{AtomicU32, Ordering};

/// Decal.BuffDefensive = 26 + VisualCategory.Defensive − 1.
pub const BUFF_DEFENSIVE: u8 = 27;
/// Decal.AuraDefensive = 29 + VisualCategory.Defensive − 1.
pub const AURA_DEFENSIVE: u8 = 30;
/// BuffStorage(BuffResistance, category, color): TTL 0.5.
pub const BUFF_TTL: f32 = 0.5;
/// AuraStorage.Execute: AuraBuff.Execute every 0.25 s.
pub const AURA_PERIOD: f32 = 0.25;

/// BuffStorage.IDCounter.
static NEXT_BUFF_ID: AtomicU32 = AtomicU32::new(0);
pub fn next_buff_id() -> u32 {
    NEXT_BUFF_ID.fetch_add(1, Ordering::Relaxed)
}

/// Spell.*COLOR for a self-cast immunity aura (ShieldSpell.CastSelf).
pub fn immunity_color(element: magicka_sim::spells::Elements) -> [f32; 3] {
    use magicka_sim::spells::Elements as E;
    match element {
        E::LIGHTNING => [0.75, 0.5, 1.0],
        E::FIRE => [1.8, 0.6, 0.4],
        E::COLD => [1.0, 1.0, 1.4],
        E::STEAM => [1.0, 1.0, 1.0],
        E::WATER => [0.0, 0.7, 1.3],
        E::LIFE => [0.2, 1.6, 0.2],
        E::ARCANE => [2.0, 0.4, 0.6],
        _ => [1.0, 1.0, 1.0],
    }
}

/// MathHelper.WrapAngle.
fn wrap(angle: f32) -> f32 {
    let mut a = (angle + PI) % TAU;
    if a < 0.0 {
        a += TAU;
    }
    a - PI
}

/// MathHelper.SmoothStep(1, 0, x).
fn smooth_fade(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// Rotate a row vector by a yaw quaternion (Vector3.Transform).
fn yaw(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
}

/// One aura's ActiveAura state (AuraStorage with VisualCategory.Defensive).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuraVisual {
    pub radius: f32,
    pub color: [f32; 3],
    /// ActiveAura's visual category selects the ring independently of its buff.
    pub decal_cell: u8,
    /// The aura buff's UniqueID, shared by every copy handed out.
    pub buff_id: u32,
    pub period: f32,
    pub rotation: f32,
    pub alpha: f32,
    pub ring: [Option<Reference>; 6],
}

impl AuraVisual {
    pub fn new(radius: f32, color: [f32; 3]) -> Self {
        Self {
            radius,
            color,
            decal_cell: AURA_DEFENSIVE,
            buff_id: next_buff_id(),
            period: 0.0,
            rotation: 0.0,
            alpha: 0.0,
            ring: [None; 6],
        }
    }

    /// AuraStorage.Execute's period: how many AuraBuff.Execute calls fire.
    pub fn tick(&mut self, dt: f32) -> usize {
        self.period -= dt;
        let mut fires = 0;
        while self.period <= 0.0 {
            self.period += AURA_PERIOD;
            fires += 1;
        }
        fires
    }

    /// ActiveAura.Execute's decal ring: six category-selected decals on the
    /// aura radius around the owner, dropped onto the scene (segment from
    /// 4 m above, 8 m down; the midpoint when nothing is hit).
    pub fn update_ring(
        &mut self,
        dt: f32,
        owner: [f32; 3],
        in_cutscene: bool,
        mut ground: impl FnMut([f32; 3], [f32; 3]) -> Option<[f32; 3]>,
        decals: &mut Decals,
    ) {
        self.alpha = (self.alpha + dt).min(1.0);
        self.rotation = wrap(self.rotation + dt * 0.2);
        let alpha = if in_cutscene { 0.0 } else { self.alpha };
        let mut value = yaw([0.0, 0.0, 1.0], self.rotation);
        // iTransform rows: Right = (−1.5, 0, 0), Up = (0, 0, 1.5), Normal =
        // (0, 1, 0), then rotated by the ring yaw and π/3 per decal.
        let mut rows = [[-1.5, 0.0, 0.0], [0.0, 0.0, 1.5], [0.0, 1.0, 0.0]];
        let turn = |rows: &mut [[f32; 3]; 3], angle: f32| {
            for r in rows.iter_mut() {
                *r = yaw(*r, angle);
            }
        };
        turn(&mut rows, self.rotation);
        for slot in &mut self.ring {
            value = yaw(value, FRAC_PI_3);
            turn(&mut rows, FRAC_PI_3);
            let origin = [
                owner[0] + value[0] * self.radius,
                owner[1] + value[1] * self.radius + 4.0,
                owner[2] + value[2] * self.radius,
            ];
            let delta = [0.0, -8.0, 0.0];
            let at = ground(origin, delta).unwrap_or([origin[0], origin[1] - 4.0, origin[2]]);
            let transform = [
                rows[0][0], rows[0][1], rows[0][2], 0.0, rows[1][0], rows[1][1], rows[1][2], 0.0,
                rows[2][0], rows[2][1], rows[2][2], 0.0, at[0], at[1], at[2], 1.0,
            ];
            let set = slot.is_some_and(|r| decals.set(r, 1.0, &transform, alpha * 0.666));
            if !set {
                *slot = decals.add(Request {
                    cell: self.decal_cell,
                    scale: [1.5, 1.5],
                    position: at,
                    direction: Some(value),
                    normal: [0.0, 1.0, 0.0],
                    ttl: 1.0,
                    color: [self.color[0], self.color[1], self.color[2], 0.0],
                    animated_owner: None,
                });
            }
        }
    }
}

/// A BuffStorage (BuffResistance, Defensive) on a character.
#[derive(Clone, Debug, PartialEq)]
pub struct ActiveBuff {
    pub id: u32,
    pub resistance: Resistance,
    pub ttl: f32,
    pub color: [f32; 3],
    pub self_casted: bool,
    pub decal: Option<Reference>,
}

/// VisualCategory.None payloads used by Grow/Shrink. Health modifiers execute
/// in insertion order; damage modifiers are read by Helper.Damage delivery.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NonvisualKind {
    BoostDamage(magicka_sim::damage::Damage),
    ModifyHitPoints { multiplier: f32, modifier: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NonvisualBuff {
    pub id: u32,
    pub ttl: f32,
    pub kind: NonvisualKind,
}

/// Self-targeted aura, retaining native 0.25 s refresh and 0.5 s buff TTL.
#[derive(Clone, Debug)]
pub struct NonvisualAura {
    pub id: u32,
    pub ttl: f32,
    pub period: f32,
    pub kind: NonvisualKind,
}

/// Character.mBuffs with mBuffRotation and mBuffCycleTimerDefensive.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Buffs {
    pub list: Vec<ActiveBuff>,
    /// Every other BuffStorage kind (item and special-ability auras).
    pub general: crate::generic_aura::GenericBuffs,
    pub nonvisual: Vec<NonvisualBuff>,
    pub rotation: f32,
    pub cycle: f32,
    /// Rewrite extension: mod modifiers' `DamagePercent` as one more
    /// BoostDamage entry (every element, multiplier only).
    pub mod_boost: Option<magicka_sim::damage::Damage>,
}

impl Buffs {
    /// AddBuff refreshes matching IDs before the dead-state gate. A refresh
    /// changes only TTL, retaining the original payload. Return true only for
    /// a new insertion (NPC current-health scaling must happen only then).
    pub fn add_nonvisual(&mut self, id: u32, kind: NonvisualKind, ttl: f32, dead: bool) -> bool {
        if let Some(b) = self.nonvisual.iter_mut().find(|b| b.id == id) {
            b.ttl = ttl;
            return false;
        }
        if dead {
            return false;
        }
        self.nonvisual.push(NonvisualBuff { id, ttl, kind });
        true
    }

    /// Character.Update resets maximum health from the template, executes
    /// buffs with positive TTL (aging them first), and removes buffs that were
    /// already expired. Crossing zero during Execute still applies this frame.
    pub fn update_nonvisual(
        &mut self,
        dt: f32,
        template_max: f32,
        health: &mut magicka_sim::character_health::CharacterHealth,
    ) {
        health.max_hit_points = template_max;
        self.nonvisual.retain_mut(|b| {
            if b.ttl <= 0.0 || b.ttl.is_nan() {
                return false;
            }
            b.ttl -= dt;
            if let NonvisualKind::ModifyHitPoints {
                multiplier,
                modifier,
            } = b.kind
            {
                health.max_hit_points *= multiplier;
                health.max_hit_points += modifier;
            }
            true
        });
        self.general
            .modify_max_hit_points(&mut health.max_hit_points);
        health.max_hit_points = (health.max_hit_points + health.max_health_bonus).max(1.0);
        if health.hit_points > health.max_hit_points {
            health.hit_points = health.max_hit_points;
        }
    }

    /// Keep expired-but-not-yet-removed entries: native GetDamageModifier
    /// reads the list without a TTL gate. prepare/modify handles element masks.
    pub fn damage_boosts(&self) -> Vec<magicka_sim::damage::Damage> {
        self.nonvisual
            .iter()
            .filter_map(|b| match b.kind {
                NonvisualKind::BoostDamage(d) => Some(d),
                NonvisualKind::ModifyHitPoints { .. } => None,
            })
            .chain(self.general.damage_boosts())
            .chain(self.mod_boost)
            .collect()
    }

    pub fn resistances(&self) -> Vec<Resistance> {
        self.list
            .iter()
            .map(|b| b.resistance)
            .chain(self.general.resistances())
            .collect()
    }

    /// Character.AddBuff: refresh the TTL of the same UniqueID, otherwise
    /// (not dead) append with its BuffDefensive decal under the character.
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        id: u32,
        resistance: Resistance,
        color: [f32; 3],
        self_casted: bool,
        dead: bool,
        position: [f32; 3],
        radius: f32,
        decals: &mut Decals,
    ) {
        if let Some(b) = self.list.iter_mut().find(|b| b.id == id) {
            b.ttl = BUFF_TTL;
            return;
        }
        if dead {
            return;
        }
        let decal = decals.add(buff_request(color, position, radius));
        self.list.push(ActiveBuff {
            id,
            resistance,
            ttl: BUFF_TTL,
            color,
            self_casted,
            decal,
        });
    }

    /// Character.Update's buff loop: decals follow the character (rotating
    /// −0.2 rad/s, cycling between several Defensive buffs at alpha 0.8),
    /// live buffs age, expired ones are removed.
    pub fn update(
        &mut self,
        dt: f32,
        position: [f32; 3],
        radius: f32,
        height_offset: f32,
        in_cutscene: bool,
        decals: &mut Decals,
    ) {
        self.rotation = wrap(self.rotation - dt * 0.2);
        // CreateFromYawPitchRoll(0, −π/2, rotation): rows (c, 0, −s),
        // (−s, 0, −c), (0, 1, 0); rows 1–2 scaled by 3·radius, row 3 by radius.
        let (s, c) = self.rotation.sin_cos();
        let (h, v) = (radius * 3.0, radius);
        let transform = [
            c * h,
            0.0,
            -s * h,
            0.0,
            -s * h,
            0.0,
            -c * h,
            0.0,
            0.0,
            v,
            0.0,
            0.0,
            position[0],
            position[1] + height_offset,
            position[2],
            1.0,
        ];
        let count = self.list.len() as f32;
        let mut k = 0;
        while k < self.list.len() {
            let index = k as f32;
            let distance = (index - self.cycle)
                .abs()
                .min((index - (self.cycle - count)).abs());
            let alpha = if in_cutscene {
                0.0
            } else {
                smooth_fade(distance) * 0.8
            };
            let b = &mut self.list[k];
            match b.decal {
                Some(r) if decals.set(r, 1.0, &transform, alpha) => {}
                _ => b.decal = decals.add(buff_request(b.color, position, radius)),
            }
            if b.ttl > 0.0 {
                b.ttl -= dt;
                k += 1;
                continue;
            }
            self.list.remove(k);
            if self.cycle > index {
                self.cycle -= 1.0;
            }
        }
        let n = self.list.len() as f32;
        self.cycle = if n > 1.0 {
            (self.cycle + dt) % n
        } else {
            (self.cycle - dt).max(0.0)
        };
    }
}

/// Character.mHaloBuffColor / mHaloBuffFade / mAuraCycleTimer: the
/// HaloAuraRenderData billboard tint (sum of the aura colours, cycled with
/// SmoothStep like the buff decals).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Halo {
    pub color: [f32; 3],
    pub fade: f32,
    pub cycle: f32,
}

impl Halo {
    /// The aura loop's colour section over the live auras' colours.
    pub fn update(&mut self, dt: f32, colors: &[[f32; 3]]) {
        self.color = [0.0; 3];
        let n = colors.len() as f32;
        for (i, c) in colors.iter().enumerate() {
            let i = i as f32;
            let d = (i - self.cycle).abs().min((i - (self.cycle - n)).abs());
            let w = smooth_fade(d);
            self.fade = 1.0;
            for (channel, source) in self.color.iter_mut().zip(c) {
                *channel += source * w;
            }
        }
        self.cycle = if n > 1.0 {
            (self.cycle + dt) % n
        } else {
            (self.cycle - dt).max(0.0)
        };
    }

    /// Draw: drawn while auras exist or the fade lasts (fade −= dt per
    /// draw); Draw overrides the tint alpha with 0.666.
    pub fn draw(&mut self, auras: bool, dt: f32) -> Option<[f32; 4]> {
        if !auras && self.fade <= 0.0 {
            return None;
        }
        self.fade -= dt;
        Some([self.color[0], self.color[1], self.color[2], 0.666])
    }
}

fn buff_request(color: [f32; 3], position: [f32; 3], radius: f32) -> Request {
    Request {
        cell: BUFF_DEFENSIVE,
        scale: [radius * 1.5, radius * 1.5],
        position,
        direction: None,
        normal: [0.0, 1.0, 0.0],
        ttl: 1.0,
        color: [color[0], color[1], color[2], 0.0],
        animated_owner: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magicka_sim::spells::Elements as E;

    fn fire() -> Resistance {
        Resistance {
            against: E::FIRE,
            multiplier: 0.0,
            modifier: 0.0,
            status_resistance: false,
        }
    }

    #[test]
    fn health_buffs_execute_in_order_on_crossing_expiry_then_restore_template() {
        use magicka_sim::character_health::CharacterHealth;
        let mut b = Buffs::default();
        let mut health = CharacterHealth {
            hit_points: 200.0,
            ..Default::default()
        };
        assert!(b.add_nonvisual(
            1,
            NonvisualKind::ModifyHitPoints {
                multiplier: 2.0,
                modifier: 10.0,
            },
            0.5,
            false
        ));
        assert!(b.add_nonvisual(
            2,
            NonvisualKind::ModifyHitPoints {
                multiplier: 0.5,
                modifier: 3.0,
            },
            0.5,
            false
        ));
        b.update_nonvisual(0.5, 100.0, &mut health);
        assert_eq!(health.max_hit_points, 108.0);
        assert_eq!(health.hit_points, 108.0);
        assert_eq!(b.nonvisual.len(), 2);
        b.update_nonvisual(0.1, 100.0, &mut health);
        assert_eq!(health.max_hit_points, 100.0);
        assert_eq!(health.hit_points, 100.0);
        assert!(b.nonvisual.is_empty());
        b.update_nonvisual(0.1, 100.0, &mut health);
        assert_eq!(health.max_hit_points, 100.0);
    }

    #[test]
    fn invisible_damage_boosts_use_element_masks_and_live_list_not_ttl() {
        use magicka_sim::{
            damage::Damage,
            damage_delivery::{prepare, AttackerKind, DamagePayload, DeliveryContext, NetworkRole},
        };
        let mut b = Buffs::default();
        let grow = Damage {
            attack: 1,
            element: E::EARTH.0,
            amount: 0.0,
            magnitude: 3.0,
        };
        assert!(b.add_nonvisual(1, NonvisualKind::BoostDamage(grow), 0.5, false));
        // Matching ID changes only TTL, even if caller supplies another payload.
        assert!(!b.add_nonvisual(
            1,
            NonvisualKind::BoostDamage(Damage {
                magnitude: 10.0,
                ..grow
            }),
            0.25,
            false
        ));
        for (element, expected) in [
            (E::EARTH, 30.0),
            (E::FIRE, 10.0),
            (E::EARTH | E::FIRE, 30.0),
        ] {
            let payload = DamagePayload::Single(Damage {
                element: element.0,
                amount: 10.0,
                magnitude: 2.0,
                ..grow
            });
            let prepared = prepare(
                payload,
                DeliveryContext {
                    network: NetworkRole::Offline,
                    attacker_kind: AttackerKind::Character,
                    target_handle: 1,
                    attacker_handle: Some(2),
                    time_stamp: 0.0,
                    target_position: [0.0; 3],
                    attack_position: [0.0; 3],
                },
                &b.damage_boosts(),
            );
            let DamagePayload::Single(d) = prepared.payload else {
                unreachable!()
            };
            assert_eq!(d.magnitude, 2.0);
            assert_eq!(d.amount, expected);
        }
        let mut health = Default::default();
        b.update_nonvisual(0.25, 100.0, &mut health);
        assert_eq!(b.damage_boosts(), vec![grow]);
        b.update_nonvisual(0.0, 100.0, &mut health);
        assert!(b.damage_boosts().is_empty());
    }

    #[test]
    fn aura_fires_at_once_then_every_quarter_second() {
        let mut a = AuraVisual::new(3.0, [1.8, 0.6, 0.4]);
        assert_eq!(a.tick(1.0 / 60.0), 1);
        let fired: usize = (0..15).map(|_| a.tick(1.0 / 60.0)).sum();
        assert_eq!(fired, 1);
    }

    #[test]
    fn buffs_refresh_by_id_and_expire_half_a_second_after_the_aura() {
        let mut decals = Decals::default();
        let mut b = Buffs::default();
        for _ in 0..3 {
            b.add(7, fire(), [1.0; 3], true, false, [0.0; 3], 0.5, &mut decals);
        }
        assert_eq!(b.list.len(), 1);
        let decal = b.list[0].decal.unwrap();
        b.update(0.1, [1.0, 2.0, 3.0], 0.5, -1.0, false, &mut decals);
        // The decal follows the character at its feet, alpha 0.8.
        let p = decals.get(decal).unwrap();
        assert_eq!(&p.transform[12..15], &[1.0, 1.0, 3.0]);
        assert!((p.color[3] - 0.8).abs() < 1e-6);
        let mut frames = 0;
        while !b.list.is_empty() {
            b.update(1.0 / 60.0, [0.0; 3], 0.5, -1.0, false, &mut decals);
            frames += 1;
        }
        assert!((23..=26).contains(&frames), "{frames}");
        // Dead characters receive no new buffs.
        b.add(8, fire(), [1.0; 3], true, true, [0.0; 3], 0.5, &mut decals);
        assert!(b.list.is_empty());
    }

    #[test]
    fn ring_places_six_decals_on_the_aura_radius() {
        let mut decals = Decals::default();
        let mut a = AuraVisual::new(3.0, [0.0, 0.7, 1.3]);
        let ground = |o: [f32; 3], _: [f32; 3]| Some([o[0], 0.0, o[2]]);
        a.update_ring(0.5, [10.0, 1.0, 0.0], false, ground, &mut decals);
        a.update_ring(0.5, [10.0, 1.0, 0.0], false, ground, &mut decals);
        for r in a.ring {
            let p = decals.get(r.unwrap()).unwrap();
            let (x, z) = (p.transform[12] - 10.0, p.transform[14]);
            assert!(((x * x + z * z).sqrt() - 3.0).abs() < 1e-4);
            assert_eq!(p.transform[13], 0.0);
            assert!((p.color[3] - 0.666).abs() < 1e-6);
        }
    }
}
