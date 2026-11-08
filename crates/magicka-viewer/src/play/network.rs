//! Local demo networking. Lightyear Apps stay on the main thread.
use super::*;
use magicka_network::connection::SnapshotWatchdog;
use magicka_network::{
    control::*,
    lobby::{DmCommand, DmRequest, Role, RoleRequest},
    presentation::GamePresentation,
    session::*,
    transport::{Admission11, JoinRequest, LoopbackClient, Welcome},
};
use magicka_runtime::network_session::HostedSession;
use std::time::Instant;
pub(super) enum NetworkPlay {
    Host(Box<HostedSession>),
    Client(Box<ClientView>),
}
pub(super) fn begin_leave(
    mut leave: MessageReader<super::LeaveSession>,
    mut network: Option<NonSendMut<NetworkPlay>>,
) {
    if leave.read().next().is_some() {
        if let Some(NetworkPlay::Client(client)) = network.as_deref_mut() {
            if let Err(e) = client.leave() {
                warn!("client departure: {e}");
            }
        }
    }
}
pub(super) struct ClientView {
    transport: LoopbackClient,
    presentation: GamePresentation,
    pub frame: GameFrame,
    display_frame: GameFrame,
    /// Respawn request sequence.
    input_sequence: u64,
    /// Held-input stream with redundant history (protocol 11).
    inputs: InputStream,
    pickup_sequence: u64,
    profile_sequence: u64,
    requested_profile: Option<PlayerProfile>,
    move_down: bool,
    /// Cast held while a tutorial dialog was up (dismiss on press only).
    dialog_cast_down: bool,
    visual: Option<(String, [String; 2])>,
    effects: magicka_runtime::network_session::RemoteEffects,
    players: magicka_runtime::network_session::RemotePlayers,
    /// Script-driven world state (pickups, level parts, tutorial, spells).
    world: magicka_runtime::network_world::RemoteWorld,
    effects_clock: Option<(u64, u64)>,
    pub status: String,
    watchdog: SnapshotWatchdog,
    /// The persistent profile ID sent with player setup (parked saves).
    profile_id: Option<String>,
    /// The player slot the local mirror currently shows (None: observer).
    shown_slot: Option<usize>,
    pub observer: ObserverView,
    role_sequence: u64,
    dm_sequence: u64,
    address: std::net::SocketAddr,
    /// This computer's submitted content/link settings, shared by local pads.
    join_request: JoinRequest,
    /// Extra local players on this computer, each with its own connection
    /// (the host sees separate players; they share this client's camera).
    local: Vec<LocalSeat>,
    /// Client-side NPC health bars: smoothed fraction, last health and the
    /// time since it last fell (the host's character update is not run here).
    npc_bars: std::collections::BTreeMap<u16, (f32, f32, f32)>,
    /// One-shot mod presentation played once per host event ID.
    mod_events: magicka_runtime::mod_presentation::RemoteEvents,
    /// This client's copy of the mods' presentation definitions.
    pub definitions: Option<std::sync::Arc<magicka_definitions::Definitions>>,
    /// Owned-wizard movement prediction (on unless `--no-predict`, plan B4).
    pub predict: bool,
    predictor: magicka_network::prediction::Predictor,
    /// The frame time of the last poll (inputs record it for replay).
    last_dt: f32,
    /// (handle, dead) of the owned wizard at the last reconciliation.
    predicted_life: Option<(u16, bool)>,
    /// The owned wizard's current top speed (its body's max × multiplier).
    predicted_speed: f32,
    /// The latest mod request result, for the mod panel.
    mod_reply: Option<String>,
}
/// One NPC health bar to draw on a client.
pub(super) struct NpcBar {
    pub feet: [f32; 3],
    pub normalized: f32,
    pub count: i32,
    pub radius: f32,
    pub damage_age: f32,
}
struct LocalSeat {
    pad: Entity,
    transport: LoopbackClient,
    profile: PlayerProfile,
    inputs: InputStream,
    /// The "joined" notice was shown.
    announced: bool,
    queued_notice: bool,
    watchdog: Option<SnapshotWatchdog>,
    newest: Option<(u64, u64, u64)>,
}
impl LocalSeat {
    fn new(pad: Entity, transport: LoopbackClient, profile: PlayerProfile) -> Self {
        Self {
            pad,
            transport,
            profile,
            inputs: default(),
            announced: false,
            queued_notice: false,
            watchdog: None,
            newest: None,
        }
    }
    fn accept_frame(&mut self, frame: &GameFrame, now: Instant) -> bool {
        let p = &frame.physics;
        let key = (p.session, p.epoch, p.tick);
        if self
            .newest
            .is_some_and(|old| key.0 != old.0 || (key.1, key.2) <= (old.1, old.2))
        {
            return false;
        }
        if !self
            .watchdog
            .as_mut()
            .is_some_and(|watch| watch.accepted_snapshot(now))
        {
            return false;
        }
        self.newest = Some(key);
        true
    }
}

fn poll_local_seats(seats: &mut Vec<LocalSeat>, now: Instant) -> Vec<String> {
    let mut notices = Vec::new();
    seats.retain_mut(|seat| {
        seat.transport.update_at(now);
        let frames = seat.transport.take_game_frames();
        seat.transport.take_profile_replies();
        seat.transport.take_pickup_receipts();
        if seat.transport.is_leaving() {
            return !seat.transport.leave_finished();
        }
        let welcome = match seat.transport.admission_at(now) {
            Admission11::Refused(reason, _) => {
                notices.push(format!("{} could not join: {reason}", seat.profile.name));
                return false;
            }
            Admission11::TimedOut => {
                notices.push(format!(
                    "{} could not join: the host did not answer",
                    seat.profile.name
                ));
                // Cancel delayed admission too. The pad is released immediately;
                // keep the socket only for the acknowledged departure handshake.
                if seat.transport.leave().is_err() {
                    return false;
                }
                return true;
            }
            Admission11::Seated(welcome) => welcome,
            _ => return true,
        };
        let watch = seat
            .watchdog
            .get_or_insert_with(|| SnapshotWatchdog::new(now));
        watch.update(now);
        if watch.lost().is_some() {
            notices.push(format!(
                "{} disconnected: the host timed out",
                seat.profile.name
            ));
            if seat.transport.leave().is_err() {
                return false;
            }
            return true;
        }
        if welcome.queued && !seat.queued_notice {
            notices.push(format!(
                "{} is waiting for a player slot",
                seat.profile.name
            ));
            seat.queued_notice = true;
        }
        for frame in frames {
            if seat.accept_frame(&frame, now)
                && !seat.announced
                && welcome.role == Role::Player
                && frame.players.iter().any(|p| p.slot == welcome.seat)
            {
                seat.announced = true;
                notices.push(format!("{} joined", seat.profile.name));
            }
        }
        true
    });
    notices
}

#[cfg(test)]
#[path = "spell_sync.rs"]
mod spell_sync;
#[cfg(test)]
mod local_recovery_tests {
    use super::*;
    use magicka_network::transport::{LoopbackHost, HELLO_ATTEMPTS, HELLO_RETRY, PLAYER_LEASE};

