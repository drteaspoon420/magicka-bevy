use magicka_sim::{
    damage::Damage,
    force_spray::*,
    spell_cast::{EffectLifecycle, EffectOwner},
    spells::{Elements, Spell},
    spray_pulse::{Segment, SpraySource, SprayTarget, SprayWorld},
};
#[derive(Default)]
struct Host {
    events: Vec<&'static str>,
    ranges: Vec<f32>,
    self_damage: Vec<Damage>,
}
impl SprayWorld for Host {
    fn scene_intersection(&mut self, s: Segment) -> Option<f32> {
        self.events.push("pulse");
        self.ranges.push(s.delta[2]);
        None
    }
    fn shields(&mut self) -> Vec<u16> {
        vec![]
    }
    fn shield_intersection(&mut self, _: u16, _: Segment) -> Option<f32> {
        panic!()
    }
    fn targets(&mut self, _: [f32; 3], _: f32) -> Vec<SprayTarget> {
        vec![]
    }
    fn arc_intersects(&mut self, _: u16, _: [f32; 3], _: [f32; 3], _: f32, _: f32, _: f32) -> bool {
        panic!()
    }
    fn damage(&mut self, _: u16, _: u16, _: [Damage; 5], _: f64, _: [f32; 3]) {
        panic!()
    }
    fn electrocute(&mut self, _: u16, _: u16, _: f32) {
        self.events.push("electrocute");
    }
}
impl ForceSprayHost for Host {
    fn initialize_force(&mut self, _: Spell, _: f32, _: SpraySource) {
        self.events.push("initialize");
    }
    fn damage_owner_single(&mut self, d: Damage, _: f64, _: SpraySource) {
        self.events.push("self_damage");
        self.self_damage.push(d);
    }
    fn rumble(&mut self, l: f32, r: f32) {
        assert_eq!((l, r), (0.8, 0.8));
        self.events.push("rumble");
    }
    fn update_force_presentation(
        &mut self,
        _: f32,
        _: f32,
        _: bool,
        _: SpraySource,
        _: [Damage; 5],
    ) {
        self.events.push("presentation_liquid");
    }
    fn deinitialize(&mut self, l: &mut EffectLifecycle) {
        self.events.push("cleanup");
        l.active = false;
    }
}
fn source() -> SpraySource {
    SpraySource {
        owner_handle: 1,
        owner_position: [0.0; 3],
        owner_direction: [0.0, 0.0, 1.0],
        cast_position: [0.0; 3],
        cast_direction: [0.0, 0.0, 1.0],
        weapon_position: [0.0; 3],
        weapon_direction: [0.0, 0.0, 1.0],
    }
}
fn facts() -> ForceCastFacts {
    ForceCastFacts {
        from_staff: true,
        character: true,
        panic_cast: false,
        greased: true,
        timestamp: 2.0,
    }
}
#[test]
fn cast_initializes_before_first_fire_self_damage_and_normalizes_staff_source() {
    let mut host = Host::default();
    let cast = ForceSpray::cast(
        Spell::default_for(Elements::FIRE),
        facts(),
        source(),
        &mut host,
    )
    .unwrap();
    assert_eq!(host.events, ["initialize", "self_damage"]);
    assert_eq!(host.self_damage.len(), 1);
    assert!(!cast.lifecycle.from_staff);
    assert_eq!(
        (
            cast.range,
            cast.ttl,
            cast.lifecycle.min_ttl,
            cast.lifecycle.max_ttl
        ),
        (6.0, 4.0, 0.5, 4.0)
    );
    assert_eq!(cast.pulse.timestamp, 2.0);
    let cast = ForceSpray::cast(
        Spell::default_for(Elements::FIRE),
        ForceCastFacts {
            panic_cast: true,
            ..facts()
        },
        source(),
        &mut host,
    )
    .unwrap();
    assert!(cast.lifecycle.from_staff);
}
#[test]
fn range_ramps_to_full_and_hit_timers_update_once_before_execution() {
    let mut host = Host::default();
    let mut cast = ForceSpray::cast(
        Spell::default_for(Elements::COLD),
        facts(),
        source(),
        &mut host,
    )
    .unwrap();
    cast.pulse.hits.add(7);
    cast.update(0.125, EffectOwner::default(), false, source(), &mut host);
    assert_eq!(cast.range_multiplier, 0.5);
    assert_eq!(cast.pulse.hits.remaining(7), Some(0.125));
    cast.update(0.125, EffectOwner::default(), false, source(), &mut host);
    assert_eq!(host.ranges, [3.0, 6.0]);
    assert!(!cast.pulse.hits.contains(7));
}
#[test]
fn final_ttl_frame_runs_presentation_before_cleanup_and_suppresses_pulse() {
    let mut host = Host::default();
    let mut cast = ForceSpray::cast(
        Spell::default_for(Elements::COLD),
        facts(),
        source(),
        &mut host,
    )
    .unwrap();
    host.events.clear();
    cast.ttl = 0.1;
    assert_eq!(
        cast.update(
            0.1,
            EffectOwner {
                avatar: true,
                ..Default::default()
            },
            true,
            source(),
            &mut host
        ),
        (false, 0.6)
    );
    assert_eq!(host.events, ["rumble", "presentation_liquid", "cleanup"]);
}
#[test]
fn release_is_checked_after_pulse_and_shared_max_expiry_waits_one_update() {
    let mut host = Host::default();
    let mut cast = ForceSpray::cast(
        Spell::default_for(Elements::COLD),
        facts(),
        source(),
        &mut host,
    )
    .unwrap();
    host.events.clear();
    assert_eq!(
        cast.update(
            0.5,
            EffectOwner {
                avatar: true,
                button_none: true,
                ..Default::default()
            },
            false,
            source(),
            &mut host
        ),
        (false, 0.6)
    );
    assert_eq!(host.events, ["pulse", "presentation_liquid", "cleanup"]);
    let mut cast = ForceSpray::cast(
        Spell::default_for(Elements::COLD),
        facts(),
        source(),
        &mut host,
    )
    .unwrap();
    cast.lifecycle.max_ttl = 0.1;
    assert!(
        cast.update(0.1, EffectOwner::default(), false, source(), &mut host)
            .0
    );
    assert!(
        !cast
            .update(0.0, EffectOwner::default(), false, source(), &mut host)
            .0
    );
}
