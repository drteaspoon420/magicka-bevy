//! Multiplayer security primitives (rewrite extension; threat model and
//! residual risks in `research/multiplayer-security.md`).
//!
//! Sessions run over netcode.io: a connect token sealed with a [`NetworkKey`]
//! carries per-session ChaCha20-Poly1305 keys, and the host's challenge proves
//! the client owns its source address before anything is seated or sent.
//!
//! - An **open** game uses a key built into the program, so anyone can join.
//!   Forged source addresses, amplification and off-path injection still fail;
//!   someone who can read the traffic can decrypt the token and the session.
//! - A **private** game derives the key from a passphrase. Without it nothing
//!   gets past the handshake, and traffic is confidential and authenticated.
//!
//! Everything else here assumes the peer has a valid session and may still be
//! hostile: every game open to the internet has such peers.
use crate::lobby::Role;
use lightyear::connection::shared::{ConnectionRequestHandler, DeniedReason};
use lightyear::prelude::PeerId;
use std::{
    collections::BTreeMap,
    net::IpAddr,
    time::{Duration, Instant},
};

/// Identifies this game's netcode traffic. Stays fixed across protocol
/// versions so a mismatched client reaches the hello and hears why it was
/// refused, instead of being silently ignored.
pub const NETCODE_PROTOCOL_ID: u64 = u64::from_le_bytes(*b"MAGICKA\x01");
const OPEN_GAME_KEY: [u8; 32] = *b"magicka-rewrite-open-game-key-01";
/// Fixed: the client must derive the key before it has heard from the host.
/// Argon2's work factor, not the salt, is what slows down a guesser.
const KDF_SALT: &[u8] = b"magicka-rewrite-passphrase-v1";
/// A short passphrase can be guessed offline from one captured connect token.
pub const MIN_PASSPHRASE_CHARS: usize = 8;

/// The key sealing connect tokens. Printing it never shows the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct NetworkKey {
    bytes: [u8; 32],
    private: bool,
}
impl Default for NetworkKey {
    fn default() -> Self {
        Self {
            bytes: OPEN_GAME_KEY,
            private: false,
        }
    }
}
impl std::fmt::Debug for NetworkKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.private {
            "NetworkKey(private)"
        } else {
            "NetworkKey(open)"
        })
    }
}
impl NetworkKey {
    /// Derive a private game's key (Argon2id; deliberately slow, so keep the
    /// result instead of deriving again for each connection).
    pub fn from_passphrase(passphrase: &str) -> Result<Self, String> {
        let passphrase = passphrase.trim();
        if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
            return Err(format!(
                "the passphrase needs at least {MIN_PASSPHRASE_CHARS} characters"
            ));
        }
        let params = argon2::Params::new(19 * 1024, 2, 1, Some(32))
            .map_err(|e| format!("key derivation: {e}"))?;
        let mut bytes = [0; 32];
        argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
            .hash_password_into(passphrase.as_bytes(), KDF_SALT, &mut bytes)
            .map_err(|e| format!("key derivation: {e}"))?;
        Ok(Self {
            bytes,
            private: true,
        })
    }
    pub fn is_private(&self) -> bool {
        self.private
    }
    pub(crate) fn bytes(&self) -> [u8; 32] {
        self.bytes
    }
}

/// Read a secret given as a file path, else from an environment variable.
/// Never a command-line value: other local users can read process arguments.
pub fn secret_from(file: Option<&str>, variable: &str) -> Result<Option<String>, String> {
    if let Some(path) = file {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        return Ok(Some(text.trim().to_string()).filter(|s| !s.is_empty()));
    }
    Ok(std::env::var(variable)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()))
}

/// Fragments a host accepts for one client message. The largest client
/// message is a mod request (16 KiB of JSON, about 15 fragments).
pub const HOST_FRAGMENT_LIMIT: u64 = 32;
/// Fragments a client accepts for one host message (about 5 MB). Larger than
/// any frame a host can send in time over UDP at its 20 Hz publish rate.
pub const CLIENT_FRAGMENT_LIMIT: u64 = 4 * 1024;
/// Fragmented client messages incomplete at once on one connection. lightyear
/// allocates each in full on its first fragment and, on a reliable channel,
/// keeps it until it completes, so this bounds that memory per connection.
pub const MAX_INCOMPLETE_FRAGMENTED: usize = 8;
/// Finished messages remembered so a retransmitted fragment is not a new one.
const RECENTLY_COMPLETED: usize = 64;

