//! VladCharacter's first body primitive and four additive historical poses.
//! Native timing and evidence: research/boss-vlad.md.
use super::*;
use bevy::mesh::skinning::SkinnedMesh;
use magicka_runtime::afterimage::{ghost_alphas, AfterImageStep, AfterImages, SNAPSHOTS};
use std::collections::{HashSet, VecDeque};

#[derive(Component)]
struct GhostJoint;

struct Ghost {
    entity: Entity,
    joints: Vec<Entity>,
    material: Handle<crate::magick_visuals::AfterImageMaterial>,
}
struct Trail {
    clock: AfterImages,
    history: VecDeque<Vec<Mat4>>,
    alphas: [Option<f32>; SNAPSHOTS - 1],
    ghosts: Vec<Ghost>,
    logged: Option<u8>,
    last_time: Option<f64>,
}
#[derive(Resource, Default)]
struct Trails(HashMap<Entity, Trail>);

pub(super) fn install(app: &mut App) {
    app.init_resource::<Trails>()
        .init_play_resource::<Qa>()
        .add_session_reset(reset)
        .add_play_systems(Update, fixture.after(step_session).before(sync_camera))
        .add_play_systems(
            PostUpdate,
            sync.after(bevy::transform::TransformSystems::Propagate),
        );
}

