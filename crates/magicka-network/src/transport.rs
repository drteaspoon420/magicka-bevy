//! Lightyear UDP transport secured with netcode.io (see [`crate::security`]).
//!
//! Each compatible connection holds a seat. Player seats are player slots;
//! spectators, dungeon masters and invaders hold seats from
//! [`OBSERVER_SEAT_BASE`]. Admission follows the host's [`LobbySettings`].
use crate::lobby::{
    Admission, DmRequest, LobbySettings, Role, RoleRequest, OBSERVER_SEAT_BASE, PLAYER_SLOT_CAP,
};
use crate::security::{
    classify, credential_matches, datagram_allowed, packet_allowed, role_needs_credential, Bucket,
    Budgets, FragmentTracker, HandshakeLimiter, HostSecurity, NetworkKey, PacketClass,
    RequestBudgets, TemporaryBans, ABUSE_BAN, CLIENT_FRAGMENT_LIMIT, HANDSHAKE_DEADLINE,
    HELLO_DEADLINE, HOST_FRAGMENT_LIMIT, MALFORMED_STRIKES, MAX_PENDING_LINKS, NETCODE_PROTOCOL_ID,
    PACKETS_PER_STRIKE, RETIRE_GRACE, STRIKE_LIMIT, STRIKE_WINDOW,
};
use crate::session::{
    GameFrame, ModReply, ModRequest, ProfileReply, ProfileRequest, ProfileSubmit, RespawnRequest,
};
use crate::telemetry::TrafficStats;
use crate::{
    control::{InputCommand, PickupReceipt, PickupRequest},
    PhysicsFrame, PROTOCOL_VERSION,
};
use bevy::{prelude::*, time::TimeUpdateStrategy};
use lightyear::connection::ConnectionSystems;
use lightyear::netcode::auth::Authentication;
use lightyear::prelude::*;
use lightyear::transport::plugin::TransportSystems;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::{Duration, Instant},
};
/// Protocol 11 admission request. Sent with bounded retries until the host
/// seats or refuses the connection; liveness afterwards is [`Heartbeat`].
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Hello {
    version: u16,
    role: Role,
    name: String,
    /// The player's selected profile, validated before any avatar exists.
    profile: Option<ProfileSubmit>,
    /// Hash of the client's authoritative mod content; must equal the host's.
    rules_hash: Option<String>,
    /// Shown for the dungeon master role when the host requires one.
    credential: Option<String>,
}
/// Minimal liveness sample after admission (sequenced-unreliable).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Heartbeat {
    sequence: u32,
}
/// The host's immediate echo of a heartbeat, for round-trip timing.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct HeartbeatEcho {
    sequence: u32,
}
/// Host description sent once with the first [`Welcome`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    pub level: String,
    pub in_progress: bool,
    pub lobby: LobbySettings,
    pub rules_hash: Option<String>,
    /// Enabled mods as `id@version`, in load order.
    pub mods: Vec<String>,
}
/// Protocol 12: the newest game update a client reconstructed.
impl ServerInfo {
    /// This description with what a menu may display: a hostile host could
    /// otherwise put anything of any length on the join page.
    fn displayable(mut self) -> Self {
        self.name = crate::security::display_text(&self.name, 64);
        self.level = crate::security::display_text(&self.level, 256);
        self.rules_hash = self
            .rules_hash
            .map(|hash| crate::security::display_text(&hash, 128));
        self.mods.truncate(256);
        for name in &mut self.mods {
            *name = crate::security::display_text(name, 128);
        }
        self
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct BaselineAck {
    id: u64,
}
/// Protocol 13: release this connection's seat, including a pending admission.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Leave;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Left;
struct PhysicsChannel;
struct ControlChannel;
/// Acknowledgements: loss-tolerant, never blocked behind other traffic.
struct AckChannel;
/// Held input: newest sample wins; lost samples ride in later packets.
struct InputChannel;
/// The connection's current seat. Sent again whenever it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    pub seat: usize,
    pub role: Role,
    /// Waiting (as a spectator) for a player slot.
    pub queued: bool,
}
/// Why the host refused a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefusalCode {
    Version,
    InvalidProfile,
    LobbyFull,
    RoleDisabled,
    ContentMismatch,
    /// The host could not admit the profile (e.g. its robe is unavailable).
    Denied,
    /// The role needs a credential this client did not show.
    Credentials,
    /// Too many connections from this address.
    TooManyConnections,
    /// This address is refused, or the connection was removed for flooding.
    Banned,
    /// The host or a dungeon master removed this connection.
    Kicked,
}
/// Admission refused; the connection stays unseated.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Refused {
    reason: String,
    code: RefusalCode,
}
/// Per-class message buckets of a seated connection.
#[derive(Default)]
struct MessageBuckets {
    inputs: Bucket,
    heartbeats: Bucket,
    acks: Bucket,
    pickups: Bucket,
    profiles: Bucket,
    respawns: Bucket,
    roles: Bucket,
    dm: Bucket,
    mods: Bucket,
}
#[derive(Component)]
struct Compatible {
    /// Seat number; equals the player slot for players.
    slot: usize,
    role: Role,
    queued: bool,
    name: String,
    last_seen: Instant,
    profile: Option<ProfileSubmit>,
    /// Newest game update this connection acknowledged (delta base).
    acked: Option<u64>,
    /// This connection's frame history: frames are filtered per client
    /// (interest), so each connection deltas against what it received.
    encoder: crate::delta::Encoder,
    /// The canonical address this connection came from.
    ip: IpAddr,
    /// Budgets apply (always, except this computer's own connections).
    enforce: bool,
    /// Showed the dungeon master credential, or the host requires none.
    dm_verified: bool,
    buckets: MessageBuckets,
    strikes: u32,
}
impl Compatible {
    /// Charge one message to a bucket. Over budget it is dropped and counted.
    fn allow(
        &mut self,
        bucket: fn(&mut MessageBuckets) -> &mut Bucket,
        rate: crate::security::Rate,
        now: Instant,
    ) -> bool {
        if !self.enforce || bucket(&mut self.buckets).take(rate, 1.0, now) {
            return true;
        }
        self.strikes += 1;
        false
    }
}
/// Every server-side link: packet budget and how long it has been around.
#[derive(Component)]
struct LinkGuard {
    first_seen: Instant,
    ip: IpAddr,
    enforce: bool,
    packets: Bucket,
    bytes: Bucket,
    dropped: u32,
    strikes: u32,
    window: Instant,
    fragments: FragmentTracker,
}
/// Refused at admission; removed after [`RETIRE_GRACE`].
#[derive(Component)]
struct Refusal(Instant);
/// A connected participant as the host sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeatInfo {
    pub seat: usize,
    pub role: Role,
    pub name: String,
    pub queued: bool,
}
/// No longer seated; removed after [`RETIRE_GRACE`].
#[derive(Component)]
struct Retired(Instant);
#[derive(Resource)]
struct HostClock(Instant);
#[derive(Resource)]
struct HostPolicy {
    security: HostSecurity,
    temporary: TemporaryBans,
}
impl HostPolicy {
    fn budgets(&self) -> Budgets {
        self.security.budgets
    }
    fn enforce(&self, ip: IpAddr) -> bool {
        !ip.is_loopback() || self.security.enforce_on_loopback
    }
    fn banned(&mut self, ip: IpAddr, now: Instant) -> bool {
        self.enforce(ip) && (self.security.banned.contains(&ip) || self.temporary.active(ip, now))
    }
}
/// Seats whose link went away underneath them (the client disconnected, or
/// the handshake layer timed it out): (slot, role).
#[derive(Resource, Default)]
struct LostSeats(Vec<(usize, Role)>);
#[derive(Resource, Default)]
struct HostRequestBudgets(RequestBudgets);
fn seat_lost(
    trigger: On<Despawn, Compatible>,
    seats: Query<&Compatible>,
    mut lost: ResMut<LostSeats>,
) {
    if let Ok(owner) = seats.get(trigger.entity) {
        lost.0.push((owner.slot, owner.role));
    }
}
/// Modern policy, not a recovered retail timeout.
pub const PLAYER_LEASE: Duration = Duration::from_secs(10);
/// The handshake layer's own silence limit, in transport time: longer than
/// [`PLAYER_LEASE`] so seats normally end through the lease.
const SESSION_TIMEOUT_SECS: i32 = 30;
#[derive(Resource)]
struct AssignedSlots {
    /// Occupied player slots.
    slots: std::collections::BTreeSet<usize>,
    first_slot: usize,
    next_observer: usize,
    lobby: LobbySettings,
    /// The round/level has begun (hot join applies).
    in_progress: bool,
    /// Local players on a playable host (slot 0 and local co-op slots).
    local: std::collections::BTreeSet<usize>,
}
impl Default for AssignedSlots {
    fn default() -> Self {
        Self {
            slots: Default::default(),
            first_slot: 1,
            next_observer: OBSERVER_SEAT_BASE,
            lobby: LobbySettings::default(),
            in_progress: true,
            local: Default::default(),
        }
    }
}
impl AssignedSlots {
    fn players_in_use(&self) -> usize {
        self.slots.len() + usize::from(self.first_slot > 0) + self.local.len()
    }
    fn free_slot(&self) -> Option<usize> {
        (self.first_slot..PLAYER_SLOT_CAP)
            .find(|slot| !self.slots.contains(slot) && !self.local.contains(slot))
    }
    fn observer_seat(&mut self) -> usize {
        let seat = self.next_observer;
        self.next_observer += 1;
        seat
    }
    /// Seat a connection asking for `role`.
    fn seat(&mut self, role: Role) -> Result<(usize, Role, bool), &'static str> {
        match self
            .lobby
            .admit(role, self.players_in_use(), self.in_progress)
        {
            Admission::Player => {
                let slot = self.free_slot().ok_or("the lobby is full")?;
                self.slots.insert(slot);
                Ok((slot, Role::Player, false))
            }
            Admission::Observer { role, queued } => Ok((self.observer_seat(), role, queued)),
            Admission::Rejected(reason) => Err(reason),
        }
    }
}
pub struct OwnedInput {
    pub slot: usize,
    pub command: InputCommand,
}
#[derive(Resource, Default)]
struct HostInputs(Vec<OwnedInput>);
pub struct OwnedPickup {
    pub slot: usize,
    pub request: PickupRequest,
}
#[derive(Resource, Default)]
struct HostPickups(Vec<OwnedPickup>);
#[derive(Resource, Default)]
struct PickupReceipts(Vec<PickupReceipt>);
#[derive(Resource, Default)]
struct ClientSlot(
    Option<Welcome>,
    Option<(String, RefusalCode)>,
    Option<ServerInfo>,
);
#[derive(Resource, Default)]
struct HostTraffic(TrafficStats);
#[derive(Resource, Default)]
struct HostModRequests(Vec<(usize, ModRequest)>);
#[derive(Resource, Default)]
struct ModReplies(Vec<ModReply>);
#[derive(Resource, Default)]
struct ClientTraffic(TrafficStats);
#[derive(Resource, Default)]
struct HostInfo(ServerInfo);
/// Replication policy shared by every connection.
#[derive(Resource, Default)]
struct HostEncoder {
    full_only: bool,
    interest: Option<f32>,
}
#[derive(Resource, Default)]
struct ClientDecoder {
    decoder: crate::delta::Decoder,
    acked: Option<u64>,
}
/// Echoes received by the client: (sequence, arrival).
#[derive(Resource, Default)]
struct Echoes(Vec<(u32, Instant)>);
/// Simulated link degradation applied to new host-side links.
#[derive(Resource, Default)]
struct HostConditions(Option<NetConditions>);
#[derive(Resource, Default)]
struct HostRoles(Vec<(Entity, RoleRequest)>);
#[derive(Resource, Default)]
struct HostLeaves(Vec<Entity>);
#[derive(Resource, Default)]
struct Departure {
    requested: Option<Instant>,
    acknowledged: bool,
}
pub struct OwnedDm {
    pub seat: usize,
    pub role: Role,
    pub request: DmRequest,
}
#[derive(Resource, Default)]
struct HostDm(Vec<OwnedDm>);
#[derive(Resource, Default)]
struct Inbox(Vec<PhysicsFrame>);
pub struct OwnedProfile {
    pub slot: usize,
    pub request: ProfileRequest,
}
#[derive(Resource, Default)]
struct HostProfiles(Vec<OwnedProfile>);
#[derive(Resource, Default)]
struct GameInbox(Vec<GameFrame>);
#[derive(Resource, Default)]
struct ProfileReplies(Vec<ProfileReply>);
pub struct OwnedRespawn {
    pub slot: usize,
    pub request: RespawnRequest,
}
#[derive(Resource, Default)]
struct HostRespawns(Vec<OwnedRespawn>);
fn receive_respawns(
    mut q: Query<(&mut Compatible, &mut MessageReceiver<RespawnRequest>)>,
    mut inbox: ResMut<HostRespawns>,
    policy: Res<HostPolicy>,
    clock: Res<HostClock>,
) {
    let rate = policy.budgets().respawns;
    for (mut owner, mut receiver) in &mut q {
        for request in receiver.receive() {
            if owner.allow(|b| &mut b.respawns, rate, clock.0) {
                inbox.0.push(OwnedRespawn {
                    slot: owner.slot,
                    request,
                });
            }
        }
    }
}
fn receive_profiles(
    mut q: Query<(&mut Compatible, &mut MessageReceiver<ProfileRequest>)>,
    mut inbox: ResMut<HostProfiles>,
    policy: Res<HostPolicy>,
    clock: Res<HostClock>,
) {
    let rate = policy.budgets().profiles;
    for (mut owner, mut receiver) in &mut q {
        for request in receiver.receive() {
            if owner.allow(|b| &mut b.profiles, rate, clock.0) {
                inbox.0.push(OwnedProfile {
                    slot: owner.slot,
                    request,
                });
            }
        }
    }
}
fn receive_games(
    mut q: Query<(
        &mut MessageReceiver<crate::delta::GameUpdate>,
        &mut MessageSender<BaselineAck>,
    )>,
    mut inbox: ResMut<GameInbox>,
    mut decoder: ResMut<ClientDecoder>,
    mut traffic: ResMut<ClientTraffic>,
) {
    for (mut receiver, mut ack) in &mut q {
        for update in receiver.receive() {
            let kind = match update.payload {
                crate::delta::Payload::Full(_) => "GameUpdate.full",
                crate::delta::Payload::Delta(_) => "GameUpdate.delta",
            };
            traffic.0.received(kind, &update);
            match decoder.decoder.apply(update) {
                Ok(frame) => inbox.0.push(frame),
                Err(_) => traffic.0.rejected(kind),
            }
        }
        let newest = decoder.decoder.acknowledged();
        if let Some(id) = newest.filter(|_| newest != decoder.acked) {
            decoder.acked = newest;
            traffic.0.timing.acked_baseline = newest;
            let message = BaselineAck { id };
            traffic.0.sent("BaselineAck", &message);
            ack.send::<AckChannel>(message);
        }
    }
}
fn receive_mod_requests(
    mut q: Query<(&mut Compatible, &mut MessageReceiver<ModRequest>)>,
    mut inbox: ResMut<HostModRequests>,
    policy: Res<HostPolicy>,
    clock: Res<HostClock>,
) {
    let rate = policy.budgets().mods;
    for (mut owner, mut receiver) in &mut q {
        for request in receiver.receive().filter(ModRequest::valid) {
            if owner.allow(|b| &mut b.mods, rate, clock.0) && owner.role == Role::Player {
                inbox.0.push((owner.slot, request));
            }
        }
    }
}
fn receive_mod_replies(
    mut q: Query<&mut MessageReceiver<ModReply>>,
    mut inbox: ResMut<ModReplies>,
) {
    for mut receiver in &mut q {
        inbox.0.extend(receiver.receive());
    }
}
fn receive_acks(
    mut q: Query<(&mut Compatible, &mut MessageReceiver<BaselineAck>)>,
    mut traffic: ResMut<HostTraffic>,
    policy: Res<HostPolicy>,
    clock: Res<HostClock>,
) {
    let rate = policy.budgets().acks;
    for (mut owner, mut receiver) in &mut q {
        for ack in receiver.receive() {
            traffic.0.received("BaselineAck", &ack);
            if owner.allow(|b| &mut b.acks, rate, clock.0) {
                owner.acked = owner.acked.max(Some(ack.id));
            }
        }
    }
}
fn receive_echoes(
    mut q: Query<&mut MessageReceiver<HeartbeatEcho>>,
    mut echoes: ResMut<Echoes>,
    mut traffic: ResMut<ClientTraffic>,
) {
    let now = Instant::now();
    for mut receiver in &mut q {
        for echo in receiver.receive() {
            traffic.0.received("HeartbeatEcho", &echo);
            echoes.0.push((echo.sequence, now));
        }
    }
}
fn receive_server_info(
    mut q: Query<&mut MessageReceiver<ServerInfo>>,
    mut slot: ResMut<ClientSlot>,
) {
    for mut receiver in &mut q {
        if let Some(info) = receiver.receive().last() {
            slot.2 = Some(info.displayable());
        }
    }
}
/// Degrade newly linked host-side connections for latency/loss QA.
fn condition_links(
    mut q: Query<&mut lightyear::link::Link, Added<lightyear::link::Link>>,
    conditions: Res<HostConditions>,
) {
    if let Some(c) = conditions.0 {
        for mut link in &mut q {
            link.recv.conditioner = Some(c.conditioner());
        }
    }
}
fn receive_profile_replies(
    mut q: Query<&mut MessageReceiver<ProfileReply>>,
    mut inbox: ResMut<ProfileReplies>,
) {
    for mut receiver in &mut q {
        inbox.0.extend(receiver.receive());
    }
}
type HelloQuery<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static mut MessageReceiver<Hello>,
        &'static mut MessageSender<Welcome>,
        &'static mut MessageSender<Refused>,
        &'static mut MessageSender<ServerInfo>,
        &'static PeerAddr,
    ),
    (Without<Compatible>, Without<Retired>, Without<Refusal>),
