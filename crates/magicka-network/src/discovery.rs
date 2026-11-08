//! LAN game advertisement (rewrite extension). Hosts broadcast a small
//! beacon about once a second on [`DISCOVERY_PORT`]; the Online menu lists
//! what it hears. Plain UDP broadcast on the local network segment, no
//! internet matchmaking. Beacons are informational: joining still goes
//! through the normal handshake and lobby policy.
use crate::lobby::{LobbySettings, Role};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    time::{Duration, Instant},
};

/// UDP port for beacons (authored, not a retail value).
pub const DISCOVERY_PORT: u16 = 17799;
const MAGIC: &str = "magicka-rewrite-lan";
const INTERVAL: Duration = Duration::from_secs(1);
/// Games not heard from for this long drop off the list.
pub const ADVERT_TTL: Duration = Duration::from_secs(4);

/// What a host advertises.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Advert {
    magic: String,
    pub version: u16,
    /// The game's UDP port on the sender's address.
    pub port: u16,
    pub name: String,
    pub level: String,
    pub players: u16,
    pub lobby: LobbySettings,
    /// Who is in the game and as what (newer hosts; capped).
    #[serde(default)]
    pub participants: Vec<AdvertSeat>,
    /// Joining needs the game's passphrase. A private game lists nobody.
    #[serde(default)]
    pub private: bool,
}

/// One participant listed in a beacon.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvertSeat {
    pub name: String,
    pub role: Role,
    /// Waiting (as a spectator) for a player slot.
    #[serde(default)]
    pub queued: bool,
}

/// Participants listed per beacon (keeps a beacon within one datagram).
pub const ADVERT_PARTICIPANTS: usize = 32;
const NAME_CHARS: usize = 24;
/// Games one browser lists: anyone on the LAN can send beacons, from as many
/// addresses as they can forge.
pub const MAX_LAN_GAMES: usize = 64;
/// Beacons read per poll, so a flood cannot stall the menu.
const MAX_BEACONS_PER_POLL: usize = 256;

impl Advert {
    pub fn new(port: u16, name: &str, level: &str, players: usize, lobby: LobbySettings) -> Self {
        Self {
            magic: MAGIC.into(),
            version: crate::PROTOCOL_VERSION,
            port,
            name: crate::security::display_text(name, 32),
            level: crate::security::display_text(
                level.rsplit(['/', '\\']).next().unwrap_or(level),
                64,
            ),
            players: players.min(usize::from(u16::MAX)) as u16,
            lobby,
            participants: Vec::new(),
            private: false,
        }
    }
    /// Advertise a private game: it can be found, but not who is in it.
    pub fn private(mut self) -> Self {
        self.private = true;
        self.participants.clear();
        self
    }
    /// List `participants` (truncated names, at most [`ADVERT_PARTICIPANTS`]).
    pub fn with_participants(mut self, participants: impl IntoIterator<Item = AdvertSeat>) -> Self {
        self.participants = participants
            .into_iter()
            .take(ADVERT_PARTICIPANTS)
            .map(|p| AdvertSeat {
                name: crate::security::display_text(&p.name, NAME_CHARS),
                ..p
            })
            .collect();
        self
    }
    /// Player slots still open.
    pub fn open_slots(&self) -> u16 {
        self.lobby.max_players.saturating_sub(self.players)
    }
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
    /// Valid beacons of a compatible protocol only. Beacons are unauthenticated:
    /// nothing in one may be believed beyond showing it in a list, so its text
    /// comes out reduced to what its sender is allowed to display.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let mut advert: Self = serde_json::from_slice(bytes).ok()?;
        advert.name = crate::security::display_text(&advert.name, 32);
        advert.level = crate::security::display_text(&advert.level, 64);
        advert.participants.truncate(ADVERT_PARTICIPANTS);
        for seat in &mut advert.participants {
            seat.name = crate::security::display_text(&seat.name, NAME_CHARS);
        }
        (advert.magic == MAGIC
            && advert.version == crate::PROTOCOL_VERSION
            && advert.port != 0
            && advert.lobby.valid()
            && !advert.name.is_empty())
        .then_some(advert)
    }
}

