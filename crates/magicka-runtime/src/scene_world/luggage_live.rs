//! SurvivalRuleset.SpawnLuggage (research/rulesets.md): a reward luggage
//! NPC dropped somewhere inside the current view, whose Death/OverKill
//! events spawn a pack item or a magick book.
use super::*;
use crate::ruleset::SurvivalPacks;
use crate::xna_math::{invert, transform_vector4};
use magicka_content::common::{Event, EventCollection, EventCondition};

/// Data/Characters/Luggage_item and Luggage_magick.
pub const LUGGAGE_ITEM_TEMPLATE: &str = "luggage_item";
pub const LUGGAGE_MAGICK_TEMPLATE: &str = "luggage_magick";
/// EventConditionType.Death / OverKill.
const DEATH: u8 = 32;
const OVERKILL: u8 = 64;

/// Factions.EVIL as the rewrite's entity faction (see survival_added).
const EVIL: i32 = 1;

impl<C: SceneContent> SceneWorld<C> {
    /// Process queued SpawnLuggage requests once a reward pool is supplied
    /// (`survival_packs`); without one they stay queued.
    pub(crate) fn spawn_pending_luggage(&mut self) {
        let Some(packs) = self.survival_packs.clone() else {
            return;
        };
        for item in std::mem::take(&mut self.pending_survival_luggage) {
            if let Err(e) = self.spawn_luggage(item, &packs) {
                self.log.push(format!("survival luggage: {e}"));
            }
        }
    }

    /// One SpawnLuggage(item): up to ten random view points projected to the
    /// plane at the camera target height, moved to the navmesh, kept when on
    /// screen (native tests X twice) and above scene geometry within 4 m.
    pub fn spawn_luggage(
        &mut self,
        item: bool,
        packs: &SurvivalPacks,
    ) -> Result<Option<u16>, WorldError> {
        let vp = crate::camera::multiply(&self.camera.view(), &self.camera.projection(self.aspect));
        let inverse = invert(&vp);
        let origin = self.camera.position();
        let plane_y = origin[1] - crate::camera::CAMERA_OFFSET[1];
        for _ in 0..10 {
            let x = (self.random() - 0.5) * 1.9;
            let y = (self.random() - 0.5) * 1.9;
            let far = transform_vector4([x, y, 1.0, 1.0], &inverse);
            let dir: [f32; 3] = std::array::from_fn(|i| far[i] / far[3] - origin[i]);
            if dir[1] == 0.0 {
                return Err("SpawnLuggage: view ray parallel to the ground plane".into());
            }
            let t = (plane_y - origin[1]) / dir[1];
            let hit = std::array::from_fn(|i| origin[i] + dir[i] * t);
            let point = self
                .nav
                .as_ref()
                .ok_or("SpawnLuggage: scene has no navmesh")?
                .nearest(hit, magicka_sim::navmesh::movement::DEFAULT)
                .map_or(hit, |(_, p, _)| p);
            let clip = transform_vector4([point[0], point[1], point[2], 1.0], &vp);
            if (clip[0] / clip[3]).abs() >= 1.0 {
                continue;
            }
            let probe = magicka_sim::spray_pulse::Segment {
                origin: point,
                delta: [0.0, -4.0, 0.0],
            };
            let Some(ground) = self.collision.collision().segment_intersect(probe, false) else {
                continue;
            };
            let (template, event) = if item && !packs.items.is_empty() {
                let name = packs
                    .pick_item(|n| ((self.random() * n as f32) as usize).min(n - 1))
                    .unwrap()
                    .to_owned();
                (LUGGAGE_ITEM_TEMPLATE, Event::SpawnItem { item: name })
            } else {
                let unlocked = self.unlocked_magick_mask;
                let Some(m) = packs.pick_magick(|| self.random() as f64, |_| true, unlocked) else {
                    break;
                };
                let magick = crate::magick_books::BOOK_MAGICKS[m as usize].to_owned();
                (LUGGAGE_MAGICK_TEMPLATE, Event::SpawnMagick { magick })
            };
            let angle = self.random() * std::f32::consts::TAU;
            let (s, c) = angle.sin_cos();
            // Matrix.CreateRotationY.
            let mut orientation = magicka_content::skinned::IDENTITY;
            orientation[0] = c;
            orientation[2] = -s;
            orientation[8] = s;
            orientation[10] = c;
            return self
                .spawn_luggage_npc(template, event, ground.hit.position, orientation, 0.0)
                .map(Some);
        }
        Ok(None)
    }