>;
/// The canonical form of a peer's address.
fn peer_ip(address: SocketAddr) -> IpAddr {
    address.ip().to_canonical()
}
fn clean_name(name: &str, role: Role) -> String {
    let name = crate::security::display_text(name, 32);
    if name.trim().is_empty() {
        role.label().into()
    } else {
        name
    }
}
/// The admission decision for one hello, before any seat is taken.
fn screen_hello(hello: &Hello, info: &ServerInfo) -> Result<(), (String, RefusalCode)> {
    if hello.version != PROTOCOL_VERSION {
        return Err((
            format!(
                "incompatible protocol {} (host {PROTOCOL_VERSION})",
                hello.version
            ),
            RefusalCode::Version,
        ));
    }
    if hello.rules_hash != info.rules_hash {
        return Err((
            "the host runs different game content (mods)".into(),
            RefusalCode::ContentMismatch,
        ));
    }
    if hello.role == Role::Player {
        if let Some(problem) = hello.profile.as_ref().and_then(ProfileSubmit::problem) {
            return Err((problem.into(), RefusalCode::InvalidProfile));
        }
    }
    Ok(())
}
fn refusal_code(reason: &str) -> RefusalCode {
    if reason.contains("full") {
        RefusalCode::LobbyFull
    } else {
        RefusalCode::RoleDisabled
    }
}
#[allow(clippy::too_many_arguments)]
fn accept_hello(
    mut q: HelloQuery<'_, '_>,
    seated: Query<&Compatible>,
    mut commands: Commands,
    mut slots: ResMut<AssignedSlots>,
    clock: Res<HostClock>,
    info: Res<HostInfo>,
    mut traffic: ResMut<HostTraffic>,
    mut policy: ResMut<HostPolicy>,
) {
    let mut per_address: BTreeMap<IpAddr, usize> = BTreeMap::new();
    for owner in &seated {
        *per_address.entry(owner.ip).or_default() += 1;
    }
    for (entity, mut receiver, mut sender, mut refuse, mut info_sender, peer) in &mut q {
        // Retries of one hello are identical; the newest decides.
        let Some(hello) = receiver.receive().last() else {
            continue;
        };
        traffic.0.received("Hello", &hello);
        let ip = peer_ip(peer.0);
        let enforce = policy.enforce(ip);
        let dm_verified = policy
            .security
            .dm_credential
            .as_deref()
            .is_none_or(|expected| credential_matches(expected, hello.credential.as_deref()));
        let limit = policy.security.connections_per_address;
        let decision = if policy.banned(ip, clock.0) {
            Err((
                "this address is not allowed to join".to_string(),
                RefusalCode::Banned,
            ))
        } else if enforce && per_address.get(&ip).copied().unwrap_or(0) >= limit {
            Err((
                "too many connections from this address".to_string(),
                RefusalCode::TooManyConnections,
            ))
        } else if role_needs_credential(hello.role, dm_verified) {
            Err((
                "the dungeon master role needs the host's credential".to_string(),
                RefusalCode::Credentials,
            ))
        } else {
            screen_hello(&hello, &info.0).and_then(|()| {
                slots
                    .seat(hello.role)
                    .map_err(|reason| (reason.to_string(), refusal_code(reason)))
            })
        };
        match decision {
            Ok((slot, role, queued)) => {
                *per_address.entry(ip).or_default() += 1;
                commands.entity(entity).insert(Compatible {
                    slot,
                    role,
                    queued,
                    name: clean_name(&hello.name, role),
                    last_seen: clock.0,
                    profile: hello.profile.filter(|_| hello.role == Role::Player),
                    acked: None,
                    encoder: Default::default(),
                    ip,
                    enforce,
                    dm_verified,
                    buckets: Default::default(),
                    strikes: 0,
                });
                let mut server = info.0.clone();
                server.lobby = slots.lobby;
                server.in_progress = slots.in_progress;
                traffic.0.sent("ServerInfo", &server);
                info_sender.send::<ControlChannel>(server);
                let welcome = Welcome {
                    seat: slot,
                    role,
                    queued,
                };
                traffic.0.sent("Welcome", &welcome);
                sender.send::<ControlChannel>(welcome);
            }
            Err((reason, code)) => {
                // Refused sockets stay unseated; a new client process retries.
                commands.entity(entity).insert(Refusal(clock.0));
                refuse.send::<ControlChannel>(Refused { reason, code });
            }
        }
    }
}
fn receive_roles(
    mut q: Query<(Entity, &mut Compatible, &mut MessageReceiver<RoleRequest>)>,
    mut inbox: ResMut<HostRoles>,
    policy: Res<HostPolicy>,
    clock: Res<HostClock>,
) {
    let rate = policy.budgets().roles;
    for (entity, mut owner, mut receiver) in &mut q {
        for request in receiver.receive() {
            if owner.allow(|b| &mut b.roles, rate, clock.0) {
                inbox.0.push((entity, request));
            }
        }
    }
}
fn receive_leaves(
    mut q: Query<(
        Entity,
        &mut MessageReceiver<Leave>,
        &mut MessageSender<Left>,
    )>,
    mut inbox: ResMut<HostLeaves>,
) {
    for (entity, mut receiver, mut sender) in &mut q {
        if receiver.receive().count() > 0 {
            inbox.0.push(entity);
            sender.send::<ControlChannel>(Left);
        }
    }
}
fn receive_left(mut q: Query<&mut MessageReceiver<Left>>, mut departure: ResMut<Departure>) {
    for mut receiver in &mut q {
        if receiver.receive().count() > 0 && departure.requested.is_some() {
            departure.acknowledged = true;
        }
    }
}
fn receive_dm(
    mut q: Query<(&mut Compatible, &mut MessageReceiver<DmRequest>)>,
    mut inbox: ResMut<HostDm>,
    policy: Res<HostPolicy>,
    clock: Res<HostClock>,
) {
    let rate = policy.budgets().dm;
    for (mut owner, mut receiver) in &mut q {
        for request in receiver.receive().filter(DmRequest::valid) {
            if owner.allow(|b| &mut b.dm, rate, clock.0) {
                inbox.0.push(OwnedDm {
                    seat: owner.slot,
                    role: owner.role,
                    request,
                });
            }
        }
    }
}
/// Messages from connections without a seat are never read by the systems
/// above; without this they would pile up until the link is removed.
#[allow(clippy::type_complexity)]
fn drain_unseated(
    mut q: Query<
        (
            &mut MessageReceiver<InputCommand>,
            &mut MessageReceiver<Heartbeat>,
            &mut MessageReceiver<BaselineAck>,
            &mut MessageReceiver<ModRequest>,
            &mut MessageReceiver<PickupRequest>,
            &mut MessageReceiver<ProfileRequest>,
            &mut MessageReceiver<RespawnRequest>,
            &mut MessageReceiver<RoleRequest>,
            &mut MessageReceiver<DmRequest>,
        ),
        Without<Compatible>,
    >,
    mut hellos: Query<&mut MessageReceiver<Hello>, Or<(With<Retired>, With<Refusal>)>>,
) {
    for (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h, mut i) in &mut q {
        a.receive().for_each(drop);
        b.receive().for_each(drop);
        c.receive().for_each(drop);
        d.receive().for_each(drop);
        e.receive().for_each(drop);
        f.receive().for_each(drop);
        g.receive().for_each(drop);
        h.receive().for_each(drop);
        i.receive().for_each(drop);
    }
    for mut hello in &mut hellos {
        hello.receive().for_each(drop);
    }
}
/// Runs on decrypted packets before the transport parses them: drop what
/// could harm the receiver, and charge each link for what it sends.
fn guard_host_links(
    mut links: Query<
        (
            Entity,
            &mut lightyear::link::Link,
            Option<&mut LinkGuard>,
            &PeerAddr,
        ),
        With<LinkOf>,
    >,
    mut commands: Commands,
    clock: Res<HostClock>,
    policy: Res<HostPolicy>,
) {
    let now = clock.0;
    let budgets = policy.budgets();
    for (entity, mut link, guard, peer) in &mut links {
        let Some(mut guard) = guard else {
            let ip = peer_ip(peer.0);
            commands.entity(entity).try_insert(LinkGuard {
                first_seen: now,
                ip,
                enforce: policy.enforce(ip),
                packets: Bucket::default(),
                bytes: Bucket::default(),
                dropped: 0,
                strikes: 0,
                window: now,
                fragments: Default::default(),
            });
            filter_packets(&mut link, HOST_FRAGMENT_LIMIT);
            continue;
        };
        for _ in 0..link.recv.len() {
            let Some(packet) = link.recv.pop() else { break };
            let allowed = match classify(&packet, HOST_FRAGMENT_LIMIT) {
                Some(PacketClass::Plain) => true,
                Some(PacketClass::Fragment(place)) => guard.fragments.admit(place),
                None => false,
            };
            if !allowed {
                guard.strikes += MALFORMED_STRIKES;
                continue;
            }
            if guard.enforce
                && !(guard.packets.take(budgets.packets, 1.0, now)
                    && guard.bytes.take(budgets.bytes, packet.len() as f32, now))
            {
                guard.dropped += 1;
                if guard.dropped.is_multiple_of(PACKETS_PER_STRIKE) {
                    guard.strikes += 1;
                }
                continue;
            }
            link.recv.push_raw(packet);
        }
    }
}
/// Host only, before netcode: one address cannot use up the handshake limit.
fn limit_requests(
    mut links: Query<(&mut lightyear::link::Link, &PeerAddr), With<LinkOf>>,
    mut budgets: ResMut<HostRequestBudgets>,
    clock: Res<HostClock>,
    policy: Res<HostPolicy>,
    mut traffic: ResMut<HostTraffic>,
) {
    for (mut link, peer) in &mut links {
        let ip = peer_ip(peer.0);
        if !policy.enforce(ip) {
            continue;
        }
        for _ in 0..link.recv.len() {
            let Some(datagram) = link.recv.pop() else {
                break;
            };
            // A connection request is the only datagram with a zero first byte.
            if datagram.first() == Some(&0) && !budgets.0.allow(ip, clock.0) {
                traffic.0.rejected("ConnectionRequest");
                continue;
            }
            link.recv.push_raw(datagram);
        }
    }
}
/// Runs on raw datagrams, before netcode parses them. Hosts and clients both.
fn filter_datagrams(mut links: Query<&mut lightyear::link::Link>) {
    for mut link in &mut links {
        for _ in 0..link.recv.len() {
            let Some(datagram) = link.recv.pop() else {
                break;
            };
            if datagram_allowed(&datagram) {
                link.recv.push_raw(datagram);
            }
        }
    }
}
fn filter_packets(link: &mut lightyear::link::Link, limit: u64) {
    for _ in 0..link.recv.len() {
        let Some(packet) = link.recv.pop() else { break };
        if packet_allowed(&packet, limit) {
            link.recv.push_raw(packet);
        }
    }
}
fn guard_client_link(mut links: Query<&mut lightyear::link::Link, With<client::Client>>) {
    for mut link in &mut links {
        filter_packets(&mut link, CLIENT_FRAGMENT_LIMIT);
    }
}
type HeartbeatQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Compatible,
        &'static mut MessageReceiver<Hello>,
        &'static mut MessageReceiver<Heartbeat>,
        &'static mut MessageSender<HeartbeatEcho>,
    ),
