//! Modern demo host. Gameplay, inventory and respawn decisions remain here.
use crate::play_session::PlaySession;
use magicka_network::{
    control::*,
    lobby::{DmCommand, LobbySettings, ParticipantState, Role},
    security::HostSecurity,
    session::*,
    transport::LoopbackHost,
};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// NPCs in one frame (`GameFrame::valid`).
const MAX_FRAME_NPCS: usize = 4096;
/// Live entities above which a dungeon master's spawn is refused, far below what
/// frames carry, so commands cannot exhaust the host or its physics bodies.
const MAX_ENTITIES_FOR_SPAWN: usize = 2048;
/// Host ticks a one-shot presentation event stays in snapshots (1 s).
pub const EVENT_WINDOW: u64 = 60;

/// A host effect older than this when first heard of is history (a late joiner):
/// it joins at the host's time, without replaying what already happened.
const EFFECT_HISTORY_SECONDS: f32 = 1.0;
/// How far an effect's clock may drift from the host's before it is corrected.
const EFFECT_DRIFT_SECONDS: f32 = 0.5;
/// How long a moving effect keeps moving without news from the host.
const EFFECT_COAST_SECONDS: f32 = 0.2;

/// One host effect playing here.
struct RemoteEffect {
    reference: crate::particles::EffectRef,
    /// The host's last transform, when it arrived, and its velocity.
    transform: [f32; 16],
    time: f64,
    velocity: [f32; 3],
    age: f32,
}

/// Client-owned emitters driven by host identities; never steps gameplay.
///
/// Each effect runs on this computer's clock from its beginning. Emitters fire
/// when their timeline crosses a moment, so jumping the timeline to the host's
/// time would skip every burst: most of what an explosion is.
#[derive(Default)]
pub struct RemoteEffects {
    epoch: Option<u64>,
    references: BTreeMap<(u16, i32), RemoteEffect>,
}
impl RemoteEffects {
    /// Between host updates, keep moving effects moving: an emitter that only
    /// moves when a frame arrives leaves its trail in clumps.
    pub fn advance(&mut self, dt: f32, effects: &mut crate::particles::EffectSystem) {
        for effect in self.references.values_mut() {
            if effect.velocity == [0.0; 3] {
                continue;
            }
            effect.age = (effect.age + dt).min(EFFECT_COAST_SECONDS);
            let mut transform = effect.transform;
            for i in 0..3 {
                transform[12 + i] += effect.velocity[i] * effect.age;
            }
            effects.update_orientation(&mut effect.reference, transform);
        }
    }
    pub fn reconcile(
        &mut self,
        epoch: u64,
        states: &[VisualEffectState],
        time: f64,
        effects: &mut crate::particles::EffectSystem,
    ) -> Result<(), String> {
        if self.epoch != Some(epoch) {
            let library = std::mem::take(&mut effects.library);
            let multiplier = effects.spawn_multiplier;
            *effects = crate::particles::EffectSystem::new(library);
            effects.spawn_multiplier = multiplier;
            self.references.clear();
            self.epoch = Some(epoch);
        }
        self.references.retain(|&(slot, instance), effect| {
            let retained = states
                .iter()
                .any(|s| s.slot == slot && s.instance == instance);
            // One that ends by itself plays out here: this computer heard of
            // it late, so stopping it with the host would cut off its end.
            if !retained && !effects.ends_by_itself(&effect.reference) {
                effects.stop(&mut effect.reference);
            }
            retained
        });
        for state in states {
            let key = (state.slot, state.instance);
            let elapsed = (time - state.started_ms as f64 / 1000.0).max(0.0) as f32;
            let Some(effect) = self.references.get_mut(&key) else {
                // A definition this computer lacks costs that one effect, not
                // the rest of the frame.
                let Some(mut reference) = effects.start(state.definition, state.transform) else {
                    continue;
                };
                if elapsed > EFFECT_HISTORY_SECONDS {
                    effects.set_elapsed(&mut reference, elapsed);
                }
                self.references.insert(
                    key,
                    RemoteEffect {
                        reference,
                        transform: state.transform,
                        time,
                        velocity: [0.0; 3],
                        age: 0.0,
                    },
                );
                continue;
            };
            // Finished here while the host still lists it: it stays finished.
            if !effects.is_active(&effect.reference) {
                continue;
            }
            let since = (time - effect.time) as f32;
            if since > 0.0 {
                let moved: [f32; 3] =
                    std::array::from_fn(|i| state.transform[12 + i] - effect.transform[12 + i]);
                let velocity = moved.map(|m| m / since);
                let speed = velocity.iter().map(|v| v * v).sum::<f32>().sqrt();
                // A jump is a teleport, not motion to carry on with.
                effect.velocity = if speed < 200.0 { velocity } else { [0.0; 3] };
                effect.time = time;
                effect.age = 0.0;
            }
            effect.transform = state.transform;
            effects.update_orientation(&mut effect.reference, state.transform);
            if effects
                .elapsed(&effect.reference)
                .is_some_and(|here| (here - elapsed).abs() > EFFECT_DRIFT_SECONDS)
            {
                effects.set_elapsed(&mut effect.reference, elapsed);
            }
        }
        Ok(())
    }
}
#[derive(Clone)]
struct Memory {
    profile: PlayerProfile,
    items: [String; 2],
    magicks: u64,
}
/// Restore host spell storage on an existing client loadout without cast events.
/// The owned avatar and remote avatars use the same checked adapter.
pub fn mirror_player_spell_state(
    session: &mut PlaySession,
    slot: usize,
    state: &PlayerState,
) -> Result<(), String> {
    if !state.valid_attached_slots() {
        return Err("invalid mirror attachment slots".into());
    }
    if state.queue.len() > 5 || state.item_queues.iter().any(|q| q.len() > 5) {
        return Err("invalid mirror queue capacity".into());
    }
    let queue = state
        .queue
        .iter()
        .map(QueuedSpell::spell)
        .collect::<Option<Vec<_>>>()
        .ok_or("invalid mirror queue")?;
    let [weapon, staff] = state
        .item_queues
        .each_ref()
        .map(|q| q.iter().map(QueuedSpell::spell).collect::<Option<Vec<_>>>());
    let stored = [
        weapon.ok_or("invalid mirror weapon queue")?,
        staff.ok_or("invalid mirror staff queue")?,
    ];
    let avatar = session
        .world
        .players
        .get_mut(slot)
        .and_then(Option::as_mut)
        .ok_or("missing mirror player")?;
    for item_slot in 0..2 {
        if avatar.items.iter().filter(|(s, _)| *s == item_slot).count() != 1 {
            return Err("missing or ambiguous mirror item slot".into());
        }
    }
    if state
        .attached_slots
        .iter()
        .any(|slot| !avatar.items.iter().any(|(s, _)| s == slot))
    {
        return Err("unknown mirror attachment slot".into());
    }
    // Decode and check the entire state before changing any spell storage.
    avatar.conjure.queue = queue;
    for (slot, item) in &mut avatar.items {
        item.attached = state.attached_slots.binary_search(slot).is_ok();
        if !item.attached {
            item.frame = None;
        }
    }
    for (item_slot, spells) in stored.into_iter().enumerate() {
        avatar
            .items
            .iter_mut()
            .find(|(s, _)| *s == item_slot as i32)
            .unwrap()
            .1
            .spells = magicka_sim::weapon_spells::WeaponSpells::from_stored_queue(spells);
    }
    session.unlocked_magicks[slot] = state.unlocked_magicks;
    // Registered Magicks resolve through this computer's own definitions.
    session.registered_unlocked[slot] = state
        .registered_magicks
        .iter()
        .filter_map(|id| {
            let id = magicka_definitions::DefinitionId::parse(id, "base").ok()?;
            session.definitions.magicks.index_of(&id).map(|i| i.0)
        })
        .collect();
    Ok(())
}
/// Load the host's advertised level before resolving its scene and player mirrors.
/// Returns whether a new level was loaded so presentation can invalidate old assets.
pub fn mirror_world(session: &mut PlaySession, frame: &GameFrame) -> Result<bool, String> {
    if !frame.valid() {
        return Err("invalid mirror world snapshot".into());
    }
    let changed = network_level(session.level_file())? != frame.level;
    if changed {
        session
            .change_level(&frame.level)
            .map_err(|e| format!("mirror level: {e:?}"))?;
        session.disable_scripts();
    }
    if frame.physics.scene != session.scene_name() {
        session
            .change_scene_named(&frame.physics.scene, "start")
            .map_err(|e| format!("mirror scene: {e:?}"))?;
    }
    Ok(changed)
}