    fn seat(address: std::net::SocketAddr) -> LocalSeat {
        let mut world = World::new();
        LocalSeat::new(
            world.spawn_empty().id(),
            LoopbackClient::connect_as(address, Role::Player, "Extra").unwrap(),
            PlayerProfile {
                name: "Extra".into(),
                ..default()
            },
        )
    }
    fn host(dedicated: bool, limit: u16) -> (PlaySession, HostedSession) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let level = root
            .join("../../scenarios/feature-lab.lvl")
            .canonicalize()
            .unwrap();
        let mut session = PlaySession::load(
            &root.join("../../../../target/Magicka/Content"),
            level.to_str().unwrap(),
            "Wizard",
        )
        .unwrap();
        session.disable_scripts();
        session.step(1. / 60.);
        let host = HostedSession::bind_lobby(
            "127.0.0.1:0".parse().unwrap(),
            level.to_str().unwrap(),
            dedicated,
            PlayerProfile::default(),
            &session,
            magicka_network::lobby::LobbySettings {
                max_players: limit,
                ..default()
            },
        )
        .unwrap();
        (session, host)
    }
    fn tick(
        host: &mut HostedSession,
        session: &mut PlaySession,
        seats: &mut Vec<LocalSeat>,
    ) -> Vec<String> {
        let notices = poll_local_seats(seats, Instant::now());
        host.poll(session).unwrap();
        session.step(1. / 60.);
        host.publish(session).unwrap();
        std::thread::sleep(Duration::from_millis(1));
        notices
    }

    #[test]
    fn unanswered_join_releases_pad_once_and_cancels_delayed_admission() {
        let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let address = silent.local_addr().unwrap();
        let mut seats = vec![seat(address)];
        let start = Instant::now();
        for attempt in 0..HELLO_ATTEMPTS {
            assert!(poll_local_seats(&mut seats, start + HELLO_RETRY * attempt).is_empty());
        }
        let now = start + HELLO_RETRY * HELLO_ATTEMPTS;
        let notices = poll_local_seats(&mut seats, now);
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("host did not answer"));
        assert!(
            seats[0].transport.is_leaving(),
            "releases the pad while cancelling the socket"
        );
        assert!(poll_local_seats(&mut seats, now).is_empty());
        drop(silent);
        let mut host = LoopbackHost::bind_lobby(address, true, default()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !seats.is_empty() && Instant::now() < deadline {
            poll_local_seats(&mut seats, Instant::now());
            host.update();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            seats.is_empty(),
            "late host must acknowledge the cancellation"
        );
        assert!(host.player_slots().is_empty());
    }

    #[test]
    fn queued_pad_gets_one_wait_notice_then_joins_after_promotion() {
        let (mut session, mut host) = host(true, 1);
        let mut first = LoopbackClient::connect(host.transport.address()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while first.player_slot().is_none() && Instant::now() < deadline {
            first.update();
            host.poll(&mut session).unwrap();
            host.publish(&session).unwrap();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(first.player_slot().is_some());
        let mut seats = vec![seat(host.transport.address())];
        let mut notices = Vec::new();
        while !seats[0].queued_notice && Instant::now() < deadline {
            first.update();
            notices.extend(tick(&mut host, &mut session, &mut seats));
        }
        assert!(seats[0].queued_notice);
        for _ in 0..15 {
            first.update();
            notices.extend(tick(&mut host, &mut session, &mut seats));
        }
        assert_eq!(notices.iter().filter(|n| n.contains("waiting")).count(), 1);
        assert!(!seats[0].announced, "queued spectators have no wizard yet");
        first.leave().unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !seats[0].announced && Instant::now() < deadline {
            first.update();
            notices.extend(tick(&mut host, &mut session, &mut seats));
        }
        assert!(seats[0].announced);
        assert_eq!(notices.iter().filter(|n| n.ends_with(" joined")).count(), 1);
        assert_eq!(seats[0].transport.player_slot(), Some(0));
    }

    #[test]
    fn stalled_extra_player_releases_pad_and_late_frames_cannot_revive_it() {
        let (mut session, mut host) = host(false, 4);
        let mut seats = vec![seat(host.transport.address())];
        let deadline = Instant::now() + Duration::from_secs(3);
        while !seats[0].announced && Instant::now() < deadline {
            tick(&mut host, &mut session, &mut seats);
        }
        assert!(seats[0].announced);
        let later = Instant::now() + PLAYER_LEASE;
        let notices = poll_local_seats(&mut seats, later);
        assert_eq!(notices.len(), 1);
        assert!(notices[0].contains("disconnected"));
        assert!(seats[0].transport.is_leaving());
        let mut delayed = host.frame(&session).unwrap();
        delayed.physics.tick += 1;
        assert!(!seats[0].accept_frame(&delayed, later));
        let deadline = Instant::now() + Duration::from_secs(3);
        while !seats.is_empty() && Instant::now() < deadline {
            assert!(tick(&mut host, &mut session, &mut seats).is_empty());
        }
        assert!(seats.is_empty());
        assert_eq!(
            host.frame(&session).unwrap().players.len(),
            1,
            "host's own wizard remains"
        );
    }

    #[test]
    fn repeated_and_old_frames_cannot_extend_the_extra_players_lease() {
        let (session, host) = host(false, 4);
        let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let mut seat = seat(silent.local_addr().unwrap());
        let start = Instant::now();
        seat.watchdog = Some(SnapshotWatchdog::new(start));
        let mut frame = host.frame(&session).unwrap();
        frame.physics.tick = 10;
        assert!(seat.accept_frame(&frame, start));
        assert!(!seat.accept_frame(&frame, start + PLAYER_LEASE - Duration::from_millis(1)));
        frame.physics.tick = 9;
        assert!(!seat.accept_frame(&frame, start + PLAYER_LEASE - Duration::from_millis(1)));
        frame.physics.session += 1;
        frame.physics.tick = 11;
        assert!(
            !seat.accept_frame(&frame, start + PLAYER_LEASE - Duration::from_millis(1)),
            "a new host requires a new connection"
        );
        seat.watchdog.as_mut().unwrap().update(start + PLAYER_LEASE);
        assert!(seat.watchdog.as_ref().unwrap().lost().is_some());
    }
}
/// Camera and selection state of a client without its own wizard
/// (spectator, dungeon master, invader) — each computer tracks its own.
#[derive(Default)]
pub(super) struct ObserverView {
    /// Followed host player slot (spectators cycle through players).
    pub follow: Option<usize>,
    /// Free camera point (spectator/dungeon master roaming).
    pub free: Option<[f32; 3]>,
    /// Dungeon master: selected character handle.
    pub selected: Option<u16>,
    /// Dungeon master: index into the spawnable templates.
    pub template: usize,
}
/// Dungeon-master spawn choices: templates every level ships with content
/// for. INFERRED convenience list, not an original menu.
pub(super) const DM_TEMPLATES: [&str; 6] = [
    "Goblin_pirate",
    "Goblin_bomber",
    "Goblin_shaman",
    "Troll_forest",
    "Skeleton_swordsman",
    "Zombie",
];
impl ClientView {
    /// Confirmed NPC poses sampled on the same delayed clock as players/props.
    pub(super) fn npc_states(&self) -> &[NpcState] {
        &self.display_frame.npcs
    }

    pub fn disconnected(&self) -> bool {
        self.watchdog.lost().is_some()
    }
    pub fn connect(address: &str, request: JoinRequest) -> Result<Self, String> {
        let address = magicka_network::transport::resolve(address)?;
        let profile_id = request.profile.as_ref().and_then(|p| p.profile_id.clone());
        let mut transport = LoopbackClient::join(address, request.clone())?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(15) {
            transport.update();
            match transport.admission() {
                magicka_network::transport::Admission11::Refused(reason, _) => {
                    return Err(format!("the host refused to seat you: {reason}"));
                }
                magicka_network::transport::Admission11::TimedOut => {
                    return Err(
                        "the host did not answer: check the address, and the passphrase if the \
                         game has one"
                            .into(),
                    );
                }
                _ => {}
            }
            for frame in transport.take_game_frames() {
                // Players wait for their own wizard; observers for any frame.
                let ready = match transport.welcome() {
                    Some(Welcome {
                        role: Role::Player,
                        seat,
                        ..
                    }) => frame.players.iter().any(|p| p.slot == seat),
                    Some(_) => true,
                    None => false,
                };
                if ready {
                    // Delay adapts to measured jitter (50..=250 ms).
                    let mut presentation = GamePresentation::adaptive();
                    presentation
                        .apply(frame.clone(), Instant::now())
                        .map_err(|e| format!("snapshot: {e:?}"))?;
                    return Ok(Self {
                        transport,
                        presentation,
                        display_frame: frame.clone(),
                        frame,
                        input_sequence: 0,
                        inputs: InputStream::default(),
                        pickup_sequence: 0,
                        profile_sequence: 0,
                        requested_profile: None,
                        move_down: false,
                        dialog_cast_down: false,
                        visual: None,
                        effects: Default::default(),
                        players: Default::default(),
                        world: Default::default(),
                        effects_clock: None,
                        status: "Connected".into(),
                        watchdog: SnapshotWatchdog::new(Instant::now()),
                        profile_id,
                        shown_slot: Some(0),
                        observer: ObserverView::default(),
                        role_sequence: 0,
                        dm_sequence: 0,
                        address,
                        join_request: request,
                        local: Vec::new(),
                        npc_bars: Default::default(),
                        mod_events: Default::default(),
                        definitions: None,
                        predict: false,
                        predictor: Default::default(),
                        last_dt: 1.0 / 60.0,
                        predicted_life: None,
                        predicted_speed: 0.0,
                        mod_reply: None,
                    });
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Err("server did not seat this client and send a snapshot within 15 seconds".into())
    }
    pub fn profile_id(&self) -> Option<&str> {
        self.profile_id.as_deref()
    }
    pub fn set_profile_id(&mut self, id: Option<String>) {
        self.profile_id = id;
    }
    /// This client's player slot once its wizard exists on the host.
    pub fn own_slot(&self) -> Option<usize> {
        self.transport
            .player_slot()
            .filter(|s| self.frame.players.iter().any(|p| p.slot == *s))
    }
    pub fn welcome(&self) -> Option<Welcome> {
        self.transport.welcome()
    }
    pub fn role(&self) -> Role {
        self.welcome().map_or(Role::Spectator, |w| w.role)
    }
    /// The local mirror slot of the host's slot 0.
    pub fn view_slot_for_host_zero(&self) -> usize {
        self.own_slot()
            .unwrap_or(magicka_runtime::network_session::OBSERVER_VIEW_SLOT)
    }
    /// Kept for callers that need a mirror index: own slot or the observer slot.
    pub fn slot(&self) -> usize {
        self.view_slot_for_host_zero()
    }
    /// This seat's participant record (non-player roles).
    pub fn participant(&self) -> Option<&magicka_network::lobby::ParticipantState> {
        let seat = self.welcome()?.seat;
        self.frame.participants.iter().find(|p| p.seat == seat)
    }
    /// Ask the host for another role (join in, step out, ...).
    pub fn request_role(&mut self, role: Role) -> Result<(), String> {
        self.require_connected()?;
        self.role_sequence += 1;
        let name = self
            .requested_profile
            .as_ref()
            .map(|p| p.name.clone())
            .or_else(|| self.participant().map(|p| p.name.clone()))
            .unwrap_or_else(|| role.label().into());
        self.transport.request_role(RoleRequest {
            sequence: self.role_sequence,
            role,
            name,
        })?;
        self.status = format!("Requested role: {}", role.label());
        Ok(())
    }
    /// Who a dungeon master on this computer can remove: (seat, name).
    pub(super) fn kickable(&self) -> Vec<(usize, String)> {
        if self.role() != Role::DungeonMaster {
            return Vec::new();
        }
        let own = self.welcome().map(|w| w.seat);
        self.frame
            .players
            .iter()
            .map(|p| (p.slot, p.profile.name.clone()))
            .chain(
                self.frame
                    .participants
                    .iter()
                    .map(|p| (p.seat, p.name.clone())),
            )
            .filter(|(seat, _)| Some(*seat) != own)
            .collect()
    }
    pub fn dm(&mut self, command: DmCommand) -> Result<(), String> {
        self.require_connected()?;
        self.dm_sequence += 1;
        self.transport.send_dm(DmRequest {
            sequence: self.dm_sequence,
            command,
        })
    }
    pub fn profile(&self) -> PlayerProfile {
        if let Some(profile) = &self.requested_profile {
            return profile.clone();
        }
        self.frame
            .players
            .iter()
            .find(|p| p.slot == self.slot())
            .unwrap()
            .profile
            .clone()
    }
    pub fn setup(&mut self, profile: PlayerProfile) -> Result<(), String> {
        self.require_connected()?;
        if !profile.valid() {
            return Err("invalid player setup".into());
        }
        self.profile_sequence = self
            .profile_sequence
            .checked_add(1)
            .ok_or("profile sequence exhausted")?;
        self.transport.send_profile(ProfileRequest {
            profile_id: self.profile_id.clone(),
            sequence: self.profile_sequence,
            profile: profile.clone(),
        })?;
        self.requested_profile = Some(profile);
        Ok(())
    }
    pub fn poll(&mut self, play: &mut Play, dt: f32) -> Result<(), String> {
        self.poll_at(play, dt, Instant::now())
    }
    /// Explicit presentation/liveness clock for deterministic disconnect checks.
    pub(super) fn poll_at(&mut self, play: &mut Play, dt: f32, now: Instant) -> Result<(), String> {
        if self.disconnected() {
            return self.poll_disconnected(play, dt);
        }
        if self.transport.is_leaving() {
            self.transport.update();
            self.transport.take_game_frames();
            self.poll_local();
            return Ok(());
        }
        self.watchdog.update(now);
        if self.transport.refusal_code() == Some(magicka_network::transport::RefusalCode::Kicked) {
            self.watchdog.kicked();
        }
        if self.watchdog.lost().is_some() {
            return self.poll_disconnected(play, dt);
        }
        self.transport.update();
        for frame in self.transport.take_game_frames() {
            if self.presentation.apply(frame.clone(), now).is_ok() {
                // Losing the wizard (stepping out, lease expiry) is not a
                // disconnect: the seat stays and the host stops sending
                // frames to retired sockets (watchdog).
                if !self.watchdog.accepted_snapshot(now) {
                    break;
                }
                self.frame = frame;
            }
        }
        if self.watchdog.lost().is_some() {
            return self.poll_disconnected(play, dt);
        }
        self.display_frame = self.presentation.sample(now).unwrap();
        for reply in self.transport.take_profile_replies() {
            if reply.sequence != self.profile_sequence {
                continue;
            }
            if reply.error.is_some() {
                self.requested_profile = None;
            }
            self.status = reply
                .error
                .unwrap_or_else(|| "Player setup accepted".into());
        }
        for reply in self.transport.take_mod_replies() {
            let value = serde_json::from_str(&reply.json).unwrap_or_default();
            self.mod_reply = Some(super::mod_panel::describe(&value));
        }
        for reply in self.transport.take_pickup_receipts() {
            self.status = format!("Pickup: {:?}", reply.outcome);
        }
        let own_slot = self.slot();
        self.sync_role(play)?;
        if let Some(rules) = &self.frame.rules {
            magicka_runtime::network_rules::mirror_rules(&mut play.session, rules, own_slot);
        }
        if magicka_runtime::network_session::mirror_world(&mut play.session, &self.frame)? {
            play.level = self.frame.level.clone();
            play.reloads += 1;
            self.visual = None;
        }
        let clock = (self.frame.physics.epoch, self.frame.physics.tick);
        if self.effects_clock != Some(clock) {
            self.effects.reconcile(
                clock.0,
                &self.frame.effects,
                self.frame.time,
                &mut play.session.effects,
            )?;
            self.effects_clock = Some(clock);
        }
        let fresh = self.mod_events.take(&self.frame);
        magicka_runtime::mod_presentation::play(
            &mut play.session,
            self.definitions.as_deref(),
            &fresh,
        );
        self.effects.advance(dt, &mut play.session.effects);
        play.session.effects.update(dt);
        play.session.effects.update_particles(dt);
        self.players
            .reconcile(&self.frame, self.own_slot(), &mut play.session)?;
        let player = self.own_slot().is_some();
        self.world
            .apply(&self.frame.world, &mut play.session, dt, player)?;
        // Missiles keep flying between host updates (they are drawn every frame).
        let coast = self.watchdog_age().unwrap_or(0.0).min(0.2);
        for missile in &mut play.session.remote_missiles {
            for i in 0..3 {
                missile.position[i] += missile.velocity[i] * coast;
            }
        }
        // Self-shields, halos, weapon trails and zap pulses of every wizard
        // the host shows, including this client's own (it casts nothing
        // locally). Fresh pulses arm the mirrors' zap timers inside.
        self.world.apply_avatars(
            &self.frame.world,
            &mut play.session,
            self.own_slot(),
            self.frame.time,
        );
        self.apply_remote_poses(play);
        self.smooth_health(play, dt);
        let Some(state) = self
            .own_slot()
            .and_then(|s| self.display_frame.players.iter().find(|p| p.slot == s))
            .cloned()
        else {
            self.observer_camera(play, dt);
            self.sync_pickups(play);
            return Ok(());
        };
        let state = &state;
        if self.requested_profile.as_ref() == Some(&state.profile) {
            self.requested_profile = None;
        }
        let visual = (state.profile.robe.clone(), state.items.clone());
        if self.visual.as_ref() != Some(&visual) {
            play.session
                .set_player_loadout(0, &visual.0, &visual.1[1], &visual.1[0])
                .map_err(|e| format!("mirror equipment: {e:?}"))?;
            play.character = visual.0.clone();
            self.visual = Some(visual);
        }
        let body_index = play.session.avatar().body;
        let body = &mut play.session.world.physics.bodies[body_index];
        self.last_dt = dt.max(1e-3);
        self.predicted_speed = body.max_speed * body.speed_multiplier();
        let position = if self.predict && !state.dead {
            // Restart from the newest authoritative state and replay the
            // inputs the host has not applied; snap across lifetimes.
            let latest = self
                .own_slot()
                .and_then(|s| self.frame.players.iter().find(|p| p.slot == s))
                .unwrap_or(state);
            let life = (latest.handle, latest.dead);
            let discontinuity = self.predicted_life.is_some_and(|l| l != life);
            self.predicted_life = Some(life);
            self.predictor.reconcile(
                latest.position,
                latest.last_input,
                self.frame.physics.epoch,
                self.predicted_speed,
                discontinuity,
            );
            self.predictor.update(dt);
            self.predictor.visible().unwrap_or(state.position)
        } else {
            self.predictor.reset();
            state.position
        };
        let error = self.predict.then_some(self.predictor.last_error).flatten();
        let age = self.watchdog_age();
        self.transport.note_presentation(age, error);
        let body = &mut play.session.world.physics.bodies[body_index];
        body.position = position;
        body.orientation.right = state.right;
        body.orientation.up = state.up;
        body.orientation.backward = state.backward;
        let avatar = play.session.world.players[0].as_mut().unwrap();
        avatar.character.combat.health.hit_points = state.health;
        avatar.character.combat.dead = state.dead;
        if avatar.motion.controller.current_animation != state.animation
            || avatar.motion.controller.clip_name().is_none()
        {
            avatar
                .motion
                .controller
                .force_animation(&avatar.motion.animations, state.animation);
        }
        avatar.motion.controller.time = state.animation_time;
        avatar.motion.controller.speed = state.animation_speed;
        avatar.motion.controller.update(0.0);
        play.pending = avatar.motion.controller.cues.pop();
        avatar.motion.controller.cues.clear();
        magicka_runtime::network_session::mirror_player_spell_state(&mut play.session, 0, state)?;
        let body = play.session.body();
        // This computer's players share the camera; remote ones never move it.
        let feet_offset = body.radius + body.length * 0.5;
        let local: Vec<usize> = self
            .local
            .iter()
            .filter_map(|s| s.transport.player_slot())
            .collect();
        // The camera follows where the wizard is drawn: following the delayed
        // position would make a predicted wizard slide against the world.
        let subjects: Vec<_> = std::iter::once((position, state.dead))
            .chain(
                self.display_frame
                    .players
                    .iter()
                    .filter(|p| local.contains(&p.slot))
                    .map(|p| (p.position, p.dead)),
            )
            .map(|(position, dead)| magicka_runtime::camera::CameraSubject {
                position,
                dead,
                grounded: true,
                feet_offset,
            })
            .collect();
        play.session
            .world
            .camera
            .update(&subjects, &[], dt.min(0.1));
        self.sync_pickups(play);
        Ok(())
    }
    /// Host player poses/animation onto their local mirrors.
    fn apply_remote_poses(&self, play: &mut Play) {
        let own = self.own_slot();
        let view = self.view_slot_for_host_zero();
        for remote in self
            .display_frame
            .players
            .iter()
            .filter(|p| Some(p.slot) != own)
        {
            let slot = if remote.slot == 0 { view } else { remote.slot };
            let Some(avatar) = play
                .session
                .world
                .players
                .get_mut(slot)
                .and_then(Option::as_mut)
            else {
                continue;
            };
            let body = &mut play.session.world.physics.bodies[avatar.body];
            body.position = remote.position;
            body.orientation.right = remote.right;
            body.orientation.up = remote.up;
            body.orientation.backward = remote.backward;
            avatar.character.combat.health.hit_points = remote.health;
            avatar.character.combat.dead = remote.dead;
            if avatar.motion.controller.current_animation != remote.animation
                || avatar.motion.controller.clip_name().is_none()
            {
                avatar
                    .motion
                    .controller
                    .force_animation(&avatar.motion.animations, remote.animation);
            }
            avatar.motion.controller.time = remote.animation_time;
            avatar.motion.controller.speed = remote.animation_speed;
            avatar.motion.controller.update(0.);
            avatar.motion.controller.cues.clear();
        }
    }
    /// Character.Update's health-bar smoothing for mirrored avatars and NPC
    /// bars, which clients otherwise never advance.
    fn smooth_health(&mut self, play: &mut Play, dt: f32) {
        use magicka_runtime::hud::smooth_health;
        for p in play.session.world.players.iter_mut().flatten() {
            let c = &mut p.character;
            c.normalized_hit_points = smooth_health(
                c.normalized_hit_points,
                c.combat.health.hit_points,
                c.combat.health.max_hit_points.max(1e-3),
                dt,
            );
        }
        let live: std::collections::BTreeSet<u16> =
            self.display_frame.npcs.iter().map(|n| n.handle).collect();
        self.npc_bars.retain(|h, _| live.contains(h));
        for n in &self.display_frame.npcs {
            let max = n.max_health.max(1e-3);
            let entry =
                self.npc_bars
                    .entry(n.handle)
                    .or_insert((n.health / max, n.health, f32::MAX));
            entry.2 = if n.health < entry.1 {
                0.0
            } else {
                entry.2 + dt
            };
            entry.1 = n.health;
            entry.0 = smooth_health(entry.0, n.health, max, dt);
        }
    }
    pub(super) fn npc_bars(&self) -> Vec<NpcBar> {
        self.display_frame
            .npcs
            .iter()
            .filter(|n| !n.dead && n.health > 0.0 && n.max_health > 0.0)
            .filter_map(|n| {
                let &(normalized, _, damage_age) = self.npc_bars.get(&n.handle)?;
                Some(NpcBar {
                    feet: [n.transform[12], n.transform[13], n.transform[14]],
                    normalized,
                    count: n.health_bars,
                    radius: n.radius,
                    damage_age,
                })
            })
            .collect()
    }
    /// Interpolated pickup poses; existence and kind come from the
    /// world presentation (`RemoteWorld`).
    fn sync_pickups(&self, play: &mut Play) {
        let world = &mut play.session.world;
        for (host, local) in self.world.pickup_pairs() {
            let Some(b) = self
                .display_frame
                .physics
                .bodies
                .iter()
                .find(|b| b.handle == host)
            else {
                continue;
            };
            let transform = b.model_transform();
            if let Some(entity) = world.entities.iter_mut().find(|e| e.handle == local) {
                entity.orientation = transform;
                entity.position = [transform[12], transform[13], transform[14]];
            }
            if let Some(book) = world.book_pickups.get_mut(&local) {
                book.transform = transform;
            }
            if let Some(item) = world.item_pickups.get_mut(&local) {
                item.transform = transform;
            }
        }
    }
    /// Keep the local slot-0 avatar in step with the seat: dormant (hidden,
    /// inactive) while observing, rebuilt when a wizard is granted.
    fn sync_role(&mut self, play: &mut Play) -> Result<(), String> {
        let own = self.own_slot();
        if own.is_some() == self.shown_slot.is_some() {
            self.shown_slot = own;
            return Ok(());
        }
        if own.is_some() {
            play.session
                .fresh_network_player_at_start(0)
                .map_err(|e| format!("player view: {e:?}"))?;
            self.visual = None;
            self.status = "You joined the game".into();
        } else {
            play.session
                .depart_network_player(0)
                .map_err(|e| format!("observer view: {e:?}"))?;
            self.status = format!("{} view", self.role().label());
        }
        self.shown_slot = own;
        Ok(())
    }
    /// Observer cameras: the possessed NPC, a free roaming point, or a
    /// followed player (each client tracks its own).
    fn observer_camera(&mut self, play: &mut Play, dt: f32) {
        let possessed = self.participant().and_then(|p| p.possessed);
        let npc = possessed.and_then(|h| {
            self.display_frame
                .npcs
                .iter()
                .find(|n| n.handle == h)
                .map(|n| [n.transform[12], n.transform[13] + 1.0, n.transform[14]])
        });
        if self
            .observer
            .follow
            .is_none_or(|s| !self.display_frame.players.iter().any(|p| p.slot == s))
        {
            self.observer.follow = self.display_frame.players.first().map(|p| p.slot);
        }
        let followed = self.observer.follow.and_then(|s| {
            self.display_frame
                .players
                .iter()
                .find(|p| p.slot == s)
                .map(|p| p.position)
        });
        let position = npc
            .or(self.observer.free)
            .or(followed)
            .unwrap_or_else(|| play.session.camera().ground_position());
        let subject = magicka_runtime::camera::CameraSubject {
            position,
            dead: false,
            grounded: false,
            feet_offset: 1.0,
        };
        play.session
            .world
            .camera
            .update(&[subject], &[], dt.min(0.1));
    }
    /// Cycle the followed player (`step` ±1).
    pub fn cycle_follow(&mut self, step: isize) {
        let slots: Vec<usize> = self.display_frame.players.iter().map(|p| p.slot).collect();
        if slots.is_empty() {
            return;
        }
        let at = self
            .observer
            .follow
            .and_then(|f| slots.iter().position(|s| *s == f))
            .unwrap_or(0) as isize;
        let next = (at + step).rem_euclid(slots.len() as isize) as usize;
        self.observer.follow = Some(slots[next]);
        self.observer.free = None;
    }
    /// The character nearest a world point (dungeon master selection).
    pub fn character_near(&self, point: [f32; 3]) -> Option<u16> {
        let d = |p: [f32; 3]| (p[0] - point[0]).powi(2) + (p[2] - point[2]).powi(2);
        self.display_frame
            .npcs
            .iter()
            .filter(|n| !n.dead)
            .map(|n| {
                (
                    n.handle,
                    [n.transform[12], n.transform[13], n.transform[14]],
                )
            })
            .chain(
                self.display_frame
                    .players
                    .iter()
                    .map(|p| (p.handle, p.position)),
            )
            .filter(|(_, p)| d(*p) < 6.25)
            .min_by(|a, b| d(a.1).total_cmp(&d(b.1)))
            .map(|(h, _)| h)
    }
    /// Seat an extra local player (one more connection to the host).
    pub fn add_local(
        &mut self,
        pad: Entity,
        profile: PlayerProfile,
        profile_id: Option<String>,
    ) -> Result<(), String> {
        self.require_connected()?;
        // The seat's profile travels with its hello (protocol 11).
        let transport = LoopbackClient::join(
            self.address,
            JoinRequest {
                role: Role::Player,
                name: profile.name.clone(),
                profile: Some(ProfileSubmit {
                    profile: profile.clone(),
                    profile_id: profile_id.clone(),
                }),
                ..self.join_request.clone()
            },
        )?;
        self.local.push(LocalSeat::new(pad, transport, profile));
        Ok(())
    }
    pub fn remove_local(&mut self, pad: Entity) {
        for seat in self.local.iter_mut().filter(|s| s.pad == pad) {
            if let Err(e) = seat.transport.leave() {
                warn!("local departure: {e}");
            }
        }
    }
    /// Keep all sockets alive until the host acknowledges their departures.
    pub fn leave(&mut self) -> Result<(), String> {
        self.transport.leave()?;
        for seat in &mut self.local {
            seat.transport.leave()?;
        }
        Ok(())
    }
    pub fn leave_finished(&self) -> bool {
        self.transport.leave_finished() && self.local.iter().all(|s| s.transport.leave_finished())
    }
    pub fn local_pads(&self) -> impl Iterator<Item = Entity> + '_ {
        self.local
            .iter()
            .filter(|s| !s.transport.is_leaving())
            .map(|s| s.pad)
    }
    /// Heartbeats, profile setup and refusals for extra local connections.
    /// Returns notices for the overlay.
    pub fn poll_local(&mut self) -> Vec<String> {
        poll_local_seats(&mut self.local, Instant::now())
    }
    /// A local seat's host slot and current position, once seated.
    pub fn local_state(&self, pad: Entity) -> Option<(usize, [f32; 3])> {
        let slot = self
            .local
            .iter()
            .find(|s| s.pad == pad && !s.transport.is_leaving())?
            .transport
            .player_slot()?;
        let p = self.display_frame.players.iter().find(|p| p.slot == slot)?;
        Some((slot, p.position))
    }
    pub fn local_input(&mut self, pad: Entity, controls: Controls) -> Result<(), String> {
        if self.require_connected().is_err() {
            return Ok(());
        }
        let (session, epoch) = (self.frame.physics.session, self.frame.physics.epoch);
        let Some(seat) = self
            .local
            .iter_mut()
            .find(|s| s.pad == pad && !s.transport.is_leaving())
        else {
            return Ok(());
        };
        if seat.transport.player_slot().is_none() {
            return Ok(());
        }
        let command = seat.inputs.next(session, epoch, controls);
        seat.transport.send_input(command)
    }
    /// Ask a server mod (the host runs each request ID at most once).
    pub fn mod_request(
        &mut self,
        id: u64,
        channel: &str,
        data: &serde_json::Value,
    ) -> Result<(), String> {
        self.require_connected()?;
        self.transport.send_mod_request(ModRequest {
            request_id: id,
            channel: channel.into(),
            json: data.to_string(),
        })
    }
    pub fn take_mod_reply(&mut self) -> Option<String> {
        self.mod_reply.take()
    }
    /// Seconds since the newest accepted game state (telemetry).
    fn watchdog_age(&self) -> Option<f32> {
        self.presentation
            .newest_received()
            .map(|t| t.elapsed().as_secs_f32())
    }
    fn send_controls(&mut self, controls: Controls) -> Result<(), String> {
        let command = self.inputs.next(
            self.frame.physics.session,
            self.frame.physics.epoch,
            controls,
        );
        if self.predict {
            self.predictor.record(
                command.sequence,
                self.last_dt,
                controls,
                self.predicted_speed,
            );
        }
        self.transport.send_input(command)
    }
    pub(super) fn require_connected(&self) -> Result<(), String> {
        if let Some(reason) = self.watchdog.lost() {
            return Err(reason.notice().into());
        }
        if self.transport.is_leaving() {
            return Err("Leaving the game".into());
        }
        Ok(())
    }
    fn poll_disconnected(&mut self, play: &mut Play, dt: f32) -> Result<(), String> {
        self.status = self.watchdog.lost().unwrap().notice().into();
        // This computer cannot render fresh state after its primary connection
        // is terminal. Release every local wizard instead of controlling blind.
        self.leave().ok();
        self.transport.update();
        self.transport.take_game_frames();
        self.poll_local();
        self.requested_profile = None;
        self.move_down = false;
        self.effects.reconcile(
            self.frame.physics.epoch,
            &[],
            self.frame.time,
            &mut play.session.effects,
        )?;
        play.session.effects.update(dt);
        play.session.effects.update_particles(dt);
        Ok(())
    }
    /// Neutral controls, so a held walk or cast stops on the host.
    fn idle_input(&mut self) -> Result<(), String> {
        if self.own_slot().is_none() && self.participant().and_then(|p| p.possessed).is_none() {
            return Ok(());
        }
        self.send_controls(Controls::default())
    }
    pub fn respawn(&mut self) -> Result<(), String> {
        self.require_connected()?;
        self.input_sequence = self
            .input_sequence
            .checked_add(1)
            .ok_or("sequence exhausted")?;
        self.transport.send_respawn(RespawnRequest {
            session: self.frame.physics.session,
            epoch: self.frame.physics.epoch,
            sequence: self.input_sequence,
        })
    }
    pub fn input(
        &mut self,
        play: &mut Play,
        input: PlayerInput,
        aspect: f32,
        select_pressed: bool,
    ) -> Result<(), String> {
        if self.watchdog.lost().is_some() || self.transport.is_leaving() {
            return Ok(());
        }
        // A tutorial dialog is modal for this computer's players only: it
        // takes their input until they dismiss their own copy.
        if !play.session.world.tutorial.dialog_done() {
            let pressed = select_pressed
                || (input.move_down && !self.move_down)
                || (input.cast_down && !self.dialog_cast_down);
            self.dialog_cast_down = input.cast_down;
            self.move_down = input.move_down;
            if pressed {
                self.world.dismiss_dialog(&mut play.session);
            }
            return self.idle_input();
        }
        self.dialog_cast_down = input.cast_down;
        if self.own_slot().is_none() {
            // Pilots steer their NPC with ordinary controls; spectators and
            // unpossessed dungeon masters send nothing.
            let Some(npc) = self.participant().and_then(|p| p.possessed) else {
                return Ok(());
            };
            let height = self
                .display_frame
                .npcs
                .iter()
                .find(|n| n.handle == npc)
                .map_or(0.0, |n| n.transform[13] + 1.0);
            let aim = input
                .cursor_ndc
                .map(|ndc| play.session.camera().mouse_pick(ndc, aspect, height));
            return self.send_controls(Controls {
                pad_movement: input.pad_movement,
                pad_attack: input.pad_attack,
                pad_block: input.pad_block,
                aim,
                move_down: input.move_down,
                cast_down: input.cast_down,
                cast_self_down: input.cast_self_down,
                shift_down: input.shift_down,
                boost_down: input.boost_down,
                elements: input.elements,
                magick_prev: false,
                magick_next: false,
                inventory: false,
            });
        }
        let aim = input.cursor_ndc.map(|ndc| {
            play.session
                .camera()
                .mouse_pick(ndc, aspect, play.session.body().position[1])
        });
        let mut move_down = input.move_down;
        if input.move_down
            && !self.move_down
            && !input.shift_down
            && play.session.avatar().conjure.queue.is_empty()
        {
            let target = if input.pad_movement.is_some() {
                play.session.pad_pickup_target()
            } else if let (Some(ndc), Some(plane)) = (input.cursor_ndc, aim) {
                let camera = play.session.camera();
                let segment = magicka_sim::spray_pulse::Segment {
                    origin: camera.position(),
                    delta: camera.mouse_ray(ndc, aspect).map(|v| v * 1000.),
                };
                play.session.cursor_pickup(segment, plane)
            } else {
                None
            };
            if let Some(handle) = target.and_then(|t| self.world.host_pickup(t)) {
                if let Some(body) = self
                    .frame
                    .physics
                    .bodies
                    .iter()
                    .find(|b| b.handle == handle)
                {
                    self.pickup_sequence = self
                        .pickup_sequence
                        .checked_add(1)
                        .ok_or("pickup sequence exhausted")?;
                    self.transport.send_pickup(PickupRequest {
                        session: self.frame.physics.session,
                        epoch: self.frame.physics.epoch,
                        sequence: self.pickup_sequence,
                        target: PickupTarget {
                            handle,
                            generation: body.generation,
                        },
                    })?;
                    move_down = false;
                }
            }
        }
        self.move_down = input.move_down;
        self.send_controls(Controls {
            pad_movement: input.pad_movement,
            pad_attack: input.pad_attack,
            pad_block: input.pad_block,
            aim,
            move_down,
            cast_down: input.cast_down,
            cast_self_down: input.cast_self_down,
            shift_down: input.shift_down,
            boost_down: input.boost_down,
            elements: input.elements,
            magick_prev: false,
            magick_next: false,
            inventory: false,
        })
    }
}
#[derive(Component)]
pub(super) struct ConnectionNotice;
pub(super) fn connection_notice(
    network: Option<NonSend<NetworkPlay>>,
    play: Res<Play>,
    mut text: Query<
        (&mut NativeBitmapText, &mut Transform, &mut Visibility),
        With<ConnectionNotice>,
    >,
    mut previous: Local<String>,
) {
    let notice = match network.as_deref() {
        Some(NetworkPlay::Client(client)) => client.watchdog.lost().map(|reason| reason.notice()),
        _ => None,
    };
    for (mut text, mut transform, mut visibility) in &mut text {
        *visibility = if notice.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        transform.translation =
            Vec3::new(-play.screen[0] * 0.5 + 24., play.screen[1] * 0.5 - 32., 10.);
        let notice = notice.unwrap_or("");
        if previous.as_str() != notice {
            text.typing.set_text(Some(notice));
            *previous = notice.into();
        }
        // Native draw publishes the glyph count after the dirty rebuild.
        if !notice.is_empty() && !text.typing.dirty {
            text.typing.finish();
        }
    }
}
impl NetworkPlay {
    pub fn profile(&self, play: &Play) -> PlayerProfile {
        match self {
            Self::Client(c) => c.profile(),
            Self::Host(h) => h
                .frame(&play.session)
                .unwrap()
                .players
                .iter()
                .find(|p| p.slot == 0)
                .unwrap()
                .profile
                .clone(),
        }
    }
    pub fn setup(&mut self, profile: PlayerProfile, play: &mut Play) -> Result<(), String> {
        match self {
            Self::Client(c) => c.setup(profile),
            Self::Host(h) => h.set_host_profile(profile, &mut play.session),
        }
    }
}
#[derive(Component)]
#[require(super::tick_clock::TickInterpolated)]
pub(super) struct NetworkAvatar {
    slot: usize,
    epoch: u64,
    handle: u16,
    robe: String,
    pub view_slot: usize,
}
impl NetworkAvatar {
    /// The handle of this root's mirrored avatar in the display world, so
    /// per-avatar draw paths (self-shields, zap looks, halos) key remote
    /// wizards like the local one.
    pub(super) fn mirror_handle(&self, play: &Play) -> Option<u16> {
        play.session
            .world
            .players
            .get(self.view_slot)
            .and_then(Option::as_ref)
            .map(|p| p.handle)
    }
}
#[derive(Default)]
pub(super) struct RejoinCapture {
    phase: usize,
    ready_frames: u32,
    old: Option<(usize, u16, Entity, u16, usize)>,
}
/// Capture actual mirror/root/equipment lifetimes on the observer, with no synthetic snapshots.
#[allow(clippy::too_many_arguments)]
pub(super) fn rejoin_capture(
    mut commands: Commands,
    pilot: Option<Res<Autopilot>>,
    network: Option<NonSend<NetworkPlay>>,
    play: Res<Play>,
    roots: Query<(Entity, &NetworkAvatar)>,
    meshes: Query<Entity, With<Mesh3d>>,
    parents: Query<&ChildOf>,
    items: Query<&equipment::HeldItem>,
    mut audit: Local<RejoinCapture>,
) {
    let Some(pilot) = pilot else { return };
    if !pilot
        .script
        .last()
        .is_some_and(|s| s.3 == "wait_for_rejoin")
        || audit.phase == 3
    {
        return;
    }
    let Some(NetworkPlay::Client(client)) = network.as_deref() else {
        return;
    };
    let drawable = |root| {
        meshes
            .iter()
            .any(|mesh| parents.iter_ancestors(mesh).any(|e| e == root))
    };
    let old_items = audit.old.map_or(0, |(_, _, _, owner, _)| {
        items.iter().filter(|i| i.owner == owner).count()
    });
    let (label, root, host_handle, mirror_handle) = match audit.phase {
        0 => {
            let Some(state) = client
                .frame
                .players
                .iter()
                .find(|p| p.profile.name == "QA Player 1")
            else {
                return;
            };
            let Some((entity, root)) = roots.iter().find(|(_, r)| {
                r.slot == state.slot
                    && r.handle == state.handle
                    && r.epoch == client.frame.physics.epoch
            }) else {
                return;
            };
            let Some(Some(mirror)) = play.session.world.players.get(root.view_slot) else {
                return;
            };
            if !drawable(entity) || !items.iter().any(|i| i.owner == mirror.handle) {
                return;
            }
            audit.old = Some((state.slot, state.handle, entity, mirror.handle, mirror.body));
            (
                "before",
                Some(entity),
                Some(state.handle),
                Some(mirror.handle),
            )
        }
        1 => {
            let (slot, _, _, _, body) = audit.old.unwrap();
            if client.frame.players.iter().any(|p| p.slot == slot)
                || roots.iter().any(|(_, r)| r.slot == slot)
                || old_items != 0
                || play.session.world.physics.bodies[body].active
            {
                return;
            }
            ("removed", None, None, None)
        }
        2 => {
            let (slot, old_handle, old_root, _, _) = audit.old.unwrap();
            let Some(state) = client.frame.players.iter().find(|p| {
                p.profile.name == "QA Player 3" && p.slot == slot && p.handle != old_handle
            }) else {
                return;
            };
            let Some((entity, root)) = roots.iter().find(|(_, r)| {
                r.slot == slot && r.handle == state.handle && r.epoch == client.frame.physics.epoch
            }) else {
                return;
            };
            let Some(Some(mirror)) = play.session.world.players.get(root.view_slot) else {
                return;
            };
            if entity == old_root
                || !drawable(entity)
                || old_items != 0
                || !items.iter().any(|i| i.owner == mirror.handle)
            {
                return;
            }
            (
                "replaced",
                Some(entity),
                Some(state.handle),
                Some(mirror.handle),
            )
        }
        _ => return,
    };
    // QA-only render warmup after hierarchy/equipment publication, before pixels are captured.
    audit.ready_frames += 1;
    if audit.ready_frames < 30 {
        return;
    }
    let record = serde_json::json!({
        "phase": label, "frame": client.frame, "host_handle": host_handle,
        "root": root.map(|r| format!("{r:?}")), "mirror_handle": mirror_handle,
        "old_equipment_count": old_items,
        "equipment_count": mirror_handle.map_or(0, |owner| items.iter().filter(|i| i.owner == owner).count()),
        "drawable": root.is_some_and(drawable),
    });
    std::fs::write(
        pilot.dir.join(format!("rejoin-{label}.json")),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .expect("write rejoin audit");
    commands
        .spawn(Screenshot::image(pilot.target.clone()))
        .observe(save_to_disk(pilot.dir.join(format!("rejoin-{label}.png"))));
    audit.phase += 1;
    audit.ready_frames = 0;
}
#[allow(clippy::too_many_arguments)]
pub(super) fn visuals(
    mut commands: Commands,
    network: Option<NonSendMut<NetworkPlay>>,
    play: Res<Play>,
    server: Res<AssetServer>,
    npcs: Res<NpcAssets>,
    mut avatar_assets: ResMut<AvatarAssets>,
    own_roots: Query<Entity, With<AvatarRoot>>,
    mut colour: ResMut<crate::robe::RobeColour>,
    mut roots: Query<
        (
            Entity,
            &NetworkAvatar,
            &mut Transform,
            &mut crate::robe::RobeTint,
        ),
        Without<AvatarRoot>,
    >,
    mut own_visual: Local<String>,
) {
    // Hosts and offline sessions (local co-op) draw their extra avatars from
    // the session itself; clients from the delayed display frame.
    let (views, epoch, own_slot, own) = match network.as_deref() {
        Some(NetworkPlay::Client(c)) => {
            let frame = &c.display_frame;
            let own = c.own_slot();
            let views = frame
                .players
                .iter()
                .filter(|p| Some(p.slot) != own)
                .map(|p| AvatarView {
                    slot: p.slot,
                    handle: p.handle,
                    robe: p.profile.robe.clone(),
                    colour: p.profile.colour,
                    position: p.position,
                    basis: [p.right, p.up, p.backward],
                })
                .collect::<Vec<_>>();
            let own_state = own.and_then(|s| frame.players.iter().find(|p| p.slot == s));
            (
                views,
                frame.physics.epoch,
                c.view_slot_for_host_zero(),
                own_state.map(|p| (p.profile.robe.clone(), p.profile.colour)),
            )
        }
        Some(NetworkPlay::Host(_)) => {
            // The host's own profile (robe swaps from the setup UI).
            let own = play.session.player_robe(0).map(|robe| {
                let colour = play.session.player_profiles[0]
                    .as_ref()
                    .map_or(colour.0, |(_, c)| *c);
                (robe.to_string(), colour)
            });
            (session_views(&play), play.session.network_epoch(), 0, own)
        }
        None => (session_views(&play), play.session.network_epoch(), 0, None),
    };
    if let Some((robe, robe_colour)) = own {
        let own = AvatarView {
            slot: 0,
            handle: 0,
            robe,
            colour: robe_colour,
            position: [0.0; 3],
            basis: [[0.0; 3]; 3],
        };
        let own = &own;
        colour.0 = own.colour;
        if *own_visual != own.robe {
            *own_visual = own.robe.clone();
            if let Some(path) = npcs.files.get(&own.robe.to_ascii_lowercase()) {
                for e in &own_roots {
                    commands.entity(e).despawn();
                }
                avatar_assets.gltf = server.load(path.clone());
                avatar_assets.scene =
                    server.load(GltfAssetLabel::Scene(0).from_asset(path.clone()));
                avatar_assets.graph = None;
                commands.spawn((
                    AvatarRoot,
                    Transform::default(),
                    Visibility::default(),
                    WorldAssetRoot(avatar_assets.scene.clone()),
                ));
            }
        }
    }
    let view_slot = |slot: usize| if slot == 0 { own_slot } else { slot };
    for (entity, root, mut transform, mut tint) in &mut roots {
        if let Some(p) = views.iter().find(|p| {
            p.slot == root.slot
                && p.robe == root.robe
                && root.epoch == epoch
                && root.handle == p.handle
        }) {
            *transform = player_transform(p, &play, root.view_slot);
            tint.0 = p.colour;
        } else {
            commands.entity(entity).despawn();
        }
    }
    for p in &views {
        if roots.iter().any(|(_, r, _, _)| {
            r.slot == p.slot && r.robe == p.robe && r.epoch == epoch && r.handle == p.handle
        }) {
            continue;
        }
        if let Some(path) = npcs.files.get(&p.robe.to_ascii_lowercase()) {
            commands.spawn((
                NetworkAvatar {
                    slot: p.slot,
                    epoch,
                    handle: p.handle,
                    robe: p.robe.clone(),
                    view_slot: view_slot(p.slot),
                },
                player_transform(p, &play, view_slot(p.slot)),
                crate::robe::RobeTint(p.colour),
                Visibility::default(),
                WorldAssetRoot(server.load(GltfAssetLabel::Scene(0).from_asset(path.clone()))),
            ));
        }
    }
}
/// An avatar drawn apart from the local primary player.
struct AvatarView {
    slot: usize,
    handle: u16,
    robe: String,
    colour: [f32; 3],
    position: [f32; 3],
    basis: [[f32; 3]; 3],
}
/// Every playing avatar but slot 0 (local co-op players and, on a host,
/// remote players), read from the authoritative session.
fn session_views(play: &Play) -> Vec<AvatarView> {
    let world = &play.session.world;
    world
        .players
        .iter()
        .enumerate()
        .skip(1)
        .filter_map(|(slot, p)| {
            let p = p.as_ref().filter(|p| p.playing)?;
            let body = &world.physics.bodies[p.body];
            Some(AvatarView {
                slot,
                handle: p.handle,
                robe: play.session.player_robe(slot)?.to_string(),
                colour: play.session.player_profiles[slot]
                    .as_ref()
                    .map_or([1.0; 3], |(_, c)| *c),
                position: body.position,
                basis: [
                    body.orientation.right,
                    body.orientation.up,
                    body.orientation.backward,
                ],
            })
        })
        .collect()
}
fn player_transform(p: &AvatarView, play: &Play, slot: usize) -> Transform {
    let body = play
        .session
        .world
        .players
        .get(slot)
        .and_then(Option::as_ref)
        .map(|p| &play.session.world.physics.bodies[p.body])
        .unwrap_or_else(|| play.session.body());
    Transform {
        translation: Vec3::from(p.position) - Vec3::Y * (body.radius + body.length * 0.5),
        rotation: Quat::from_mat3(&Mat3::from_cols(
            Vec3::from(p.basis[0]),
            Vec3::from(p.basis[1]),
            Vec3::from(p.basis[2]),
        ))
        .normalize(),
        ..default()
    }
}

#[derive(Component)]
pub(super) struct NetworkProp {
    handle: u16,
    generation: u64,
    asset: String,
}
pub(super) fn physics_visuals(
    mut commands: Commands,
    network: Option<NonSend<NetworkPlay>>,
    play: Res<Play>,
    server: Res<AssetServer>,
    mut roots: Query<(Entity, &NetworkProp, &mut Transform)>,
    mut files: Local<Option<HashMap<String, String>>>,
) {
    let Some(NetworkPlay::Client(client)) = network.as_deref() else {
        return;
    };
    let files = files.get_or_insert_with(|| {
        let mut files = HashMap::new();
        equipment::exported(&play.cache.join("gltf"), &play.cache, &mut files);
        files
    });
    for (entity, root, mut transform) in &mut roots {
        if let Some(body) = client.display_frame.physics.bodies.iter().find(|b| {
            b.handle == root.handle && b.generation == root.generation && b.asset == root.asset
        }) {
            *transform = Transform::from_matrix(Mat4::from_cols_array(&body.model_transform()));
        } else {
            commands.entity(entity).despawn();
        }
    }
    for body in &client.display_frame.physics.bodies {
        if roots.iter().any(|(_, r, _)| {
            r.handle == body.handle && r.generation == body.generation && r.asset == body.asset
        }) {
            continue;
        }
        if let Some(path) = files.get(&format!("gltf/{}.glb", body.asset).to_ascii_lowercase()) {
            commands.spawn((
                NetworkProp {
                    handle: body.handle,
                    generation: body.generation,
                    asset: body.asset.clone(),
                },
                Transform::from_matrix(Mat4::from_cols_array(&body.model_transform())),
                Visibility::default(),
                WorldAssetRoot(server.load(GltfAssetLabel::Scene(0).from_asset(path.clone()))),
            ));
        }
    }
}

/// Role controls on a client (rewrite extension; keys avoid the element
/// bindings Q W E R A S D F):
/// F6 join as a player, F7 step out to spectate; observers cycle the followed
/// player with Tab (Shift+Tab back; pad shoulder buttons) and roam with F9 +
/// arrows/left stick. Dungeon masters click to select, PageUp/PageDown pick a
/// template, Insert spawns it at the cursor, P possesses, O releases,
/// K smites, H heals, L toggles the hot-join lock, +/- change the limit.
#[allow(clippy::too_many_arguments)]
pub(super) fn role_keys(
    view: &mut ClientView,
    play: &Play,
    keys: &ButtonInput<KeyCode>,
    click: bool,
    pads: &Query<&Gamepad>,
    cursor_ndc: Option<[f32; 2]>,
    aspect: f32,
    dt: f32,
) -> Result<(), String> {
    use magicka_network::lobby::Role;
    if view.disconnected() || view.transport.is_leaving() {
        return Ok(());
    }
    if keys.just_pressed(KeyCode::F6) {
        view.request_role(Role::Player)?;
    }
    if keys.just_pressed(KeyCode::F7) {
        view.request_role(Role::Spectator)?;
    }
    if view.own_slot().is_some() {
        return Ok(());
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let pad = |b: GamepadButton| pads.iter().any(|p| p.just_pressed(b));
    if (keys.just_pressed(KeyCode::Tab) && !shift) || pad(GamepadButton::RightTrigger) {
        view.cycle_follow(1);
    }
    if (keys.just_pressed(KeyCode::Tab) && shift) || pad(GamepadButton::LeftTrigger) {
        view.cycle_follow(-1);
    }
    let possessing = view.participant().and_then(|p| p.possessed).is_some();
    if keys.just_pressed(KeyCode::F9) || pad(GamepadButton::LeftThumb) {
        view.observer.free = match view.observer.free {
            Some(_) => None,
            None => Some(play.session.camera().ground_position()),
        };
    }
    if let Some(free) = view.observer.free.as_mut().filter(|_| !possessing) {
        let axis = |a: KeyCode, b: KeyCode| f32::from(keys.pressed(a)) - f32::from(keys.pressed(b));
        let mut x = axis(KeyCode::ArrowRight, KeyCode::ArrowLeft);
        let mut z = axis(KeyCode::ArrowDown, KeyCode::ArrowUp);
        for p in pads.iter() {
            let sx = p.get(GamepadAxis::LeftStickX).unwrap_or(0.0);
            let sy = p.get(GamepadAxis::LeftStickY).unwrap_or(0.0);
            if sx.abs() + sy.abs() > 0.25 {
                x += sx;
                z -= sy;
            }
        }
        free[0] += x * 12.0 * dt;
        free[2] += z * 12.0 * dt;
    }
    if view.role() != Role::DungeonMaster {
        return Ok(());
    }
    let cursor = cursor_ndc.map(|ndc| {
        let height = play.session.camera().ground_position()[1];
        play.session.camera().mouse_pick(ndc, aspect, height)
    });
    if let Some(point) = cursor.filter(|_| click && !possessing) {
        view.observer.selected = view.character_near(point);
    }
    let count = DM_TEMPLATES.len();
    if keys.just_pressed(KeyCode::PageUp) {
        view.observer.template = (view.observer.template + count - 1) % count;
    }
    if keys.just_pressed(KeyCode::PageDown) {
        view.observer.template = (view.observer.template + 1) % count;
    }
    if keys.just_pressed(KeyCode::Insert) {
        if let Some(position) = cursor {
            let template = DM_TEMPLATES[view.observer.template].to_string();
            view.dm(DmCommand::SpawnNpc { template, position })?;
        }
    }
    let selected = view.observer.selected;
    if keys.just_pressed(KeyCode::KeyP) {
        if let Some(npc) = selected {
            view.dm(DmCommand::Possess { npc })?;
        }
    }
    if keys.just_pressed(KeyCode::KeyO) {
        view.dm(DmCommand::Release)?;
    }
    if keys.just_pressed(KeyCode::KeyK) {
        if let Some(handle) = selected {
            view.dm(DmCommand::Smite { handle })?;
        }
    }
    if keys.just_pressed(KeyCode::KeyH) {
        if let Some(handle) = selected {
            view.dm(DmCommand::Heal { handle })?;
        }
    }
    let lobby = view.frame.lobby;
    if keys.just_pressed(KeyCode::KeyL) {
        view.dm(DmCommand::SetHotJoin(!lobby.hot_join))?;
    }
    let limit = |delta: i32| {
        (i32::from(lobby.max_players) + delta)
            .clamp(1, magicka_network::lobby::PLAYER_SLOT_CAP as i32) as u16
    };
    if keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd) {
        view.dm(DmCommand::SetPlayerLimit(limit(1)))?;
    }
    if keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract) {
        view.dm(DmCommand::SetPlayerLimit(limit(-1)))?;
    }
    Ok(())
}

#[derive(Component)]
pub(super) struct RoleOverlay;
/// The seat's role, lobby policy and controls for clients without a wizard;
/// hides the dormant local avatar while observing.
pub(super) fn role_overlay(
    network: Option<NonSend<NetworkPlay>>,
    play: Res<Play>,
    mut text: Query<(&mut NativeBitmapText, &mut Transform, &mut Visibility), With<RoleOverlay>>,
    mut avatars: Query<&mut Visibility, (With<AvatarRoot>, Without<RoleOverlay>)>,
    coop: Option<Res<super::local_coop::LocalCoop>>,
    host_role: Option<Res<super::HostRole>>,
    mut previous: Local<String>,
) {
    use magicka_network::lobby::Role;
    let lines = match network.as_deref() {
        Some(NetworkPlay::Client(c)) if !c.disconnected() => {
            let lobby = c.frame.lobby;
            let policy = format!(
                "Players {}/{}  Hot join {}",
                c.frame.players.len(),
                lobby.max_players,
                if lobby.hot_join { "open" } else { "locked" }
            );
            match (c.own_slot(), c.role()) {
                (Some(_), _) => String::new(),
                (None, role) => {
                    let queued = c.welcome().is_some_and(|w| w.queued);
                    let mut lines = vec![format!(
                        "{}{}  -  {policy}",
                        role.label(),
                        if queued {
                            " (waiting for a player slot)"
                        } else {
                            ""
                        }
                    )];
                    let possessed = c.participant().and_then(|p| p.possessed);
                    match role {
                        Role::Invader => lines.push(match possessed {
                            Some(h) => format!("Piloting #{h}: move and attack as usual"),
                            None => "Waiting for an enemy to pilot".into(),
                        }),
                        Role::DungeonMaster => {
                            lines.push(format!(
                                "Spawn [PgUp/PgDn] {}  Insert: spawn at cursor",
                                DM_TEMPLATES[c.observer.template]
                            ));
                            lines.push(format!(
                                "Selected {}  P possess  O release  K smite  H heal  L lock  +/- limit",
                                c.observer
                                    .selected
                                    .map_or("none".into(), |h| format!("#{h}"))
                            ));
                        }
                        _ => {}
                    }
                    lines.push("Tab: next player  F9: free camera  F6: join as player".into());
                    lines.join("\n")
                }
            }
        }
        // A host without a wizard (lobby role badge).
        Some(NetworkPlay::Host(h)) if h.is_dedicated() => format!(
            "{} (host)  -  {}",
            host_role.as_ref().map_or("Observer", |r| r.0.label()),
            if h.has_players() {
                "in play"
            } else {
                "waiting for players to join"
            }
        ),
        _ => String::new(),
    };
    // The idle slot-0 wizard of a wizardless host is nobody's until the
    // first player joins.
    let observing = match network.as_deref() {
        Some(NetworkPlay::Host(h)) => h.is_dedicated() && !h.has_players(),
        _ => !lines.is_empty(),
    };
    let lines = match coop.as_ref().and_then(|c| c.notice.as_ref()) {
        Some((notice, _)) if lines.is_empty() => notice.clone(),
        Some((notice, _)) => format!("{lines}\n{notice}"),
        None => lines,
    };
    let shown = !lines.is_empty();
    for mut v in &mut avatars {
        let want = if observing {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *v != want {
            *v = want;
        }
    }
    for (mut text, mut transform, mut visibility) in &mut text {
        *visibility = if shown {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        transform.translation =
            Vec3::new(-play.screen[0] * 0.5 + 24., play.screen[1] * 0.5 - 72., 10.);
        if *previous != lines {
            text.typing.set_text(Some(&lines));
            *previous = lines.clone();
        }
        if shown && !text.typing.dirty {
            text.typing.finish();
        }
    }
}