>;
fn receive_heartbeat(
    mut q: HeartbeatQuery<'_, '_>,
    clock: Res<HostClock>,
    mut traffic: ResMut<HostTraffic>,
    policy: Res<HostPolicy>,
) {
    let rate = policy.budgets().heartbeats;
    for (mut owner, mut hellos, mut receiver, mut echo) in &mut q {
        // Late hello retries after admission carry nothing new.
        hellos.receive().for_each(drop);
        for beat in receiver.receive() {
            traffic.0.received("Heartbeat", &beat);
            if !owner.allow(|b| &mut b.heartbeats, rate, clock.0) {
                continue;
            }
            owner.last_seen = clock.0;
            let reply = HeartbeatEcho {
                sequence: beat.sequence,
            };
            traffic.0.sent("HeartbeatEcho", &reply);
            echo.send::<PhysicsChannel>(reply);
        }
    }
}
fn receive_inputs(
    mut q: Query<(&mut Compatible, &mut MessageReceiver<InputCommand>)>,
    mut inbox: ResMut<HostInputs>,
    clock: Res<HostClock>,
    mut traffic: ResMut<HostTraffic>,
    policy: Res<HostPolicy>,
) {
    let rate = policy.budgets().inputs;
    for (mut owner, mut receiver) in &mut q {
        for command in receiver.receive() {
            traffic.0.received("InputCommand", &command);
            if !command.valid() {
                traffic.0.rejected("InputCommand");
                owner.strikes += 1;
                continue;
            }
            if !owner.allow(|b| &mut b.inputs, rate, clock.0) {
                traffic.0.rejected("InputCommand");
                continue;
            }
            // Input proves liveness as well as heartbeats do.
            owner.last_seen = clock.0;
            inbox.0.push(OwnedInput {
                slot: owner.slot,
                command,
            });
        }
    }
}
fn receive_welcome(mut q: Query<&mut MessageReceiver<Welcome>>, mut slot: ResMut<ClientSlot>) {
    for mut receiver in &mut q {
        for welcome in receiver.receive() {
            let valid = match welcome.role {
                Role::Player => welcome.seat < PLAYER_SLOT_CAP,
                _ => welcome.seat >= OBSERVER_SEAT_BASE,
            };
            // The host re-sends Welcome on every role change.
            if valid {
                slot.0 = Some(welcome);
            }
        }
    }
}
fn receive_refusal(mut q: Query<&mut MessageReceiver<Refused>>, mut slot: ResMut<ClientSlot>) {
    for mut receiver in &mut q {
        if let Some(refusal) = receiver.receive().last() {
            slot.1 = Some((
                crate::security::display_text(&refusal.reason, 256),
                refusal.code,
            ));
        }
    }
}
fn receive_pickups(
    mut q: Query<(&mut Compatible, &mut MessageReceiver<PickupRequest>)>,
    mut inbox: ResMut<HostPickups>,
    policy: Res<HostPolicy>,
    clock: Res<HostClock>,
) {
    let rate = policy.budgets().pickups;
    for (mut owner, mut receiver) in &mut q {
        for request in receiver.receive().filter(PickupRequest::valid) {
            if owner.allow(|b| &mut b.pickups, rate, clock.0) {
                inbox.0.push(OwnedPickup {
                    slot: owner.slot,
                    request,
                });
            }
        }
    }
}
fn receive_receipts(
    mut q: Query<&mut MessageReceiver<PickupReceipt>>,
    mut inbox: ResMut<PickupReceipts>,
) {
    for mut receiver in &mut q {
        inbox.0.extend(receiver.receive());
    }
}
fn receive_frames(
    mut q: Query<&mut MessageReceiver<PhysicsFrame>>,
    mut inbox: ResMut<Inbox>,
    mut traffic: ResMut<ClientTraffic>,
) {
    for mut receiver in &mut q {
        for frame in receiver.receive() {
            traffic.0.received("PhysicsFrame", &frame);
            inbox.0.push(frame);
        }
    }
}
/// Simulated latency, jitter and loss on a receiving link (QA harness).
/// Applied on receive, so condition both ends for a round trip.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NetConditions {
    pub latency: Duration,
    pub jitter: Duration,
    /// Independent loss probability per packet (0..=1).
    pub loss: f32,
}
impl NetConditions {
    fn conditioner(self) -> lightyear::link::RecvLinkConditioner {
        lightyear::link::LinkConditioner::new(lightyear::link::prelude::LinkConditionerConfig {
            incoming_latency: self.latency,
            incoming_jitter: self.jitter,
            good_loss: self.loss.clamp(0.0, 1.0),
            bad_loss: self.loss.clamp(0.0, 1.0),
            good_to_bad: 0.0,
            bad_to_good: 1.0,
        })
    }
}
/// Everything a client states when it asks to be seated.
#[derive(Clone, Debug, Default)]
pub struct JoinRequest {
    pub role: Role,
    pub name: String,
    pub profile: Option<ProfileSubmit>,
    pub rules_hash: Option<String>,
    pub conditions: Option<NetConditions>,
    /// The game's key: open by default, or derived from its passphrase.
    pub key: NetworkKey,
    /// The dungeon master credential, for hosts that require one.
    pub credential: Option<String>,
}
/// Where a client is in admission.
#[derive(Clone, Debug, PartialEq)]
pub enum Admission11 {
    /// Hello sent `attempts` times, no answer yet.
    Negotiating {
        attempts: u32,
    },
    Seated(Welcome),
    Refused(String, RefusalCode),
    /// No answer after every bounded retry.
    TimedOut,
    Leaving,
    Left,
}
/// Hello retry policy (modern; not a retail value).
pub const HELLO_RETRY: Duration = Duration::from_millis(500);
pub const HELLO_ATTEMPTS: u32 = 40;
/// Hellos sent on consecutive updates before the slow retry begins.
pub const HELLO_BURST: u32 = 10;
/// Heartbeat period after admission; far inside [`PLAYER_LEASE`].
pub const HEARTBEAT_PERIOD: Duration = Duration::from_millis(250);
/// ...or after this many client updates, whichever comes first, so a host
/// driven by its own clock (dedicated fixed step, deterministic tests) sees
/// liveness in its own time base.
pub const HEARTBEAT_UPDATES: u32 = 15;
/// Resolve a join address ("localhost:17700", "host.lan:17700",
/// "[::1]:17700"), preferring IPv4: hosts usually listen on IPv4 and many
/// resolvers list "localhost" as ::1 first.
pub fn resolve(address: &str) -> Result<SocketAddr, String> {
    use std::net::ToSocketAddrs;
    let all: Vec<SocketAddr> = address
        .to_socket_addrs()
        .map_err(|e| format!("cannot resolve {address}: {e}"))?
        .collect();
    all.iter()
        .find(|a| a.is_ipv4())
        .or_else(|| all.first())
        .copied()
        .ok_or_else(|| format!("{address} did not resolve"))
}
pub struct LoopbackHost {
    app: App,
    server: Entity,
    departed: Vec<usize>,
    departed_observers: Vec<usize>,
}
pub struct LoopbackClient {
    app: App,
    role: Role,
    name: String,
    request: JoinRequest,
    /// Hellos sent since the secure session came up.
    hellos_sent: u32,
    /// The secure session failed (no answer, wrong passphrase, refused).
    link_failed: bool,
    hello_attempts: u32,
    last_hello: Option<Instant>,
    heartbeat: u32,
    last_heartbeat: Option<Instant>,
    updates_since_beat: u32,
    /// Sent heartbeats awaiting an echo.
    pending_beats: std::collections::VecDeque<(u32, Instant)>,
}
fn protocol(app: &mut App) {
    app.register_message::<PhysicsFrame>()
        .add_direction(NetworkDirection::ServerToClient);
    app.register_message::<Hello>()
        .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<Heartbeat>()
        .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<HeartbeatEcho>()
        .add_direction(NetworkDirection::ServerToClient);
    app.register_message::<ServerInfo>()
        .add_direction(NetworkDirection::ServerToClient);
    app.register_message::<Welcome>()
        .add_direction(NetworkDirection::ServerToClient);
    app.register_message::<InputCommand>()
        .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<PickupRequest>()
        .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<PickupReceipt>()
        .add_direction(NetworkDirection::ServerToClient);
    app.register_message::<crate::delta::GameUpdate>()
        .add_direction(NetworkDirection::ServerToClient);
    app.register_message::<BaselineAck>()
        .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<ModRequest>()
        .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<ModReply>()
        .add_direction(NetworkDirection::ServerToClient);
    app.register_message::<RespawnRequest>()
        .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<ProfileRequest>()
        .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<ProfileReply>()
        .add_direction(NetworkDirection::ServerToClient);
    app.register_message::<Refused>()
        .add_direction(NetworkDirection::ServerToClient);
    app.register_message::<RoleRequest>()
        .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<DmRequest>()
        .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<Leave>()
        .add_direction(NetworkDirection::ClientToServer);
    app.register_message::<Left>()
        .add_direction(NetworkDirection::ServerToClient);
    app.add_channel::<PhysicsChannel>(ChannelSettings {
        mode: ChannelMode::SequencedUnreliable,
        ..default()
    })
    .add_direction(NetworkDirection::Bidirectional);
    app.add_channel::<ControlChannel>(ChannelSettings {
        mode: ChannelMode::OrderedReliable(default()),
        ..default()
    })
    .add_direction(NetworkDirection::Bidirectional);
    app.add_channel::<InputChannel>(ChannelSettings {
        mode: ChannelMode::SequencedUnreliable,
        ..default()
    })
    .add_direction(NetworkDirection::Bidirectional);
    app.add_channel::<AckChannel>(ChannelSettings {
        mode: ChannelMode::UnorderedUnreliable,
        ..default()
    })
    .add_direction(NetworkDirection::Bidirectional);
}
fn base() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1. / 60.,
        )));
    app
}
impl LoopbackHost {
    pub fn bind(address: SocketAddr) -> Result<Self, String> {
        Self::bind_mode(address, false)
    }
    pub fn bind_mode(address: SocketAddr, dedicated: bool) -> Result<Self, String> {
        Self::bind_lobby(address, dedicated, LobbySettings::default())
    }
    pub fn bind_lobby(
        address: SocketAddr,
        dedicated: bool,
        lobby: LobbySettings,
    ) -> Result<Self, String> {
        Self::bind_secured(address, dedicated, lobby, HostSecurity::default())
    }
    pub fn bind_secured(
        address: SocketAddr,
        dedicated: bool,
        lobby: LobbySettings,
        security: HostSecurity,
    ) -> Result<Self, String> {
        if !lobby.valid() {
            return Err("invalid lobby settings".into());
        }
        // lightyear treats a failed bind as a fatal system error; find out
        // here. (Closed again at once: the transport opens its own socket.)
        drop(
            std::net::UdpSocket::bind(address)
                .map_err(|e| format!("cannot listen on {address}: {e}"))?,
        );
        let key = security.key.bytes();
        let mut app = base();
        app.add_plugins(server::ServerPlugins::default())
            .insert_resource(HostClock(Instant::now()))
            .init_resource::<AssignedSlots>()
            .init_resource::<HostInputs>()
            .init_resource::<HostPickups>()
            .init_resource::<HostProfiles>()
            .init_resource::<HostRespawns>()
            .init_resource::<HostRoles>()
            .init_resource::<HostLeaves>()
            .init_resource::<HostDm>()
            .init_resource::<HostTraffic>()
            .init_resource::<HostInfo>()
            .init_resource::<HostConditions>()
            .init_resource::<HostEncoder>()
            .init_resource::<HostModRequests>()
            .init_resource::<LostSeats>()
            .init_resource::<HostRequestBudgets>()
            .insert_resource(HostPolicy {
                security,
                temporary: Default::default(),
            })
            .add_observer(seat_lost)
            .add_systems(
                PreUpdate,
                (
                    (filter_datagrams, limit_requests)
                        .chain()
                        .after(lightyear::link::LinkSystems::Receive)
                        .before(ConnectionSystems::Receive),
                    guard_host_links
                        .after(ConnectionSystems::Receive)
                        .before(TransportSystems::Receive),
                ),
            )
            .insert_resource(AssignedSlots {
                first_slot: usize::from(!dedicated),
                lobby,
                ..default()
            })
            .add_systems(
                Update,
                (
                    condition_links,
                    accept_hello,
                    receive_heartbeat,
                    receive_acks,
                    receive_mod_requests,
                    receive_inputs,
                    receive_pickups,
                    receive_profiles,
                    receive_respawns,
                    receive_roles,
                    receive_dm,
                    receive_leaves,
                    drain_unseated,
                )
                    .chain(),
            );
        protocol(&mut app);
        app.finish();
        app.cleanup();
        app.update();
        let entity = app
            .world_mut()
            .spawn((
                server::NetcodeServer::new(server::NetcodeConfig {
                    // Connections are limited by the lobby and the per-address
                    // limit. The handshake layer's own count must not be able
                    // to fill up: a peer can choose never to time out in it.
                    max_clients: usize::MAX,
                    client_timeout_secs: SESSION_TIMEOUT_SECS,
                    protocol_id: NETCODE_PROTOCOL_ID,
                    private_key: key,
                    // Hosts bind 0.0.0.0 and may sit behind NAT, so the address
                    // a client dialled is not one the host can recognise.
                    server_addr_check: false,
                    connection_request_handler: Some(std::sync::Arc::new(HandshakeLimiter::new())),
                    ..Default::default()
                }),
                server::ServerUdpIo::default(),
                LocalAddr(address),
            ))
            .id();
        app.world_mut().trigger(server::Start { entity });
        app.world_mut().flush();
        app.update();
        if !app.world().entity(entity).contains::<server::Started>() {
            return Err("UDP server did not start".into());
        }
        Ok(Self {
            app,
            server: entity,
            departed: Vec::new(),
            departed_observers: Vec::new(),
        })
    }
    pub fn address(&self) -> SocketAddr {
        self.app.world().get::<LocalAddr>(self.server).unwrap().0
    }
    /// Links currently held for remote addresses, seated or not.
    pub fn link_count(&mut self) -> usize {
        let world = self.app.world_mut();
        world
            .query_filtered::<(), With<LinkOf>>()
            .iter(world)
            .count()
    }
    /// Lose a seat's link the way a disconnect at the handshake layer does.
    #[doc(hidden)]
    pub fn despawn_link_for_test(&mut self, seat: usize) {
        let world = self.app.world_mut();
        let mut q = world.query::<(Entity, &Compatible)>();
        let found = q.iter(world).find(|(_, o)| o.slot == seat).map(|(e, _)| e);
        if let Some(entity) = found {
            world.entity_mut(entity).despawn();
        }
    }
    pub fn update(&mut self) {
        self.update_at(Instant::now());
    }
    /// Explicit monotonic clock for deterministic lease verification.
    pub fn update_at(&mut self, now: Instant) {
        self.app.world_mut().resource_mut::<HostClock>().0 = now;
        self.app.update();
        self.release_lost_seats();
        self.enforce_strikes(now);
        let world = self.app.world_mut();
        let mut q = world.query::<(Entity, &Compatible)>();
        let expired: Vec<_> = q
            .iter(world)
            .filter(|(_, owner)| now.saturating_duration_since(owner.last_seen) >= PLAYER_LEASE)
            .map(|(entity, _)| entity)
            .collect();
        for entity in expired {
            self.retire(entity);
        }
        let leaving = std::mem::take(&mut self.app.world_mut().resource_mut::<HostLeaves>().0);
        for entity in leaving {
            self.retire(entity);
        }
        self.apply_role_requests();
        let promote = {
            let slots = self.app.world().resource::<AssignedSlots>();
            slots.lobby.hot_join || !slots.in_progress
        };
        if promote {
            self.promote_queued();
        }
        self.sweep_links(now);
    }

