//! Fear magick: `Character.Fear`/`RemoveFear`, the fear timer in
//! `Character.Update` and PanicState's feared movement (research/fear.md).
//! Caster-independent: a source is a world position or a character handle.

pub type Vector = [f32; 3];

/// Fear.RANGE: EntityManager.GetEntities radius around the caster.
pub const RANGE: f32 = 6.0;
/// Fear.FEAR_TIME, also the literal Character.Fear writes.
pub const FEAR_TIME: f32 = 5.0;
/// Factions.DEMON | Factions.UNDEAD: never feared.
pub const IMMUNE_FACTIONS: i32 = 0x08 | 0x10;
/// PanicState: SpeedMultiplier *= 0.8 while feared (not panicking).
pub const FEARED_SPEED: f32 = 0.8;

/// Character.mFearedBy / mFearPosition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FearSource {
    Position(Vector),
    Character(u16),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fear {
    /// Character.mFearTimer.
    pub timer: f32,
    /// Character.mFearedBy (handle); None leaves `position` in use.
    pub by: Option<u16>,
    /// Character.mFearPosition (kept when a character fears instead).
    pub position: Vector,
}

impl Fear {
    /// Character.IsFeared.
    pub fn is_feared(&self, dead: bool) -> bool {
        self.timer > 0.0 && !dead
    }

    /// Character.Fear(Vector3) / Fear(Character). Returns false when the
    /// faction or IsFearless rejects it. The caller starts `magick_feared`
    /// when the effect is not already active.
    pub fn apply(&mut self, faction: i32, fearless: bool, source: FearSource) -> bool {
        if faction & IMMUNE_FACTIONS != 0 || fearless {
            return false;
        }
        self.timer = FEAR_TIME;
        match source {
            FearSource::Position(p) => {
                self.by = None;
                self.position = p;
            }
            FearSource::Character(h) => self.by = Some(h),
        }
        true
    }

    /// Character.RemoveFear.
    pub fn remove(&mut self) {
        self.timer = 0.0;
        self.by = None;
    }

    /// Character.Update: while feared the timer drops by dt; RemoveFear runs
    /// the frame it stops being feared. Returns true when it was removed.
    pub fn tick(&mut self, dt: f32, dead: bool) -> bool {
        if !self.is_feared(dead) {
            return false;
        }
        self.timer -= dt;
        if !self.is_feared(dead) {
            self.remove();
            return true;
        }
        false
    }

    /// Character.FearPosition: the fearing character's live position, else
    /// the stored point. `lookup` resolves a handle (None: entity gone,
    /// which the original cannot express; the stored point is used).
    pub fn source_position(&self, lookup: impl Fn(u16) -> Option<Vector>) -> Vector {
        self.by.and_then(lookup).unwrap_or(self.position)
    }
}

/// XNA Vector3.Normalize (multiply by the reciprocal length).
fn normalize(v: Vector) -> Vector {
    let inv = 1.0 / (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] * inv, v[1] * inv, v[2] * inv]
}

/// PanicState.Update, feared branch (not panicking): flee along the
/// horizontal direction away from `source` (X nudged by 1e-4 before
/// normalising), mixed half and half with `current` movement when that is
/// non-zero. `current` is CharacterBody.Movement for NPCs and
/// DesiredInputDirection for avatars.
pub fn flee_movement(position: Vector, source: Vector, current: Vector) -> Vector {
    let mut away = [position[0] - source[0], 0.0, position[2] - source[2]];
    away[0] += 0.0001;
    let away = normalize(away);
    if current[0] * current[0] + current[1] * current[1] + current[2] * current[2] > 1e-6 {
        normalize([
            current[0] * 0.5 + away[0] * 0.5,
            current[1] * 0.5 + away[1] * 0.5,
            current[2] * 0.5 + away[2] * 0.5,
        ])
    } else {
        away
    }
}

/// Fear.Execute(ISpellCaster) target filter for one entity in range: a
/// character sharing no faction bit with the caster whose self shield is
/// absent or solid. The caster itself is removed from the list beforehand.
pub fn targets(caster_faction: i32, faction: i32, self_shielded: bool, solid_shield: bool) -> bool {
    faction & caster_faction == 0 && (!self_shielded || solid_shield)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demons_undead_and_fearless_are_immune() {
        let mut f = Fear::default();
        assert!(!f.apply(0x08, false, FearSource::Character(1)));
        assert!(!f.apply(0x10 | 0x01, false, FearSource::Character(1)));
        assert!(!f.apply(0x01, true, FearSource::Character(1)));
        assert_eq!(f.timer, 0.0);
        assert!(f.apply(0x01, false, FearSource::Character(7)));
        assert_eq!((f.timer, f.by), (FEAR_TIME, Some(7)));
    }

    #[test]
    fn position_source_clears_character_and_character_keeps_position() {
        let mut f = Fear::default();
        f.apply(1, false, FearSource::Character(3));
        f.apply(1, false, FearSource::Position([1.0, 2.0, 3.0]));
        assert_eq!((f.by, f.position), (None, [1.0, 2.0, 3.0]));
        f.apply(1, false, FearSource::Character(4));
        assert_eq!(f.position, [1.0, 2.0, 3.0]);
        assert_eq!(
            f.source_position(|_| Some([9.0, 0.0, 9.0])),
            [9.0, 0.0, 9.0]
        );
        assert_eq!(f.source_position(|_| None), [1.0, 2.0, 3.0]);
    }

    #[test]
    fn timer_expires_and_dead_characters_are_not_feared() {
        let mut f = Fear::default();
        f.apply(1, false, FearSource::Character(2));
        assert!(!f.is_feared(true));
        assert!(!f.tick(1.0, true));
        assert_eq!(f.timer, FEAR_TIME);
        assert!(!f.tick(4.5, false));
        assert!(f.tick(0.5, false));
        assert_eq!((f.timer, f.by), (0.0, None));
    }

    #[test]
    fn flee_is_horizontal_and_blends_with_movement() {
        let m = flee_movement([0.0, 5.0, 3.0], [0.0, 0.0, 0.0], [0.0; 3]);
        assert!((m[2] - 1.0).abs() < 1e-6 && m[1] == 0.0 && m[0] > 0.0);
        let m = flee_movement([0.0, 0.0, 3.0], [0.0; 3], [1.0, 0.0, 0.0]);
        let s = std::f32::consts::FRAC_1_SQRT_2;
        assert!((m[0] - s).abs() < 1e-3 && (m[2] - s).abs() < 1e-3);
    }

    #[test]
    fn target_filter() {
        assert!(targets(0x144, 0x01, false, false));
        assert!(!targets(0x144, 0x04, false, false));
        assert!(!targets(0x144, 0x01, true, false));
        assert!(targets(0x144, 0x01, true, true));
    }
}
