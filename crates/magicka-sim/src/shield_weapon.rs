//! ShieldSpell's Weapon branch (research/weapon-enchant.md): CastWeapon only
//! stores the spell; AnimationEnd (or DeInitialize when AnimationEnd never
//! ran) places one entity in front of the caster. Pure Shield raises a WALL
//! shield; Arcane/Life without physical elements starts a forward mine line;
//! anything else starts a forward barrier line. Scene probes and spawning
//! are host work.
use crate::barrier::BarrierPlacement;
use crate::damage::{calculate_damage, CastType, Damage, DamageError};
use crate::shield_entity::{ShieldKind, SHIELD_COLOR};
use crate::spell_mine::MinePlacement;
use crate::spells::{Elements as E, Spell, SpellType};

pub type Vector = [f32; 3];

/// Pure Shield: the WALL shield's radius and hit points.
pub const WALL_RADIUS: f32 = 5.5;
pub const WALL_HIT_POINTS: f32 = 800.0;
/// Spacing of the forward mine/barrier line.
pub const LINE_SPACING: f32 = 2.5;

#[derive(Debug, Clone, PartialEq)]
pub enum ShieldWeapon {
    /// Shield.Initialize(owner, position, 5.5, Direction, WALL, 800,
    /// SHIELDCOLOR).
    Wall {
        position: Vector,
        radius: f32,
        direction: Vector,
        kind: ShieldKind,
        hit_points: f32,
        color: Vector,
    },
    /// SpellMine.Initialize after a 4 m drop probe from `probe_origin`.
    Mine(MinePlacement),
    /// Barrier.Initialize after a 4 m drop probe, with Shield/Weapon damage.
    Barrier {
        placement: BarrierPlacement,
        damage: [Damage; 5],
    },
}

fn add(a: Vector, b: Vector) -> Vector {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scaled(v: Vector, s: f32) -> Vector {
    v.map(|c| c * s)
}

/// ShieldSpell.AnimationEnd with mCastType = Weapon. The anchor is
/// Position + Direction·Radius; the line has 2 + ⌊max(TotalMagnitude/2, 1)⌋
/// slots of 2.5 m (range = 2.5·slots), starting 2.5 m beyond the anchor,
/// facing (Direction.z, 0, −Direction.x) with an identity next rotation.
/// Mines probe from 1 m above, barriers from 2 m above.
pub fn animation_end(
    spell: &Spell,
    owner_position: Vector,
    owner_direction: Vector,
    owner_radius: f32,
) -> Result<ShieldWeapon, DamageError> {
    let anchor = add(owner_position, scaled(owner_direction, owner_radius));
    let rest = spell.element.0 & !E::SHIELD.0;
    if rest == 0 {
        return Ok(ShieldWeapon::Wall {
            position: add(anchor, scaled(owner_direction, WALL_RADIUS + 2.0)),
            radius: WALL_RADIUS,
            direction: owner_direction,
            kind: ShieldKind::Wall,
            hit_points: WALL_HIT_POINTS,
            color: SHIELD_COLOR,
        });
    }
    let slots = 2 + (spell.total_magnitude() * 0.5).max(1.0) as i32;
    let range = LINE_SPACING * slots as f32;
    let start = add(anchor, scaled(owner_direction, LINE_SPACING));
    let facing = [owner_direction[2], 0.0, -owner_direction[0]];
    let next_direction = scaled(owner_direction, LINE_SPACING);
    let mine = rest & E::PHYSICAL_ELEMENTS.0 == 0 && rest & (E::ARCANE.0 | E::LIFE.0) != 0;
    // Both branches compute CalculateDamage(Shield, Weapon) first; SpellMine
    // ignores the argument and recomputes Shield/Force itself.
    let list = calculate_damage(spell, SpellType::Shield, CastType::Weapon)?;
    let lift = if mine { 1.0 } else { 2.0 };
    let probe_origin = [start[0], start[1] + lift, start[2]];
    if mine {
        return Ok(ShieldWeapon::Mine(MinePlacement {
            probe_origin,
            direction: facing,
            scale: 1.0,
            range,
            next_direction,
            next_yaw: 0.0,
            distance: LINE_SPACING,
        }));
    }
    let mut damage = [Damage::default(); 5];
    for (slot, d) in damage.iter_mut().zip(list) {
        *slot = d;
    }
    Ok(ShieldWeapon::Barrier {
        placement: BarrierPlacement {
            probe_origin,
            direction: facing,
            scale: 1.0,
            range,
            next_direction,
            next_yaw: 0.0,
            distance: LINE_SPACING,
        },
        damage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spell(elements: &[(E, usize)]) -> Spell {
        let list: Vec<E> = elements
            .iter()
            .flat_map(|&(e, n)| std::iter::repeat_n(e, n))
            .collect();
        Spell::default_for_list(&list)
    }

    #[test]
    fn pure_shield_raises_a_wall_seven_and_a_half_metres_past_the_body() {
        let w = animation_end(
            &spell(&[(E::SHIELD, 1)]),
            [1.0, 0.5, 2.0],
            [0.0, 0.0, 1.0],
            0.5,
        )
        .unwrap();
        let ShieldWeapon::Wall {
            position,
            radius,
            kind,
            hit_points,
            ..
        } = w
        else {
            panic!("{w:?}");
        };
        assert_eq!(position, [1.0, 0.5, 2.0 + 0.5 + 7.5]);
        assert_eq!((radius, kind, hit_points), (5.5, ShieldKind::Wall, 800.0));
    }

    #[test]
    fn arcane_shield_starts_a_forward_mine_line() {
        let s = spell(&[(E::SHIELD, 1), (E::ARCANE, 2)]);
        let ShieldWeapon::Mine(p) = animation_end(&s, [0.0; 3], [1.0, 0.0, 0.0], 0.5).unwrap()
        else {
            panic!();
        };
        // TotalMagnitude 3 → 2 + ⌊1.5⌋ = 3 slots.
        assert_eq!(p.range, 7.5);
        assert_eq!(p.probe_origin, [3.0, 1.0, 0.0]);
        assert_eq!(p.direction, [0.0, 0.0, -1.0]);
        assert_eq!(p.next_direction, [2.5, 0.0, 0.0]);
        assert_eq!((p.next_yaw, p.distance, p.scale), (0.0, 2.5, 1.0));
    }

    #[test]
    fn other_shield_combinations_start_a_barrier_line_with_weapon_damage() {
        let s = spell(&[(E::SHIELD, 1), (E::EARTH, 1)]);
        let ShieldWeapon::Barrier { placement, damage } =
            animation_end(&s, [0.0; 3], [0.0, 0.0, -1.0], 0.5).unwrap()
        else {
            panic!();
        };
        assert_eq!(placement.probe_origin, [0.0, 2.0, -3.0]);
        // TotalMagnitude 2 → 2 + max(1, 1) = 3 slots.
        assert_eq!(placement.range, 7.5);
        let expected = calculate_damage(&s, SpellType::Shield, CastType::Weapon).unwrap();
        assert_eq!(damage[..expected.len()], expected[..]);
        // Earth + Arcane is physical: a barrier, not a mine.
        let s = spell(&[(E::SHIELD, 1), (E::EARTH, 1), (E::ARCANE, 1)]);
        assert!(matches!(
            animation_end(&s, [0.0; 3], [0.0, 0.0, -1.0], 0.5),
            Ok(ShieldWeapon::Barrier { .. })
        ));
    }
}
