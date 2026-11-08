use super::*;
use crate::{missiles::LiveMissile, play_session::PlaySession};
use magicka_sim::{
    projectile_launch::{ForceProjectile, LaunchSource, ProjectileLaunch, ProjectileLaunchHost},
    spell_cast::EffectOwner,
    spells::Elements as E,
};
const DT: f32 = 1. / 60.;
#[derive(Default)]
struct Launches(Vec<ProjectileLaunch>);
impl ProjectileLaunchHost for Launches {
    fn shields(&mut self) -> Vec<u16> {
        Vec::new()
    }
    fn shield_intersection(&mut self, _: u16, _: Segment, _: f32) -> Option<[f32; 3]> {
        None
    }
    fn blocked_effect(&mut self, _: E, _: [f32; 3], _: [f32; 3]) {}
    fn next_double(&mut self) -> f64 {
        0.5
    }
    fn spawn(&mut self, launch: ProjectileLaunch) {
        self.0.push(launch);
    }
}
fn session(level: &str) -> Option<PlaySession> {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, level, "Wizard").unwrap();
    s.disable_scripts();
    Some(s)
}
#[test]
fn native_self_cast_rock_and_ice_rock_hit_sampled_head_before_caster() {
    for spell in [
        Spell::default_for_list(&[E::EARTH]),
        Spell::default_for_list(&[E::EARTH, E::ICE]),
    ] {
        let Some(mut s) = session("Mountaindale.lvl") else {
            return;
        };
        s.change_scene_named("md_s6BF", "start").unwrap();
        s.disable_scripts();
        s.world
            .boss_action(
                &crate::boss_action::BossAction {
                    delay: 0.,
                    command: crate::boss_action::BossCommand::Spawn {
                        boss: "jormungandr".into(),
                        prop: false,
                        generic: false,
                        area: custom_hash("boss0"),
                        unique_id: 0,
                        mesh: -1,
                        on_death: 0,
                        on_damage: 0,
                        setup: Default::default(),
                    },
                },
                false,
            )
            .unwrap()
            .unwrap();
        let handle = *s.world.jormungandr.keys().next().unwrap();
        let b = s.world.jormungandr.get_mut(&handle).unwrap();
        let target = b.position;
        for _ in 0..2400 {
            b.update(DT, true, false, Some(target), false, |p, angle| {
                Some([p[0] + angle.cos() * 14., p[1], p[2] + angle.sin() * 14.])
            });
            if b.state == crate::jormungandr::State::Risen
                && b.controller.clip_name() == Some("emerge")
                && b.controller.time / b.controller.clip_duration() >= 0.8
            {
                break;
            }
        }
        assert!(b.vulnerable() && b.update_animation);
        let head = b.frame.head;
        let floor = b.position[1];
        let before = b.health.hp;
        let p = s.world.players[0].as_mut().unwrap();
        p.character.immortal_time = 0.;
        p.character.set_ethereal(false);
        let player_hp = p.character.combat.health.hit_points;
        let body = &mut s.world.physics.bodies[p.body];
        body.position = [
            head[0],
            floor + body.radius + body.length * 0.5 + 0.1,
            head[2],
        ];
        let source = LaunchSource {
            owner_position: body.position,
            owner_direction: [0., 0., -1.],
            cast_position: body.position,
            may_spawn: true,
        };
        let mut launches = Launches::default();
        let mut cast = ForceProjectile::cast_self(spell, &mut 0., false, 0.).unwrap();
        for _ in 0..60 {
            cast.update(
                DT,
                EffectOwner {
                    avatar: true,
                    ..Default::default()
                },
                source,
                &mut launches,
            );
            if !launches.0.is_empty() {
                break;
            }
        }
        assert_eq!(launches.0.len(), 1);
        assert!(launches.0[0].position[1] > head[1]);
        let mut missile = LiveMissile::spawn(900, p.handle, true, &launches.0[0]).unwrap();
        missile.handle = Some(s.world.new_handle());
        s.world.missiles.push(missile);
        for _ in 0..90 {
            s.world.missile_physics(DT);
        }
        let boss = &s.world.jormungandr[&handle];
        assert!(
            boss.health.hp < before,
            "falling {:?} missed head {:?}",
            spell.element,
            head
        );
        assert_eq!(s.avatar().character.combat.health.hit_points, player_hp);
        let missile = &s.world.missiles[0];
        assert!(missile.body.position[1] > head[1]);
        assert!(missile.state.hits.contains(handle));
    }
}
#[test]
fn incorporeal_character_does_not_select_contact_or_displace_rock() {
    let Some(mut s) = session("Havindr.lvl") else {
        return;
    };
    let p = s.world.players[0].as_mut().unwrap();
    p.character.set_ethereal(true);
    let body = &mut s.world.physics.bodies[p.body];
    body.position[1] += 40.;
    let start = [body.position[0] + 0.01, body.position[1], body.position[2]];
    let owner = p.handle;
    let hp = p.character.combat.health.hit_points;
    let launch = ProjectileLaunch {
        position: start,
        velocity: [0., -25., 0.],
        spell: Spell::default_for(E::EARTH),
        splash: 0.,
        total_projectiles: 1,
    };
    s.world
        .missiles
        .push(LiveMissile::spawn(901, 1234, false, &launch).unwrap());
    s.world.missile_physics(DT);
    let missile = &s.world.missiles[0];
    assert_eq!(missile.body.position[0], start[0]);
    assert_eq!(missile.body.position[2], start[2]);
    assert!(missile.body.position[1] < start[1]);
    assert!(missile.body.velocity[1] < -25.);
    assert!(!missile.state.hits.contains(owner));
    assert_eq!(s.avatar().character.combat.health.hit_points, hp);
}

#[test]
fn ghost_template_is_incorporeal_and_projection_keeps_native_appearance_flag() {
    let Some(mut s) = session("Havindr.lvl") else {
        return;
    };
    let ghost = s.spawn_npc("Ghost_Acolyte", "idle", 3.).unwrap();
    let projection = s.spawn_npc("Boss_Vlad_Spirit", "idle", 6.).unwrap();
    let spirit = &s.world.npcs[&projection].character;
    assert!(!spirit.ethereal && spirit.definition.looks_ethereal);
    let n = s.world.npcs.get_mut(&ghost).unwrap();
    assert!(n.character.ethereal && n.character.definition.looks_ethereal);
    let body = &mut s.world.physics.bodies[n.body];
    body.position[1] += 40.;
    let start = [body.position[0] + 0.01, body.position[1], body.position[2]];
    let hp = n.character.combat.health.hit_points;
    s.world.missiles.push(
        LiveMissile::spawn(
            902,
            1234,
            false,
            &ProjectileLaunch {
                position: start,
                velocity: [0., -25., 0.],
                spell: Spell::default_for(E::EARTH),
                splash: 0.,
                total_projectiles: 1,
            },
        )
        .unwrap(),
    );
    s.world.missile_physics(DT);
    let m = &s.world.missiles[0];
    assert_eq!(m.body.position[0], start[0]);
    assert!(m.body.velocity[1] < -25.);
    assert!(!m.state.hits.contains(ghost));
    assert_eq!(s.world.npcs[&ghost].character.combat.health.hit_points, hp);
}
