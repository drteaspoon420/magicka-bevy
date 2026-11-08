//! Impact movement and frontal blocking extracted from Character/Entity.
//! See research/impact-motion.md. Inputs use original XNA world coordinates.
use crate::damage::{attack as A, Damage};
use crate::spells::Elements as E;
pub type Vector = [f32; 3];
fn length_squared(v: Vector) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}
fn horizontal_normalize(mut v: Vector) -> Vector {
    v[1] = 0.0;
    let inverse = 1.0 / (length_squared(v) as f64).sqrt() as f32;
    [v[0] * inverse, v[1] * inverse, v[2] * inverse]
}
fn max(a: f32, b: f32) -> f32 {
    if a > b || a.is_nan() {
        a
    } else {
        b
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MotionContext {
    pub position: Vector,
    pub mass: f32,
    pub gravity_y: f32,
    pub immortal: bool,
    pub ethereal: bool,
    pub solid_shield: bool,
    pub entangled: bool,
    pub dead: bool,
}
impl MotionContext {
    /// Character override gates the original Entity ballistic formula.
    pub fn impulse(
        &self,
        direction: Vector,
        elevation: f32,
        mass_power: f32,
        distance: f32,
    ) -> Vector {
        if self.immortal
            || self.ethereal
            || self.solid_shield
            || self.mass > 1000.0
            || self.entangled
            || self.dead
        {
            return [0.0; 3];
        }
        self.entity_impulse(direction, elevation, mass_power, distance)
    }

    /// Entity.CalcImpulseVelocity, without Character's status/mass gates.
    /// MissileEntity delegates directly to this base implementation.
    pub fn entity_impulse(
        &self,
        direction: Vector,
        elevation: f32,
        mass_power: f32,
        mut distance: f32,
    ) -> Vector {
        distance *= max(
            0.0,
            (1.0 - (self.mass as f64 - mass_power as f64 * 0.25) / mass_power as f64).sqrt() as f32
                * 2.0,
        );
        if distance.is_nan() || distance < 1e-6 {
            return [0.0; 3];
        }
        let mut direction = horizontal_normalize(direction);
        if direction[0].is_nan() {
            return [0.0; 3];
        }
        // Preserve the original translated-endpoint distance calculation;
        // replacing it with `distance` changes rounding at large coordinates.
        let end_x = direction[0] * distance + self.position[0];
        let end_z = direction[2] * distance + self.position[2];
        let dx = end_x - self.position[0];
        let dz = end_z - self.position[2];
        let n = ((dx * dx + dz * dz) as f64).sqrt() as f32;
        let sin = (elevation as f64).sin() as f32;
        let cos = (elevation as f64).cos() as f32;
        direction[1] = sin;
        direction[0] *= cos;
        direction[2] *= cos;
        let speed = 1.4
            * ((-self.gravity_y * n * n / (2.0 * (n * sin / cos) * cos * cos)) as f64).sqrt()
                as f32;
        [
            direction[0] * speed,
            direction[1] * speed,
            direction[2] * speed,
        ]
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ImpactMotion {
    pub velocity: Vector,
    pub impulse: Vector,
    pub is_pushed: bool,
    pub leaving_ground: bool,
    pub knocked_down: bool,
    pub knockdown_tolerance: f32,
    pub grip_damage: f32,
}
impl ImpactMotion {
    pub fn can_be_knocked_down(&self) -> bool {
        self.knockdown_tolerance != 5000.0
    }
    pub fn add_impulse(&mut self, impulse: Vector, entangled: bool) {
        if entangled {
            return;
        }
        for (value, add) in self.impulse.iter_mut().zip(impulse) {
            *value += add;
        }
        self.is_pushed = true;
        if self.impulse[1] > 2.7 {
            self.leaving_ground = true;
        }
    }
    pub fn knock_down(&mut self, magnitude: Option<f32>) {
        if self.can_be_knocked_down() && magnitude.is_none_or(|m| m >= self.knockdown_tolerance) {
            self.knocked_down = true;
            self.velocity[0] = 0.0;
            self.velocity[2] = 0.0;
            if magnitude.is_none() {
                self.grip_damage = 0.0;
            }
        }
    }
    /// Returns original DamageResult bits: Knockedback 8, Pushed 16.
    /// Knockdown-only deliberately contributes no result bit in this path.
    pub fn apply_impact(
        &mut self,
        damage: Damage,
        attack_position: Vector,
        context: MotionContext,
        knockback_enabled: bool,
    ) -> u16 {
        let delta = [
            context.position[0] - attack_position[0],
            0.0,
            context.position[2] - attack_position[2],
        ];
        if damage.attack & A::KNOCKBACK == A::KNOCKBACK {
            if length_squared(delta) > f32::from_bits(1) {
                let v = context.impulse(
                    horizontal_normalize(delta),
                    0.6980619,
                    damage.amount,
                    damage.magnitude,
                );
                if length_squared(v) > 1e-6 {
                    if knockback_enabled {
                        self.add_impulse(v, context.entangled);
                    }
                    if self.can_be_knocked_down() {
                        self.knocked_down = true;
                    }
                    return 8;
                }
            }
            return 0;
        }
        let mut result = 0;
        if damage.attack & A::PUSHED != 0 {
            let v = context.impulse(
                horizontal_normalize(delta),
                0.17453292,
                damage.amount,
                damage.magnitude,
            );
            if length_squared(v) > 1e-6 {
                if knockback_enabled {
                    self.add_impulse(v, context.entangled);
                }
                result |= 16;
            }
        }
        if damage.attack & A::KNOCKDOWN != 0 {
            self.knock_down(Some(damage.magnitude));
        }
        result
    }
}

/// Original BlockDamage: equipment values are summed; vertical components are
/// discarded; a coincident point gives NaN and fails the angle comparison.
pub fn frontal_block(
    position: Vector,
    forward: Vector,
    attack_position: Vector,
    values: &[i32],
) -> Option<f32> {
    let mut amount = 0.0;
    for value in values {
        amount += *value as f32;
    }
    let direction = horizontal_normalize([
        attack_position[0] - position[0],
        0.0,
        attack_position[2] - position[2],
    ]);
    let forward = horizontal_normalize(forward);
    let dot = forward[0] * direction[0] + forward[1] * direction[1] + forward[2] * direction[2];
    let angle = (dot.clamp(-1.0, 1.0) as f64).acos() as f32;
    (angle <= 1.0995574).then_some(amount)
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlockOutcome {
    pub effect: bool,
    pub deflected: bool,
}
/// The blocking portion of InternalDamage. A full block leaves Damage intact
/// and marks Deflected; status application still follows before the early exit.
pub fn block_impact(
    damage: &mut Damage,
    block_item: i32,
    position: Vector,
    forward: Vector,
    attack_position: Vector,
    values: &[i32],
) -> BlockOutcome {
    if damage.attack & A::DAMAGE == 0
        || damage.element & (E::EARTH | E::LIGHTNING).0 == 0
        || block_item < 0
    {
        return BlockOutcome::default();
    }
    if let Some(blocked) = frontal_block(position, forward, attack_position, values) {
        let deflected = blocked >= damage.amount * damage.magnitude;
        if !deflected {
            damage.amount = max(0.0, damage.amount - blocked);
        }
        BlockOutcome {
            effect: true,
            deflected,
        }
    } else {
        BlockOutcome::default()
    }
}
