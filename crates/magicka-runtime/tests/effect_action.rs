use magicka_runtime::{
    animation_action::AnimationAction, effect_action::*, level_variables::LevelVariables,
    scene_loading::custom_hash, scene_script::SceneScriptLayout, script_runtime::*,
};
use magicka_sim::damage_delivery::NetworkRole;

#[derive(Clone, Debug, PartialEq)]
struct Fx {
    kind: i32,
    active: bool,
    transform: [f32; 16],
    starts: u32,
    updates: u32,
}
#[derive(Default)]
struct Backend {
    ops: Vec<String>,
}
impl EffectBackend for Backend {
    type Error = &'static str;
    type Effect = Fx;
    fn get_effect(&mut self, effect: i32) -> Result<Fx, Self::Error> {
        Ok(Fx {
            kind: effect,
            active: false,
            transform: [0.0; 16],
            starts: 0,
            updates: 0,
        })
    }
    fn is_active(&self, e: &Fx) -> bool {
        e.active
    }
    fn start(&mut self, e: &mut Fx, t: [f32; 16]) -> Result<(), Self::Error> {
        e.active = true;
        e.transform = t;
        e.starts += 1;
        self.ops.push(format!("start {}", t[12]));
        Ok(())
    }
    fn stop(&mut self, e: &mut Fx) -> Result<(), Self::Error> {
        e.active = false;
        self.ops.push("stop".into());
        Ok(())
    }
    fn transform(&self, e: &Fx) -> [f32; 16] {
        e.transform
    }
    fn set_transform(&mut self, e: &mut Fx, t: [f32; 16]) {
        e.transform = t;
    }
    fn update(&mut self, e: &mut Fx, _: f32) -> Result<(), Self::Error> {
        e.updates += 1;
        Ok(())
    }
}
fn locator(x: f32) -> [f32; 16] {
    let mut m = [0.0; 16];
    m[0] = 1.0;
    m[5] = 1.0;
    m[10] = 1.0;
    m[15] = 1.0;
    m[12] = x;
    m
}

/// A world host built on the reconstructed registry.
#[derive(Default)]
struct Host {
    effects: SceneEffects<Fx>,
    backend: Backend,
    single: Vec<i32>,
}
impl EffectActionHost for Host {
    type Error = &'static str;
    fn locator(&mut self, area: i32) -> Result<[f32; 16], Self::Error> {
        (area == custom_hash("altar"))
            .then(|| locator(5.0))
            .ok_or("No area")
    }
    fn effect_is_single(&mut self, effect: i32) -> Result<bool, Self::Error> {
        Ok(self.single.contains(&effect))
    }
    fn add_effect(
        &mut self,
        id: i32,
        effect: i32,
        t: [f32; 16],
        range: f32,
    ) -> Result<(), Self::Error> {
        self.effects.add(&mut self.backend, id, effect, t, range)
    }
    fn stop_effect(&mut self, id: i32) -> Result<bool, Self::Error> {
        self.effects.stop(&mut self.backend, id)
    }
    fn start_effect(&mut self, id: i32) -> Result<(), Self::Error> {
        self.effects.start(&mut self.backend, id)
    }
    fn generated_effect_id(&mut self) -> Result<i32, Self::Error> {
        Ok(self.effects.generated_id())
    }
}
fn read(tag: &str, attrs: &[(&str, &str)]) -> EffectAction {
    EffectAction::read(tag, attrs.iter().copied()).unwrap()
}

#[test]
fn decoding_defaults() {
    assert_eq!(
        read(
            "SpawnEffect",
            &[
                ("effect", "Fire"),
                ("name", "F1"),
                ("area", "Altar"),
                ("range", "3"),
                ("delay", "1")
            ]
        ),
        EffectAction {
            delay: 1.0,
            kind: EffectActionKind::Spawn {
                effect: custom_hash("fire"),
                name: custom_hash("f1"),
                area: custom_hash("altar"),
                range: 3.0
            }
        }
    );
    assert_eq!(
        read("startEffect", &[("name", "")]).kind,
        EffectActionKind::Start { name: None }
    );
    assert_eq!(
        read("stopeffect", &[]).kind,
        EffectActionKind::Stop { name: 0 }
    );
    assert!(EffectAction::read("spawneffect", [("range", "far")]).is_err());
}

#[test]
fn registry_add_stop_start_and_generated_id_quirk() {
    let mut host = Host::default();
    let spawn = read(
        "spawneffect",
        &[
            ("effect", "fire"),
            ("name", "f1"),
            ("area", "altar"),
            ("range", "2"),
        ],
    );
    spawn.execute(&mut host).unwrap();
    // Active effects are not replaced.
    read(
        "spawneffect",
        &[("effect", "smoke"), ("name", "f1"), ("area", "altar")],
    )
    .execute(&mut host)
    .unwrap();
    let id = custom_hash("f1");
    assert_eq!(host.effects.effects[&id].effect.kind, custom_hash("fire"));
    read("stopeffect", &[("name", "f1")])
        .execute(&mut host)
        .unwrap();
    read("stopeffect", &[("name", "missing")])
        .execute(&mut host)
        .unwrap();
    read("starteffect", &[("name", "f1")])
        .execute(&mut host)
        .unwrap();
    assert_eq!(host.effects.effects[&id].effect.starts, 2);
    // Inactive effects are replaced by a new spawn with a cleared animation link.
    host.effects.effects.get_mut(&id).unwrap().animation = Some(9);
    read("stopeffect", &[("name", "f1")])
        .execute(&mut host)
        .unwrap();
    read(
        "spawneffect",
        &[("effect", "smoke"), ("name", "f1"), ("area", "altar")],
    )
    .execute(&mut host)
    .unwrap();
    let stored = &host.effects.effects[&id];
    assert_eq!(
        (stored.effect.kind, stored.animation, stored.range),
        (custom_hash("smoke"), None, 0.0)
    );
    assert_eq!(
        host.backend.ops,
        ["start 5", "stop", "start 5", "stop", "start 5"]
    );

    // Unnamed StartEffect: generated IDs skip occupied keys, so nothing starts.
    host.effects
        .effects
        .insert(1, host.effects.effects[&id].clone());
    host.effects.effects.get_mut(&1).unwrap().effect.active = false;
    read("starteffect", &[]).execute(&mut host).unwrap();
    assert_eq!(host.effects.last_generated, 2);
    assert!(!host.effects.effects[&1].effect.active);
    host.effects.last_generated = -2;
    host.effects
        .effects
        .insert(-1, host.effects.effects[&1].clone());
    host.effects
        .effects
        .insert(0, host.effects.effects[&1].clone());
    // The scan stops at zero even when zero is occupied, which can start it.
    read("starteffect", &[]).execute(&mut host).unwrap();
    assert_eq!(host.effects.last_generated, 0);
    assert!(host.effects.effects[&0].effect.active);

    assert_eq!(
        read("spawneffect", &[("area", "nowhere")]).execute(&mut host),
        Err("No area")
    );
}

