//! Campaign-spawned boss capture, pinned after its authored emerge finishes.
use super::*;
use magicka_runtime::{
    boss_action::{BossAction, BossCommand},
    script_runtime::ScriptWorld,
};
pub(super) const SCRIPT: &[Step] = &[
    (1, [0., 0.], Act::BossStart, "spawn"),
    (1800, [0., 0.], Act::BossWait, "risen"),
    (30, [0., 0.], Act::BossVisible, "boss_visible"),
    (30, [0., 0.], Act::BossHidden, "boss_hidden"),
    (30, [0., 0.], Act::BossVisible, "boss_restored"),
];
pub(super) const MOTION_SCRIPT: &[Step] = &[
    (1, [0., 0.], Act::BossStart, "spawn"),
    (120, [0., 0.], Act::BossWait, "assets_settled"),
    (30, [0., 0.], Act::BossMotion(0), "leap_warning"),
    (30, [0., 0.], Act::BossMotion(1), "leap_early"),
    (30, [0., 0.], Act::BossMotion(2), "leap_late"),
    (30, [0., 0.], Act::BossMotion(3), "death_early"),
    (30, [0., 0.], Act::BossMotion(4), "death_late"),
];
pub(super) const DEATH_AFTERIMAGES_SCRIPT: &[Step] = &[
    (1, [0., 0.], Act::DeathAfterimages(0), "moving_pinned"),
    (60, [0., 0.], Act::DeathAfterimages(1), "history_visible"),
    (30, [0., 0.], Act::DeathAfterimages(2), "history_hidden"),
    (30, [0., 0.], Act::DeathAfterimages(3), "history_restored"),
];
pub(super) const GRIMNIR_SCRIPT: &[Step] = &[
    (1, [0., 0.], Act::Grimnir(0), "first_wave"),
    (60, [0., 0.], Act::Grimnir(1), "apparition_visible"),
    (30, [0., 0.], Act::Grimnir(2), "apparition_hidden"),
    (30, [0., 0.], Act::Grimnir(3), "apparition_restored"),
    (60, [0., 0.], Act::Grimnir(4), "second_venue"),
    (30, [0., 0.], Act::Grimnir(5), "second_hidden"),
    (30, [0., 0.], Act::Grimnir(6), "second_restored"),
];
pub(super) const GRIMNIR2_SCRIPT: &[Step] = &[
    (1, [0., 0.], Act::Grimnir2(0), "physical_start"),
    (540, [0., 0.], Act::Idle, "battle"),
    (30, [0., 0.], Act::Grimnir2(1), "body_and_spirit"),
    (30, [0., 0.], Act::Grimnir2(2), "hidden"),
    (30, [0., 0.], Act::Grimnir2(3), "restored"),
    (300, [0., 0.], Act::Grimnir2(4), "assatur_heals"),
    (30, [0., 0.], Act::Grimnir2(5), "assatur_pinned"),
];
#[derive(Resource, Default)]
pub(super) struct Audit {
    pub(super) frozen: bool,
    motion_phase: Option<u8>,
    warlord_phase: Option<Act>,
    motion_camera: Option<[f32; 3]>,
    death_pinned: bool,
    pub(super) grimnir_hidden: bool,
    pub(super) grimnir2_hidden: bool,
    grimnir2_phase: Option<u8>,
    grimnir_phase: Option<u8>,
}
impl Audit {
    pub(super) fn prepare(&mut self, s: &mut PlaySession, act: Option<Act>) -> bool {
        if matches!(act, Some(Act::VladAfterimages(1..))) {
            return true;
        }
        if let Some(Act::Grimnir2(phase)) = act {
            use magicka_runtime::scene_world::boss_live::BossRef;
            let changed = self.grimnir2_phase != Some(phase);
            self.grimnir2_phase = Some(phase);
            self.grimnir2_hidden = phase == 2;
            let host = s
                .world
                .boss_fight
                .bosses
                .iter_mut()
                .find_map(|b| {
                    if let BossRef::Grimnir2(g) = b {
                        Some(g)
                    } else {
                        None
                    }
                })
                .expect("authored physical Grimnir boss");
            assert_eq!(host.actor.status.max_hit_points, 35000.);
            if changed && phase == 0 {
                // Exercise the native summoning branch without a long capture.
                host.actor.spirit_timer = 50.;
                s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
            }
            if changed && phase == 4 {
                let h = host.zones[0];
                s.world
                    .damage_physics(
                        h,
                        magicka_sim::damage::Damage {
                            attack: magicka_sim::damage::attack::DAMAGE,
                            element: magicka_sim::spells::Elements::ARCANE.0,
                            amount: 100000.,
                            magnitude: 1.,
                        },
                        15,
                    )
                    .unwrap();
            }
            if changed {
                eprintln!(
                    "Grimnir2 capture phase={phase} hp={} state={:?}",
                    s.world.boss_fight.normalized_health,
                    s.world.boss_fight.bosses.iter().find_map(
                        |b| if let BossRef::Grimnir2(g) = b {
                            Some(g.actor.state)
                        } else {
                            None
                        }
                    )
                );
            }
            return matches!(phase, 1 | 2 | 3 | 5);
        }
        if let Some(Act::Grimnir(phase)) = act {
            return self.grimnir(s, phase);
        }
        if let Some(Act::DeathAfterimages(phase)) = act {
            return self.death_history(s, phase);
        }
        if let Some(frozen) = warlord_qa::prepare(s, act, &mut self.warlord_phase) {
            return frozen;
        }
        if let Some(Act::BossMotion(phase)) = act {
            return self.motion(s, phase);
        }
        if act == Some(Act::BossStart) {
            s.script_error = None;
            s.run_trigger("StartOfLevel", true)
                .expect("boss scene initial trigger");
            s.world.flags.cutscene = false;
            s.world.flags.input_limited = false;
            s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
            let locator = magicka_runtime::ai_event::AiEventLevel::locator(
                &s.world,
                magicka_runtime::scene_loading::custom_hash("boss0"),
            )
            .expect("authored boss0 locator");
            let p = s.world.players[0].as_mut().unwrap();
            p.ai.set_order(0, None);
            let body = &mut s.world.physics.bodies[p.body];
            body.position = [
                locator[12],
                locator[13] + body.radius + body.length * 0.5 + 0.1,
                locator[14],
            ];
            body.velocity = [0.; 3];
            s.step(0.);
            s.disable_scripts();
            // The authored trigger queued AvatarMove during that script tick.
            // Keep this isolated capture at its actual arena spawn afterward.
            let p = s.world.players[0].as_mut().unwrap();
            p.ai.set_order(0, None);
            assert_eq!(
                s.world.jormungandr.len(),
                1,
                "authored spawn must execute before visual controls"
            );
            s.world
                .boss_action(
                    &BossAction {
                        delay: 0.,
                        command: BossCommand::Activate,
                    },
                    false,
                )
                .unwrap()
                .unwrap();
            s.world.flags.cutscene = false;
            s.world.flags.input_limited = false;
        }
        if !matches!(
            act,
            Some(Act::BossStart | Act::BossWait | Act::BossVisible | Act::BossHidden)
        ) {
            return false;
        }
        let Some(boss) = s.world.jormungandr.values_mut().next() else {
            return false;
        };
        if act == Some(Act::BossWait)
            && !self.frozen
            && boss.state == magicka_runtime::jormungandr::State::Risen
            && boss.update_animation
            && (boss.controller.clip_name() == Some("idle")
                || (boss.controller.clip_name() == Some("emerge")
                    && boss.controller.time / boss.controller.clip_duration() >= 0.8))
            && !boss.controller.crossfade_enabled()
        {
            self.frozen = true;
            eprintln!(
                "boss pinned: root={:?} head={:?} health={} spine={}",
                boss.position,
                boss.frame.head,
                boss.health.hp,
                boss.frame.spine.len()
            );
        }
        if self.frozen {
            boss.frame.visible = act != Some(Act::BossHidden);
            let mut target = boss.position;
            target[1] += 2.;
            s.world.camera.move_to(target, 0.);
            s.world.camera.set_position(target, false);
            s.world.camera.set_magnification(1.0);
        }
        if matches!(act, Some(Act::BossVisible | Act::BossHidden)) && !self.frozen {
            panic!(
                "boss never rose: state={:?} clip={:?} time={} root={:?} head={:?}",
                boss.state,
                boss.controller.clip_name(),
                boss.controller.time,
                boss.position,
                boss.frame.head
            );
        }
        self.frozen
    }
    fn grimnir(&mut self, s: &mut PlaySession, phase: u8) -> bool {
        use magicka_runtime::{entity_command::EntityCommandHost, scene_world::boss_live::BossRef};
        if self.grimnir_phase == Some(phase) {
            return true;
        }
        if phase == 0 {
            s.change_scene_named("ew_s4", "starta").unwrap();
        }
        s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
        if phase == 0 || phase == 4 {
            let venue = usize::from(phase == 4);
            if venue == 1 {
                let handles = s
                    .world
                    .boss_fight
                    .bosses
                    .iter()
                    .find_map(|b| {
                        if let BossRef::Grimnir(g) = b {
                            Some(g.minions.clone())
                        } else {
                            None
                        }
                    })
                    .expect("first dream");
                for h in handles {
                    EntityCommandHost::kill(&mut s.world, h).unwrap();
                }
            }
            let mut ready = false;
            for _ in 0..600 {
                s.step(FIXED_DT);
                assert!(s.script_error.is_none(), "{:?}", s.script_error);
                ready = s.world.boss_fight.bosses.iter().any(|b| {
                    if let BossRef::Grimnir(g) = b {
                        assert!(g.failed.is_none(), "{:?}", g.failed);
                        g.dreams.venue == venue
                            && g.minions.len() == magicka_runtime::boss_grimnir::COUNTS[venue]
                    } else {
                        false
                    }
                });
                if ready {
                    break;
                }
            }
            assert!(ready, "Grimnir venue {venue} did not spawn");
            for b in &s.world.boss_fight.bosses {
                if let BossRef::Grimnir(g) = b {
                    eprintln!(
                        "Grimnir pinned venue={} health={} minions={:?} parent={:?} player={:?}",
                        g.dreams.venue,
                        g.dreams.health,
                        g.minions,
                        g.pose.parent,
                        s.body().position
                    );
                }
            }
        }
        self.grimnir_hidden = phase == 2 || phase == 5;
        self.grimnir_phase = Some(phase);
        true
    }
    fn death_history(&mut self, s: &mut PlaySession, phase: u8) -> bool {
        use magicka_runtime::{boss_death::State, scene_world::boss_live::BossRef};
        if !self.death_pinned {
            assert_eq!(phase, 0);
            s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
            // Follow the authored intro, including StartOfScene's AvatarMove.
            for _ in 0..60 {
                s.step(FIXED_DT);
            }
            s.run_trigger("Move to the throne", true).unwrap();
            for tick in 0..7200 {
                if tick % 30 == 0 {
                    s.world.advance_dialog(0);
                }
                s.step(FIXED_DT);
                assert!(s.script_error.is_none(), "{:?}", s.script_error);
                let ready = s.world.boss_fight.bosses.iter().any(|b| {
                    let BossRef::Death(h) = b else { return false };
                    let d = h.actor.lock().unwrap();
                    d.state == State::Move
                        && d.afterimages.drawn
                        && d.afterimages.poses.iter().all(Option::is_some)
                        && d.afterimages.poses.windows(2).all(|pair| {
                            let a = &pair[0].as_ref().unwrap().parent;
                            let b = &pair[1].as_ref().unwrap().parent;
                            (a[12] - b[12]).powi(2) + (a[14] - b[14]).powi(2) < 1.
                        })
                        && {
                            let newest = &d.afterimages.poses[0].as_ref().unwrap().parent;
                            let oldest = &d.afterimages.poses[4].as_ref().unwrap().parent;
                            (newest[12] - oldest[12]).powi(2) + (newest[14] - oldest[14]).powi(2)
                                > 0.25
                        }
                });
                if ready {
                    self.death_pinned = true;
                    break;
                }
            }
            assert!(
                self.death_pinned,
                "Death never produced a natural moving history"
            );
        }
        let actor = s
            .world
            .boss_fight
            .bosses
            .iter()
            .find_map(|b| {
                if let BossRef::Death(h) = b {
                    Some(h.actor.clone())
                } else {
                    None
                }
            })
            .unwrap();
        let mut d = actor.lock().unwrap();
        d.afterimages.drawn = phase != 2;
        let mut target = [d.orientation[12], d.orientation[13] + 1., d.orientation[14]];
        // Include the retained trail rather than centring on the player.
        target[0] = (target[0] + d.afterimages.poses[4].as_ref().unwrap().parent[12]) * 0.5;
        target[2] = (target[2] + d.afterimages.poses[4].as_ref().unwrap().parent[14]) * 0.5;
        s.world.camera.move_to(target, 0.);
        s.world.camera.set_position(target, false);
        s.world.camera.set_magnification(0.8);
        if phase == 0 {
            eprintln!(
                "Death history pinned state={:?} hp={} ethereal={} alphas={:?} parents={:?}",
                d.state,
                d.combat.hit_points(),
                d.ethereal,
                d.afterimages.draw_alphas(),
                d.afterimages
                    .poses
                    .iter()
                    .map(|p| &p.as_ref().unwrap().parent[12..15])
                    .collect::<Vec<_>>()
            );
        }
        true
    }
    fn motion(&mut self, s: &mut PlaySession, phase: u8) -> bool {
        use magicka_runtime::jormungandr::State;
        if self.motion_phase != Some(phase) {
            let handle = *s.world.jormungandr.keys().next().expect("motion boss");
            if self.motion_camera.is_none() {
                let mut at = s.body().position;
                at[1] += 1.;
                self.motion_camera = Some(at);
            }
            if phase == 0 || phase == 3 {
                for _ in 0..3600 {
                    let boss = s.world.jormungandr.get_mut(&handle).unwrap();
                    boss.health.hp = if phase == 0 { 1000. } else { 10000. };
                    let ready = if phase == 0 {
                        boss.state == State::Leap && !boss.update_animation
                    } else {
                        boss.state == State::Risen
                            && boss.controller.clip_name() == Some("idle")
                            && !boss.controller.crossfade_enabled()
                    };
                    if ready {
                        break;
                    }
                    s.step(1. / 60.);
                    s.handle_input(magicka_runtime::avatar::PlayerInput::default(), 16. / 9.);
                }
                let boss = s.world.jormungandr.get_mut(&handle).unwrap();
                if phase == 0 {
                    assert_eq!(
                        boss.state,
                        State::Leap,
                        "motion fixture must select a real leap"
                    );
                } else {
                    assert_eq!(boss.state, State::Risen);
                    assert_eq!(boss.controller.clip_name(), Some("idle"));
                    boss.health.hp = 0.;
                }
            }
            let fraction = match phase {
                0 => None,
                1 => Some(0.25),
                2 => Some(0.85),
                3 => Some(0.35),
                4 => Some(0.95),
                _ => unreachable!(),
            };
            if let Some(fraction) = fraction {
                for _ in 0..600 {
                    let boss = &s.world.jormungandr[&handle];
                    let expected = if phase < 3 { State::Leap } else { State::Dead };
                    if boss.state == expected
                        && boss.controller.time / boss.controller.clip_duration() >= fraction
                    {
                        break;
                    }
                    s.step(1. / 60.);
                }
                let boss = &s.world.jormungandr[&handle];
                assert_eq!(
                    boss.state,
                    if phase < 3 { State::Leap } else { State::Dead }
                );
                assert!(boss.controller.time / boss.controller.clip_duration() >= fraction);
            } else {
                // Give the authored warning emitters time to produce particles.
                for _ in 0..10 {
                    s.step(1. / 60.);
                }
            }
            let boss = &s.world.jormungandr[&handle];
            eprintln!("boss motion phase={phase} state={:?} clip={:?} fraction={} head={:?} root={:?} visible={} effects={:?} live_effects={}",
                boss.state, boss.controller.clip_name(), boss.controller.time / boss.controller.clip_duration(),
                boss.frame.head, &boss.orientation[12..15], boss.frame.visible,
                boss.ground_effects, s.effects.active_effects());
            self.motion_phase = Some(phase);
        }
        let target = self.motion_camera.unwrap();
        s.world.camera.move_to(target, 0.);
        s.world.camera.set_position(target, false);
        s.world.camera.set_magnification(0.8);
        self.frozen = true;
        true
    }
}
