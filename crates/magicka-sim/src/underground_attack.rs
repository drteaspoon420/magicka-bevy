//! UnderGroundAttack (research/weapon-enchant.md): the rock wave of an
//! Earth-enchanted weapon swing. It follows the ground at a fixed planar
//! velocity, stops at shields, walls and ledges, and damages each damageable
//! entity within 1 m once.
use crate::{damage::Damage, impact_motion::Vector, spray_pulse::Segment};

/// EFFECTS by element index (0 = none).
pub const EFFECTS: [&str; 11] = [
    "underground_earth",
    "underground_water",
    "underground_cold",
    "underground_fire",
    "",
    "underground_arcane",
    "underground_life",
    "",
    "",
    "underground_steam",
    "underground_steam",
];

pub trait UndergroundWorld {
    /// Level.CurrentScene.SegmentIntersect: hit position and normal.
    fn ground(&mut self, seg: Segment) -> Option<(Vector, Vector)>;
    /// Any EntityManager.Shields[i].SegmentIntersect(seg, 1).
    fn shield_blocks(&mut self, seg: Segment) -> bool;
    /// IDamageable handles from GetEntities(center, radius, true).
    fn damageables(&mut self, center: Vector, radius: f32) -> Vec<u16>;
    fn damage(&mut self, target: u16, damages: [Damage; 5], timestamp: f64, position: Vector);
}

#[derive(Clone, Debug, PartialEq)]
pub struct UndergroundAttack {
    pub position: Vector,
    pub velocity: [f32; 2],
    pub direction: Vector,
    pub range: f32,
    pub damages: [Damage; 5],
    pub timestamp: f64,
    pub hits: Vec<u16>,
}

impl UndergroundAttack {
    /// Initialize: the hit list starts with the owner; the attack exists only
    /// when a ground hit (normal.y ≥ 0.7) lies within 1 above to 3 below.
    pub fn initialize(
        position: Vector,
        velocity: [f32; 2],
        owner: u16,
        timestamp: f64,
        range: f32,
        damages: [Damage; 5],
        world: &mut impl UndergroundWorld,
    ) -> Option<Self> {
        let l = (velocity[0] * velocity[0] + velocity[1] * velocity[1]).sqrt();
        let direction = [velocity[0] / l, 0.0, velocity[1] / l];
        let seg = Segment {
            origin: [position[0], position[1] + 1.0, position[2]],
            delta: [0.0, -4.0, 0.0],
        };
        let (ground, normal) = world.ground(seg)?;
        if normal[1] < 0.7 {
            return None;
        }
        Some(Self {
            position: ground,
            velocity,
            direction,
            range,
            damages,
            timestamp,
            hits: vec![owner],
        })
    }