    /// VersusRuleset.SpawnLuggage: at the GetAnyArea locator (else a random
    /// point of that trigger area), uniform pool draws without the unlock
    /// filter, drops despawning after 12 s.
    pub fn spawn_versus_luggage(
        &mut self,
        item: bool,
        area: i32,
        packs: &SurvivalPacks,
    ) -> Result<Option<u16>, WorldError> {
        let (template, event) = if item && !packs.items.is_empty() {
            let n = packs.items.len();
            let name = packs.items[((self.random() * n as f32) as usize).min(n - 1)].clone();
            (LUGGAGE_ITEM_TEMPLATE, Event::SpawnItem { item: name })
        } else if !packs.magicks.is_empty() {
            let n = packs.magicks.len();
            let m = packs.magicks[((self.random() * n as f32) as usize).min(n - 1)];
            let magick = crate::magick_books::BOOK_MAGICKS[m as usize].to_owned();
            (LUGGAGE_MAGICK_TEMPLATE, Event::SpawnMagick { magick })
        } else {
            return Ok(None);
        };
        let mut locator = match self.locators.get(&area) {
            Some(m) => *m,
            None => {
                let mut m = magicka_content::skinned::IDENTITY;
                if let Some(p) = self.random_area_point(area) {
                    m[12..15].copy_from_slice(&p);
                }
                m
            }
        };
        let feet = [locator[12], locator[13], locator[14]];
        locator[12..15].fill(0.0);
        self.spawn_luggage_npc(
            template,
            event,
            feet,
            locator,
            crate::ruleset::LUGGAGE_DESPAWN,
        )
        .map(Some)
    }