fn read_varint(bytes: &[u8]) -> Option<(u64, &[u8])> {
    let first = *bytes.first()?;
    let len = 1usize << (first >> 6);
    let (head, rest) = bytes.split_at_checked(len)?;
    let value = head[1..]
        .iter()
        .fold(u64::from(first & 0x3f), |v, b| (v << 8) | u64::from(*b));
    Some((value, rest))
}

/// Whether a raw datagram may reach netcode's parser, which runs before anything
/// is authenticated. That parser slices an 8-byte array by a length taken from
/// the first byte (up to 15) and adds to the sequence number unchecked, so one
/// forged datagram panics a client, or a host that knows its source address
/// (`research/multiplayer-security.md`, MS-7).
pub fn datagram_allowed(datagram: &[u8]) -> bool {
    const LAST_PACKET_KIND: u8 = 6;
    // Far above any real sequence: hosts start at 2^62, clients at zero.
    const LARGEST_SEQUENCE: u64 = u64::MAX - (1 << 32);
    let Some(prefix) = datagram.first() else {
        return false;
    };
    // A connection request is exactly zero.
    if *prefix == 0 {
        return true;
    }
    let (sequence_len, kind) = ((prefix >> 4) as usize, prefix & 0xf);
    if kind == 0 || kind > LAST_PACKET_KIND || !(1..=8).contains(&sequence_len) {
        return false;
    }
    let Some(sequence) = datagram.get(1..1 + sequence_len) else {
        return false;
    };
    let mut bytes = [0; 8];
    bytes[..sequence_len].copy_from_slice(sequence);
    u64::from_le_bytes(bytes) <= LARGEST_SEQUENCE
}

/// One fragment's place in its message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FragmentPlace {
    channel: u64,
    message: u16,
    index: u64,
    count: u64,
}
/// What a transport packet is, if it is one this build may process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PacketClass {
    Plain,
    Fragment(FragmentPlace),
}
pub(crate) fn classify(packet: &[u8], fragment_limit: u64) -> Option<PacketClass> {
    const HEADER: usize = 13;
    const DATA: u8 = 0;
    const DATA_FRAGMENT: u8 = 1;
    const ACK_ONLY: u8 = 4;
    let (kind, body) = (packet.first()?, packet.get(HEADER..)?);
    match *kind {
        DATA | ACK_ONLY => Some(PacketClass::Plain),
        DATA_FRAGMENT => {
            let (channel, rest) = read_varint(body)?;
            let (message, rest) = rest.split_at_checked(2)?;
            let message = u16::from_be_bytes([message[0], message[1]]);
            let (index, rest) = read_varint(rest)?;
            let (count, _) = read_varint(rest)?;
            (count != 0 && count <= fragment_limit && index < count).then_some(
                PacketClass::Fragment(FragmentPlace {
                    channel,
                    message,
                    index,
                    count,
                }),
            )
        }
        // Compressed packets: this build has no compression, and everything
        // else is not a packet type at all.
        _ => None,
    }
}
/// Whether a decrypted transport packet may reach the channel receivers.
///
/// lightyear allocates `num_fragments * fragment_size` as soon as it sees the
/// first fragment of a message, with the count taken from the packet. One
/// packet declaring 2^40 fragments aborts the process, so it must be rejected
/// before the transport parses it (`research/multiplayer-security.md`, MS-1).
pub fn packet_allowed(packet: &[u8], fragment_limit: u64) -> bool {
    classify(packet, fragment_limit).is_some()
}
/// Follows a connection's fragmented messages (see [`MAX_INCOMPLETE_FRAGMENTED`]).
#[derive(Debug, Default)]
pub(crate) struct FragmentTracker {
    /// (channel, message) to the fragment indices seen so far, as a bit set:
    /// indices stay below [`HOST_FRAGMENT_LIMIT`], which fits 64 bits.
    incomplete: BTreeMap<(u64, u16), u64>,
    completed: std::collections::VecDeque<(u64, u16)>,
}
impl FragmentTracker {
    /// Whether this fragment may be processed. False for one that would start
    /// a message beyond the limit.
    pub(crate) fn admit(&mut self, place: FragmentPlace) -> bool {
        let key = (place.channel, place.message);
        if self.completed.contains(&key) {
            return true;
        }
        if !self.incomplete.contains_key(&key) && self.incomplete.len() >= MAX_INCOMPLETE_FRAGMENTED
        {
            return false;
        }
        let seen = self.incomplete.entry(key).or_default();
        *seen |= 1u64 << place.index.min(63);
        if u64::from(seen.count_ones()) >= place.count {
            self.incomplete.remove(&key);
            if self.completed.len() >= RECENTLY_COMPLETED {
                self.completed.pop_front();
            }
            self.completed.push_back(key);
        }
        true
    }
}
const _: () = assert!(HOST_FRAGMENT_LIMIT <= 64);