/// Host side: broadcasts the current advert at most once per interval.
pub struct Beacon {
    socket: Option<UdpSocket>,
    last: Option<Instant>,
    port: u16,
}

impl Default for Beacon {
    fn default() -> Self {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .and_then(|s| {
                s.set_broadcast(true)?;
                s.set_nonblocking(true)?;
                Ok(s)
            })
            .ok();
        Self {
            socket,
            last: None,
            port: DISCOVERY_PORT,
        }
    }
}

impl Beacon {
    /// A beacon for another discovery port (tests, side-by-side setups).
    pub fn with_port(port: u16) -> Self {
        Self {
            port,
            ..Self::default()
        }
    }
    /// Broadcast `advert` on the LAN (and to this machine) when the
    /// interval has passed. Send failures are ignored: a
    /// network without broadcast simply hides the game from browsers.
    pub fn tick(&mut self, now: Instant, advert: &Advert) {
        if self
            .last
            .is_some_and(|t| now.saturating_duration_since(t) < INTERVAL)
        {
            return;
        }
        self.last = Some(now);
        let Some(socket) = &self.socket else { return };
        let bytes = advert.encode();
        // Broadcast for other computers; loopback too, because a broadcast
        // is not always delivered back to listeners on the sending machine.
        for target in [Ipv4Addr::BROADCAST, Ipv4Addr::LOCALHOST] {
            let _ = socket.send_to(&bytes, (target, self.port));
        }
    }
}

/// A game heard on the LAN.
#[derive(Clone, Debug, PartialEq)]
pub struct LanGame {
    pub address: SocketAddr,
    pub advert: Advert,
    pub seen: Instant,
}

/// Browser side: listens for beacons and keeps the recently heard games.
pub struct LanBrowser {
    socket: Option<UdpSocket>,
    games: BTreeMap<SocketAddr, LanGame>,
}

impl LanBrowser {
    /// Listen on `port` (normally [`DISCOVERY_PORT`]). Another listener on
    /// the same machine may hold the port; the browser then stays empty.
    pub fn bind(port: u16) -> Self {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port))
            .and_then(|s| {
                s.set_nonblocking(true)?;
                Ok(s)
            })
            .ok();
        Self {
            socket,
            games: BTreeMap::new(),
        }
    }
    pub fn listening(&self) -> bool {
        self.socket.is_some()
    }
    /// Drain received beacons and expire silent games; newest names first by
    /// address order.
    pub fn poll(&mut self, now: Instant) -> Vec<LanGame> {
        let mut buffer = [0u8; 16384];
        if let Some(socket) = &self.socket {
            for _ in 0..MAX_BEACONS_PER_POLL {
                let Ok((n, from)) = socket.recv_from(&mut buffer) else {
                    break;
                };
                if let Some(advert) = Advert::decode(&buffer[..n]) {
                    let address = SocketAddr::new(from.ip(), advert.port);
                    if !self.games.contains_key(&address) && self.games.len() >= MAX_LAN_GAMES {
                        continue;
                    }
                    self.games.insert(
                        address,
                        LanGame {
                            address,
                            advert,
                            seen: now,
                        },
                    );
                }
            }
        }
        self.games
            .retain(|_, g| now.saturating_duration_since(g.seen) < ADVERT_TTL);
        // A local game heard both ways is listed once, by its LAN address.
        let lan: Vec<(u16, String)> = self
            .games
            .values()
            .filter(|g| !g.address.ip().is_loopback())
            .map(|g| (g.advert.port, g.advert.name.clone()))
            .collect();
        self.games
            .values()
            .filter(|g| {
                !g.address.ip().is_loopback()
                    || !lan.contains(&(g.advert.port, g.advert.name.clone()))
            })
            .cloned()
            .collect()
    }
}