    /// The shared luggage NPC: raised by the capsule half height (and
    /// again by Character.Initialize; PROVEN, decompiled), spawn animation,
    /// EVIL, Death/OverKill drop events and the luggage_spawn effect.
    fn spawn_luggage_npc(
        &mut self,
        template: &str,
        event: Event,
        ground: [f32; 3],
        orientation: [f32; 16],
        despawn: f32,
    ) -> Result<u16, WorldError> {
        self.load_character_template(Some(template))?;
        let type_id = custom_hash(template);
        let t = self.templates[&type_id].clone();
        let position = [ground[0], ground[1] + t.length * 0.5 + t.radius, ground[2]];
        let handle = self.acquire_npc()?.handle;
        self.initialize_npc(handle, type_id, 0, position, 0)?;
        if let Some(n) = self.npcs.get(&handle) {
            let body = &mut self.physics.bodies[n.body];
            let m = orientation;
            let o = magicka_sim::character_physics::Orientation {
                right: [m[0], m[1], m[2]],
                up: [m[4], m[5], m[6]],
                backward: [m[8], m[9], m[10]],
            };
            body.move_to(body.position, o);
            body.set_desired_direction(o.forward());
        }
        self.entity_mut(handle)?.orientation = orientation;
        if despawn > 0.0 {
            self.luggage_despawn.insert(handle, despawn);
        } else {
            self.luggage_despawn.remove(&handle);
        }
        let spawn = crate::animation_controller::animation_value("spawn").unwrap_or(0);
        self.begin_resurrection(handle, spawn)?;
        crate::spawn_action::SpawnHost::add_entity(self, handle)?;
        let collection = |condition_type| EventCollection {
            condition: EventCondition {
                condition_type,
                hitpoints: 0,
                elements: 0,
                threshold: 0.0,
                time: 0.0,
            },
            repeat: false,
            events: vec![event.clone()],
        };
        if let Some(n) = self.npcs.get_mut(&handle) {
            n.definition.event_conditions =
                std::sync::Arc::new(vec![collection(DEATH), collection(OVERKILL)]);
        }
        let mut effect = orientation;
        effect[12..15].copy_from_slice(&position);
        self.missile_visuals
            .push(crate::missiles::MissileVisual::Effect {
                missile: 0,
                name: crate::missiles::intern("luggage_spawn"),
                position,
                direction: crate::xna_math::forward(&effect),
                follow: false,
            });
        let e = self.entity_mut(handle)?;
        e.faction = EVIL;
        Ok(handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play_session::PlaySession;
    use crate::ruleset::SurvivalPacks;

    fn glade() -> Option<PlaySession> {
        let root = magicka_content::retail_content_dir();
        root.is_dir()
            .then(|| PlaySession::load(&root, "Challenges/ch_glade.lvl", "Wizard").unwrap())
    }

    fn luggage(s: &PlaySession, template: &str) -> Vec<u16> {
        s.world
            .entities
            .iter()
            .filter(|e| e.added && !e.removed && e.template_name.as_deref() == Some(template))
            .map(|e| e.handle)
            .collect()
    }

    /// No injected pools: loading a Survival map reads PackMan's packs from the
    /// user's Magicka.exe, and the first reward luggage spawns from them.
    #[test]
    fn survival_map_reads_packs_from_the_game_and_spawns_luggage() {
        let root = magicka_content::retail_content_dir();
        if !root.join("../Magicka.exe").is_file() {
            return;
        }
        let mut s = glade().unwrap();
        let packs = s
            .world
            .survival_packs
            .clone()
            .expect("packs read from Magicka.exe");
        assert_eq!(packs.items.len(), 47);
        assert!(!packs.magicks.is_empty());
        for _ in 0..(7.6 * 60.0) as usize {
            s.step(1.0 / 60.0);
        }
        assert_eq!(
            luggage(&s, LUGGAGE_MAGICK_TEMPLATE).len(),
            1,
            "log: {:?}",
            s.world.log
        );
    }

    #[test]
    fn first_luggage_carries_a_magick_book_and_drops_it_on_death() {
        let Some(mut s) = glade() else {
            return;
        };
        s.set_survival_packs(SurvivalPacks {
            items: vec!["staff_gnarled".into()],
            magicks: vec![crate::ruleset::REVIVE],
        });
        // TIMEBETWEENLUGGAGE × ¼ = 7.5 s.
        for _ in 0..(7.4 * 60.0) as usize {
            s.step(1.0 / 60.0);
        }
        assert!(luggage(&s, LUGGAGE_MAGICK_TEMPLATE).is_empty());
        for _ in 0..12 {
            s.step(1.0 / 60.0);
        }
        let spawned = luggage(&s, LUGGAGE_MAGICK_TEMPLATE);
        assert_eq!(spawned.len(), 1, "log: {:?}", s.world.log);
        let handle = spawned[0];
        assert_eq!(s.world.entity(handle).unwrap().faction, EVIL);
        let books = s.world.book_pickups.len();
        crate::entity_command::EntityCommandHost::kill(&mut s.world, handle).unwrap();
        for _ in 0..600 {
            s.step(1.0 / 60.0);
        }
        let book = s
            .world
            .book_pickups
            .values()
            .find(|b| !b.dead && !b.immovable)
            .expect("luggage death spawns a movable book");
        assert_eq!(book.magick, crate::ruleset::REVIVE);
        assert!(s.world.book_pickups.len() > books);
    }

    #[test]
    fn item_luggage_drops_its_pack_item() {
        let Some(mut s) = glade() else {
            return;
        };
        let packs = SurvivalPacks {
            items: vec!["staff_gnarled".into()],
            magicks: vec![],
        };
        s.step(1.0 / 60.0);
        let handle = s
            .world
            .spawn_luggage(true, &packs)
            .unwrap()
            .expect("on-screen spot");
        let items = s.world.item_pickups.len();
        crate::entity_command::EntityCommandHost::kill(&mut s.world, handle).unwrap();
        // RessurectionState holds a killed character until the 4.17 s
        // spawn clip has finished; Die() follows the death clip.
        for _ in 0..600 {
            s.step(1.0 / 60.0);
        }
        assert!(s.world.item_pickups.len() > items, "log: {:?}", s.world.log);
        assert!(s.world.item_pickups.values().any(|p| p
            .item
            .as_ref()
            .is_some_and(|i| i.definition.name.eq_ignore_ascii_case("staff_gnarled"))));
    }
}