/// A token bucket: `per_second` sustained, `burst` at once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    pub per_second: f32,
    pub burst: f32,
}
const fn rate(per_second: f32, burst: f32) -> Rate {
    Rate { per_second, burst }
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Bucket {
    spent: f32,
    last: Option<Instant>,
}
impl Bucket {
    pub(crate) fn take(&mut self, rate: Rate, amount: f32, now: Instant) -> bool {
        if let Some(last) = self.last {
            let refill = now.saturating_duration_since(last).as_secs_f32() * rate.per_second;
            self.spent = (self.spent - refill).max(0.0);
        }
        self.last = Some(now);
        if self.spent + amount > rate.burst {
            return false;
        }
        self.spent += amount;
        true
    }
}

/// What a connection may send. Generous for play: a client's frame rate sets
/// its input rate, and these only have to stop floods.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Budgets {
    pub packets: Rate,
    pub bytes: Rate,
    pub inputs: Rate,
    pub heartbeats: Rate,
    pub acks: Rate,
    pub pickups: Rate,
    /// Profile changes can load a robe from disk.
    pub profiles: Rate,
    pub respawns: Rate,
    pub roles: Rate,
    pub dm: Rate,
    /// Each mod request runs server Lua.
    pub mods: Rate,
}
impl Default for Budgets {
    fn default() -> Self {
        Self {
            packets: rate(5_000.0, 10_000.0),
            bytes: rate(4_000_000.0, 8_000_000.0),
            inputs: rate(2_000.0, 4_000.0),
            heartbeats: rate(120.0, 240.0),
            acks: rate(2_000.0, 4_000.0),
            pickups: rate(30.0, 60.0),
            profiles: rate(2.0, 8.0),
            respawns: rate(5.0, 10.0),
            roles: rate(2.0, 6.0),
            dm: rate(120.0, 240.0),
            mods: rate(60.0, 120.0),
        }
    }
}

/// A connection reaching this many strikes inside [`STRIKE_WINDOW`] is removed
/// and its address refused for [`ABUSE_BAN`].
pub const STRIKE_LIMIT: u32 = 64;
pub const STRIKE_WINDOW: Duration = Duration::from_secs(10);
pub const ABUSE_BAN: Duration = Duration::from_secs(60);
/// Strikes for one thing a real client never sends.
pub const MALFORMED_STRIKES: u32 = 8;
/// Dropped packets per strike.
pub const PACKETS_PER_STRIKE: u32 = 64;
/// A link that has not finished the handshake is dropped after this long.
pub const HANDSHAKE_DEADLINE: Duration = Duration::from_secs(10);
/// Longer than a client keeps retrying its hello.
pub const HELLO_DEADLINE: Duration = Duration::from_secs(25);
/// Links still in the handshake at once; the oldest go first beyond this.
pub const MAX_PENDING_LINKS: usize = 1024;
/// How long a removed connection stays around so its refusal can arrive.
pub const RETIRE_GRACE: Duration = Duration::from_secs(3);
/// Connections accepted from one address by default. Above one because a
/// household or LAN party can share one, and local co-op opens one per seat.
pub const DEFAULT_CONNECTIONS_PER_ADDRESS: usize = 16;
/// New handshakes admitted per second; the burst covers a whole lobby joining.
pub const HANDSHAKE_RATE: Rate = rate(16.0, 1024.0);
/// Connection requests one address may send. A client resends about ten a
/// second only until answered, which takes a moment, so the burst holds several
/// clients at once. The sustained rate must stay below [`HANDSHAKE_RATE`]'s:
/// otherwise one address drains the shared limit and locks out everyone else.
pub const REQUESTS_PER_ADDRESS: Rate = rate(8.0, 64.0);
const _: () = assert!(REQUESTS_PER_ADDRESS.per_second < HANDSHAKE_RATE.per_second);
/// Addresses followed at once; the longest idle go first beyond this.
const TRACKED_REQUEST_ADDRESSES: usize = 4096;