/// Observer clients (no player slot) keep their dormant local avatar in slot
/// 0 and mirror the host's slot 0 here instead.
pub const OBSERVER_VIEW_SLOT: usize = magicka_network::lobby::PLAYER_SLOT_CAP;

/// Client mirror lifetimes. Server slots identify ownership, not avatar instances.
#[derive(Default)]
pub struct RemotePlayers {
    epoch: Option<u64>,
    handles: BTreeMap<usize, u16>,
    own: Option<Option<usize>>,
}
impl RemotePlayers {
    /// `own_slot`: this client's player slot, or `None` for spectators,
    /// dungeon masters and invaders (and players waiting for their avatar).
    pub fn reconcile(
        &mut self,
        frame: &GameFrame,
        own_slot: Option<usize>,
        session: &mut PlaySession,
    ) -> Result<(), String> {
        if !frame.valid()
            || own_slot.is_some_and(|own| !frame.players.iter().any(|p| p.slot == own))
        {
            return Err("invalid remote player snapshot".into());
        }
        let view = own_slot.unwrap_or(OBSERVER_VIEW_SLOT);
        let desired: BTreeMap<_, _> = frame
            .players
            .iter()
            .filter(|p| Some(p.slot) != own_slot)
            .map(|p| (if p.slot == 0 { view } else { p.slot }, p))
            .collect();
        // A role change remaps every mirror.
        let changed_epoch = self.epoch != Some(frame.physics.epoch) || self.own != Some(own_slot);
        self.own = Some(own_slot);
        for (&slot, &handle) in &self.handles {
            if (changed_epoch || desired.get(&slot).is_none_or(|p| p.handle != handle))
                && session.world.players.get(slot).is_some_and(Option::is_some)
            {
                session
                    .depart_network_player(slot)
                    .map_err(|e| format!("mirror departure: {e:?}"))?;
            }
        }
        for (&slot, player) in &desired {
            if changed_epoch
                || self.handles.get(&slot) != Some(&player.handle)
                || session.world.players.get(slot).is_none_or(Option::is_none)
            {
                if session.world.players.get(slot).is_some_and(Option::is_some) {
                    session
                        .depart_network_player(slot)
                        .map_err(|e| format!("mirror replacement: {e:?}"))?;
                }
                session
                    .join_player_at_start(slot, &player.profile.robe)
                    .map_err(|e| format!("mirror join: {e:?}"))?;
            }
            session
                .set_player_loadout_if_changed(
                    slot,
                    &player.profile.robe,
                    &player.items[1],
                    &player.items[0],
                )
                .map_err(|e| format!("mirror loadout: {e:?}"))?;
            mirror_player_spell_state(session, slot, player)?;
        }
        self.handles = desired.iter().map(|(&slot, p)| (slot, p.handle)).collect();
        self.epoch = Some(frame.physics.epoch);
        Ok(())
    }
}
pub struct HostedSession {
    pub transport: LoopbackHost,
    pub identity: u64,
    authority: InputAuthority,
    epoch: u64,
    tick: u64,
    dedicated: bool,
    /// A wizardless host's own role (dungeon master, spectator).
    host_role: Option<Role>,
    /// The ceiling for in-game player limit changes.
    limit_ceiling: u16,
    memory: BTreeMap<usize, Memory>,
    starting_magicks: crate::slots::PerSlot<u64>,
    profile_sequences: BTreeMap<usize, u64>,
    respawn_sequences: BTreeMap<usize, u64>,
    last_input: BTreeMap<usize, Instant>,
    /// Newest input sequence applied per seat (reconciliation, protocol 11).
    processed_input: BTreeMap<usize, u64>,
    /// Connected non-players by seat (rewrite extension).
    observers: BTreeMap<usize, Observer>,
    /// Joins after this point are hot joins (spawn beside the party).
    started: bool,
    /// Extra local (same-machine) players on a playable host.
    local: BTreeMap<usize, PlayerProfile>,
    /// LAN advertisement (rewrite extension).
    beacon: magicka_network::discovery::Beacon,
    /// Mod UI net tables published to clients (protocol 12).
    mod_ui: Vec<ModUiEntry>,
    /// Recent one-shot presentation events, kept briefly for loss.
    events: std::collections::VecDeque<PresentationEventState>,
    next_event: u64,
    /// Per-wizard one-shot pulses (protocol 14), serialised in frames.
    pulses: crate::network_world::PulseTracker,
    /// Recently changed ground decals, serialised in frames (protocol 17).
    decals: crate::network_world::DecalTracker,
    advert_name: String,
    /// Joining needs the game's passphrase (shown in LAN beacons).
    private: bool,
}
/// A spectator, dungeon master or invader connection.
#[derive(Clone, Debug)]
struct Observer {
    role: Role,
    name: String,
    queued: bool,
    possessed: Option<u16>,
    /// Where the pilot last was; invaders respawn in the nearest enemy.
    last_position: Option<[f32; 3]>,
    dm_sequence: u64,
}
fn network_level(level: &str) -> Result<String, String> {
    if std::path::Path::new(level).is_absolute() {
        std::path::Path::new(level)
            .file_name()
            .and_then(|s| s.to_str())
            .map(str::to_string)
            .ok_or_else(|| "invalid level".into())
    } else {
        Ok(level.replace('\\', "/"))
    }
}
fn items(session: &PlaySession, slot: usize) -> [String; 2] {
    std::array::from_fn(|i| {
        session.world.players[slot]
            .as_ref()
            .unwrap()
            .items
            .iter()
            .find(|(s, _)| *s == i as i32)
            .map(|(_, item)| item.definition.name.clone())
            .unwrap_or_default()
    })
}
impl HostedSession {
    pub fn has_players(&self) -> bool {
        !self.memory.is_empty()
    }
    /// The host has no wizard of its own (slot 0 goes to the first joiner).
    pub fn is_dedicated(&self) -> bool {
        self.dedicated
    }
    /// A wizardless host's own role, listed in LAN beacons.
    pub fn set_host_role(&mut self, role: Option<Role>) {
        self.host_role = role;
    }
    pub fn bind(
        address: SocketAddr,
        level: &str,
        dedicated: bool,
        profile: PlayerProfile,
        session: &PlaySession,
    ) -> Result<Self, String> {
        Self::bind_lobby(
            address,
            level,
            dedicated,
            profile,
            session,
            Default::default(),
        )
    }
    pub fn bind_lobby(
        address: SocketAddr,
        level: &str,
        dedicated: bool,
        profile: PlayerProfile,
        session: &PlaySession,
        lobby: LobbySettings,
    ) -> Result<Self, String> {
        Self::bind_secured(
            address,
            level,
            dedicated,
            profile,
            session,
            lobby,
            HostSecurity::default(),
        )
    }
    pub fn bind_secured(
        address: SocketAddr,
        level: &str,
        dedicated: bool,
        profile: PlayerProfile,
        session: &PlaySession,
        lobby: LobbySettings,
        security: HostSecurity,
    ) -> Result<Self, String> {
        if !profile.valid() {
            return Err("invalid host profile".into());
        }
        let advert_name = if dedicated {
            "Dedicated server".to_string()
        } else {
            format!("{}'s game", profile.name)
        };
        let identity = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos() as u64
            | 1;
        let epoch = session
            .physics_frame(identity, 0)
            .map_err(|e| format!("physics snapshot: {e:?}"))?
            .epoch;
        let mut memory = BTreeMap::new();
        if !dedicated {
            memory.insert(
                0,
                Memory {
                    profile,
                    items: items(session, 0),
                    magicks: session.unlocked_magicks[0],
                },
            );
        }
        if network_level(level)? != network_level(session.level_file())? {
            return Err("host level differs from loaded session".into());
        }
        let private = security.key.is_private();
        Ok(Self {
            private,
            transport: LoopbackHost::bind_secured(address, dedicated, lobby, security)?,
            identity,
            authority: if dedicated {
                InputAuthority::dedicated(identity, epoch)
            } else {
                InputAuthority::new(identity, epoch)
            }
            .map_err(|e| format!("authority: {e:?}"))?,
            epoch,
            tick: 0,
            dedicated,
            host_role: None,
            limit_ceiling: LobbySettings::in_game_limit_ceiling(lobby.max_players),
            memory,
            starting_magicks: session.unlocked_magicks.clone(),
            profile_sequences: Default::default(),
            respawn_sequences: Default::default(),
            last_input: Default::default(),
            processed_input: Default::default(),
            observers: Default::default(),
            started: false,
            local: Default::default(),
            beacon: Default::default(),
            mod_ui: Vec::new(),
            events: Default::default(),
            next_event: 0,
            pulses: Default::default(),
            decals: Default::default(),
            advert_name,
        })
    }
    fn sync_epoch(&mut self, session: &PlaySession) -> Result<(), String> {
        let epoch = session
            .physics_frame(self.identity, self.tick)
            .map_err(|e| format!("physics: {e:?}"))?
            .epoch;
        if epoch != self.epoch {
            self.authority
                .advance_epoch(epoch)
                .map_err(|e| format!("epoch: {e:?}"))?;
            self.epoch = epoch;
            self.tick = 0;
        }
        Ok(())
    }
    /// Mirror each player's profile name and colour into the session (the
    /// statistics sheets and Versus panels show them).
    pub fn publish_profiles(&self, session: &mut PlaySession) {
        let slots = session
            .world
            .players
            .len()
            .max(session.player_profiles.len())
            .max(crate::slots::ORIGINAL_PLAYER_LIMIT);
        for slot in 0..slots {
            session.player_profiles[slot] = self
                .memory
                .get(&slot)
                .map(|m| (m.profile.name.clone(), m.profile.colour));
        }
    }