    /// Retire only this socket; late messages cannot affect its replacement.
    fn retire(&mut self, entity: Entity) {
        let world = self.app.world_mut();
        let owner = world.get::<Compatible>(entity).map(|o| (o.slot, o.role));
        let now = world.resource::<HostClock>().0;
        // Cancellation can arrive before an admission or after a previous leave.
        if !world.entity(entity).contains::<Retired>() {
            world.entity_mut(entity).insert(Retired(now));
        }
        let Some((slot, role)) = owner else { return };
        world.entity_mut(entity).remove::<Compatible>();
        self.free_seat(slot, role, Some(entity));
    }

    /// Free a seat and forget everything still queued in its name, so late
    /// work for it cannot reach a replacement or a connection that is gone.
    fn free_seat(&mut self, slot: usize, role: Role, entity: Option<Entity>) {
        self.forget_seat(slot, role, entity);
        if role == Role::Player {
            self.departed.push(slot);
        } else {
            self.departed_observers.push(slot);
        }
    }
    fn forget_seat(&mut self, slot: usize, role: Role, entity: Option<Entity>) {
        let world = self.app.world_mut();
        if role == Role::Player {
            world.resource_mut::<AssignedSlots>().slots.remove(&slot);
        }
        world
            .resource_mut::<HostInputs>()
            .0
            .retain(|m| m.slot != slot);
        world
            .resource_mut::<HostPickups>()
            .0
            .retain(|m| m.slot != slot);
        world
            .resource_mut::<HostProfiles>()
            .0
            .retain(|m| m.slot != slot);
        world
            .resource_mut::<HostRespawns>()
            .0
            .retain(|m| m.slot != slot);
        world.resource_mut::<HostDm>().0.retain(|m| m.seat != slot);
        world
            .resource_mut::<HostModRequests>()
            .0
            .retain(|(seat, _)| *seat != slot);
        if let Some(entity) = entity {
            world
                .resource_mut::<HostRoles>()
                .0
                .retain(|(e, _)| *e != entity);
        }
    }