/// A host's security configuration.
#[derive(Clone, Debug)]
pub struct HostSecurity {
    pub key: NetworkKey,
    /// When set, the dungeon master role also needs this credential.
    pub dm_credential: Option<String>,
    pub connections_per_address: usize,
    /// Addresses never admitted.
    pub banned: Vec<IpAddr>,
    pub budgets: Budgets,
    /// Apply budgets, per-address limits and bans to this computer's own
    /// connections too. Off by default: loopback peers are local processes
    /// (QA harnesses, several clients on one machine). Tests turn it on.
    pub enforce_on_loopback: bool,
}
impl Default for HostSecurity {
    fn default() -> Self {
        Self {
            key: NetworkKey::default(),
            dm_credential: None,
            connections_per_address: DEFAULT_CONNECTIONS_PER_ADDRESS,
            banned: Vec::new(),
            budgets: Budgets::default(),
            enforce_on_loopback: false,
        }
    }
}
impl HostSecurity {
    pub fn private(passphrase: &str) -> Result<Self, String> {
        Ok(Self {
            key: NetworkKey::from_passphrase(passphrase)?,
            ..Self::default()
        })
    }
    /// Warnings for the host's log about what this configuration exposes.
    pub fn warnings(&self, public_bind: bool, dm_enabled: bool) -> Vec<&'static str> {
        let mut out = Vec::new();
        if public_bind && !self.key.is_private() {
            out.push(
                "open game: anyone who can reach this port can join, and traffic can be read \
                 on the network path. Set a passphrase for a private game.",
            );
        }
        if public_bind && dm_enabled && self.dm_credential.is_none() {
            out.push(
                "dungeon masters are enabled without a credential: everyone who joins can \
                 take that role and control the game.",
            );
        }
        out
    }
}

/// Constant-time credential check (compares digests of both values).
pub(crate) fn credential_matches(expected: &str, given: Option<&str>) -> bool {
    // blake3::Hash equality is constant time.
    blake3::hash(expected.as_bytes()) == blake3::hash(given.unwrap_or_default().as_bytes())
}

/// Whether `role` needs a credential this connection has not shown.
pub(crate) fn role_needs_credential(role: Role, verified: bool) -> bool {
    role == Role::DungeonMaster && !verified
}

/// Temporary refusals of addresses that were removed for abuse.
#[derive(Clone, Debug, Default)]
pub(crate) struct TemporaryBans(BTreeMap<IpAddr, Instant>);
impl TemporaryBans {
    pub(crate) fn add(&mut self, ip: IpAddr, until: Instant) {
        self.0.insert(ip, until);
    }
    pub(crate) fn active(&mut self, ip: IpAddr, now: Instant) -> bool {
        self.0.retain(|_, until| *until > now);
        self.0.contains_key(&ip)
    }
}

/// Connection-request budgets by source address.
#[derive(Debug, Default)]
pub(crate) struct RequestBudgets(BTreeMap<IpAddr, (Bucket, Instant)>);
impl RequestBudgets {
    pub(crate) fn allow(&mut self, ip: IpAddr, now: Instant) -> bool {
        if !self.0.contains_key(&ip) && self.0.len() >= TRACKED_REQUEST_ADDRESSES {
            let idle = self
                .0
                .iter()
                .min_by_key(|(_, (_, seen))| *seen)
                .map(|(ip, _)| *ip);
            if let Some(idle) = idle {
                self.0.remove(&idle);
            }
        }
        let (bucket, seen) = self.0.entry(ip).or_insert((Bucket::default(), now));
        *seen = now;
        bucket.take(REQUESTS_PER_ADDRESS, 1.0, now)
    }
}

/// Limits new handshakes. netcode.io keeps an entry for every handshake that
/// is never finished, and on an open game anyone can start one with a valid
/// token, so the rate bounds that memory growth (residual risk MS-R2).
#[derive(Debug)]
pub(crate) struct HandshakeLimiter(std::sync::Mutex<Bucket>);
impl HandshakeLimiter {
    pub(crate) fn new() -> Self {
        Self(std::sync::Mutex::new(Bucket::default()))
    }
}
impl ConnectionRequestHandler for HandshakeLimiter {
    fn handle_request(&self, _client: PeerId) -> Option<DeniedReason> {
        let mut bucket = self.0.lock().unwrap_or_else(|p| p.into_inner());
        (!bucket.take(HANDSHAKE_RATE, 1.0, Instant::now())).then_some(DeniedReason::ServerFull)
    }
}

/// Characters that are invisible or reorder text, which let a name imitate
/// another. Joiners used by scripts that need them (U+200C, U+200D) stay.
pub fn deceptive_char(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{180E}' | '\u{200B}' | '\u{200E}' | '\u{200F}'
        | '\u{202A}'..='\u{202F}' | '\u{205F}'..='\u{206F}' | '\u{3164}' | '\u{FEFF}'
        | '\u{FFF9}'..='\u{FFFB}')
}
/// What a displayed name or label from another computer may contain.
pub fn displayable(c: char) -> bool {
    !c.is_control() && !deceptive_char(c)
}
/// `text` reduced to at most `chars` displayable characters.
pub fn display_text(text: &str, chars: usize) -> String {
    text.chars()
        .filter(|c| displayable(*c))
        .take(chars)
        .collect()
}
