//! Read-only save regression: load, restart, damage, restart, defeat chieftain.
use super::*;
use magicka_runtime::{scene_loading::custom_hash, scene_world::boss_live::BossRef};

pub(super) const SCRIPT: &[Step] = &[
    (120, [0., 0.], Act::Houses(0), "saved_missing"),
    (90, [0., 0.], Act::Houses(1), "checkpoint_restored"),
    (60, [0., 0.], Act::Houses(2), "damaged"),
    (90, [0., 0.], Act::Houses(3), "checkpoint_again"),
    (90, [0., 0.], Act::Houses(4), "chieftain_defeated"),
];

#[derive(Resource, Default)]
pub(super) struct Audit {
    phase: Option<u8>,
}

impl Audit {
    pub(super) fn prepare(
        &mut self,
        play: &mut Play,
        act: Option<Act>,
        dir: &std::path::Path,
    ) -> bool {
        let Some(Act::Houses(phase)) = act else {
            return false;
        };
        if self.phase == Some(phase) {
            return true;
        }
        if phase == 0 {
            let path = std::env::var_os("MAGICKA_HIGHLANDS_SAVE")
                .expect("set MAGICKA_HIGHLANDS_SAVE to a private save copy");
            let data = std::fs::read(path).expect("read QA save");
            let saved = serde_json::from_slice(&data).expect("parse QA save");
            play.session =
                PlaySession::load_disk_save(&play.content, &saved).expect("load QA save");
            play.reloads += 1;
            play.session.step(FIXED_DT);
        }
        let s = &mut play.session;
        assert_eq!(s.scene_name(), "hl_s5bf");
        if phase == 1 || phase == 3 {
            assert!(s.restart_at_checkpoint().unwrap());
            play.reloads += 1;
            s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
            for _ in 0..65 {
                s.step(FIXED_DT);
            }
        }
        if phase == 2 {
            for (i, fraction) in [0., 0.2, 0.5, 0.9, 1.].into_iter().enumerate() {
                let h = s
                    .world
                    .entities
                    .iter()
                    .find(|e| {
                        e.unique_id == custom_hash(&format!("house{i}"))
                            && e.added
                            && !e.removed
                            && !e.dead
                    })
                    .unwrap()
                    .handle;
                let d = s.world.physics_damage.get_mut(&h).unwrap();
                if i == 0 {
                    d.health = 1.;
                } else {
                    d.health = d.maximum * fraction;
                }
                if i == 0 {
                    s.world
                        .damage_physics(
                            h,
                            magicka_sim::damage::Damage {
                                attack: magicka_sim::damage::attack::DAMAGE,
                                element: magicka_sim::spells::Elements::FIRE.0,
                                amount: 100_000.,
                                magnitude: 1.,
                            },
                            15,
                        )
                        .unwrap();
                }
            }
            for _ in 0..2 {
                s.step(FIXED_DT);
            }
        }
        if phase == 4 {
            let h = s
                .world
                .boss_fight
                .bosses
                .iter()
                .find_map(|b| match b {
                    BossRef::CountryDefense(c) => c.chieftain,
                    _ => None,
                })
                .unwrap();
            s.world
                .npcs
                .get_mut(&h)
                .unwrap()
                .character
                .combat
                .health
                .hit_points = 0.;
            for _ in 0..600 {
                s.step(FIXED_DT);
                if s.runtime
                    .trigger(custom_hash("startend"))
                    .is_some_and(|t| t.has_triggered)
                {
                    break;
                }
            }
            for _ in 0..10 {
                s.step(FIXED_DT);
            }
            assert!(
                s.runtime
                    .trigger(custom_hash("startend"))
                    .unwrap()
                    .has_triggered
            );
            assert!(
                !s.runtime
                    .trigger(custom_hash("defeat condition"))
                    .unwrap()
                    .has_triggered
            );
        }
        let icons = s
            .world
            .boss_fight
            .bosses
            .iter_mut()
            .find_map(|b| match b {
                BossRef::CountryDefense(c) => {
                    c.timer = 1.;
                    Some(c.slices)
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(
            icons,
            match phase {
                0 => [3; 5],
                2 => [3, 2, 1, 0, 0],
                _ => [0; 5],
            }
        );
        let houses: Vec<_> = (0..5).map(|i| {
            let e = s.world.entities.iter().find(|e| e.unique_id == custom_hash(&format!("house{i}")) && e.added && !e.removed);
            serde_json::json!({"house":i,"handle":e.map(|e|e.handle),"drawn":e.is_some_and(|e|!e.dead),"health":e.map(|e|e.hit_points),"maximum":e.map(|e|e.max_hit_points)})
        }).collect();
        let report = serde_json::json!({"phase":phase,"icons":icons,"houses":houses,"counter":s.runtime.variables.counter(custom_hash("houses")),"defeat":s.runtime.trigger(custom_hash("defeat condition")).unwrap().has_triggered,"start_end":s.runtime.trigger(custom_hash("startend")).unwrap().has_triggered});
        std::fs::write(
            dir.join(format!("houses-{phase}.json")),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        self.phase = Some(phase);
        true
    }
}