#[test]
fn quick_execute_skips_single_effects() {
    let mut host = Host {
        single: vec![custom_hash("flash")],
        ..Default::default()
    };
    read(
        "spawneffect",
        &[("effect", "flash"), ("name", "a"), ("area", "altar")],
    )
    .quick_execute(&mut host)
    .unwrap();
    assert!(host.effects.effects.is_empty());
    read(
        "spawneffect",
        &[("effect", "fire"), ("name", "a"), ("area", "altar")],
    )
    .quick_execute(&mut host)
    .unwrap();
    read("stopeffect", &[("name", "a")])
        .quick_execute(&mut host)
        .unwrap();
    read("starteffect", &[("name", "a")])
        .quick_execute(&mut host)
        .unwrap();
    assert_eq!(host.backend.ops, ["start 5", "stop", "start 5"]);
}

#[test]
fn update_applies_animation_and_range_clipping() {
    let mut effects = SceneEffects::default();
    let mut b = Backend::default();
    let mut forward = locator(1.0);
    // XNA Forward is -row 3; use a +Z third row so Forward = (0,0,-1).
    forward[10] = 1.0;
    effects.add(&mut b, 1, 0, forward, 4.0).unwrap();
    effects.add(&mut b, 2, 0, locator(2.0), 0.0).unwrap();
    effects.add(&mut b, 3, 0, locator(3.0), 0.0).unwrap();
    effects.effects.get_mut(&2).unwrap().animation = Some(7);
    effects.stop(&mut b, 3).unwrap();
    let mut segments = vec![];
    effects
        .update(
            &mut b,
            0.1,
            |part| {
                assert_eq!(part, 7);
                locator(10.0)
            },
            |origin, delta| {
                segments.push((origin, delta));
                0.25
            },
        )
        .unwrap();
    assert_eq!(segments, [([1.0, 0.0, 0.0], [-0.0, -0.0, -4.0])]);
    let clipped = effects.effects[&1].effect.transform;
    assert_eq!(
        (clipped[0], clipped[10], clipped[12], clipped[15]),
        (0.25, 0.25, 1.0, 1.0)
    );
    // Animated transform = stored * absolute: translations add here.
    assert_eq!(effects.effects[&2].effect.transform[12], 12.0);
    assert_eq!(effects.effects[&3].effect.updates, 0);
    assert_eq!(effects.effects[&1].effect.updates, 1);
}

struct World {
    host: Option<Host>,
}
impl ScriptWorld for World {
    type Error = &'static str;
    fn facts(&self) -> ScriptWorldFacts {
        ScriptWorldFacts::default()
    }
    fn effect_host(&mut self) -> Option<&mut dyn EffectActionHost<Error = Self::Error>> {
        self.host.as_mut().map(|h| h as _)
    }
    fn choose_group(&mut self, _: usize) -> Result<usize, Self::Error> {
        Ok(0)
    }
    fn notify_trigger(
        &mut self,
        _: &mut ScriptRuntime,
        _: i32,
        _: usize,
        _: Option<u16>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn animation(&mut self, _: &mut ScriptRuntime, _: &AnimationAction) -> Result<(), Self::Error> {
        Ok(())
    }
    fn action_executed(&mut self, _: &mut ScriptRuntime, _: usize) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[test]
fn runtime_dispatch_and_failure_mapping() {
    let layout = SceneScriptLayout::read(
        br#"<Scene><trigger id="t" autorun="false"><then><spawnEffect effect="fire" name="f" area="altar"/><spawnEffect area="bad"/></then></trigger></Scene>"#,
        false,
    )
    .unwrap();
    let mut rt = ScriptRuntime::new(layout, LevelVariables::default());
    let mut world = World {
        host: Some(Host::default()),
    };
    rt.execute_trigger(
        custom_hash("t"),
        None,
        false,
        NetworkRole::Server,
        &mut world,
    )
    .unwrap();
    rt.update_action(0, 0.0, NetworkRole::Server, &mut world)
        .unwrap();
    assert_eq!(
        rt.update_action(1, 0.0, NetworkRole::Server, &mut world),
        Err(ScriptRuntimeError::World("No area"))
    );
    assert_eq!(rt.history(), [0]);
    rt.quick_execute_effect(0, &mut world).unwrap();
    assert_eq!(
        rt.quick_execute_effect(5, &mut world),
        Err(ScriptRuntimeError::InvalidAction(5))
    );
}
