//! `Bosses.CountryDefense` (Highlands hl_s5BF): a UI-only named boss. The
//! boss bar follows the beastman chieftain, and five house icons (slices of
//! the UI/Boss/CountryDefense Texture3D) show the houses' remaining health.
//! See research/boss-battles.md.
use super::*;

/// UI/Boss/CountryDefense depth (Texture3D 64x64x4).
pub const HOUSE_SLICES: u32 = 4;
pub const HOUSES: usize = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct CountryDefenseHost {
    /// Stable house0..house4 slots. Missing houses show the burnt slice;
    /// compacting native null entries left stale icons after save restoration.
    houses: [Option<u16>; HOUSES],
    pub chieftain: Option<u16>,
    /// mTimer: advances outside cutscenes; the icons' alpha is min(t, 1).
    pub timer: f32,
    /// mHouseAnimations as Texture3D slice indices.
    pub slices: [u32; HOUSES],
}

impl CountryDefenseHost {
    pub fn alpha(&self) -> f32 {
        self.timer.min(1.0)
    }
}

/// The slice for a health fraction: W = 1 − (⌈f·(d−1)⌉/d + 1/d) + 1/(2d),
/// i.e. slice d − 1 − ⌈f·(d−1)⌉ (full health is slice 0).
pub fn house_slice(fraction: f32) -> u32 {
    let d = HOUSE_SLICES as f32;
    let full = (fraction.max(0.0) * (d - 1.0)).ceil() as u32;
    (HOUSE_SLICES - 1).saturating_sub(full)
}

impl<C: SceneContent> SceneWorld<C> {
    /// CountryDefense.Initialize.
    pub(super) fn spawn_country_defense(&mut self) -> CountryDefenseHost {
        let houses = std::array::from_fn(|i| {
            self.by_unique_id(custom_hash(&format!("house{i}")))
                .map(|e| e.handle)
        });
        let chieftain = self
            .by_unique_id(custom_hash("#ent_beast_chieftain"))
            .map(|e| e.handle);
        CountryDefenseHost {
            houses,
            chieftain,
            timer: 0.0,
            slices: [0; HOUSES],
        }
    }

    pub(super) fn country_defense_health(&self, host: &CountryDefenseHost) -> (f32, f32) {
        match host.chieftain.and_then(|h| self.npcs.get(&h)) {
            Some(n) => (
                n.character.combat.health.hit_points,
                n.character.combat.health.max_hit_points,
            ),
            // A chieftain entity without a live character keeps its entity health.
            None => match host.chieftain.and_then(|h| self.entity(h).ok()) {
                Some(e) => (e.hit_points, e.max_hit_points),
                None => (1.0, 1.0),
            },
        }
    }

    /// CountryDefense.UpdateBoss (independent of the fight having started).
    pub(super) fn update_country_defense(&mut self, host: &mut CountryDefenseHost, dt: f32) {
        if !self.flags.cutscene {
            host.timer += dt;
        }
        if host.chieftain.is_none() {
            host.chieftain = self
                .by_unique_id(custom_hash("#ent_beast_chieftain"))
                .map(|e| e.handle);
        }
        for (i, house) in host.houses.iter().enumerate() {
            let Some(house) = *house else {
                host.slices[i] = HOUSE_SLICES - 1;
                continue;
            };
            let present = self
                .entity(house)
                .ok()
                .is_some_and(|e| e.added && !e.removed && !e.dead);
            let fraction = if !present {
                0.
            } else {
                match self.physics_damage.get(&house) {
                    Some(s) if s.maximum > 0.0 => s.health / s.maximum,
                    _ => self
                        .entity(house)
                        .ok()
                        .filter(|e| e.max_hit_points > 0.0)
                        .map_or(0.0, |e| e.hit_points / e.max_hit_points),
                }
            };
            host.slices[i] = house_slice(fraction);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::house_slice;

    #[test]
    fn slices_step_down_with_health() {
        assert_eq!(house_slice(1.0), 0);
        assert_eq!(house_slice(0.9), 0);
        assert_eq!(house_slice(0.6), 1);
        assert_eq!(house_slice(0.3), 2);
        assert_eq!(house_slice(0.0), 3);
        assert_eq!(house_slice(-0.5), 3);
    }
}
