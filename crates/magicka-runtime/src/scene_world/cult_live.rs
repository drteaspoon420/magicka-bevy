//! `Bosses.Cult` (Mines mi_s5BF): 26 spectator warlocks (two of them
//! "bill" and "bull"), their leader, the spectator camera show, rounds of
//! teleported fighters, the leader's revive and the final fight where he
//! keeps reviving the fallen. See research/boss-battles.md.
use super::*;

const TEMPLATES: [&str; 5] = [
    "boss_cult_cold",
    "boss_cult_fire",
    "boss_cult_lightning",
    "boss_cult_nature",
    "boss_cult_leader",
];
/// WARLOCK_POSITIONS[i][0].
const POSITIONS: [&str; 5] = [
    "spawn_warlock5",
    "spawn_warlock1",
    "spawn_warlock2",
    "spawn_warlock3",
    "spawn_warlock4",
];
const SPECTATORS: usize = 26;
/// Spell.COLDCOLOR / FIRECOLOR / LIGHTNINGCOLOR / LIFECOLOR by template.
const TINTS: [[f32; 3]; 4] = [
    [1.0, 1.0, 1.4],
    [1.8, 0.6, 0.4],
    [0.75, 0.5, 1.0],
    [0.2, 1.6, 0.2],
];
const PAUSE_TIME: f32 = 2.5;
const REVIVE_COOLDOWN: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CultState {
    Intro,
    Battle,
    Revive,
    Spectator,
    Final,
    Dead,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CultHost {
    pub state: CultState,
    /// mWarlocks (a revive replaces the handle in place).
    pub warlocks: Vec<u16>,
    pub spectators: Vec<u16>,
    pub fighting: Vec<u16>,
    pub leader: u16,
    positions: [[f32; 3]; 5],
    directions: [[f32; 3]; 5],
    pub total_max: f32,
    pub total_hp: f32,
    pause_timer: f32,
    spectator_show: bool,
    timer: f32,
    release: bool,
    pan: bool,
    back_pan: bool,
    teleported: bool,
    changed_state: bool,
    revive_cooldown: f32,
    revive_index: Option<usize>,
    revivee_template: [usize; 5],
    trigger_executed: bool,
    last_positions: BTreeMap<u16, [f32; 3]>,
    rng: u64,
}

fn forward(m: &[f32; 16]) -> [f32; 3] {
    [-m[8], -m[9], -m[10]]
}

impl<C: SceneContent> SceneWorld<C> {
    fn cult_random(host: &mut CultHost, n: usize) -> usize {
        host.rng ^= host.rng << 13;
        host.rng ^= host.rng >> 7;
        host.rng ^= host.rng << 17;
        (host.rng % n.max(1) as u64) as usize
    }

    /// Initialize(template, position, uniqueID, deadTimer) through the spawn
    /// path, then placed at `position` facing `direction`.
    fn cult_spawn(
        &mut self,
        template: &str,
        unique: Option<&str>,
        dead_timer: f32,
        position: [f32; 3],
        direction: [f32; 3],
    ) -> Result<u16, WorldError> {
        let mut attributes = vec![
            ("type".into(), template.into()),
            ("area".into(), POSITIONS[0].into()),
            ("order".into(), "idle".into()),
            ("reactto".into(), "none".into()),
        ];
        if let Some(id) = unique {
            attributes.push(("id".into(), id.into()));
        }
        let node = crate::scene_script::XmlElement::synthetic("spawn", attributes);
        let spawn = crate::spawn_action::SpawnAction::read(&node)
            .map_err(|e| format!("cult {template}: {e:?}"))?;
        let handle = self.spawn_generic_boss(&spawn)?;
        if let Some(n) = self.npcs.get_mut(&handle) {
            // Character.Color: the element colour per cultist, zero for the
            // leader.
            n.tint = Some(
                TEMPLATES
                    .iter()
                    .position(|t| *t == template)
                    .and_then(|i| TINTS.get(i).copied())
                    .unwrap_or([0.0; 3]),
            );
            n.motion.dead_timer = dead_timer;
            let body = &mut self.physics.bodies[n.body];
            let o = magicka_sim::character_physics::Orientation::from_forward_up(
                direction,
                [0.0, 1.0, 0.0],
            );
            body.move_to(position, o);
            body.set_desired_direction(direction);
        }
        Ok(handle)
    }

    fn cult_dead(&self, handle: u16) -> bool {
        match self.npcs.get(&handle) {
            Some(n) => n.dead() || n.character.combat.health.hit_points <= 0.0,
            None => true,
        }
    }
    fn cult_hp(&self, handle: u16) -> f32 {
        self.npcs
            .get(&handle)
            .map_or(0.0, |n| n.character.combat.health.hit_points.max(0.0))
    }
    fn cult_max(&self, handle: u16) -> f32 {
        self.npcs
            .get(&handle)
            .map_or(0.0, |n| n.character.combat.health.max_hit_points)
    }
    fn cult_position(&self, host: &CultHost, handle: u16) -> [f32; 3] {
        self.npcs
            .get(&handle)
            .map(|n| self.physics.bodies[n.body].position)
            .or_else(|| host.last_positions.get(&handle).copied())
            .unwrap_or([0.0; 3])
    }

    fn cult_order(
        &mut self,
        handle: u16,
        attack: bool,
        events: Option<Vec<crate::ai_event::AiEvent>>,
    ) {
        let (order, react, reaction) = if attack {
            (
                crate::npc::ORDER_ATTACK,
                crate::npc::REACT_ATTACK | crate::npc::REACT_PROXIMITY,
                crate::npc::ORDER_ATTACK,
            )
        } else {
            (0, 0, 0)
        };
        if let Some(n) = self.npcs.get_mut(&handle) {
            n.set_order(order, react, reaction, 0, events);
        }
        if let Ok(e) = self.entity_mut(handle) {
            e.order = Some(order);
        }
    }

    fn cult_dialog(&mut self, dialog: &str) {
        let action = crate::dialog_action::DialogAction {
            delay: 0.0,
            kind: crate::dialog_action::DialogActionKind::Start(
                crate::dialog_action::StartDialogAction {
                    dialog: custom_hash(dialog),
                    owner: custom_hash("leader"),
                    position: [0.0; 3],
                    is_2d: false,
                    is_3d: false,
                    force_on_screen: false,
                },
            ),
        };
        if let Some(Err(e)) = self.world_dialog_action(&action, None) {
            self.log.push(format!("cult dialog {dialog}: {e}"));
        }
    }
    fn cult_dialog_active(&self, dialog: &str) -> bool {
        self.dialogs
            .as_ref()
            .is_some_and(|d| d.dialog_active(custom_hash(dialog)))
    }
    fn cult_dialog_done(&self, dialog: &str) -> bool {
        self.dialogs
            .as_ref()
            .is_some_and(|d| d.is_done(custom_hash(dialog), -1).unwrap_or(false))
    }

    /// Cult.TeleportWarlock: effects, ground snap (5 m down) and MoveTo.
    fn cult_teleport(&mut self, handle: u16, position: [f32; 3], direction: [f32; 3]) {
        let Some(body) = self.npcs.get(&handle).map(|n| n.body) else {
            return;
        };
        let from = self.physics.bodies[body].position;
        self.status_contact_effects
            .push(("magick_teleport_disappear", from, [1.0, 0.0, 0.0]));
        self.play_cue(crate::audio::banks::SPELLS, "magick_teleporta", Some(from));
        let mut p = position;
        let segment = magicka_sim::spray_pulse::Segment {
            origin: position,
            delta: [0.0, -5.0, 0.0],
        };
        if let Some(hit) = self.collision.collision().segment_intersect(segment, false) {
            p = hit.hit.position;
        }
        let b = &mut self.physics.bodies[body];
        p[1] += b.radius + b.length * 0.5 + 0.1;
        let o = magicka_sim::character_physics::Orientation::from_forward_up(
            direction,
            [0.0, 1.0, 0.0],
        );
        b.move_to(p, o);
        self.status_contact_effects
            .push(("magick_teleport_appear", p, [1.0, 0.0, 0.0]));
        self.play_cue(crate::audio::banks::SPELLS, "magick_teleportb", Some(p));
    }

    /// Cult.Initialize.
    pub(super) fn spawn_cult(&mut self) -> Result<CultHost, WorldError> {
        let mut positions = [[0.0; 3]; 5];
        let mut directions = [[0.0, 0.0, -1.0]; 5];
        for (i, name) in POSITIONS.iter().enumerate() {
            let m = self
                .locators
                .get(&custom_hash(name))
                .copied()
                .ok_or_else(|| format!("cult: missing locator {name}"))?;
            positions[i] = [m[12], m[13], m[14]];
            directions[i] = forward(&m);
        }
        let mut host = CultHost {
            state: CultState::Intro,
            warlocks: Vec::new(),
            spectators: Vec::new(),
            fighting: Vec::new(),
            leader: 0,
            positions,
            directions,
            total_max: 0.0,
            total_hp: 0.0,
            pause_timer: 0.0,
            spectator_show: false,
            timer: 0.0,
            release: false,
            pan: false,
            back_pan: false,
            teleported: false,
            changed_state: false,
            revive_cooldown: 0.0,
            revive_index: None,
            revivee_template: [0; 5],
            trigger_executed: false,
            last_positions: BTreeMap::new(),
            rng: 0x6375_6c74,
        };
        for j in 0..SPECTATORS {
            let m = self
                .locators
                .get(&custom_hash(&format!("spectator{j}")))
                .copied()
                .ok_or_else(|| format!("cult: missing locator spectator{j}"))?;
            let unique = match j {
                0 => Some("bill"),
                1 => Some("bull"),
                _ => None,
            };
            let handle = self.cult_spawn(
                TEMPLATES[j % 4],
                unique,
                2.0,
                [m[12], m[13], m[14]],
                forward(&m),
            )?;
            self.cult_order(handle, false, None);
            host.warlocks.push(handle);
            host.spectators.push(handle);
            host.total_max += self.cult_max(handle);
        }
        let idle_long = crate::animation_controller::animation_value("idlelong_agg").unwrap_or(0);
        let leader = self.cult_spawn(
            TEMPLATES[4],
            Some("leader"),
            f32::INFINITY,
            positions[0],
            directions[0],
        )?;
        self.cult_order(
            leader,
            false,
            Some(vec![crate::ai_event::AiEvent::Animation(
                crate::ai_event::AnimationEvent {
                    animation: idle_long,
                    idle_animation: 0,
                    delay: 0.0,
                    blend_time: 0.25,
                    trigger: 0,
                },
            )]),
        );
        host.leader = leader;
        // The first two spectators (bill, bull) start fighting.
        for slot in [1, 4] {
            let h = host.spectators.remove(0);
            if let Some(n) = self.npcs.get(&h) {
                let o = magicka_sim::character_physics::Orientation::from_forward_up(
                    directions[slot],
                    [0.0, 1.0, 0.0],
                );
                self.physics.bodies[n.body].move_to(positions[slot], o);
            }
            host.fighting.push(h);
        }
        host.total_max += self.cult_max(leader);
        self.log.push(format!("boss cult: leader {leader}"));
        Ok(host)
    }

    pub(super) fn cult_health(&self, host: &CultHost) -> (f32, f32) {
        (host.total_hp, host.total_max)
    }

    fn cult_change(&mut self, host: &mut CultHost, state: CultState) {
        // OnExit.
        match host.state {
            CultState::Spectator => {
                host.spectator_show = true;
                self.camera.lock_input = false;
            }
            CultState::Revive => {
                self.flags.input_limited = false;
                self.camera.release(1.0);
            }
            _ => {}
        }
        host.state = state;
        // OnEnter.
        match state {
            CultState::Battle => host.pause_timer = 0.0,
            CultState::Spectator => {
                self.camera.lock_input = true;
                host.timer = 0.0;
                host.pan = false;
                host.changed_state = false;
            }
            CultState::Revive => {
                let (p, d) = (host.positions[0], host.directions[0]);
                self.cult_teleport(host.leader, p, d);
                let event = crate::ai_event::AiEvent::Move(crate::ai_event::MoveEvent {
                    waypoint: p,
                    direction: d,
                    fixed_direction: true,
                    speed: 2.0,
                    delay: 0.1,
                    ..Default::default()
                });
                self.cult_order(host.leader, false, Some(vec![event]));
                host.fighting.clear();
                if let Some(n) = self.npcs.get_mut(&host.leader) {
                    n.character.combat.cannot_die_without_explicit_kill = true;
                }
                self.flags.input_limited = true;
            }
            CultState::Final => {
                host.total_max = self.cult_max(host.leader);
                self.cult_order(host.leader, true, None);
                for h in host.fighting.clone() {
                    host.total_max += self.cult_max(h);
                    self.cult_order(h, true, None);
                }
                host.revive_index = None;
            }
            CultState::Dead => {
                self.cult_dialog("cult_death");
                host.trigger_executed = false;
            }
            CultState::Intro => {}
        }
    }

    /// Revive a dead cultist as `template` at `position` facing `direction`.
    fn cult_revive(
        &mut self,
        template: usize,
        position: [f32; 3],
        direction: [f32; 3],
    ) -> Option<u16> {
        let handle = self
            .cult_spawn(
                TEMPLATES[template],
                None,
                f32::INFINITY,
                position,
                direction,
            )
            .ok()?;
        self.status_contact_effects
            .push(("magick_revive", position, direction));
        self.play_cue(
            crate::audio::banks::SPELLS,
            "magick_revive_cast",
            Some(position),
        );
        Some(handle)
    }

    /// Cult.UpdateBoss.
    pub(super) fn update_cult(&mut self, host: &mut CultHost, dt: f32, fight_started: bool) {
        if host.state == CultState::Intro && fight_started {
            self.cult_change(host, CultState::Battle);
        }
        for &h in host.warlocks.iter().chain(&host.fighting) {
            if let Some(n) = self.npcs.get(&h) {
                host.last_positions
                    .insert(h, self.physics.bodies[n.body].position);
            }
        }
        host.total_hp = 0.0;
        if host.state == CultState::Final {
            host.total_hp = host.fighting.iter().map(|&h| self.cult_hp(h)).sum();
        } else {
            host.total_hp = host.warlocks.iter().map(|&h| self.cult_hp(h)).sum();
            let dead: Vec<u16> = host
                .fighting
                .iter()
                .copied()
                .filter(|&h| self.cult_dead(h))
                .collect();
            host.fighting.retain(|h| !dead.contains(h));
        }
        host.total_hp += self.cult_hp(host.leader);
        if self.cult_hp(host.leader) <= 0.0 && host.state != CultState::Dead {
            self.cult_change(host, CultState::Dead);
        }
        match host.state {
            CultState::Battle => self.cult_battle(host, dt),
            CultState::Spectator => self.cult_spectator(host, dt),
            CultState::Revive => self.cult_revive_state(host),
            CultState::Final => self.cult_final(host, dt),
            CultState::Dead => {
                if !host.trigger_executed && self.cult_dialog_done("cult_death") {
                    if let Some(n) = self.npcs.get_mut(&host.leader) {
                        n.character.combat.cannot_die_without_explicit_kill = false;
                    }
                    let _ = crate::entity_command::EntityCommandHost::kill(self, host.leader);
                    host.trigger_executed = true;
                    self.pending_triggers
                        .push((custom_hash("boss_defeated"), None));
                }
            }
            CultState::Intro => {}
        }
    }

    fn cult_battle(&mut self, host: &mut CultHost, dt: f32) {
        if host.fighting.iter().any(|&h| !self.cult_dead(h)) {
            return;
        }
        if !host.spectator_show {
            self.cult_change(host, CultState::Spectator);
            return;
        }
        if host.pause_timer <= 0.0 {
            host.pause_timer = PAUSE_TIME;
            return;
        }
        host.pause_timer -= dt;
        if host.pause_timer <= 1.25 && host.pause_timer > 0.0 {
            if !self.cult_dialog_active("cult_taunt1") && !self.cult_dialog_active("cult_taunt2") {
                let taunt = if Self::cult_random(host, 2) == 0 {
                    "cult_taunt1"
                } else {
                    "cult_taunt2"
                };
                self.cult_dialog(taunt);
            }
        } else if host.pause_timer <= 0.0 {
            if host.spectators.is_empty() {
                self.cult_change(host, CultState::Revive);
                return;
            }
            let n = host.spectators.len().min(4);
            for k in 0..n {
                let h = host.spectators.remove(0);
                let (p, d) = (host.positions[1 + k], host.directions[1 + k]);
                self.cult_teleport(h, p, d);
                self.cult_order(h, true, None);
                host.fighting.push(h);
            }
        }
    }

    fn cult_spectator(&mut self, host: &mut CultHost, dt: f32) {
        host.timer += dt;
        if !host.pan {
            if host.timer > 1.5 {
                if let Some(&h) = host.spectators.get(8) {
                    let p = self.cult_position(host, h);
                    self.camera.move_to(p, 2.0);
                }
                host.timer = 0.0;
                host.pan = true;
            }
            return;
        }
        if host.timer <= 2.75 {
            return;
        }
        let group: Vec<u16> = host.spectators.iter().copied().skip(8).take(4).collect();
        if host.teleported && !host.changed_state {
            self.camera.release(2.0);
            self.cult_change(host, CultState::Battle);
            host.changed_state = true;
            for _ in 0..4 {
                if host.spectators.len() > 8 {
                    let h = host.spectators.remove(8);
                    host.fighting.push(h);
                }
            }
            for h in host.fighting.clone() {
                self.cult_order(h, true, None);
            }
        } else if host.release {
            for (i, h) in group.into_iter().enumerate() {
                let (p, d) = (host.positions[1 + i % 4], host.directions[1 + i % 4]);
                self.cult_teleport(h, p, d);
            }
            host.teleported = true;
            host.timer -= 0.75;
        } else if host.back_pan {
            self.camera.move_to(host.positions[0], 1.75);
            host.release = true;
            host.timer -= 2.25;
        } else {
            for h in group {
                let p = self
                    .random_area_point(custom_hash("area_temp"))
                    .unwrap_or_else(|| self.cult_position(host, h));
                self.cult_teleport(h, p, [1.0, 0.0, 0.0]);
                self.cult_order(h, false, None);
            }
            host.back_pan = true;
            host.timer -= 1.0;
        }
    }

    fn cult_revive_state(&mut self, host: &mut CultHost) {
        let leader_position = self.cult_position(host, host.leader);
        self.camera.follow(leader_position);
        if self.cult_dialog_done("cult_domyself") {
            for i in 0..5 {
                let dead: Vec<usize> = (0..host.warlocks.len())
                    .filter(|&k| self.cult_dead(host.warlocks[k]))
                    .collect();
                if dead.is_empty() {
                    break;
                }
                let k = dead[Self::cult_random(host, dead.len())];
                let old = host.warlocks[k];
                let p = self.cult_position(host, old);
                let d = [0, 1, 2].map(|c| leader_position[c] - p[c]);
                host.revivee_template[i] = k % 4;
                if let Some(h) = self.cult_revive(k % 4, p, d) {
                    self.cult_order(h, false, None);
                    host.warlocks[k] = h;
                    host.fighting.push(h);
                }
            }
            self.flags.input_limited = false;
            self.camera.release(1.0);
            self.cult_change(host, CultState::Final);
        } else if !self.cult_dialog_active("cult_domyself") {
            self.cult_order(host.leader, false, None);
            self.cult_dialog("cult_domyself");
        }
    }

    fn cult_final(&mut self, host: &mut CultHost, dt: f32) {
        host.revive_cooldown -= dt;
        if host.revive_cooldown >= 0.0 {
            return;
        }
        let leader_events = self
            .npcs
            .get(&host.leader)
            .map(|n| n.ai.events().map(|e| e.len()));
        match host.revive_index {
            None => {
                let n = host.fighting.len();
                if n == 0 {
                    return;
                }
                let start = Self::cult_random(host, n);
                host.revive_index = (start..start + n)
                    .map(|i| i % n)
                    .find(|&i| self.cult_dead(host.fighting[i]));
            }
            Some(_) if leader_events == Some(None) => {
                let cast = crate::animation_controller::animation_value("cast_spell0").unwrap_or(0);
                self.cult_order(
                    host.leader,
                    false,
                    Some(vec![crate::ai_event::AiEvent::Animation(
                        crate::ai_event::AnimationEvent {
                            animation: cast,
                            idle_animation: 0,
                            delay: 0.0,
                            blend_time: 0.25,
                            trigger: 0,
                        },
                    )]),
                );
            }
            Some(index) => {
                let done = self
                    .npcs
                    .get(&host.leader)
                    .is_some_and(|n| n.ai.events().is_some_and(|e| n.ai.current_event >= e.len()));
                if !done {
                    return;
                }
                let slot = index.min(4);
                let (p, d) = (host.positions[slot], host.directions[slot]);
                if let Some(h) = self.cult_revive(host.revivee_template[slot], p, d) {
                    self.cult_order(h, true, None);
                    host.fighting[index] = h;
                }
                host.revive_index = None;
                host.revive_cooldown = REVIVE_COOLDOWN;
                self.cult_order(host.leader, true, None);
            }
        }
    }
}