    /// The effects started at Initialize (one per element present).
    pub fn effect_names(&self) -> Vec<&'static str> {
        let all = self.damages.iter().fold(0u32, |a, d| a | d.element);
        (0..11)
            .filter(|i| all & (1 << i) != 0 && !EFFECTS[*i].is_empty())
            .map(|i| EFFECTS[i])
            .collect()
    }

    pub fn dead(&self) -> bool {
        self.range <= 0.0
    }

    /// Update: step along the ground; a missing/steep ground hit, a shield
    /// or a level wall between the old and new point kills the attack.
    pub fn update(&mut self, dt: f32, world: &mut impl UndergroundWorld) {
        let step = [self.velocity[0] * dt, self.velocity[1] * dt];
        let origin = self.position;
        let seg = Segment {
            origin: [origin[0] + step[0], origin[1] + 1.0, origin[2] + step[1]],
            delta: [0.0, -2.0, 0.0],
        };
        let Some((ground, normal)) = world.ground(seg).filter(|(_, n)| n[1] >= 0.7) else {
            self.range = 0.0;
            return;
        };
        let _ = normal;
        let mut path = Segment {
            origin,
            delta: [
                ground[0] - origin[0],
                ground[1] - origin[1],
                ground[2] - origin[2],
            ],
        };
        if world.shield_blocks(path) {
            self.range = 0.0;
            return;
        }
        path.origin[1] += 1.0;
        if world.ground(path).is_some() {
            self.range = 0.0;
            return;
        }
        self.position = ground;
        self.range -= (step[0] * step[0] + step[1] * step[1]).sqrt();
        for target in world.damageables(self.position, 1.0) {
            if !self.hits.contains(&target) {
                self.hits.push(target);
                world.damage(target, self.damages, self.timestamp, self.position);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Flat {
        wall_x: Option<f32>,
        targets: Vec<(u16, Vector)>,
        damaged: Vec<u16>,
    }
    impl UndergroundWorld for Flat {
        fn ground(&mut self, seg: Segment) -> Option<(Vector, Vector)> {
            let end = [seg.origin[0] + seg.delta[0], seg.origin[2] + seg.delta[2]];
            if let Some(x) = self.wall_x {
                // A wall plane at x: horizontal segments crossing it hit.
                if (seg.origin[0] - x) * (end[0] - x) < 0.0 {
                    return Some(([x, seg.origin[1], seg.origin[2]], [-1.0, 0.0, 0.0]));
                }
            }
            // Ground plane y = 0 for downward segments.
            let (y0, y1) = (seg.origin[1], seg.origin[1] + seg.delta[1]);
            (y0 >= 0.0 && y1 <= 0.0 && seg.delta[1] < 0.0)
                .then_some(([seg.origin[0], 0.0, seg.origin[2]], [0.0, 1.0, 0.0]))
        }
        fn shield_blocks(&mut self, _: Segment) -> bool {
            false
        }
        fn damageables(&mut self, c: Vector, r: f32) -> Vec<u16> {
            self.targets
                .iter()
                .filter(|(_, p)| ((p[0] - c[0]).powi(2) + (p[2] - c[2]).powi(2)).sqrt() <= r)
                .map(|(h, _)| *h)
                .collect()
        }
        fn damage(&mut self, t: u16, _: [Damage; 5], _: f64, _: Vector) {
            self.damaged.push(t);
        }
    }

    fn earth() -> [Damage; 5] {
        let mut d = [Damage::default(); 5];
        d[0].element = 1;
        d[0].amount = 100.0;
        d
    }

    #[test]
    fn travels_fifteen_metres_and_hits_each_target_once() {
        let mut w = Flat {
            targets: vec![(7, [5.0, 0.0, 0.0]), (1, [0.0, 0.0, 0.0])],
            ..Default::default()
        };
        let mut a = UndergroundAttack::initialize(
            [0.0, 0.5, 0.0],
            [40.0, 0.0],
            1,
            0.0,
            15.0,
            earth(),
            &mut w,
        )
        .unwrap();
        assert_eq!(a.position, [0.0, 0.0, 0.0]);
        assert_eq!(a.effect_names(), ["underground_earth"]);
        let mut frames = 0;
        while !a.dead() {
            a.update(1.0 / 60.0, &mut w);
            frames += 1;
            assert!(frames < 100);
        }
        // 15 m at 40 m/s ≈ 0.375 s.
        assert!((22..=24).contains(&frames), "{frames}");
        assert_eq!(w.damaged, [7]);
    }

    #[test]
    fn a_wall_stops_the_wave() {
        let mut w = Flat {
            wall_x: Some(3.0),
            ..Default::default()
        };
        let mut a = UndergroundAttack::initialize(
            [0.0, 0.5, 0.0],
            [40.0, 0.0],
            1,
            0.0,
            15.0,
            earth(),
            &mut w,
        )
        .unwrap();
        while !a.dead() {
            a.update(1.0 / 60.0, &mut w);
        }
        assert!(a.position[0] < 3.0, "{:?}", a.position);
    }
}
