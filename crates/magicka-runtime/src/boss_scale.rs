//! Boss health for parties larger than the original's four (rewrite extension;
//! `research/boss-scaling.md`). Native boss health ignores the party, which was fine
//! with four players. The fight would otherwise end in seconds at 16 or more.
use crate::slots::ORIGINAL_PLAYER_LIMIT;

/// What incoming boss damage is divided by. Damage rather than each boss's own
/// health constant, so health bars and phase thresholds keep their fractions.
/// Up to four players this is exactly native; above that a fight takes about as long
/// as it does for four.
pub fn damage_divisor(enabled: bool, players: usize) -> f32 {
    if enabled && players > ORIGINAL_PLAYER_LIMIT {
        players as f32 / ORIGINAL_PLAYER_LIMIT as f32
    } else {
        1.0
    }
}

/// The damage a boss takes under `divisor`. Bosses lose `amount * magnitude`, and
/// `amount` also takes resistance modifiers, so the magnitude is what scales. Death
/// reads the amount alone (`PhysicsDamage::deliver`).
pub fn scaled(
    mut damage: magicka_sim::damage::Damage,
    divisor: f32,
) -> magicka_sim::damage::Damage {
    damage.magnitude /= divisor;
    damage
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_changes_up_to_four_players() {
        for players in 0..=4 {
            assert_eq!(damage_divisor(true, players), 1.0);
        }
    }

    #[test]
    fn larger_parties_scale_up_to_the_server_limit() {
        assert_eq!(damage_divisor(true, 8), 2.0);
        assert_eq!(damage_divisor(true, 16), 4.0);
        assert_eq!(damage_divisor(true, 256), 64.0);
    }

    #[test]
    fn a_zone_hands_over_scaled_damage_once() {
        let mut zone = crate::physics_damage::PhysicsDamage::boss_zone(
            0,
            magicka_content::physics::CollisionBox {
                name: String::new(),
                position: [0.; 3],
                sides: [1.; 3],
                orientation: [0., 0., 0., 1.],
            },
        );
        let damage = magicka_sim::damage::Damage {
            attack: 1,
            element: 1,
            amount: 100.,
            magnitude: 2.,
        };
        zone.boss_inbox.push((damage, 15));
        let inbox = zone.take_boss_inbox(4.0);
        assert_eq!(inbox[0].0.magnitude, 0.5);
        assert_eq!(inbox[0].0.amount, 100.);
        assert!(zone.take_boss_inbox(4.0).is_empty());
    }

    #[test]
    fn the_option_turns_it_off() {
        assert_eq!(damage_divisor(false, 256), 1.0);
    }
}