    /// Seats whose link was despawned without going through [`Self::retire`].
    fn release_lost_seats(&mut self) {
        let lost = std::mem::take(&mut self.app.world_mut().resource_mut::<LostSeats>().0);
        for (slot, role) in lost {
            self.free_seat(slot, role, None);
        }
        // Role requests of despawned links.
        let world = self.app.world_mut();
        let requests = std::mem::take(&mut world.resource_mut::<HostRoles>().0);
        let kept = requests
            .into_iter()
            .filter(|(entity, _)| world.get_entity(*entity).is_ok())
            .collect();
        world.resource_mut::<HostRoles>().0 = kept;
    }

    /// Remove connections that kept exceeding their budgets or sent what no
    /// client sends, and refuse their address for a while.
    fn enforce_strikes(&mut self, now: Instant) {
        let world = self.app.world_mut();
        let mut q = world.query::<(Entity, &mut LinkGuard, Option<&mut Compatible>)>();
        let mut abusers = Vec::new();
        for (entity, mut guard, owner) in q.iter_mut(world) {
            if !guard.enforce {
                continue;
            }
            let mut owner = owner;
            let strikes = guard.strikes + owner.as_ref().map_or(0, |o| o.strikes);
            if strikes >= STRIKE_LIMIT {
                abusers.push((entity, guard.ip));
            } else if now.saturating_duration_since(guard.window) >= STRIKE_WINDOW {
                guard.window = now;
                guard.strikes = 0;
                guard.dropped = 0;
                if let Some(owner) = owner.as_mut() {
                    owner.strikes = 0;
                }
            }
        }
        for (entity, ip) in abusers {
            let world = self.app.world_mut();
            if let Some(mut sender) = world.get_mut::<MessageSender<Refused>>(entity) {
                sender.send::<ControlChannel>(Refused {
                    reason: "removed for flooding the host".into(),
                    code: RefusalCode::Banned,
                });
            }
            world
                .resource_mut::<HostPolicy>()
                .temporary
                .add(ip, now + ABUSE_BAN);
            world.resource_mut::<HostTraffic>().0.rejected("abuse");
            self.retire(entity);
            // A guard that stays would be counted again until it is removed.
            if let Some(mut guard) = self.app.world_mut().get_mut::<LinkGuard>(entity) {
                guard.enforce = false;
            }
        }
    }

    /// Reclaim links nobody owns: handshakes that never finish, sessions that
    /// never say hello, and removed connections past their grace period.
    fn sweep_links(&mut self, now: Instant) {
        let world = self.app.world_mut();
        let mut q = world.query::<(
            Entity,
            &LinkGuard,
            Has<client::Connected>,
            Has<Compatible>,
            Option<&Retired>,
            Option<&Refusal>,
        )>();
        let links: Vec<_> = q
            .iter(world)
            .map(|(entity, guard, connected, seated, retired, refused)| {
                (
                    entity,
                    guard.first_seen,
                    guard.ip,
                    connected,
                    seated,
                    retired.map(|r| r.0).or(refused.map(|r| r.0)),
                )
            })
            .collect();
        let mut remove = Vec::new();
        let mut unseated = Vec::new();
        let mut pending = Vec::new();
        for (entity, first_seen, ip, connected, seated, gone_at) in links {
            let age = now.saturating_duration_since(first_seen);
            if let Some(at) = gone_at {
                if now.saturating_duration_since(at) >= RETIRE_GRACE {
                    remove.push(entity);
                }
            } else if !connected {
                let banned = self
                    .app
                    .world_mut()
                    .resource_mut::<HostPolicy>()
                    .banned(ip, now);
                if banned || age >= HANDSHAKE_DEADLINE {
                    remove.push(entity);
                } else {
                    pending.push((first_seen, entity));
                }
            } else if !seated && age >= HELLO_DEADLINE {
                unseated.push(entity);
            }
        }
        if pending.len() > MAX_PENDING_LINKS {
            pending.sort_unstable();
            let excess = pending.len() - MAX_PENDING_LINKS;
            remove.extend(pending.into_iter().take(excess).map(|(_, entity)| entity));
        }
        for entity in unseated {
            self.retire(entity);
        }
        for entity in remove {
            if let Ok(link) = self.app.world_mut().get_entity_mut(entity) {
                link.despawn();
            }
        }
    }