fn reset(world: &mut World) {
    let old = world.remove_resource::<Trails>().unwrap_or_default();
    if let Some(mut materials) =
        world.get_resource_mut::<Assets<crate::magick_visuals::AfterImageMaterial>>()
    {
        for trail in old.0.values() {
            for ghost in &trail.ghosts {
                materials.remove(ghost.material.id());
            }
        }
    }
    world.insert_resource(Trails::default());
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync(
    mut commands: Commands,
    play: Res<Play>,
    mut trails: ResMut<Trails>,
    server: Res<AssetServer>,
    roots: Query<(Entity, &NpcRoot, &WorldAssetRoot)>,
    parents: Query<&ChildOf>,
    sources: Query<
        (
            Entity,
            &Mesh3d,
            &SkinnedMesh,
            Option<&MeshMaterial3d<crate::lighting::PlayerLitMaterial>>,
            Option<&MeshMaterial3d<crate::lighting::LitMaterial>>,
        ),
        Or<(
            With<MeshMaterial3d<crate::lighting::PlayerLitMaterial>>,
            With<MeshMaterial3d<crate::lighting::LitMaterial>>,
        )>,
    >,
    globals: Query<&GlobalTransform, Without<GhostJoint>>,
    mut joints: Query<(&mut Transform, &mut GlobalTransform), With<GhostJoint>>,
    mut visibility: Query<&mut Visibility>,
    lit: Res<Assets<crate::lighting::PlayerLitMaterial>>,
    simple_lit: Res<Assets<crate::lighting::LitMaterial>>,
    mut materials: ResMut<Assets<crate::magick_visuals::AfterImageMaterial>>,
    qa: Res<Qa>,
) {
    let mut live = HashSet::new();
    for (source, mesh, skin, material, simple_material) in &sources {
        let Some((_, root, asset)) = parents
            .iter_ancestors(source)
            .find_map(|e| roots.get(e).ok())
        else {
            continue;
        };
        let Some(npc) = play
            .session
            .world
            .npcs
            .get(&root.handle)
            .filter(|n| n.vlad_character)
        else {
            continue;
        };
        let (Some(mesh_path), Some(root_path)) =
            (server.get_path(&mesh.0), server.get_path(&asset.0))
        else {
            continue;
        };
        if mesh_path.path() != root_path.path() || mesh_path.label() != Some("Mesh0/Primitive0") {
            continue;
        }
        live.insert(source);
        let Some(texture) = material
            .and_then(|m| lit.get(&m.0))
            .map(|m| m.base.base_color_texture.clone())
            .or_else(|| {
                simple_material
                    .and_then(|m| simple_lit.get(&m.0))
                    .map(|m| m.base.base_color_texture.clone())
            })
        else {
            continue;
        };
        let trail = trails.0.entry(source).or_insert_with(|| {
            let ghosts = (1..SNAPSHOTS)
                .map(|_| {
                    let material = materials.add(crate::magick_visuals::AfterImageMaterial {
                        params: Vec4::ZERO,
                        texture: texture.clone(),
                    });
                    let entity = commands
                        .spawn((
                            Mesh3d(mesh.0.clone()),
                            MeshMaterial3d(material.clone()),
                            Transform::IDENTITY,
                            Visibility::Hidden,
                            bevy::camera::visibility::NoFrustumCulling,
                        ))
                        .id();
                    let joints: Vec<_> = skin
                        .joints
                        .iter()
                        .map(|_| {
                            commands
                                .spawn((GhostJoint, Transform::IDENTITY, ChildOf(entity)))
                                .id()
                        })
                        .collect();
                    commands.entity(entity).insert(SkinnedMesh {
                        inverse_bindposes: skin.inverse_bindposes.clone(),
                        joints: joints.clone(),
                    });
                    Ghost {
                        entity,
                        joints,
                        material,
                    }
                })
                .collect();
            Trail {
                clock: AfterImages::vlad(),
                history: VecDeque::new(),
                alphas: [None; SNAPSHOTS - 1],
                ghosts,
                logged: None,
                last_time: None,
            }
        });
        let body = &play.session.world.physics.bodies[npc.body];
        // Follow the entity clock: pause, a pinned fixture and TimeWarp
        // must not advance history using unscaled presentation time.
        let now = play.session.world.play_time;
        let dt = trail.last_time.map_or(
            play.dt * play.session.world.time_modifier * play.session.world.time_multiplier,
            |previous| (now - previous) as f32,
        );
        trail.last_time = Some(now);
        if dt < 0.0 {
            trail.clock = AfterImages::vlad();
            trail.history.clear();
            trail.alphas = [None; SNAPSHOTS - 1];
        }
        if dt > 0.0 {
            match trail.clock.step_vlad(
                dt,
                body.speed_multiplier(),
                body.velocity[0].powi(2) + body.velocity[2].powi(2),
            ) {
                AfterImageStep::Hidden => trail.alphas = [None; SNAPSHOTS - 1],
                AfterImageStep::Drawn {
                    snapshots,
                    intensity,
                } => {
                    if snapshots > 0 {
                        let pose: Option<Vec<_>> = skin
                            .joints
                            .iter()
                            .map(|j| globals.get(*j).ok().map(GlobalTransform::to_matrix))
                            .collect();
                        if let Some(pose) = pose {
                            for _ in 0..snapshots.min(SNAPSHOTS as u32) {
                                trail.history.push_front(pose.clone());
                            }
                            trail.history.truncate(SNAPSHOTS);
                        }
                    }
                    trail.alphas = ghost_alphas(intensity);
                }
            }
        }
        for (index, ghost) in trail.ghosts.iter().enumerate() {
            let sample = if qa.phase == Some(2) {
                None
            } else {
                trail.alphas[index].zip(trail.history.get(index + 1))
            };
            if let Ok(mut visible) = visibility.get_mut(ghost.entity) {
                visible.set_if_neq(if sample.is_some() {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                });
            }
            let Some((alpha, pose)) = sample else {
                continue;
            };
            for (joint, matrix) in ghost.joints.iter().zip(pose) {
                if let Ok((mut local, mut global)) = joints.get_mut(*joint) {
                    *local = Transform::from_matrix(*matrix);
                    *global = GlobalTransform::from(*local);
                }
            }
            if let Some(mut material) = materials.get_mut(&ghost.material) {
                material.params.x = alpha;
            }
        }
        if let Some(phase) = qa
            .phase
            .filter(|p| *p > 0 && qa.handle == Some(root.handle) && trail.logged != Some(*p))
        {
            assert_eq!(
                trail.history.len(),
                SNAPSHOTS,
                "Vlad fixture has no full history"
            );
            assert!(trail.alphas.iter().all(Option::is_some));
            let first = trail.history[0][0].w_axis;
            let last = trail.history[SNAPSHOTS - 1][0].w_axis;
            assert!(
                (first - last).length() > 0.1,
                "Vlad history must contain distinct world poses"
            );
            eprintln!(
                "Vlad afterimages phase={phase} poses={} alphas={:?} span={}",
                trail.history.len(),
                trail.alphas,
                (first - last).length()
            );
            trail.logged = Some(phase);
        }
    }
    trails.0.retain(|source, trail| {
        if live.contains(source) {
            return true;
        }
        for ghost in &trail.ghosts {
            if let Ok(mut entity) = commands.get_entity(ghost.entity) {
                entity.despawn();
            }
            materials.remove(ghost.material.id());
        }
        false
    });
}

pub(super) const SCRIPT: &[Step] = &[
    (180, [0., 0.], Act::VladAfterimages(0), "moving"),
    (30, [0., 0.], Act::VladAfterimages(1), "visible"),
    (30, [0., 0.], Act::VladAfterimages(2), "hidden"),
    (30, [0., 0.], Act::VladAfterimages(3), "restored"),
];
#[derive(Resource, Default)]
struct Qa {
    phase: Option<u8>,
    handle: Option<u16>,
    origin: [f32; 3],
    ticks: u32,
}
// Authored renderer fixture: retain a genuine Vlad boss and pose its body
// along a short path. This is not an original-game runtime measurement.
fn fixture(
    mut play: ResMut<Play>,
    pilot: Option<Res<Autopilot>>,
    mut qa: ResMut<Qa>,
    mut animations: Query<&mut AnimationPlayer>,
) {
    use magicka_runtime::{
        boss_action::{BossAction, BossCommand, BossSetup},
        script_runtime::ScriptWorld,
    };
    let Some(pilot) = pilot else { return };
    let Some(started) = pilot.started else { return };
    let Some(Act::VladAfterimages(phase)) = script_act(pilot.script, pilot.frame - started) else {
        return;
    };
    qa.phase = Some(phase);
    if phase > 0 {
        for mut animation in &mut animations {
            animation.pause_all();
        }
    }
    let s = &mut play.session;
    if qa.handle.is_none() {
        s.disable_scripts();
        s.world.flags.cutscene = false;
        s.world.flags.input_limited = false;
        s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
        // The authored scene can already have spawned its delayed boss.
        // Reuse it instead of leaving a second, stationary Vlad in the audit.
        if !s.world.npcs.values().any(|n| n.vlad_character) {
            s.world
                .boss_action(
                    &BossAction {
                        delay: 0.,
                        command: BossCommand::Spawn {
                            boss: "vlad".into(),
                            prop: false,
                            generic: false,
                            area: magicka_runtime::scene_loading::custom_hash("boss_spawn"),
                            unique_id: 0,
                            mesh: -1,
                            on_death: 0,
                            on_damage: 0,
                            setup: BossSetup {
                                delayed: true,
                                ..Default::default()
                            },
                        },
                    },
                    false,
                )
                .unwrap()
                .unwrap();
        }
        let n = s
            .world
            .npcs
            .values_mut()
            .find(|n| n.vlad_character)
            .expect("Vlad boss");
        n.set_order(1, 0, 0, 0, None);
        qa.handle = Some(n.handle);
        qa.origin = s.world.physics.bodies[n.body].position;
    }
    let n = &s.world.npcs[&qa.handle.unwrap()];
    let body = n.body;
    if phase == 0 {
        qa.ticks += 1;
        let phase = qa.ticks as f32 * FIXED_DT * 2.0;
        let b = &mut s.world.physics.bodies[body];
        b.active = false;
        b.position = [qa.origin[0] + phase.sin() * 2.0, qa.origin[1], qa.origin[2]];
        b.velocity = [phase.cos() * 4.0, 0., 0.];
        b.set_speed_multiplier(1.5);
    }
    let p = s.world.physics.bodies[body].position;
    let target = [p[0] - 0.5, p[1] + 1., p[2]];
    s.world.camera.move_to(target, 0.);
    s.world.camera.set_position(target, false);
    s.world.camera.set_magnification(0.8);
}
