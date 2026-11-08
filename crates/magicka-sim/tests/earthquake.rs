use magicka_sim::{damage::Damage, earthquake::*, impact_motion::Vector};

#[derive(Default)]
struct Host {
    client: bool,
    ground: Option<Vector>,
    targets: Vec<Target>,
    draws: usize,
    impulses: Vec<(u16, Vector)>,
    damage: Vec<(u16, Option<u16>, Damage, f64, Vector)>,
    velocities: Vec<(u16, Vector)>,
    centers: Vec<(Vector, f32)>,
    pulses: usize,
    stops: usize,
    failures: usize,
}
impl EarthquakeHost for Host {
    fn random(&mut self) -> f64 {
        self.draws += 1;
        0.25
    }
    fn client(&self) -> bool {
        self.client
    }
    fn ground(&mut self, _: Vector) -> Option<Vector> {
        self.ground
    }
    fn targets(&mut self, _: Vector, radius: f32) -> Vec<u16> {
        assert_eq!(radius, 10.0);
        self.targets.iter().map(|t| t.handle).collect()
    }
    fn target(&self, handle: u16) -> Option<Target> {
        self.targets.iter().find(|t| t.handle == handle).copied()
    }
    fn impulse_velocity(&mut self, handle: u16, v: Vector) {
        self.impulses.push((handle, v));
    }
    fn damage(&mut self, handle: u16, owner: Option<u16>, d: Damage, time: f64, p: Vector) {
        assert!(self.impulses.iter().any(|(h, _)| *h == handle));
        self.damage.push((handle, owner, d, time, p));
    }
    fn add_velocity(&mut self, handle: u16, v: Vector) {
        self.velocities.push((handle, v));
    }
    fn new_quake(&mut self, _: Option<u16>, p: Vector, ttl: f32) {
        self.centers.push((p, ttl));
    }
    fn pulse(&mut self, _: Vector, scale: Vector) {
        assert_eq!(scale, [2.0, 1.0, 2.0]);
        self.pulses += 1;
    }
    fn stop_presentation(&mut self) {
        self.stops += 1;
    }
    fn failed(&mut self, _: Vector) {
        self.failures += 1;
    }
}
fn grounded(handle: u16) -> Target {
    Target {
        handle,
        position: [1.0, 2.0, 3.0],
        mass: 80.0,
        damageable: true,
        character: Some(CharacterTarget {
            levitating: false,
            immortal: false,
            ethereal: false,
            in_event: false,
            grounded: true,
        }),
    }
}

#[test]
fn recast_and_failed_cast_retain_singleton_fields() {
    let mut q = Earthquake::default();
    let mut h = Host {
        ground: Some([1.0, 0.0, 2.0]),
        ..Default::default()
    };
    assert!(q.cast(Some(7), [1.0; 3], &mut h));
    q.new_quake([4.0; 3], 1.5, &mut h);
    q.hits.add(9);
    assert!(q.cast(Some(8), [2.0; 3], &mut h));
    assert_eq!(q.quake_position, [4.0; 3]);
    assert_eq!(q.quake_ttl, 1.5);
    assert!(q.hits.contains(9));
    h.ground = None;
    assert!(!q.cast(Some(10), [0.0; 3], &mut h));
    assert!(
        q.registered,
        "failed recast leaves the prior effect registered"
    );
    assert_eq!(q.owner, None);
    assert_eq!(q.ttl, 16.0);
    assert_eq!(h.failures, 1);
    q.remove(&mut h);
    q.remove(&mut h);
    assert_eq!(h.stops, 1);
}

#[test]
fn pulses_filter_characters_and_apply_props_after_exact_random_draws() {
    let mut h = Host {
        ground: Some([0.0; 3]),
        ..Default::default()
    };
    h.targets.push(grounded(1));
    for id in 2..=7 {
        let mut t = grounded(id);
        let c = t.character.as_mut().unwrap();
        match id {
            2 => c.levitating = true,
            3 => t.mass = 3000.0,
            4 => c.immortal = true,
            5 => c.ethereal = true,
            6 => c.in_event = true,
            _ => c.grounded = false,
        }
        h.targets.push(t);
    }
    let mut prop = grounded(8);
    prop.character = None;
    prop.damageable = false;
    h.targets.push(prop);
    let mut q = Earthquake::default();
    q.cast(Some(1), [0.0; 3], &mut h);
    q.update(0.5, &mut h);
    assert_eq!(
        h.draws, 7,
        "three center draws, two hit-point draws, two prop draws"
    );
    assert_eq!(h.impulses, [(1, [0.0, 6.0, 0.0])]);
    assert_eq!(h.damage.len(), 1);
    let (id, owner, d, timestamp, p) = h.damage[0];
    assert_eq!(
        (id, owner, d.amount, d.magnitude, timestamp),
        (1, Some(1), 80.0, 1.0, 0.0)
    );
    assert_eq!(p, [0.75, 2.0, 2.75]);
    assert_eq!(h.velocities, [(8, [-0.25, 5.0, -0.25])]);
    assert_eq!(q.hits.remaining(1), Some(0.25));
    assert!(!q.hits.contains(2));
    assert_eq!(h.centers[0].1, 1.25);
    assert!((h.centers[0].0[2] - 7.5).abs() < 1e-5);
    q.update(1.0, &mut h);
    assert_eq!(h.pulses, 2, "no catch-up pulses for a long frame");
}

#[test]
fn client_uses_received_center_and_expiry_keeps_final_pulse() {
    let mut q = Earthquake::default();
    let mut h = Host {
        client: true,
        ground: Some([0.0; 3]),
        ..Default::default()
    };
    q.cast(None, [0.0; 3], &mut h);
    q.new_quake([8.0, 0.5, 9.0], 2.0, &mut h);
    q.update(16.0, &mut h);
    assert_eq!(h.draws, 0);
    assert_eq!(h.pulses, 1);
    assert_eq!(h.stops, 1);
    assert!(!q.registered);
    q.update(1.0, &mut h);
    assert_eq!(h.pulses, 1);
}