    /// Re-seat connections that asked for another role.
    fn apply_role_requests(&mut self) {
        let requests = std::mem::take(&mut self.app.world_mut().resource_mut::<HostRoles>().0);
        for (entity, request) in requests {
            self.reseat(entity, request.role, Some(&request.name));
        }
    }

    /// Move one connection to `role` (or keep it if refused). Returns true
    /// when the seat changed.
    fn reseat(&mut self, entity: Entity, role: Role, name: Option<&str>) -> bool {
        let world = self.app.world_mut();
        let Some(owner) = world.get::<Compatible>(entity) else {
            return false;
        };
        let (old_seat, old_role, old_queued) = (owner.slot, owner.role, owner.queued);
        if old_role == role && !old_queued {
            return false;
        }
        if role_needs_credential(role, owner.dm_verified) {
            if let Some(mut owner) = world.get_mut::<Compatible>(entity) {
                owner.strikes += crate::security::MALFORMED_STRIKES;
            }
            return false;
        }
        let mut slots = world.resource_mut::<AssignedSlots>();
        // A player leaving their slot frees it before admission.
        if old_role == Role::Player {
            slots.slots.remove(&old_seat);
        }
        let seated = slots.seat(role);
        let (seat, new_role, queued) = match seated {
            Ok(s) => s,
            Err(_) if old_role == Role::Player => {
                slots.slots.insert(old_seat);
                return false;
            }
            Err(_) => return false,
        };
        // A queued spectator asking again stays on its existing seat.
        if new_role == old_role && queued == old_queued && new_role != Role::Player {
            return false;
        }
        let seat = if new_role != Role::Player && old_role != Role::Player {
            old_seat
        } else {
            seat
        };
        if old_role == Role::Player {
            self.departed.push(old_seat);
        } else if new_role == Role::Player {
            self.departed_observers.push(old_seat);
        }
        let world = self.app.world_mut();
        let mut owner = world.get_mut::<Compatible>(entity).unwrap();
        owner.slot = seat;
        owner.role = new_role;
        owner.queued = queued;
        if let Some(name) = name {
            owner.name = clean_name(name, new_role);
        }
        if let Some(mut sender) = world.get_mut::<MessageSender<Welcome>>(entity) {
            sender.send::<ControlChannel>(Welcome {
                seat,
                role: new_role,
                queued,
            });
        }
        true
    }

    /// Seat queued spectators in free player slots (oldest seats first).
    /// Returns the new player slots.
    pub fn promote_queued(&mut self) -> Vec<usize> {
        let world = self.app.world_mut();
        let mut q = world.query::<(Entity, &Compatible)>();
        let mut queued: Vec<_> = q
            .iter(world)
            .filter(|(_, o)| o.queued)
            .map(|(e, o)| (o.slot, e))
            .collect();
        queued.sort_unstable();
        let mut promoted = Vec::new();
        for (_, entity) in queued {
            let in_progress = {
                let mut slots = self.app.world_mut().resource_mut::<AssignedSlots>();
                std::mem::replace(&mut slots.in_progress, false)
            };
            let changed = self.reseat(entity, Role::Player, None);
            self.app
                .world_mut()
                .resource_mut::<AssignedSlots>()
                .in_progress = in_progress;
            if !changed {
                break;
            }
            if let Some(owner) = self.app.world().get::<Compatible>(entity) {
                promoted.push(owner.slot);
            }
        }
        promoted
    }

    /// A new round or level: queued players join regardless of the hot-join
    /// lock (the original game's own join point between games).
    pub fn begin_round(&mut self) -> Vec<usize> {
        self.promote_queued()
    }

