//! Modern host authority. Input carries no player slot or position assignment.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Controls {
    /// World-space aim point; the host computes direction, movement and casts.
    pub aim: Option<[f32; 3]>,
    /// Independent gamepad movement in world X/Z (rewrite protocol extension).
    #[serde(default)]
    pub pad_movement: Option<[f32; 2]>,
    #[serde(default)]
    pub pad_attack: bool,
    #[serde(default)]
    pub pad_block: bool,
    pub move_down: bool,
    pub cast_down: bool,
    pub cast_self_down: bool,
    pub shift_down: bool,
    pub boost_down: bool,
    pub elements: [bool; 8],
    #[serde(default)]
    pub magick_prev: bool,
    #[serde(default)]
    pub magick_next: bool,
    #[serde(default)]
    pub inventory: bool,
}
impl Controls {
    pub fn valid(&self) -> bool {
        self.pad_movement
            .is_none_or(|m| m.iter().all(|v| v.is_finite() && v.abs() <= 1.0))
            && self
                .aim
                .is_none_or(|a| a.iter().all(|v| v.is_finite() && v.abs() <= 1_000_000.))
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InputCommand {
    pub session: u64,
    pub epoch: u64,
    pub sequence: u64,
    pub controls: Controls,
    /// Protocol 11: the samples before `sequence`, newest first. Input
    /// travels sequenced-unreliable, so a lost packet's press/release edges
    /// arrive with the next one.
    #[serde(default)]
    pub history: Vec<Controls>,
}
/// Redundant samples carried by each input packet.
pub const INPUT_REDUNDANCY: usize = 3;
impl InputCommand {
    pub fn valid(&self) -> bool {
        self.session != 0
            && self.epoch != 0
            && self.sequence != 0
            && self.controls.valid()
            && self.history.len() <= INPUT_REDUNDANCY
            && self.history.iter().all(Controls::valid)
    }
}
/// Client-side input stream: numbers samples and attaches the redundant
/// history.
#[derive(Clone, Debug, Default)]
pub struct InputStream {
    sequence: u64,
    recent: std::collections::VecDeque<Controls>,
}
impl InputStream {
    pub fn sequence(&self) -> u64 {
        self.sequence
    }
    /// A new epoch restarts the host's sequence stream.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn next(&mut self, session: u64, epoch: u64, controls: Controls) -> InputCommand {
        self.sequence += 1;
        let command = InputCommand {
            session,
            epoch,
            sequence: self.sequence,
            controls,
            history: self.recent.iter().copied().collect(),
        };
        self.recent.push_front(controls);
        self.recent.truncate(INPUT_REDUNDANCY);
        command
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputError {
    Slot,
    Session,
    Epoch,
    Stale,
    Invalid,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickupTarget {
    pub handle: u16,
    pub generation: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickupRequest {
    pub session: u64,
    pub epoch: u64,
    pub sequence: u64,
    pub target: PickupTarget,
}
impl PickupRequest {
    pub fn valid(&self) -> bool {
        self.session != 0 && self.epoch != 0 && self.sequence != 0
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PickupRejection {
    Player,
    InputLimited,
    Unavailable,
    Lifetime,
    OutOfRange,
    Authority(InputError),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PickupFailure {
    Content,
    Script,
    World,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PickupOutcome {
    Book { magick: u8 },
    Swapped,
    Bound,
    Rejected(PickupRejection),
    Failed(PickupFailure),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PickupReceipt {
    pub session: u64,
    pub epoch: u64,
    pub sequence: u64,
    pub target: PickupTarget,
    pub outcome: PickupOutcome,
}
impl PickupRequest {
    pub fn receipt(&self, outcome: PickupOutcome) -> PickupReceipt {
        PickupReceipt {
            session: self.session,
            epoch: self.epoch,
            sequence: self.sequence,
            target: self.target,
            outcome,
        }
    }
}
pub struct InputAuthority {
    session: u64,
    epoch: u64,
    first_slot: usize,
    sequences: std::collections::BTreeMap<usize, u64>,
    pickup_sequences: std::collections::BTreeMap<usize, u64>,
}
impl InputAuthority {
    pub fn new(session: u64, epoch: u64) -> Result<Self, InputError> {
        if session == 0 || epoch == 0 {
            return Err(InputError::Invalid);
        }
        Ok(Self {
            session,
            epoch,
            first_slot: 1,
            sequences: Default::default(),
            pickup_sequences: Default::default(),
        })
    }
    /// Player seats from the first remote slot up to the slot cap, plus
    /// non-player seats (invaders pilot NPCs with ordinary controls).
    fn seat_allowed(&self, seat: usize) -> bool {
        seat >= self.first_slot
            && (seat < crate::lobby::PLAYER_SLOT_CAP || !crate::lobby::is_player_seat(seat))
    }
    pub fn dedicated(session: u64, epoch: u64) -> Result<Self, InputError> {
        let mut authority = Self::new(session, epoch)?;
        authority.first_slot = 0;
        Ok(authority)
    }
    pub fn advance_epoch(&mut self, epoch: u64) -> Result<(), InputError> {
        if epoch <= self.epoch {
            return Err(InputError::Epoch);
        }
        self.epoch = epoch;
        self.sequences.clear();
        self.pickup_sequences.clear();
        Ok(())
    }
    /// A replacement connection starts fresh sequence streams for its slot.
    pub fn release_slot(&mut self, slot: usize) -> Result<(), InputError> {
        if !self.seat_allowed(slot) {
            return Err(InputError::Slot);
        }
        self.sequences.remove(&slot);
        self.pickup_sequences.remove(&slot);
        Ok(())
    }
    /// Slot comes from the connection, never the client payload. Rejections are atomic.
    pub fn accept(&mut self, slot: usize, command: &InputCommand) -> Result<Controls, InputError> {
        if !self.seat_allowed(slot) {
            return Err(InputError::Slot);
        }
        if !command.valid() {
            return Err(InputError::Invalid);
        }
        if command.session != self.session {
            return Err(InputError::Session);
        }
        if command.epoch != self.epoch {
            return Err(InputError::Epoch);
        }
        if command.sequence <= self.sequences.get(&slot).copied().unwrap_or(0) {
            return Err(InputError::Stale);
        }
        self.sequences.insert(slot, command.sequence);
        Ok(command.controls)
    }
    /// Accept a command and the redundant samples covering any gap since the
    /// last accepted sequence, oldest first.
    pub fn accept_with_history(
        &mut self,
        slot: usize,
        command: &InputCommand,
    ) -> Result<Vec<Controls>, InputError> {
        let last = self.sequences.get(&slot).copied().unwrap_or(0);
        let controls = self.accept(slot, command)?;
        let missed = if last == 0 {
            0
        } else {
            (command.sequence - last - 1).min(command.history.len() as u64) as usize
        };
        let mut out: Vec<_> = command.history[..missed].iter().rev().copied().collect();
        out.push(controls);
        Ok(out)
    }
    /// Pickup requests have a separate sequence stream from held input.
    pub fn accept_pickup(
        &mut self,
        slot: usize,
        request: &PickupRequest,
    ) -> Result<PickupTarget, InputError> {
        if !self.seat_allowed(slot) {
            return Err(InputError::Slot);
        }
        if !request.valid() {
            return Err(InputError::Invalid);
        }
        if request.session != self.session {
            return Err(InputError::Session);
        }
        if request.epoch != self.epoch {
            return Err(InputError::Epoch);
        }
        if request.sequence <= self.pickup_sequences.get(&slot).copied().unwrap_or(0) {
            return Err(InputError::Stale);
        }
        self.pickup_sequences.insert(slot, request.sequence);
        Ok(request.target)
    }
}