    pub fn capture(&mut self, session: &PlaySession) {
        for (&slot, memory) in &mut self.memory {
            if session.world.players.get(slot).is_some_and(Option::is_some) {
                memory.items = items(session, slot);
                memory.magicks |= session.unlocked_magicks[slot];
            }
        }
    }
    pub fn poll(&mut self, session: &mut PlaySession) -> Result<(), String> {
        self.poll_at(session, Instant::now())
    }
    pub fn poll_at(&mut self, session: &mut PlaySession, now: Instant) -> Result<(), String> {
        if let Some(next) = session.next_level() {
            self.capture(session);
            session
                .change_level(next)
                .map_err(|e| format!("campaign transition: {e:?}"))?;
            // A new level is a join point even with hot join locked.
            self.transport.begin_round();
        }
        session.world.boss_scale = self.transport.lobby().boss_scale;
        self.sync_epoch(session)?;
        self.transport.update_at(now);
        for seat in self.transport.take_departed_observers() {
            if let Some(observer) = self.observers.remove(&seat) {
                if let Some(npc) = observer.possessed {
                    session.world.release_possession(npc);
                }
            }
            self.authority.release_slot(seat).ok();
            self.last_input.remove(&seat);
            self.processed_input.remove(&seat);
        }
        for slot in self.transport.take_departed() {
            // A hello and cancellation can arrive in the same transport poll,
            // before this session has created an avatar or authority for it.
            if !self.memory.contains_key(&slot) {
                continue;
            }
            // A departing profile's player stays in the save (and returns
            // if that profile rejoins).
            session.park_player(slot);
            if let Err(e) = session.depart_network_player(slot) {
                session.world.log.push(format!("departure: {e:?}"));
            }
            self.memory.remove(&slot);
            if let Err(e) = self.authority.release_slot(slot) {
                session
                    .world
                    .log
                    .push(format!("departure authority: {e:?}"));
            }
            self.profile_sequences.remove(&slot);
            self.respawn_sequences.remove(&slot);
            self.last_input.remove(&slot);
            self.processed_input.remove(&slot);
        }
        for slot in self.transport.player_slots() {
            if self.memory.contains_key(&slot) {
                continue;
            }
            // Protocol 11: the joiner's profile was submitted with its hello
            // and is validated before any avatar exists (never a placeholder).
            let submitted = self.transport.admitted_profile(slot);
            if submitted
                .as_ref()
                .and_then(|s| s.profile_id.as_deref())
                .and_then(|id| session.profile_slot(id))
                .is_some_and(|owner| owner != slot)
            {
                self.transport
                    .refuse_seat(slot, "that profile is already playing on this host");
                continue;
            }
            let profile = match &submitted {
                Some(submit) => submit.profile.clone(),
                // A raw connection without a profile (tools, tests) plays as
                // a plain wizard under the name it gave.
                None => PlayerProfile {
                    name: self
                        .transport
                        .seat_name(slot)
                        .filter(|n| !n.trim().is_empty() && n != "Player")
                        .unwrap_or_else(|| format!("Player {}", slot + 1)),
                    ..Default::default()
                },
            };
            // Hot join (rewrite extension): after the game began, joiners
            // arrive beside a living party member, not at the level start.
            let hot = self.started && session.party_position().is_some();
            let joined = if self.dedicated && slot == 0 {
                session
                    .fresh_network_player_at_start(0)
                    .map_err(|e| format!("first player spawn: {e:?}"))
                    .and_then(|_| {
                        if hot {
                            session
                                .move_near_party(0)
                                .map_err(|e| format!("hot join: {e:?}"))?;
                        }
                        if profile.robe == "Wizard" {
                            Ok(())
                        } else {
                            let items = items(session, 0);
                            session
                                .set_player_loadout(0, &profile.robe, &items[1], &items[0])
                                .map_err(|e| format!("{e:?}"))
                        }
                    })
            } else if session.world.players.get(slot).is_none_or(Option::is_none) {
                if hot {
                    session.join_player_near_party(slot, &profile.robe)
                } else {
                    session.join_player_at_start(slot, &profile.robe)
                }
                .map(drop)
                .map_err(|e| format!("{e:?}"))
            } else {
                Ok(())
            };
            if let Err(error) = joined {
                // The selected robe cannot be loaded here: refuse with a
                // reason instead of substituting a generic wizard.
                session.world.log.push(format!("join refused: {error}"));
                if session.world.players.get(slot).is_some_and(Option::is_some) {
                    session.depart_network_player(slot).ok();
                }
                self.transport
                    .refuse_seat(slot, "the selected robe is not available on this host");
                self.authority.release_slot(slot).ok();
                continue;
            }
            session.unlocked_magicks[slot] = self.starting_magicks[slot];
            if let Some(id) = submitted.as_ref().and_then(|s| s.profile_id.as_ref()) {
                // Trusted-LAN restore of this profile's parked player.
                if let Err(error) = session.claim_parked(slot, id, &profile.robe) {
                    session
                        .world
                        .log
                        .push(format!("profile restore refused: {error:?}"));
                    session.depart_network_player(slot).ok();
                    self.transport
                        .refuse_seat(slot, "the saved profile could not be restored");
                    self.authority.release_slot(slot).ok();
                    continue;
                }
            }
            self.memory.insert(
                slot,
                Memory {
                    profile,
                    items: items(session, slot),
                    magicks: session.unlocked_magicks[slot],
                },
            );
            self.last_input.insert(slot, now);
        }
        self.sync_observers(session);
        self.capture(session);
        self.publish_profiles(session);
        for owned in self.transport.take_profiles() {
            let slot = owned.slot;
            let error = if !self.memory.contains_key(&slot)
                || session.world.players.get(slot).is_none_or(Option::is_none)
            {
                Some("only players have a profile".into())
            } else if owned.request.sequence == 0
                || owned.request.sequence <= self.profile_sequences.get(&slot).copied().unwrap_or(0)
            {
                Some("outdated player setup request".into())
            } else if !owned.request.profile.valid() {
                Some("invalid player name, robe or colour".into())
            } else if owned
                .request
                .profile_id
                .as_deref()
                .and_then(|id| session.profile_slot(id))
                .is_some_and(|owner| owner != slot)
            {
                Some("that profile is already playing on this host".into())
            } else if owned.request.profile_id.as_ref().is_some_and(|id| {
                session.player_ids[slot]
                    .as_ref()
                    .is_some_and(|current| current != id)
            }) {
                Some("leave the game before choosing a different profile".into())
            } else {
                self.profile_sequences.insert(slot, owned.request.sequence);
                let memory = &self.memory[&slot];
                let dead = session.world.players[slot]
                    .as_ref()
                    .unwrap()
                    .character
                    .combat
                    .dead;
                let health = session.world.players[slot]
                    .as_ref()
                    .unwrap()
                    .character
                    .combat
                    .health
                    .clone();
                let result = if memory.profile.robe == owned.request.profile.robe {
                    Ok(())
                } else {
                    session.set_player_loadout(
                        slot,
                        &owned.request.profile.robe,
                        &memory.items[1],
                        &memory.items[0],
                    )
                };
                let result = result.and_then(|()| match &owned.request.profile_id {
                    Some(id) => session
                        .claim_parked(slot, id, &owned.request.profile.robe)
                        .map(drop),
                    None => Ok(()),
                });
                match result {
                    Ok(()) => {
                        session.world.players[slot]
                            .as_mut()
                            .unwrap()
                            .character
                            .combat
                            .health = health;
                        session.world.players[slot]
                            .as_mut()
                            .unwrap()
                            .character
                            .combat
                            .dead = dead;
                        self.memory.get_mut(&slot).unwrap().profile = owned.request.profile;
                        None
                    }
                    Err(_) => Some("selected robe could not be loaded".into()),
                }
            };
            // Clients decide what arrives and when, so nothing about a request
            // may stop the host. (The asker can be gone by now.)
            if let Err(e) = self.transport.send_profile_reply(
                slot,
                ProfileReply {
                    sequence: owned.request.sequence,
                    error,
                },
            ) {
                session.world.log.push(format!("profile reply: {e}"));
            }
        }
        for owned in self.transport.take_inputs() {
            // Samples a lost packet carried arrive with the next one.
            let Ok(samples) = self
                .authority
                .accept_with_history(owned.slot, &owned.command)
            else {
                continue;
            };
            for controls in samples {
                if let Some(observer) = self.observers.get(&owned.slot) {
                    if let Some(npc) = observer.possessed {
                        session.pilot_npc(npc, controls);
                        self.last_input.insert(owned.slot, now);
                    }
                } else if self.memory.contains_key(&owned.slot) {
                    // A mod script driving this wizard holds its controls.
                    if !session.scripted_input[owned.slot] {
                        if let Err(e) = session.handle_remote_input(owned.slot, controls) {
                            session.world.log.push(format!("player input: {e:?}"));
                        }
                    }
                    self.last_input.insert(owned.slot, now);
                }
            }
            self.processed_input
                .insert(owned.slot, owned.command.sequence);
        }
        self.apply_dm_requests(session);
        self.update_invaders(session);
        for owned in self.transport.take_pickups() {
            if !self.memory.contains_key(&owned.slot) {
                continue;
            }
            let outcome = match self.authority.accept_pickup(owned.slot, &owned.request) {
                Ok(target) => session.pickup_remote(owned.slot, target),
                Err(error) => PickupOutcome::Rejected(PickupRejection::Authority(error)),
            };
            if let Err(e) = self
                .transport
                .send_pickup_receipt(owned.slot, owned.request.receipt(outcome))
            {
                session.world.log.push(format!("pickup receipt: {e}"));
            }
        }
        for owned in self.transport.take_respawns() {
            let request = owned.request;
            if request.session != self.identity
                || request.epoch != self.epoch
                || request.sequence == 0
                || request.sequence
                    <= self
                        .respawn_sequences
                        .get(&owned.slot)
                        .copied()
                        .unwrap_or(0)
                || !self.memory.contains_key(&owned.slot)
                // Off unless this host opted in: a dead client coming back at
                // will is not in the original game, and cheats in normal play.
                || !self.transport.lobby().debug_respawn
            {
                continue;
            }
            self.respawn_sequences.insert(owned.slot, request.sequence);
            if session
                .world
                .players
                .get(owned.slot)
                .and_then(Option::as_ref)
                .is_some_and(|p| p.dead())
                // Versus decides respawns; the demo's request would skip its
                // timers and limits.
                && session.world.versus().is_none()
            {
                if let Err(e) = self.respawn_player(owned.slot, session) {
                    session.world.log.push(format!("respawn: {e}"));
                }
            }
        }
        // A stopped/disconnected client cannot leave movement or casting held.
        for (&slot, input) in &self.last_input {
            if now.saturating_duration_since(*input) > Duration::from_millis(500) {
                if let Some(observer) = self.observers.get(&slot) {
                    if let Some(npc) = observer.possessed {
                        session.pilot_npc(npc, Controls::default());
                    }
                } else if !session.scripted_input[slot] {
                    // A wizard can be gone while its input timer is not.
                    if let Err(e) = session.handle_remote_input(slot, Controls::default()) {
                        session.world.log.push(format!("input timeout: {e:?}"));
                    }
                }
            }
        }
        self.capture(session);
        self.started = true;
        // Only this machine's players steer the host's camera; remote
        // players follow their own cameras on their own computers.
        session.camera_slots = (!self.dedicated).then(|| {
            std::iter::once(0)
                .chain(self.local.keys().copied())
                .collect()
        });
        Ok(())
    }

