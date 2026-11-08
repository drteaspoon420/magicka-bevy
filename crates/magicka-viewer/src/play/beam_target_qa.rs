//! Rewrite-only beam collision and rendering audit. Targets are placed through
//! public runtime APIs; element keys and Cast create and steer the actual beam.
use super::*;
use magicka_sim::railgun::{RailId, RailRender};

pub(super) const PORTAL: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "before"),
    (1, [0., 0.5], Act::BeamTarget(0, 0), "setup"),
    (60, [0., 0.5], Act::Idle, "portals"),
    (2, [0., 0.5], Act::Arcane, "arcane"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (45, [0., 0.5], Act::BeamTarget(0, 1), "hit"),
    (30, [0., 0.5], Act::BeamTarget(0, 2), "visible"),
    (30, [0., 0.5], Act::BeamTarget(0, 3), "hidden"),
    (30, [0., 0.5], Act::BeamTarget(0, 4), "restored"),
    (31, [0., 0.5], Act::BeamTarget(0, 5), "disconnected"),
    (1, [0., 0.5], Act::BeamTarget(0, 6), "stop"),
    (60, [0., 0.5], Act::Idle, "fade"),
    (1, [0., 0.5], Act::BeamTarget(0, 7), "ended"),
];
pub(super) const ELEMENTAL: &[Step] = &[
    (90, [0., 0.5], Act::Idle, "before"),
    (1, [0., 0.5], Act::BeamTarget(1, 0), "setup"),
    (60, [0., 0.5], Act::Idle, "egg"),
    (2, [0., 0.5], Act::Arcane, "arcane"),
    (12, [0., 0.5], Act::Idle, "gap"),
    (2, [0., 0.5], Act::Fire, "fire"),
    (12, [0., 0.5], Act::Idle, "queued"),
    (45, [0., 0.5], Act::BeamTarget(1, 1), "hatched"),
    (30, [0., 0.5], Act::BeamTarget(1, 2), "visible"),
    (30, [0., 0.5], Act::BeamTarget(1, 3), "hidden"),
    (30, [0., 0.5], Act::BeamTarget(1, 4), "restored"),
    (1, [0., 0.5], Act::BeamTarget(1, 5), "stop"),
    (60, [0., 0.5], Act::Idle, "fade"),
    (1, [0., 0.5], Act::BeamTarget(1, 7), "ended"),
];

#[derive(Resource, Default)]
pub(super) struct Audit {
    phase: Option<u8>,
    mode: u8,
    root: Option<RailId>,
    target: Option<u16>,
    before_hp: f32,
    frozen: bool,
    hidden: bool,
    beams: Vec<RailRender>,
}
impl Audit {
    pub(super) fn prepare(
        &mut self,
        s: &mut PlaySession,
        act: Option<Act>,
        dir: &std::path::Path,
    ) -> bool {
        if self.root.is_none() {
            let owner = s.avatar().handle;
            self.root = s.world.rails.active().iter().copied().find(|&id| {
                s.world.rails.rail(id).is_some_and(|r| {
                    r.depth == 0 && r.damages.iter().any(|(handle, _)| *handle == owner)
                })
            });
        }
        if let Some(Act::BeamTarget(mode, phase)) = act {
            if self.phase != Some(phase) {
                self.mode = mode;
                match phase {
                    0 => self.setup(s),
                    1 => {
                        if mode == 0 {
                            self.before_hp = s.world.npcs[&self.target.unwrap()]
                                .character
                                .combat
                                .health
                                .hit_points;
                        }
                    }
                    2 => {
                        assert!(
                            s.avatar().current_spell.is_some(),
                            "ordinary Cast must own the beam"
                        );
                        if mode == 0 {
                            assert!(s.world.portals.portal.connected);
                            assert!(
                                s.world
                                    .rails
                                    .rail(self.root.unwrap())
                                    .unwrap()
                                    .child
                                    .is_some(),
                                "portal must create child"
                            );
                            assert!(
                                s.world.npcs[&self.target.unwrap()]
                                    .character
                                    .combat
                                    .health
                                    .hit_points
                                    < self.before_hp,
                                "exit target must receive beam damage"
                            );
                        } else {
                            let cache = s.world.elemental_eggs.as_ref().unwrap();
                            assert!(
                                !cache
                                    .slots
                                    .iter()
                                    .find(|slot| Some(slot.handle) == self.target)
                                    .unwrap()
                                    .actor
                                    .registered,
                                "damaged egg must hatch"
                            );
                            assert!(
                                s.world
                                    .npcs
                                    .values()
                                    .any(|n| n.definition.id == "elemental_fire"),
                                "beam payload must select Fire"
                            );
                        }
                        self.beams.clone_from(&s.beams);
                        self.frozen = true;
                    }
                    3 => self.hidden = true,
                    4 => self.hidden = false,
                    5 => {
                        self.frozen = false;
                        if mode == 0 {
                            s.world.portals.portal.kill();
                        }
                        // Releasing Cast ends the ordinary owner spell.
                    }
                    6 => {
                        assert!(!s.world.portals.portal.connected);
                        let root = s.world.rails.rail(self.root.unwrap()).unwrap();
                        assert!(
                            root.child.is_none_or(|id| s
                                .world
                                .rails
                                .rail(id)
                                .is_none_or(|r| r.dead)),
                            "disconnect must end continuation"
                        );
                    }
                    7 => assert!(s.world.rails.is_empty(), "beam network must expire"),
                    _ => unreachable!(),
                }
                self.phase = Some(phase);
                self.apply_visibility(s);
                self.report(s, dir, phase);
            }
        }
        self.apply_visibility(s);
        self.frozen
    }