    pub fn lobby(&self) -> LobbySettings {
        self.app.world().resource::<AssignedSlots>().lobby
    }
    pub fn set_lobby(&mut self, lobby: LobbySettings) -> Result<(), String> {
        if !lobby.valid() {
            return Err("invalid lobby settings".into());
        }
        self.app.world_mut().resource_mut::<AssignedSlots>().lobby = lobby;
        Ok(())
    }
    /// Whether the round/level has begun (hot join applies).
    pub fn set_in_progress(&mut self, in_progress: bool) {
        self.app
            .world_mut()
            .resource_mut::<AssignedSlots>()
            .in_progress = in_progress;
    }
    /// Player slots used by local players on this host (beyond slot 0).
    pub fn set_local_slots(&mut self, local: impl IntoIterator<Item = usize>) {
        self.app.world_mut().resource_mut::<AssignedSlots>().local = local.into_iter().collect();
    }
    /// The lowest player slot free for a local player (none when full).
    pub fn free_local_slot(&self) -> Option<usize> {
        let slots = self.app.world().resource::<AssignedSlots>();
        (slots.players_in_use() < usize::from(slots.lobby.max_players))
            .then(|| slots.free_slot())
            .flatten()
    }
    pub fn take_departed_observers(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.departed_observers)
    }
    /// Every seated connection, ordered by seat.
    pub fn seats(&mut self) -> Vec<SeatInfo> {
        let world = self.app.world_mut();
        let mut q = world.query::<&Compatible>();
        let mut seats: Vec<_> = q
            .iter(world)
            .map(|o| SeatInfo {
                seat: o.slot,
                role: o.role,
                name: o.name.clone(),
                queued: o.queued,
            })
            .collect();
        seats.sort_unstable_by_key(|s| s.seat);
        seats
    }
    /// Players' mod requests since the last call, as (slot, request).
    pub fn take_mod_requests(&mut self) -> Vec<(usize, ModRequest)> {
        std::mem::take(&mut self.app.world_mut().resource_mut::<HostModRequests>().0)
    }
    pub fn send_mod_reply(&mut self, slot: usize, reply: ModReply) -> Result<(), String> {
        let world = self.app.world_mut();
        let mut q = world.query::<(&Compatible, &mut MessageSender<ModReply>)>();
        for (owner, mut sender) in q.iter_mut(world) {
            if owner.slot == slot && owner.role == Role::Player {
                sender.send::<ControlChannel>(reply);
                return Ok(());
            }
        }
        Err("mod request owner unavailable".into())
    }
    /// The profile a player seat submitted when it joined (protocol 11).
    pub fn admitted_profile(&mut self, slot: usize) -> Option<ProfileSubmit> {
        let world = self.app.world_mut();
        let mut q = world.query::<&Compatible>();
        q.iter(world)
            .find(|o| o.slot == slot && o.role == Role::Player)
            .and_then(|o| o.profile.clone())
    }
    /// The submitted name of a seat.
    pub fn seat_name(&mut self, seat: usize) -> Option<String> {
        let world = self.app.world_mut();
        let mut q = world.query::<&Compatible>();
        q.iter(world)
            .find(|o| o.slot == seat)
            .map(|o| o.name.clone())
    }
    /// Refuse an already seated player (e.g. its robe is unavailable): the
    /// connection is retired and told why; its slot frees immediately.
    pub fn refuse_seat(&mut self, slot: usize, reason: &str) -> bool {
        let world = self.app.world_mut();
        let mut q = world.query::<(Entity, &Compatible)>();
        let Some((entity, role)) = q
            .iter(world)
            .find(|(_, o)| o.slot == slot)
            .map(|(e, o)| (e, o.role))
        else {
            return false;
        };
        if let Some(mut sender) = world.get_mut::<MessageSender<Refused>>(entity) {
            sender.send::<ControlChannel>(Refused {
                reason: reason.into(),
                code: RefusalCode::Denied,
            });
        }
        let now = world.resource::<HostClock>().0;
        world
            .entity_mut(entity)
            .remove::<Compatible>()
            .insert(Retired(now));
        // Nothing may be answered on this seat after this (a queued request
        // from the same tick would otherwise find no connection to reply to).
        self.forget_seat(slot, role, Some(entity));
        true
    }
    /// Remove a connection from a running game, telling it why, and refuse its
    /// address for the rest of the session. The seat is released like a
    /// departure. False when no connection holds the seat (a local player has
    /// none, and cannot be kicked over the network).
    pub fn kick(&mut self, seat: usize, reason: &str) -> bool {
        let world = self.app.world_mut();
        let mut q = world.query::<(Entity, &Compatible)>();
        let Some((entity, ip)) = q
            .iter(world)
            .find(|(_, o)| o.slot == seat)
            .map(|(e, o)| (e, o.ip))
        else {
            return false;
        };
        if let Some(mut sender) = world.get_mut::<MessageSender<Refused>>(entity) {
            sender.send::<ControlChannel>(Refused {
                reason: crate::security::display_text(reason, 256),
                code: RefusalCode::Kicked,
            });
        }
        let mut policy = world.resource_mut::<HostPolicy>();
        if !policy.security.banned.contains(&ip) {
            policy.security.banned.push(ip);
        }
        self.retire(entity);
        true
    }
    /// What clients learn about this host when admitted.
    pub fn set_server_info(&mut self, info: ServerInfo) {
        self.app.world_mut().resource_mut::<HostInfo>().0 = info;
    }
    pub fn server_info(&self) -> &ServerInfo {
        &self.app.world().resource::<HostInfo>().0
    }
    /// Degrade every connection made from now on (QA harness).
    pub fn set_conditions(&mut self, conditions: Option<NetConditions>) {
        self.app.world_mut().resource_mut::<HostConditions>().0 = conditions;
    }
    pub fn traffic(&self) -> &TrafficStats {
        &self.app.world().resource::<HostTraffic>().0
    }
    pub fn traffic_mut(&mut self) -> &mut TrafficStats {
        &mut self
            .app
            .world_mut()
            .resource_mut::<HostTraffic>()
            .into_inner()
            .0
    }
    pub fn take_dm_requests(&mut self) -> Vec<OwnedDm> {
        std::mem::take(&mut self.app.world_mut().resource_mut::<HostDm>().0)
    }

    pub fn take_departed(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.departed)
    }
    pub fn player_slots(&mut self) -> Vec<usize> {
        let world = self.app.world_mut();
        let mut q = world.query::<&Compatible>();
        let mut slots: Vec<_> = q
            .iter(world)
            .filter(|owner| owner.role == Role::Player)
            .map(|owner| owner.slot)
            .collect();
        slots.sort_unstable();
        slots
    }
    pub fn take_inputs(&mut self) -> Vec<OwnedInput> {
        std::mem::take(&mut self.app.world_mut().resource_mut::<HostInputs>().0)
    }
    pub fn take_pickups(&mut self) -> Vec<OwnedPickup> {
        std::mem::take(&mut self.app.world_mut().resource_mut::<HostPickups>().0)
    }
    /// Send only to the connection which owns this slot.
    pub fn send_pickup_receipt(
        &mut self,
        slot: usize,
        receipt: PickupReceipt,
    ) -> Result<(), String> {
        let world = self.app.world_mut();
        let mut q = world.query::<(&Compatible, &mut MessageSender<PickupReceipt>)>();
        for (owner, mut sender) in q.iter_mut(world) {
            if owner.slot == slot {
                sender.send::<ControlChannel>(receipt);
                return Ok(());
            }
        }
        Err("pickup receipt owner unavailable".into())
    }
    pub fn take_respawns(&mut self) -> Vec<OwnedRespawn> {
        std::mem::take(&mut self.app.world_mut().resource_mut::<HostRespawns>().0)
    }
    pub fn take_profiles(&mut self) -> Vec<OwnedProfile> {
        std::mem::take(&mut self.app.world_mut().resource_mut::<HostProfiles>().0)
    }
    pub fn send_profile_reply(&mut self, slot: usize, reply: ProfileReply) -> Result<(), String> {
        let world = self.app.world_mut();
        let mut q = world.query::<(&Compatible, &mut MessageSender<ProfileReply>)>();
        for (owner, mut sender) in q.iter_mut(world) {
            if owner.slot == slot {
                sender.send::<ControlChannel>(reply);
                return Ok(());
            }
        }
        Err("profile owner unavailable".into())
    }
    /// Send a frame to every seated connection: a delta against what each
    /// acknowledged, or a full frame when it has no usable baseline.
    pub fn broadcast_game(&mut self, frame: &GameFrame) -> Result<usize, String> {
        if !frame.valid() {
            return Err("invalid game frame".into());
        }
        let world = self.app.world_mut();
        let policy = {
            let p = world.resource::<HostEncoder>();
            (p.full_only, p.interest)
        };
        let mut q = world.query::<(
            &mut Compatible,
            &mut MessageSender<crate::delta::GameUpdate>,
        )>();
        let mut count = 0;
        let mut sent = Vec::new();
        for (mut owner, mut sender) in q.iter_mut(world) {
            let view = match (policy.1, owner.role) {
                (Some(radius), Role::Player) => crate::delta::interest(frame, owner.slot, radius),
                _ => frame.clone(),
            };
            owner.encoder.full_only = policy.0;
            owner.encoder.push(view);
            let acked = owner.acked;
            let Some(update) = owner.encoder.update_for(acked) else {
                continue;
            };
            let kind = match update.payload {
                crate::delta::Payload::Full(_) => "GameUpdate.full",
                crate::delta::Payload::Delta(_) => "GameUpdate.delta",
            };
            sent.push((kind, crate::telemetry::payload_size(&update)));
            sender.send::<PhysicsChannel>(update);
            count += 1;
        }
        let mut traffic = world.resource_mut::<HostTraffic>();
        for (kind, bytes) in sent {
            traffic.0.sent_bytes(kind, bytes);
        }
        Ok(count)
    }
    /// Send full frames only (A/B measurement and fallback; plan B1).
    pub fn set_deltas(&mut self, enabled: bool) {
        self.app.world_mut().resource_mut::<HostEncoder>().full_only = !enabled;
    }
    /// Presentation interest (plan B3): one-shot events and emitters farther
    /// than `radius` from a player's own wizard are not sent to that
    /// player. Gameplay state is never culled, and spectators, dungeon
    /// masters and invaders always receive everything. Off by default.
    pub fn set_interest_radius(&mut self, radius: Option<f32>) {
        self.app.world_mut().resource_mut::<HostEncoder>().interest =
            radius.filter(|r| r.is_finite() && *r > 0.0);
    }
    pub fn broadcast(&mut self, frame: &PhysicsFrame) -> Result<usize, String> {
        frame
            .validate()
            .map_err(|e| format!("invalid host frame: {e:?}"))?;
        let world = self.app.world_mut();
        let mut q=world.query_filtered::<&mut MessageSender<PhysicsFrame>,(With<client::Connected>,With<Compatible>)>();
        let mut count = 0;
        for mut sender in q.iter_mut(world) {
            sender.send::<PhysicsChannel>(frame.clone());
            count += 1;
        }
        Ok(count)
    }
}
impl LoopbackClient {
    pub fn connect(server: SocketAddr) -> Result<Self, String> {
        Self::connect_as(server, Role::Player, "")
    }
    /// Connect asking for `role`; the host may seat a player as a queued
    /// spectator (full lobby or locked hot join) or refuse.
    pub fn connect_as(server: SocketAddr, role: Role, name: &str) -> Result<Self, String> {
        Self::join(
            server,
            JoinRequest {
                role,
                name: name.into(),
                ..default()
            },
        )
    }
    /// Connect with a full admission request: a player's selected profile
    /// travels in the hello, so the host never creates a placeholder.
    pub fn join(server: SocketAddr, request: JoinRequest) -> Result<Self, String> {
        if server.ip().is_unspecified() {
            return Err("cannot connect to an unspecified address".into());
        }
        let mut app = base();
        app.add_plugins(client::ClientPlugins::default())
            .init_resource::<Inbox>()
            .init_resource::<ClientSlot>()
            .init_resource::<PickupReceipts>()
            .init_resource::<GameInbox>()
            .init_resource::<ProfileReplies>()
            .init_resource::<ClientTraffic>()
            .init_resource::<ClientDecoder>()
            .init_resource::<ModReplies>()
            .init_resource::<Echoes>()
            .init_resource::<Departure>()
            .add_systems(
                Update,
                (
                    receive_echoes,
                    receive_left,
                    receive_mod_replies,
                    receive_server_info,
                    receive_frames,
                    receive_welcome,
                    receive_refusal,
                    receive_receipts,
                    receive_games,
                    receive_profile_replies,
                ),
            )
            .add_systems(
                PreUpdate,
                (
                    filter_datagrams
                        .after(lightyear::link::LinkSystems::Receive)
                        .before(ConnectionSystems::Receive),
                    guard_client_link
                        .after(ConnectionSystems::Receive)
                        .before(TransportSystems::Receive),
                ),
            );
        protocol(&mut app);
        app.finish();
        app.cleanup();
        app.update();
        let entity = app
            .world_mut()
            .spawn((
                client::NetcodeClient::new(
                    Authentication::Manual {
                        server_addr: server,
                        client_id: u64::from_le_bytes(
                            lightyear::netcode::generate_key()[..8]
                                .try_into()
                                .expect("eight bytes"),
                        ),
                        private_key: request.key.bytes(),
                        protocol_id: NETCODE_PROTOCOL_ID,
                    },
                    client::NetcodeConfig {
                        client_timeout_secs: SESSION_TIMEOUT_SECS,
                        // Tokens are minted on the spot; an expiry would only
                        // refuse players whose clock differs from the host's.
                        token_expire_secs: -1,
                        ..Default::default()
                    },
                )
                .map_err(|e| format!("secure session: {e:?}"))?,
                UdpIo::default(),
                // The local socket must match the server's address family
                // (an IPv4 socket cannot reach "localhost" as ::1).
                LocalAddr(if server.is_ipv6() {
                    SocketAddr::from((std::net::Ipv6Addr::UNSPECIFIED, 0))
                } else {
                    SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))
                }),
            ))
            .id();
        if let Some(c) = request.conditions {
            app.world_mut()
                .entity_mut(entity)
                .insert(lightyear::link::Link::default().with_conditioner(c.conditioner()));
        }
        app.world_mut().trigger(client::Connect { entity });
        app.world_mut().flush();
        app.update();
        Ok(Self {
            app,
            role: request.role,
            name: request.name.clone(),
            request,
            hellos_sent: 0,
            link_failed: false,
            hello_attempts: 0,
            last_hello: None,
            heartbeat: 0,
            last_heartbeat: None,
            updates_since_beat: 0,
            pending_beats: Default::default(),
        })
    }
    /// Send raw bytes as if they were a transport packet (inside the session).
    #[doc(hidden)]
    pub fn inject_packet_for_test(&mut self, packet: Vec<u8>) {
        let world = self.app.world_mut();
        let mut q = world.query::<&mut lightyear::link::Link>();
        for mut link in q.iter_mut(world) {
            link.send.push(packet.clone().into());
        }
    }
    /// The admission state machine's current state.
    pub fn admission(&self) -> Admission11 {
        self.admission_at(Instant::now())
    }
    /// Explicit admission clock for deterministic retry/timeout verification.
    pub fn admission_at(&self, now: Instant) -> Admission11 {
        let departure = self.app.world().resource::<Departure>();
        if departure.acknowledged {
            return Admission11::Left;
        }
        if departure.requested.is_some() {
            return Admission11::Leaving;
        }
        let slot = self.app.world().resource::<ClientSlot>();
        if let Some((reason, code)) = &slot.1 {
            Admission11::Refused(reason.clone(), *code)
        } else if let Some(welcome) = slot.0 {
            Admission11::Seated(welcome)
        } else if self.link_failed
            || (self.hello_attempts >= HELLO_ATTEMPTS
                && self
                    .last_hello
                    .is_some_and(|t| now.saturating_duration_since(t) >= HELLO_RETRY))
        {
            Admission11::TimedOut
        } else {
            Admission11::Negotiating {
                attempts: self.hello_attempts,
            }
        }
    }
    /// Send a heartbeat on the next update regardless of pacing (before a
    /// known stall, or when a test drives the host on its own clock).
    pub fn heartbeat_now(&mut self) {
        self.last_heartbeat = None;
    }
    /// The host's description, once admitted.
    pub fn server_info(&self) -> Option<&ServerInfo> {
        self.app.world().resource::<ClientSlot>().2.as_ref()
    }
    pub fn refusal_code(&self) -> Option<RefusalCode> {
        self.app
            .world()
            .resource::<ClientSlot>()
            .1
            .as_ref()
            .map(|r| r.1)
    }
    pub fn traffic(&self) -> &TrafficStats {
        &self.app.world().resource::<ClientTraffic>().0
    }
    pub fn traffic_mut(&mut self) -> &mut TrafficStats {
        &mut self
            .app
            .world_mut()
            .resource_mut::<ClientTraffic>()
            .into_inner()
            .0
    }
    /// Record the age of the newest accepted game state (seconds) and the
    /// latest prediction error, for telemetry export.
    pub fn note_presentation(&mut self, snapshot_age: Option<f32>, prediction_error: Option<f32>) {
        let timing = &mut self.traffic_mut().timing;
        timing.snapshot_age_ms = snapshot_age.map(|s| s * 1000.0);
        if prediction_error.is_some() {
            timing.prediction_error = prediction_error;
        }
    }
    /// The owned player slot, while seated as a player.
    pub fn player_slot(&self) -> Option<usize> {
        self.welcome()
            .filter(|w| w.role == Role::Player)
            .map(|w| w.seat)
    }
    /// The current seat (any role), once the host has admitted this client.
    pub fn welcome(&self) -> Option<Welcome> {
        if self.is_leaving() {
            return None;
        }
        self.app.world().resource::<ClientSlot>().0
    }
    /// Why the host refused admission.
    pub fn refusal(&self) -> Option<&str> {
        self.app
            .world()
            .resource::<ClientSlot>()
            .1
            .as_ref()
            .map(|r| r.0.as_str())
    }
    /// Begin a reliable departure. Keep updating until `leave_finished` before
    /// dropping this client, so loss can be retried and the host can acknowledge.
    pub fn leave(&mut self) -> Result<(), String> {
        if self.is_leaving() {
            return Ok(());
        }
        let world = self.app.world_mut();
        let mut q = world.query::<&mut MessageSender<Leave>>();
        q.single_mut(world)
            .map_err(|e| e.to_string())?
            .send::<ControlChannel>(Leave);
        world.resource_mut::<Departure>().requested = Some(Instant::now());
        Ok(())
    }
    pub fn is_leaving(&self) -> bool {
        self.app.world().resource::<Departure>().requested.is_some()
    }
    /// Acknowledged by the host, or the lease fallback has elapsed while silent.
    pub fn leave_finished(&self) -> bool {
        let departure = self.app.world().resource::<Departure>();
        departure.acknowledged
            || departure
                .requested
                .is_some_and(|at| at.elapsed() >= PLAYER_LEASE)
    }

    /// Ask to change role (join in from spectating, step out, ...).
    pub fn request_role(&mut self, request: RoleRequest) -> Result<(), String> {
        if self.welcome().is_none() {
            return Err("seat handshake pending".into());
        }
        self.role = request.role;
        self.name = request.name.clone();
        self.request.role = request.role;
        self.request.name = request.name.clone();
        let world = self.app.world_mut();
        let mut q = world.query::<&mut MessageSender<RoleRequest>>();
        q.single_mut(world)
            .map_err(|e| e.to_string())?
            .send::<ControlChannel>(request);
        Ok(())
    }
    pub fn send_dm(&mut self, request: DmRequest) -> Result<(), String> {
        if self.welcome().is_none() {
            return Err("seat handshake pending".into());
        }
        if !request.valid() {
            return Err("invalid dungeon master command".into());
        }
        let world = self.app.world_mut();
        let mut q = world.query::<&mut MessageSender<DmRequest>>();
        q.single_mut(world)
            .map_err(|e| e.to_string())?
            .send::<ControlChannel>(request);
        Ok(())
    }
    /// Players steer their wizard; invaders and dungeon masters steer the
    /// NPC they pilot (the host ignores input without a possessed body).
    pub fn send_input(&mut self, command: InputCommand) -> Result<(), String> {
        if self.welcome().is_none_or(|w| w.role == Role::Spectator) {
            return Err("player ownership handshake pending".into());
        }
        if !command.valid() {
            return Err("invalid input command".into());
        }
        let bytes = crate::telemetry::payload_size(&command);
        let world = self.app.world_mut();
        let mut q = world.query::<&mut MessageSender<InputCommand>>();
        let mut sender = q.single_mut(world).map_err(|e| e.to_string())?;
        sender.send::<InputChannel>(command);
        self.traffic_mut().sent_bytes("InputCommand", bytes);
        Ok(())
    }
    pub fn send_pickup(&mut self, request: PickupRequest) -> Result<(), String> {
        if self.player_slot().is_none() {
            return Err("player ownership handshake pending".into());
        }
        if !request.valid() {
            return Err("invalid pickup request".into());
        }
        let world = self.app.world_mut();
        let mut q = world.query::<&mut MessageSender<PickupRequest>>();
        q.single_mut(world)
            .map_err(|e| e.to_string())?
            .send::<ControlChannel>(request);
        Ok(())
    }
    pub fn take_pickup_receipts(&mut self) -> Vec<PickupReceipt> {
        std::mem::take(&mut self.app.world_mut().resource_mut::<PickupReceipts>().0)
    }
    pub fn send_respawn(&mut self, request: RespawnRequest) -> Result<(), String> {
        if self.player_slot().is_none() {
            return Err("ownership handshake pending".into());
        }
        let world = self.app.world_mut();
        let mut q = world.query::<&mut MessageSender<RespawnRequest>>();
        q.single_mut(world)
            .map_err(|e| e.to_string())?
            .send::<ControlChannel>(request);
        Ok(())
    }
    pub fn send_profile(&mut self, request: ProfileRequest) -> Result<(), String> {
        if self.player_slot().is_none() {
            return Err("player ownership handshake pending".into());
        }
        let world = self.app.world_mut();
        let mut q = world.query::<&mut MessageSender<ProfileRequest>>();
        q.single_mut(world)
            .map_err(|e| e.to_string())?
            .send::<ControlChannel>(request);
        Ok(())
    }
    /// Ask a server mod for something (reliable; deduplicated by ID).
    pub fn send_mod_request(&mut self, request: ModRequest) -> Result<(), String> {
        if self.player_slot().is_none() {
            return Err("only players send mod requests".into());
        }
        if !request.valid() {
            return Err("invalid mod request".into());
        }
        let world = self.app.world_mut();
        let mut q = world.query::<&mut MessageSender<ModRequest>>();
        q.single_mut(world)
            .map_err(|e| e.to_string())?
            .send::<ControlChannel>(request);
        Ok(())
    }
    pub fn take_mod_replies(&mut self) -> Vec<ModReply> {
        std::mem::take(&mut self.app.world_mut().resource_mut::<ModReplies>().0)
    }
    pub fn take_game_frames(&mut self) -> Vec<GameFrame> {
        std::mem::take(&mut self.app.world_mut().resource_mut::<GameInbox>().0)
    }
    pub fn take_profile_replies(&mut self) -> Vec<ProfileReply> {
        std::mem::take(&mut self.app.world_mut().resource_mut::<ProfileReplies>().0)
    }
    pub fn update(&mut self) -> Vec<PhysicsFrame> {
        self.update_at(Instant::now())
    }
    /// Explicit clock for hello retries and heartbeat pacing; link I/O still
    /// advances through the normal Bevy transport update.
    pub fn update_at(&mut self, now: Instant) -> Vec<PhysicsFrame> {
        let admitted = {
            let slot = self.app.world().resource::<ClientSlot>();
            slot.0.is_some() || slot.1.is_some()
        };
        let connected = {
            let world = self.app.world_mut();
            let mut session = world.query_filtered::<(), With<client::Connected>>();
            session.iter(world).next().is_some()
        };
        let retry_due = self
            .last_hello
            .is_none_or(|t| now.saturating_duration_since(t) >= HELLO_RETRY);
        if !connected {
            // No session yet, so nothing can be sent; the wait still counts
            // towards giving up (and the burst is saved for when it is up).
            if !self.is_leaving() && !admitted && self.hello_attempts < HELLO_ATTEMPTS && retry_due
            {
                self.hello_attempts += 1;
                self.last_hello = Some(now);
            }
            let world = self.app.world_mut();
            let mut failed = world.query_filtered::<(), With<Disconnected>>();
            self.link_failed |= failed.iter(world).next().is_some() && !admitted;
        } else if !self.is_leaving()
            && !admitted
            && (self.hellos_sent < HELLO_BURST
                || (self.hello_attempts < HELLO_ATTEMPTS && retry_due))
        {
            // Bounded retries cover a lost hello: a short burst when the
            // session comes up, then a slow retry.
            let hello = Hello {
                version: PROTOCOL_VERSION,
                role: self.role,
                name: self.name.clone(),
                profile: self.request.profile.clone(),
                rules_hash: self.request.rules_hash.clone(),
                credential: self.request.credential.clone(),
            };
            let bytes = crate::telemetry::payload_size(&hello);
            let world = self.app.world_mut();
            let mut senders = world.query::<&mut MessageSender<Hello>>();
            for mut sender in senders.iter_mut(world) {
                sender.send::<PhysicsChannel>(hello.clone());
            }
            self.traffic_mut().sent_bytes("Hello", bytes);
            self.hellos_sent += 1;
            self.hello_attempts += 1;
            self.last_hello = Some(now);
        }
        self.updates_since_beat += 1;
        if self.welcome().is_some()
            && (self.updates_since_beat >= HEARTBEAT_UPDATES
                || self
                    .last_heartbeat
                    .is_none_or(|t| now.saturating_duration_since(t) >= HEARTBEAT_PERIOD))
        {
            self.updates_since_beat = 0;
            self.heartbeat = self.heartbeat.wrapping_add(1);
            let beat = Heartbeat {
                sequence: self.heartbeat,
            };
            let world = self.app.world_mut();
            let mut senders = world.query::<&mut MessageSender<Heartbeat>>();
            for mut sender in senders.iter_mut(world) {
                sender.send::<PhysicsChannel>(beat);
            }
            self.traffic_mut().sent("Heartbeat", &beat);
            self.pending_beats.push_back((beat.sequence, now));
            self.pending_beats.truncate(64);
            self.last_heartbeat = Some(now);
        }
        self.app.update();
        let echoes = std::mem::take(&mut self.app.world_mut().resource_mut::<Echoes>().0);
        for (sequence, arrived) in echoes {
            if let Some(at) = self.pending_beats.iter().position(|(s, _)| *s == sequence) {
                let sent = self.pending_beats[at].1;
                self.pending_beats.drain(..=at);
                let rtt = arrived.saturating_duration_since(sent).as_secs_f32() * 1000.0;
                self.traffic_mut().round_trip(rtt);
            }
        }
        std::mem::take(&mut self.app.world_mut().resource_mut::<Inbox>().0)
    }
}