    /// Seat an extra local (same-machine) player on a playable host:
    /// spawn beside the party and own the slot like a remote player, minus
    /// the connection. Returns the slot.
    pub fn add_local_player(
        &mut self,
        profile: PlayerProfile,
        session: &mut PlaySession,
    ) -> Result<usize, String> {
        if self.dedicated || !profile.valid() {
            return Err("local players need a playable host".into());
        }
        let slot = self
            .transport
            .free_local_slot()
            .ok_or("the lobby is full")?;
        session
            .join_player_near_party(slot, &profile.robe)
            .map_err(|e| format!("local join: {e:?}"))?;
        session.unlocked_magicks[slot] = self.starting_magicks[slot];
        self.memory.insert(
            slot,
            Memory {
                profile: profile.clone(),
                items: items(session, slot),
                magicks: session.unlocked_magicks[slot],
            },
        );
        self.local.insert(slot, profile);
        self.transport.set_local_slots(self.local.keys().copied());
        Ok(slot)
    }
    pub fn remove_local_player(
        &mut self,
        slot: usize,
        session: &mut PlaySession,
    ) -> Result<(), String> {
        if self.local.remove(&slot).is_none() {
            return Err("not a local player".into());
        }
        session.park_player(slot);
        session
            .depart_network_player(slot)
            .map_err(|e| format!("local leave: {e:?}"))?;
        self.memory.remove(&slot);
        self.transport.set_local_slots(self.local.keys().copied());
        Ok(())
    }
    pub fn local_slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.local.keys().copied()
    }

    /// Track non-player seats from the transport.
    fn sync_observers(&mut self, session: &mut PlaySession) {
        let seats = self.transport.seats();
        for seat in &seats {
            if seat.role == Role::Player {
                continue;
            }
            let observer = self.observers.entry(seat.seat).or_insert_with(|| Observer {
                role: seat.role,
                name: seat.name.clone(),
                queued: seat.queued,
                possessed: None,
                last_position: session.party_position(),
                dm_sequence: 0,
            });
            if observer.role != seat.role {
                // Role change: drop any piloted body.
                if let Some(npc) = observer.possessed.take() {
                    session.world.release_possession(npc);
                }
            }
            observer.role = seat.role;
            observer.name = seat.name.clone();
            observer.queued = seat.queued;
        }
    }

    /// Invaders always pilot an enemy: on joining and after their body dies,
    /// they take over the nearest living hostile NPC.
    fn update_invaders(&mut self, session: &mut PlaySession) {
        for observer in self.observers.values_mut() {
            if let Some(npc) = observer.possessed {
                if session.world.possessed.contains_key(&npc) {
                    if let Some(n) = session.world.npcs.get(&npc) {
                        observer.last_position =
                            Some(session.world.physics.bodies[n.body].position);
                    }
                    continue;
                }
                observer.possessed = None;
            }
            if observer.role != Role::Invader {
                continue;
            }
            let near = observer
                .last_position
                .or_else(|| session.party_position())
                .unwrap_or([0.0; 3]);
            if let Some(&npc) = session.world.invasion_candidates(near).first() {
                if session.world.possess(npc) {
                    observer.possessed = Some(npc);
                }
            }
        }
    }

    fn apply_dm_requests(&mut self, session: &mut PlaySession) {
        for owned in self.transport.take_dm_requests() {
            let Some(observer) = self.observers.get_mut(&owned.seat) else {
                continue;
            };
            if owned.request.sequence <= observer.dm_sequence {
                continue;
            }
            observer.dm_sequence = owned.request.sequence;
            let dm = observer.role == Role::DungeonMaster;
            let result = match owned.request.command {
                DmCommand::Possess { npc } => {
                    let allowed = dm
                        || (observer.role == Role::Invader
                            && session.world.invasion_candidates([0.0; 3]).contains(&npc));
                    if allowed && session.world.possess(npc) {
                        if let Some(old) = observer.possessed.replace(npc) {
                            session.world.release_possession(old);
                        }
                        Ok(())
                    } else {
                        Err("cannot possess that character".to_string())
                    }
                }
                DmCommand::Release => {
                    if let Some(old) = observer.possessed.take() {
                        session.world.release_possession(old);
                    }
                    Ok(())
                }
                _ if !dm => Err("dungeon master only".into()),
                DmCommand::SpawnNpc { .. } | DmCommand::Spawn { .. }
                    if session.world.entities.iter().filter(|e| !e.removed).count()
                        >= MAX_ENTITIES_FOR_SPAWN =>
                {
                    Err("too many entities to spawn more".to_string())
                }
                DmCommand::SpawnNpc { template, position } => {
                    session.spawn_npc_at(&template, position).map(drop)
                }
                DmCommand::Spawn {
                    spawnable,
                    transform,
                    order,
                    movable,
                } => session
                    .editor_spawn(
                        &spawnable,
                        transform,
                        &crate::play_session::EditorSpawnOptions { order, movable },
                    )
                    .map(|handle| {
                        let at = &transform[12..15];
                        session.world.log.push(format!(
                            "dungeon master: spawned {spawnable} {handle:?} at {at:.1?}"
                        ));
                    }),
                DmCommand::Move { handle, transform } => session.editor_move(handle, transform),
                DmCommand::Remove { handle } => session.editor_remove(handle),
                DmCommand::Smite { handle } => session.smite(handle),
                DmCommand::Heal { handle } => session.heal_character(handle),
                DmCommand::Kick { seat } if seat == owned.seat => {
                    Err("cannot kick yourself".to_string())
                }
                DmCommand::Kick { seat } => {
                    if self
                        .transport
                        .kick(seat, "Removed from the game by a dungeon master.")
                    {
                        Ok(())
                    } else {
                        Err("no such connection".to_string())
                    }
                }
                DmCommand::SetHotJoin(open) => {
                    let lobby = LobbySettings {
                        hot_join: open,
                        ..self.transport.lobby()
                    };
                    self.transport.set_lobby(lobby)
                }
                DmCommand::SetPlayerLimit(limit) if limit > self.limit_ceiling => Err(format!(
                    "player limit {limit} is above this server's maximum of {}",
                    self.limit_ceiling
                )),
                DmCommand::SetPlayerLimit(limit) => {
                    let lobby = LobbySettings {
                        max_players: limit,
                        ..self.transport.lobby()
                    };
                    self.transport.set_lobby(lobby)
                }
            };
            if let Err(e) = result {
                session.world.log.push(format!("dungeon master: {e}"));
            }
        }
    }

    pub fn lobby(&self) -> LobbySettings {
        self.transport.lobby()
    }
    /// The host removes a connection from the game.
    pub fn kick(&mut self, seat: usize) -> bool {
        self.transport
            .kick(seat, "Removed from the game by the host.")
    }
    /// Everyone who can be removed, by connection: (seat, name).
    pub fn kickable(&mut self) -> Vec<(usize, String)> {
        let names: BTreeMap<usize, String> = self
            .memory
            .iter()
            .map(|(&slot, m)| (slot, m.profile.name.clone()))
            .collect();
        self.transport
            .seats()
            .into_iter()
            .map(|s| {
                let name = names.get(&s.seat).cloned().unwrap_or(s.name);
                (s.seat, name)
            })
            .collect()
    }
    pub fn set_lobby(&mut self, lobby: LobbySettings) -> Result<(), String> {
        self.transport.set_lobby(lobby)
    }

    fn participants(&self) -> Vec<ParticipantState> {
        self.observers
            .iter()
            .map(|(&seat, o)| ParticipantState {
                seat,
                role: o.role,
                name: o.name.clone(),
                possessed: o.possessed,
                queued: o.queued,
            })
            .collect()
    }
    pub fn set_host_profile(
        &mut self,
        profile: PlayerProfile,
        session: &mut PlaySession,
    ) -> Result<(), String> {
        if self.dedicated || !profile.valid() {
            return Err("host profile unavailable".into());
        }
        self.capture(session);
        let memory = &self.memory[&0];
        let dead = session.avatar().character.combat.dead;
        let health = session.avatar().character.combat.health.clone();
        if profile.robe != memory.profile.robe {
            session
                .set_player_loadout(0, &profile.robe, &memory.items[1], &memory.items[0])
                .map_err(|e| format!("loadout: {e:?}"))?;
        }
        session.world.players[0]
            .as_mut()
            .unwrap()
            .character
            .combat
            .health = health;
        session.world.players[0]
            .as_mut()
            .unwrap()
            .character
            .combat
            .dead = dead;
        self.memory.get_mut(&0).unwrap().profile = profile;
        Ok(())
    }
    pub fn respawn_player(&mut self, slot: usize, session: &mut PlaySession) -> Result<(), String> {
        self.capture(session);
        let memory = self
            .memory
            .get(&slot)
            .ok_or("player is not owned by this server")?
            .clone();
        session
            .respawn_player(slot, &memory.profile.robe)
            .map_err(|e| format!("respawn: {e:?}"))?;
        session.unlocked_magicks[slot] |= memory.magicks;
        Ok(())
    }
    pub fn restart(&mut self, session: &mut PlaySession) -> Result<(), String> {
        self.capture(session);
        let restored = session
            .restart_at_checkpoint()
            .map_err(|e| format!("checkpoint: {e:?}"))?;
        if !restored {
            // Before TriggerCheckpoint, Restart(Checkpoint) returns to the
            // level's initial state. Respawning alone leaves the defeated
            // scene and its end-game restart request alive.
            let level = session.level_file().to_string();
            session
                .change_level(&level)
                .map_err(|e| format!("initial checkpoint: {e:?}"))?;
        }
        for (&slot, memory) in &self.memory {
            if restored {
                session
                    .reset_player_equipment(slot, &memory.profile.robe)
                    .map_err(|e| format!("restore equipment: {e:?}"))?;
            } else {
                session
                    .respawn_player(slot, &memory.profile.robe)
                    .map_err(|e| format!("respawn: {e:?}"))?;
            }
            session.unlocked_magicks[slot] |= memory.magicks;
        }
        // Restarting is a new round: queued joiners take their slots.
        self.transport.begin_round();
        self.sync_epoch(session)
    }
    pub fn frame(&self, session: &PlaySession) -> Result<GameFrame, String> {
        let physics = session
            .physics_frame(self.identity, self.tick)
            .map_err(|e| format!("physics snapshot: {e:?}"))?;
        let mut players = Vec::new();
        for (&slot, memory) in &self.memory {
            let Some(player) = &session.world.players[slot] else {
                continue;
            };
            let body = &session.world.physics.bodies[player.body];
            let c = &player.motion.controller;
            players.push(PlayerState {
                slot,
                handle: player.handle,
                profile: memory.profile.clone(),
                position: body.position,
                right: body.orientation.right,
                up: body.orientation.up,
                backward: body.orientation.backward,
                animation: c.current_animation,
                animation_time: c.time,
                animation_speed: c.speed,
                health: player.character.combat.health.hit_points,
                dead: player.dead(),
                items: items(session, slot),
                attached_slots: {
                    let mut slots: Vec<_> = player
                        .items
                        .iter()
                        .filter_map(|(slot, item)| item.attached.then_some(*slot))
                        .collect();
                    slots.sort_unstable();
                    slots
                },
                unlocked_magicks: session.unlocked_magicks[slot],
                last_input: self.processed_input.get(&slot).copied().unwrap_or(0),
                registered_magicks: session.registered_unlocked[slot]
                    .iter()
                    .filter_map(|&i| {
                        session
                            .definitions
                            .magicks
                            .at(magicka_definitions::RegistryIndex(i))
                            .map(|m| m.common.id.to_string())
                    })
                    .collect(),
                queue: player.conjure.queue.iter().map(QueuedSpell::from).collect(),
                item_queues: std::array::from_fn(|slot| {
                    player
                        .items
                        .iter()
                        .find(|(s, _)| *s == slot as i32)
                        .map(|(_, item)| {
                            item.spells.queue().iter().map(QueuedSpell::from).collect()
                        })
                        .unwrap_or_default()
                }),
            });
        }
        let mut frame = GameFrame {
            mod_ui: self.mod_ui.clone(),
            time: session.play_time,
            events: self.events.iter().cloned().collect(),
            lobby: self.transport.lobby(),
            participants: self.participants(),
            rules: Some(crate::network_rules::rules_state(session)),
            world: {
                let mut world = crate::network_world::world_presentation(session);
                world.pulses = self.pulses.recent();
                world.decals = self.decals.recent();
                world.trim_to_limits();
                world
            },
            physics,
            level: network_level(session.level_file())?,
            players,
            npcs: session
                .world
                .entities
                .iter()
                .filter_map(|entity| {
                    if !entity.added
                        || entity.removed
                        || session.world.overkilled.contains(&entity.handle)
                    {
                        return None;
                    }
                    let npc = session.world.npcs.get(&entity.handle)?;
                    let body = &session.world.physics.bodies[npc.body];
                    let mut transform = entity.orientation;
                    transform[12..15].copy_from_slice(&body.feet());
                    let c = &npc.motion.controller;
                    Some(NpcState {
                        handle: entity.handle,
                        template: entity.template_name.clone()?,
                        transform,
                        clip: c.clip_name().map(str::to_owned),
                        clip_time: c.time,
                        looping: c.is_looping(),
                        health: npc.character.combat.health.hit_points,
                        dead: npc.dead(),
                        max_health: npc.character.combat.health.max_hit_points,
                        health_bars: npc.character.definition.health_bars,
                        radius: npc.character.definition.radius,
                    })
                })
                .collect(),
            effects: session
                .effects
                .presentation()
                .into_iter()
                .map(|e| VisualEffectState {
                    slot: e.reference.id as u16,
                    instance: e.reference.hash,
                    definition: e.definition,
                    transform: e.transform,
                    started_ms: ((session.play_time - f64::from(e.elapsed)) * 1000.0)
                        .round()
                        .max(0.0) as i64,
                })
                .collect(),
        };
        // Beyond what a frame carries, clients do not see the rest.
        frame.npcs.truncate(MAX_FRAME_NPCS);
        if let Some(part) = frame.invalid_part() {
            return Err(format!("invalid authoritative game snapshot: {part}"));
        }
        Ok(frame)
    }
    /// Advertise the match's mod content: clients whose rules hash differs
    /// are refused at admission (plan workstream C).
    pub fn set_content(&mut self, rules_hash: Option<String>, mods: Vec<String>) {
        let info = magicka_network::transport::ServerInfo {
            rules_hash,
            mods,
            ..self.transport.server_info().clone()
        };
        self.transport.set_server_info(info);
    }
    /// Publish server mod state to clients: net tables (mod UI) and the
    /// mods' new one-shot presentation events. Clients play each event once
    /// by ID; events older than [`EVENT_WINDOW`] ticks are dropped so late
    /// joiners never replay history.
    pub fn publish_mods(&mut self, mods: &mut crate::mod_runtime::ModRuntime) {
        let tables = mods.scripts.net_tables();
        self.mod_ui = tables
            .into_iter()
            .flat_map(|(table, entries)| {
                entries.into_iter().map(move |(key, v)| ModUiEntry {
                    table: table.clone(),
                    key,
                    json: v.to_string(),
                })
            })
            .collect();
        for e in mods.presentation.drain(..) {
            self.next_event += 1;
            self.events.push_back(PresentationEventState {
                id: self.next_event,
                sound: e.sound,
                name: e.name,
                position: e.position,
                owner: e.owner,
                tick: self.tick,
            });
        }
        let tick = self.tick;
        self.events
            .retain(|e| tick.saturating_sub(e.tick) <= EVENT_WINDOW && e.tick <= tick);
        while self.events.len() > magicka_network::session::MAX_EVENTS {
            self.events.pop_front();
        }
    }
    /// Run players' mod requests through the mod runtime and reply to each
    /// requester (exactly once per request ID; see `ModRuntime::request`).
    pub fn serve_mod_requests(
        &mut self,
        mods: &mut crate::mod_runtime::ModRuntime,
        session: &mut PlaySession,
    ) {
        for (slot, request) in self.transport.take_mod_requests() {
            let data: serde_json::Value =
                serde_json::from_str(&request.json).unwrap_or(serde_json::Value::Null);
            let result = mods.request(session, slot, request.request_id, &request.channel, &data);
            let reply = ModReply {
                request_id: request.request_id,
                json: result.to_string(),
            };
            if let Err(e) = self.transport.send_mod_reply(slot, reply) {
                session.world.log.push(format!("mod reply: {e}"));
            }
        }
    }
    /// The recent one-shot presentation events (the playable host shows
    /// them too, through the same once-per-ID player as clients).
    pub fn recent_events(&self) -> Vec<PresentationEventState> {
        self.events.iter().cloned().collect()
    }
    /// The mod UI state published to clients.
    pub fn mod_ui(&self) -> &[ModUiEntry] {
        &self.mod_ui
    }
    /// The name shown in LAN game lists.
    pub fn set_advert_name(&mut self, name: &str) {
        if !name.trim().is_empty() {
            self.advert_name = name.into();
        }
    }
    /// Broadcast this game on the LAN unless it only listens on loopback.
    fn advertise(&mut self, session: &PlaySession) {
        let address = self.transport.address();
        if address.ip().is_loopback() {
            return;
        }
        use magicka_network::discovery::AdvertSeat;
        // Players by slot with their profile names, then roles without a
        // wizard (the host's own first when it has none).
        let players = self.memory.values().map(|m| AdvertSeat {
            name: m.profile.name.clone(),
            role: Role::Player,
            queued: false,
        });
        let host = self.host_role.map(|role| AdvertSeat {
            name: self.advert_name.clone(),
            role,
            queued: false,
        });
        let observers: Vec<AdvertSeat> = self
            .transport
            .seats()
            .into_iter()
            .filter(|s| s.role != Role::Player || s.queued)
            .map(|s| AdvertSeat {
                name: s.name,
                role: s.role,
                queued: s.queued,
            })
            .collect();
        let advert = magicka_network::discovery::Advert::new(
            address.port(),
            &self.advert_name,
            session.level_file(),
            self.memory.len(),
            self.transport.lobby(),
        )
        .with_participants(players.chain(host).chain(observers));
        let advert = if self.private {
            advert.private()
        } else {
            advert
        };
        self.beacon.tick(Instant::now(), &advert);
    }
    pub fn publish(&mut self, session: &PlaySession) -> Result<(), String> {
        self.advertise(session);
        let level = network_level(session.level_file())?;
        if self.transport.server_info().level != level
            || self.transport.server_info().name != self.advert_name
        {
            let info = magicka_network::transport::ServerInfo {
                name: self.advert_name.clone(),
                level,
                ..self.transport.server_info().clone()
            };
            self.transport.set_server_info(info);
        }
        self.sync_epoch(session)?;
        self.tick = self.tick.checked_add(1).ok_or("network tick exhausted")?;
        self.pulses.observe(session, session.play_time);
        self.decals.observe(session, session.play_time);
        if self.tick.is_multiple_of(3) {
            self.transport.broadcast_game(&self.frame(session)?)?;
        }
        Ok(())
    }
}
