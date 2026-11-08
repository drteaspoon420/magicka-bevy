//! Pinned checkpoints from the real Havindr Orcs2 spawn sequence.
use super::*;
use magicka_runtime::character_motion::CharacterState;

pub(super) const SCRIPT: &[Step] = &[
    (120, [0., 0.], Act::Entrance(0), "loaded"),
    (30, [0., 0.], Act::Entrance(1), "early"),
    (30, [0., 0.], Act::Entrance(2), "middle"),
    (30, [0., 0.], Act::Entrance(3), "hidden"),
    (30, [0., 0.], Act::Entrance(4), "restored"),
    (30, [0., 0.], Act::Entrance(5), "late"),
    (30, [0., 0.], Act::Entrance(6), "complete"),
];

#[derive(Resource, Default)]
pub(super) struct Audit {
    phase: Option<u8>,
    handle: Option<u16>,
    camera: [f32; 3],
    elapsed: f32,
    hidden: bool,
}
impl Audit {
    pub(super) fn prepare(
        &mut self,
        s: &mut PlaySession,
        act: Option<Act>,
        dir: &std::path::Path,
    ) -> bool {
        let Some(Act::Entrance(phase)) = act else {
            return false;
        };
        if self.phase != Some(phase) {
            if phase == 0 {
                assert_eq!(s.scene_name(), "havindr_s6", "use --scene havindr_s6");
                s.script_error = None;
                s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
                let old: std::collections::BTreeSet<_> = s.world.npcs.keys().copied().collect();
                assert!(s.run_trigger("Orcs2", true).unwrap());
                s.step(FIXED_DT);
                self.elapsed += FIXED_DT;
                self.handle = s.world.npcs.keys().copied().find(|h| {
                    !old.contains(h)
                        && s.world.entities.iter().any(|e| {
                            e.handle == *h && e.template_name.as_deref() == Some("orc_regular")
                        })
                });
                let npc = &s.world.npcs[&self.handle.expect("authored first wall-entry orc")];
                self.camera = s.world.physics.bodies[npc.body].position;
                self.camera[1] += 1.;
                s.world.players[0].as_mut().unwrap().ai.set_order(0, None);
            }
            let handle = self.handle.expect("entry fixture initialized");
            if matches!(phase, 1 | 2 | 5) {
                let fraction = match phase {
                    1 => 0.2,
                    2 => 0.5,
                    _ => 0.85,
                };
                for _ in 0..600 {
                    let n = &s.world.npcs[&handle];
                    assert_eq!(n.motion.state, CharacterState::Resurrection);
                    if n.motion.controller.time >= n.motion.controller.clip_duration() * fraction {
                        break;
                    }
                    s.step(FIXED_DT);
                    self.elapsed += FIXED_DT;
                }
            } else if phase == 6 {
                for _ in 0..600 {
                    if s.world.npcs[&handle].motion.state != CharacterState::Resurrection {
                        break;
                    }
                    s.step(FIXED_DT);
                    self.elapsed += FIXED_DT;
                }
                for _ in 0..15 {
                    s.step(FIXED_DT);
                    self.elapsed += FIXED_DT;
                }
            }
            self.hidden = phase == 3;
            let n = &s.world.npcs[&handle];
            let body = &s.world.physics.bodies[n.body];
            if phase < 6 {
                assert_eq!(n.motion.state, CharacterState::Resurrection);
                assert!(!body.allow_move && !body.allow_rotate);
                assert_eq!(n.motion.controller.clip_name(), Some("spawn"));
            } else {
                assert_ne!(n.motion.state, CharacterState::Resurrection);
                assert!(body.allow_move && body.allow_rotate);
            }
            let rig = n.rig.as_ref().expect("native orc rig");
            let poses = rig.skeleton.local_poses(n.motion.controller.pose_sample());
            let roots: Vec<_> = rig
                .skeleton
                .skeleton
                .bones
                .iter()
                .enumerate()
                .filter(|(_, b)| b.parent.is_none())
                .map(|(i, b)| (&b.name, poses[i].0))
                .collect();
            let report = serde_json::json!({"phase":phase,"handle":handle,"elapsed":self.elapsed,
                "state":format!("{:?}",n.motion.state),"clip":n.motion.controller.clip_name(),
                "clip_time":n.motion.controller.time,"clip_duration":n.motion.controller.clip_duration(),
                "position":body.position,"allow_move":body.allow_move,"allow_rotate":body.allow_rotate,
                "roots":roots,"hidden":self.hidden,"camera":self.camera});
            std::fs::write(
                dir.join(format!("entry-{phase}.json")),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            eprintln!("NPC entry checkpoint {report}");
            self.phase = Some(phase);
        }
        s.world.camera.move_to(self.camera, 0.);
        s.world.camera.set_position(self.camera, false);
        s.world.camera.set_magnification(1.);
        true
    }
}

/// Hide only the selected body, keeping identical pose, equipment and scene.
pub(super) fn visibility(
    audit: Res<Audit>,
    parents: Query<&ChildOf>,
    roots: Query<&NpcRoot>,
    mut meshes: Query<
        (Entity, &mut Visibility),
        (
            With<bevy::mesh::skinning::SkinnedMesh>,
            Without<crate::magick_visuals::InvisibilityPass>,
            Without<crate::self_shield::SolidShieldPass>,
        ),
    >,
) {
    let Some(handle) = audit.handle else { return };
    for (entity, mut visibility) in &mut meshes {
        if parents
            .iter_ancestors(entity)
            .any(|e| roots.get(e).is_ok_and(|n| n.handle == handle))
        {
            visibility.set_if_neq(if audit.hidden {
                Visibility::Hidden
            } else {
                Visibility::Inherited
            });
        }
    }
}