    fn setup(&mut self, s: &mut PlaySession) {
        s.stop_level_effects();
        s.world.flags.cutscene = false;
        s.world.flags.input_limited = false;
        let p = s.body().position;
        if self.mode == 0 {
            let forward = s.body().direction();
            let entry = std::array::from_fn(|i| p[i] + forward[i] * 3.);
            assert!(s.world.portal_at(entry));
            assert!(s.world.portal_at([entry[0] + 3.5, entry[1], entry[2]]));
            let exit = s.world.portals.portal.a.position;
            let target = s
                .spawn_npc("Goblin_miner", "idle", 7.)
                .expect("portal target");
            let npc = s.world.npcs.get_mut(&target).unwrap();
            npc.ai.set_order(0, None);
            npc.react_to = 0;
            npc.reaction_order = 0;
            s.world.physics.bodies[npc.body].set_position([
                exit[0] + forward[0] * 4. - forward[2] * 0.2,
                exit[1] - 0.4,
                exit[2] + forward[2] * 4. + forward[0] * 0.2,
            ]);
            self.before_hp = npc.character.combat.health.hit_points;
            self.target = Some(target);
        } else {
            let owner = s.avatar().handle;
            let target = s
                .world
                .summon_elemental_from(owner)
                .expect("egg fixture")
                .unwrap();
            self.target = Some(target);
        }
    }
    fn apply_visibility(&self, s: &mut PlaySession) {
        if self.frozen {
            if self.hidden {
                s.beams.clear();
            } else {
                s.beams.clone_from(&self.beams);
            }
        }
    }
    fn report(&self, s: &PlaySession, dir: &std::path::Path, phase: u8) {
        let rails: Vec<_> = s
            .world
            .rails
            .active()
            .iter()
            .filter_map(|&id| {
                s.world.rails.rail(id).map(|r| {
                    serde_json::json!({
                        "id":id,"position":r.position,"direction":r.direction,"length":r.length,
                        "child":r.child,"depth":r.depth,"out_portal":r.out_portal,"dead":r.dead,
                    })
                })
            })
            .collect();
        let npcs: Vec<_> = s
            .world
            .npcs
            .iter()
            .map(|(&id, n)| {
                serde_json::json!({
                    "id":id,"template":n.definition.id,"hp":n.character.combat.health.hit_points,
                    "position":s.world.physics.bodies[n.body].position,
                })
            })
            .collect();
        let report = serde_json::json!({"mode":self.mode,"phase":phase,
            "time":s.play_time,"cast_input":true,"active_spell":s.avatar().current_spell.is_some(),"root":self.root,"target":self.target,"before_hp":self.before_hp,
            "render_beams":s.beams.len(),"rails":rails,"npcs":npcs,
            "view":s.camera().view(), "projection":s.camera().projection(1280. / 720.),
            "portal_connected":s.world.portals.portal.connected,
            "portal_a":s.world.portals.portal.a.position,"portal_b":s.world.portals.portal.b.position});
        std::fs::write(
            dir.join(format!("beam-target-{phase}.json")),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
    }
}
