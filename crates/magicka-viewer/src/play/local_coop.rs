//! Local co-op: extra players on this computer, each on their own pad,
//! sharing the camera as local players did in the original. More than the
//! original four and joining mid-game are rewrite extensions
//! (research/multiplayer-scaling.md).
//!
//! The keyboard/mouse (or primary pad) keeps slot 0. Seats chosen in the
//! lobby spawn with the level; during play an unseated pad presses Start to
//! hot join beside the party (when the lobby allows it), and Back + Start
//! leaves. Seated pads never drive the shared menu or the primary player.
use super::*;
use crate::controller::native;
use magicka_network::{control::Controls, session::PlayerProfile};
use magicka_runtime::fixed_tick::PendingSample;

/// A lobby seat handed to play (`--local-players` JSON).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LobbySeat {
    /// The pad's entity bits (same process: the menu and play share a world).
    pub pad: u64,
    pub id: Option<String>,
    pub name: String,
    pub robe: String,
    pub colour: [f32; 3],
}
impl LobbySeat {
    pub(crate) fn from_json(v: &serde_json::Value) -> Result<Self, String> {
        let text = |k: &str| {
            v[k].as_str()
                .map(str::to_owned)
                .ok_or(format!("local player {k}"))
        };
        Ok(Self {
            pad: v["pad"].as_u64().ok_or("local player pad")?,
            id: v["id"].as_str().map(str::to_owned),
            name: text("name")?,
            robe: text("robe")?,
            colour: serde_json::from_value(v["colour"].clone()).map_err(|e| e.to_string())?,
        })
    }
}

struct Seat {
    pad: Entity,
    slot: usize,
    gesture: native::Gesture,
    aim: Vec2,
    ready: bool,
    style: crate::controller::ElementStyle,
    /// Controls waiting for the next simulation tick.
    pending: PendingSample<Controls>,
}

#[derive(Resource, Default)]
pub(crate) struct LocalCoop {
    seats: Vec<Seat>,
    pending: Vec<LobbySeat>,
    max_players: usize,
    hot_join: bool,
    pub notice: Option<(String, f32)>,
    /// Gesture state for a joining client's extra local players.
    client_seats: Vec<Seat>,
    drafts: std::collections::VecDeque<super::join_profile::JoinProfile>,
    identities: HashMap<Entity, String>,
}

impl LocalCoop {
    /// Applies each seat's controls for one simulation tick.
    pub(super) fn dispatch_seats(&mut self, session: &mut PlaySession) {
        for seat in &mut self.seats {
            let Some(controls) = seat.pending.take() else {
                continue;
            };
            if let Err(e) = session.handle_remote_input(seat.slot, controls) {
                warn!("local player input: {e:?}");
            }
        }
    }
    /// The live local party, rather than the lobby roster from session start.
    pub(super) fn continuation_players(&self, session: &PlaySession) -> Vec<serde_json::Value> {
        self.seats
            .iter()
            .filter_map(|seat| {
                session
                    .world
                    .players
                    .get(seat.slot)?
                    .as_ref()
                    .filter(|p| p.playing)?;
                let (name, colour) = session.player_profiles[seat.slot].as_ref()?;
                Some(
                    serde_json::json!({"pad":seat.pad.to_bits(), "id":session.player_ids[seat.slot],
                "name":name, "robe":session.player_robe(seat.slot)?, "colour":colour}),
                )
            })
            .collect()
    }
    pub(super) fn pending_pads(&self) -> impl Iterator<Item = Entity> + '_ {
        self.pending.iter().map(|s| Entity::from_bits(s.pad))
    }
    pub(super) fn join_text(&self) -> Option<String> {
        self.drafts.front().map(|d| d.text())
    }
    pub(crate) fn from_args(args: &[String]) -> Result<Self, String> {
        let lobby = super::lobby_settings(args)?;
        let pending = match arg(args, "--local-players") {
            Some(json) => serde_json::from_str::<serde_json::Value>(&json)
                .map_err(|e| format!("local players: {e}"))?
                .as_array()
                .ok_or("local players: expected a list")?
                .iter()
                .map(LobbySeat::from_json)
                .collect::<Result<_, _>>()?,
            None => Vec::new(),
        };
        Ok(Self {
            seats: Vec::new(),
            pending,
            max_players: usize::from(lobby.max_players),
            hot_join: lobby.hot_join,
            notice: None,
            client_seats: Vec::new(),
            drafts: default(),
            identities: default(),
        })
    }
}

fn controls(pad: &Gamepad, seat: &mut Seat, style: crate::controller::ElementStyle) -> Controls {
    use GamepadButton as B;
    if seat.style != style {
        seat.ready = false;
        seat.style = style;
    }
    if !seat.ready {
        seat.gesture = default();
        seat.ready = pad.get_pressed().next().is_none()
            && ![B::LeftTrigger2, B::RightTrigger2]
                .into_iter()
                .any(|b| pad.get(b).unwrap_or(0.) > 0.5)
            && [GamepadAxis::RightStickX, GamepadAxis::RightStickY]
                .into_iter()
                .all(|a| pad.get(a).unwrap_or(0.).abs() < 0.3);
        return Controls::default();
    }
    let stick = |x, y| {
        let v = Vec2::new(pad.get(x).unwrap_or(0.0), pad.get(y).unwrap_or(0.0));
        if !v.is_finite() || v.length() < 0.25 {
            Vec2::ZERO
        } else {
            v.clamp_length_max(1.0)
        }
    };
    let movement = stick(GamepadAxis::LeftStickX, GamepadAxis::LeftStickY);
    if movement != Vec2::ZERO {
        seat.aim = movement.normalize();
    }
    let trigger = |b| pad.pressed(b) || pad.get(b).unwrap_or(0.0) > 0.5;
    let right = Vec2::new(
        pad.get(GamepadAxis::RightStickX).unwrap_or(0.0),
        pad.get(GamepadAxis::RightStickY).unwrap_or(0.0),
    );
    let help = pad.pressed(B::RightThumb);
    let mut elements = [false; 8];
    // The original right-stick element gestures (controller_native.rs).
    let dpad = [B::DPadUp, B::DPadDown, B::DPadLeft, B::DPadRight];
    let bank = style == crate::controller::ElementStyle::BumperDpad;
    let browsing = bank && pad.pressed(B::RightTrigger);
    if bank {
        seat.gesture = default();
        if !browsing {
            let offset = if pad.pressed(B::LeftTrigger) { 4 } else { 0 };
            for (i, b) in dpad.into_iter().enumerate() {
                elements[offset + i] = pad.pressed(b);
            }
        }
    } else if let Some(element) = seat.gesture.update(right, help) {
        elements[element] = true;
    }
    Controls {
        aim: None,
        pad_movement: Some([movement.x, -movement.y]),
        pad_attack: pad.pressed(B::South),
        pad_block: pad.pressed(B::RightTrigger)
            && (!bank || (!pad.pressed(B::DPadLeft) && !pad.pressed(B::DPadRight))),
        magick_prev: pad.just_pressed(if bank { B::DPadLeft } else { B::DPadUp })
            && (!bank || browsing),
        magick_next: pad.just_pressed(if bank { B::DPadRight } else { B::DPadDown })
            && (!bank || browsing),
        inventory: pad.just_pressed(B::Select),
        move_down: pad.pressed(B::West),
        cast_down: trigger(B::RightTrigger2) || trigger(B::LeftTrigger2),
        cast_self_down: pad.pressed(B::North),
        shift_down: trigger(B::LeftTrigger2),
        boost_down: pad.pressed(B::East),
        elements,
    }
}

/// Seat, hot join, drive and release local co-op players.
pub(super) fn local_coop(
    mut play: ResMut<Play>,
    mut network: Option<NonSendMut<network::NetworkPlay>>,
    mut coop: ResMut<LocalCoop>,
    mut menu: Option<ResMut<crate::menu_input::MenuInput>>,
    controller: Res<crate::controller::ControllerInput>,
    pads: Query<(Entity, &Gamepad)>,
    time: Res<Time>,
    mut devices: ResMut<crate::controller::LocalDevices>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    pause: Res<menu::PauseMenu>,
    #[cfg(feature = "main-menu")] mut frontend: Option<ResMut<crate::frontend::Frontend>>,
) {
    let coop = &mut *coop;
    // A DM can change the host's admission policy while a picker is open.
    if let Some(network::NetworkPlay::Host(host)) = network.as_deref() {
        let lobby = host.lobby();
        coop.hot_join = lobby.hot_join;
        coop.max_players = usize::from(lobby.max_players);
    }
    // Standalone play has no menu input; read Start from the pads.
    let started: Vec<Entity> = match menu.as_ref() {
        Some(m) => m
            .pads
            .iter()
            .filter(|(_, e)| e.start)
            .map(|(p, _)| *p)
            .collect(),
        None => pads
            .iter()
            .filter(|(_, p)| p.just_pressed(GamepadButton::Start))
            .map(|(e, _)| e)
            .collect(),
    };
    if let Some((_, left)) = coop.notice.as_mut() {
        *left -= time.delta_secs();
    }
    coop.notice = coop.notice.take().filter(|(_, left)| *left > 0.0);
    let client_error = match network.as_deref() {
        Some(network::NetworkPlay::Client(view)) => view.require_connected().err(),
        _ => None,
    };
    if let Some(error) = client_error
        .as_ref()
        .filter(|_| !coop.drafts.is_empty() || !coop.pending.is_empty())
    {
        coop.drafts.clear();
        coop.pending.clear();
        coop.notice = Some((error.clone(), 3.0));
    }
    // Start reserves a device immediately; it cannot become player one's pad.
    let client_pads: Vec<_> = match network.as_deref() {
        Some(network::NetworkPlay::Client(view)) => view.local_pads().collect(),
        _ => Vec::new(),
    };
    let mut occupied: std::collections::HashSet<_> = coop
        .seats
        .iter()
        .map(|s| s.pad)
        .chain(client_pads.iter().copied())
        .chain(coop.pending_pads())
        .collect();
    coop.identities.retain(|pad, _| occupied.contains(pad));
    coop.drafts.retain(|d| pads.get(d.pad).is_ok());
    occupied.extend(coop.drafts.iter().map(|d| d.pad));
    let focused = windows.iter().next().is_none_or(|w| w.focused);
    let mut approved = Vec::new();
    if focused {
        let mut starts = started.clone();
        starts.sort_by_key(|e| e.to_bits());
        for pad in starts {
            if devices.primary == Some(pad) || occupied.contains(&pad) {
                continue;
            }
            if let Some(error) = &client_error {
                coop.notice = Some((error.clone(), 3.0));
                continue;
            }
            let (hot_join, max_players, used_count) = match network.as_deref() {
                Some(network::NetworkPlay::Client(view)) => (
                    view.frame.lobby.hot_join,
                    usize::from(view.frame.lobby.max_players),
                    view.frame.players.len(),
                ),
                _ => (
                    coop.hot_join,
                    coop.max_players,
                    play.session
                        .world
                        .players
                        .iter()
                        .flatten()
                        .filter(|p| p.playing)
                        .count(),
                ),
            };
            if !hot_join || used_count + coop.pending.len() + coop.drafts.len() >= max_players {
                coop.notice = Some((
                    if hot_join {
                        "The lobby is full"
                    } else {
                        "Joining a game in progress is locked"
                    }
                    .into(),
                    3.0,
                ));
                continue;
            }
            let mut used: Vec<String> = play
                .session
                .world
                .players
                .iter()
                .enumerate()
                .filter(|(_, p)| p.as_ref().is_some_and(|p| p.playing))
                .filter_map(|(i, _)| play.session.player_ids[i].clone())
                .collect();
            if let Some(network::NetworkPlay::Client(view)) = network.as_deref() {
                used.extend(view.profile_id().map(str::to_owned));
            }
            used.extend(coop.identities.values().cloned());
            used.extend(coop.pending.iter().filter_map(|s| s.id.clone()));
            // Other editors reserve their selected identities when confirming;
            // a final collision check below handles simultaneous choices.
            let Ok((_, gamepad)) = pads.get(pad) else {
                continue;
            };
            let family = crate::controller::Family::detect("", gamepad.vendor_id());
            match super::join_profile::JoinProfile::new(
                pad,
                &play.content,
                &play.cache,
                &used,
                used_count + coop.drafts.len() + 1,
                family,
            ) {
                Ok(draft) => {
                    coop.drafts.push_back(draft);
                    occupied.insert(pad);
                }
                Err(e) => coop.notice = Some((e, 3.0)),
            }
        }
        let mut used: Vec<_> = coop.identities.values().cloned().collect();
        used.extend(
            play.session
                .world
                .players
                .iter()
                .enumerate()
                .filter(|(_, p)| p.as_ref().is_some_and(|p| p.playing))
                .filter_map(|(slot, _)| play.session.player_ids[slot].clone()),
        );
        if let Some(network::NetworkPlay::Client(view)) = network.as_deref() {
            used.extend(view.profile_id().map(str::to_owned));
        }
        for draft in &mut coop.drafts {
            draft.exclude_used(&used);
        }
        if let Some(draft) = coop.drafts.front_mut() {
            if let Ok((_, pad)) = pads.get(draft.pad) {
                use GamepadButton as B;
                if pad.just_pressed(B::East) {
                    occupied.remove(&draft.pad);
                    coop.drafts.pop_front();
                } else if !pad.just_pressed(B::Start) {
                    if pad.just_pressed(B::DPadUp) {
                        draft.row = (draft.row + 3) % 4;
                    }
                    if pad.just_pressed(B::DPadDown) {
                        draft.row = (draft.row + 1) % 4;
                    }
                    if pad.just_pressed(B::DPadLeft) {
                        draft.change(true);
                    }
                    if pad.just_pressed(B::DPadRight) {
                        draft.change(false);
                    }
                    if pad.just_pressed(B::South) {
                        if draft.row < 3 {
                            draft.row += 1;
                        } else {
                            let result = draft.selected().and_then(|seat| {
                                if let Some(network::NetworkPlay::Client(view)) = network.as_deref()
                                {
                                    view.require_connected()?;
                                }
                                // Opening the picker does not reserve a player slot. Recheck
                                // the current policy before saving or creating a connection.
                                let (open, limit, count) = match network.as_deref() {
                                    Some(network::NetworkPlay::Client(view)) => (
                                        view.frame.lobby.hot_join,
                                        usize::from(view.frame.lobby.max_players),
                                        view.frame.players.len(),
                                    ),
                                    _ => (
                                        coop.hot_join,
                                        coop.max_players,
                                        play.session
                                            .world
                                            .players
                                            .iter()
                                            .flatten()
                                            .filter(|p| p.playing)
                                            .count(),
                                    ),
                                };
                                if !open {
                                    return Err("Joining a game in progress is locked".into());
                                }
                                if count + coop.pending.len() >= limit {
                                    return Err("The lobby is full".into());
                                }
                                if seat.id.as_ref().is_some_and(|id| {
                                    coop.identities.values().any(|used| used == id)
                                        || play.session.world.players.iter().enumerate().any(
                                            |(slot, p)| {
                                                p.as_ref().is_some_and(|p| p.playing)
                                                    && play.session.player_ids[slot].as_ref()
                                                        == Some(id)
                                            },
                                        )
                                }) {
                                    return Err("That profile is already playing".into());
                                }
                                draft.save(&play.cache)?;
                                Ok(seat)
                            });
                            match result {
                                Ok(seat) => {
                                    #[cfg(feature = "main-menu")]
                                    if let Some(frontend) = frontend.as_mut() {
                                        frontend.refresh_profiles();
                                    }
                                    approved.push(seat);
                                    coop.drafts.pop_front();
                                }
                                Err(e) => draft.error = Some(e),
                            }
                        }
                    }
                }
            }
        }
    }
    // Suppress held gameplay actions while unfocused or in the shared pause UI.
    let gameplay = focused && !pause.visible && !pause.consume_frame;
    devices.occupied = occupied;
    if let Some(m) = menu.as_mut() {
        m.seated = devices.occupied.clone();
    }
    // On a joining client each extra local player is one more connection to
    // the host; they share this computer's camera.
    if let Some(network::NetworkPlay::Client(view)) = network.as_deref_mut() {
        for notice in view.poll_local() {
            coop.notice = Some((notice, 3.0));
        }
        let joining: Vec<(Entity, PlayerProfile, Option<String>)> =
            std::mem::take(&mut coop.pending)
                .into_iter()
                .chain(approved)
                .map(|seat| {
                    (
                        Entity::from_bits(seat.pad),
                        PlayerProfile {
                            name: seat.name,
                            robe: seat.robe,
                            colour: seat.colour,
                        },
                        seat.id,
                    )
                })
                .collect();
        for (pad, profile, id) in joining {
            if pads.get(pad).is_err() {
                continue;
            }
            match view.add_local(pad, profile, id.clone()) {
                Ok(()) => {
                    if let Some(id) = id {
                        coop.identities.insert(pad, id);
                    }
                }
                Err(e) => coop.notice = Some((e, 3.0)),
            }
        }
        for pad in view.local_pads().collect::<Vec<_>>() {
            let leave = match pads.get(pad) {
                Ok((_, p)) => {
                    p.pressed(GamepadButton::Select) && p.just_pressed(GamepadButton::Start)
                }
                Err(_) => true,
            };
            if leave {
                view.remove_local(pad);
                coop.notice = Some(("A local player left".into(), 3.0));
            }
        }
        let local: Vec<Entity> = view.local_pads().collect();
        if let Some(m) = menu.as_mut() {
            m.seated = local
                .iter()
                .copied()
                .chain(coop.drafts.iter().map(|d| d.pad))
                .collect();
            devices.occupied = m.seated.clone();
        }
        devices.occupied = local
            .iter()
            .copied()
            .chain(coop.drafts.iter().map(|d| d.pad))
            .collect();
        let mut seats = std::mem::take(&mut coop.client_seats);
        seats.retain(|s| local.contains(&s.pad));
        for pad in local {
            if !seats.iter().any(|s| s.pad == pad) {
                seats.push(Seat {
                    pad,
                    slot: 0,
                    gesture: default(),
                    aim: Vec2::ZERO,
                    ready: false,
                    style: default(),
                    pending: default(),
                });
            }
        }
        for seat in &mut seats {
            let (Ok((_, pad)), Some((host_slot, position))) =
                (pads.get(seat.pad), view.local_state(seat.pad))
            else {
                continue;
            };
            let mut c = if gameplay {
                controls(pad, seat, controller.element_style)
            } else {
                seat.ready = false;
                Controls::default()
            };
            let mirror_slot = if host_slot == 0 {
                view.view_slot_for_host_zero()
            } else {
                host_slot
            };
            play.session
                .handle_player_ui(mirror_slot, c.magick_prev, c.magick_next, c.inventory);
            c.magick_prev = false;
            c.magick_next = false;
            c.inventory = false;
            let aim = if seat.aim == Vec2::ZERO {
                Vec2::new(0.0, 1.0)
            } else {
                Vec2::new(seat.aim.x, -seat.aim.y)
            };
            c.aim = Some([position[0] + aim.x, position[1], position[2] + aim.y]);
            if let Err(e) = view.local_input(seat.pad, c) {
                warn!("local player input: {e}");
            }
        }
        coop.client_seats = seats;
        return;
    }
    let join = |play: &mut Play,
                network: &mut Option<NonSendMut<network::NetworkPlay>>,
                profile: PlayerProfile,
                id: Option<&str>,
                hot: bool|
     -> Result<usize, String> {
        if id.and_then(|id| play.session.profile_slot(id)).is_some() {
            return Err("That profile is already playing".into());
        }
        let slot = match network.as_deref_mut() {
            Some(network::NetworkPlay::Host(host)) => {
                host.add_local_player(profile.clone(), &mut play.session)?
            }
            _ => {
                let used = play
                    .session
                    .world
                    .players
                    .iter()
                    .flatten()
                    .filter(|p| p.playing)
                    .count();
                if used >= coop.max_players {
                    return Err("the lobby is full".into());
                }
                let slot = (1..)
                    .find(|s| {
                        play.session
                            .world
                            .players
                            .get(*s)
                            .is_none_or(Option::is_none)
                    })
                    .unwrap();
                if hot {
                    play.session.join_player_near_party(slot, &profile.robe)
                } else {
                    play.session.join_player_at_start(slot, &profile.robe)
                }
                .map_err(|e| format!("{e:?}"))?;
                slot
            }
        };
        play.session.player_profiles[slot] = Some((profile.name.clone(), profile.colour));
        if let Some(id) = id {
            if let Err(e) = play.session.claim_parked(slot, id, &profile.robe) {
                match network.as_deref_mut() {
                    Some(network::NetworkPlay::Host(host)) => {
                        host.remove_local_player(slot, &mut play.session).ok();
                    }
                    _ => {
                        play.session.depart_network_player(slot).ok();
                    }
                }
                return Err(format!("Saved player could not be restored: {e:?}"));
            }
        }
        Ok(slot)
    };
    // Lobby seats join with the level.
    let requests = std::mem::take(&mut coop.pending)
        .into_iter()
        .map(|s| (s, false))
        .chain(approved.into_iter().map(|s| (s, true)));
    for (seat, hot) in requests {
        let pad = Entity::from_bits(seat.pad);
        if pads.get(pad).is_err() || coop.seats.iter().any(|s| s.pad == pad) {
            continue;
        }
        let profile = PlayerProfile {
            name: seat.name.clone(),
            robe: seat.robe.clone(),
            colour: seat.colour,
        };
        match join(&mut play, &mut network, profile, seat.id.as_deref(), hot) {
            Ok(slot) => {
                if let Some(id) = seat.id {
                    coop.identities.insert(pad, id);
                }
                coop.notice = Some((format!("{} joined", seat.name), 3.0));
                coop.seats.push(Seat {
                    pad,
                    slot,
                    gesture: default(),
                    aim: Vec2::ZERO,
                    ready: false,
                    style: default(),
                    pending: default(),
                });
            }
            Err(e) => {
                warn!("local player {}: {e}", seat.name);
                coop.notice = Some((e, 5.0));
            }
        }
    }
    // Leave (Back + Start), a disconnected pad, or a removed avatar.
    let mut leaving = Vec::new();
    for (i, seat) in coop.seats.iter().enumerate() {
        let gone = match pads.get(seat.pad) {
            Ok((_, pad)) => {
                pad.pressed(GamepadButton::Select) && pad.just_pressed(GamepadButton::Start)
            }
            Err(_) => true,
        };
        if gone
            || play
                .session
                .world
                .players
                .get(seat.slot)
                .is_none_or(Option::is_none)
        {
            leaving.push(i);
        }
    }
    for i in leaving.into_iter().rev() {
        let seat = coop.seats.remove(i);
        let result = match network.as_deref_mut() {
            Some(network::NetworkPlay::Host(host)) => {
                host.remove_local_player(seat.slot, &mut play.session)
            }
            _ if play
                .session
                .world
                .players
                .get(seat.slot)
                .is_some_and(Option::is_some) =>
            {
                play.session.park_player(seat.slot);
                play.session
                    .depart_network_player(seat.slot)
                    .map_err(|e| format!("{e:?}"))
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            warn!("local player left: {e}");
        }
        coop.notice = Some((format!("Player {} left", seat.slot + 1), 3.0));
    }
    if let Some(m) = menu.as_mut() {
        m.seated = coop
            .seats
            .iter()
            .map(|s| s.pad)
            .chain(coop.drafts.iter().map(|d| d.pad))
            .collect();
    }
    devices.occupied = coop
        .seats
        .iter()
        .map(|s| s.pad)
        .chain(coop.drafts.iter().map(|d| d.pad))
        .collect();
    // Offline every player is local; hosts track their own set.
    if network.is_none() {
        play.session.camera_slots = None;
    }
    for seat in &mut coop.seats {
        let Ok((_, pad)) = pads.get(seat.pad) else {
            seat.pending = default();
            continue;
        };
        let mut controls = if gameplay {
            controls(pad, seat, controller.element_style)
        } else {
            seat.ready = false;
            Controls::default()
        };
        // Face the stick direction through the aim point the host expects.
        if let Some(p) = play.session.world.players[seat.slot].as_ref() {
            let position = play.session.world.physics.bodies[p.body].position;
            let aim = if seat.aim == Vec2::ZERO {
                let d = play.session.world.physics.bodies[p.body].direction();
                Vec2::new(d[0], d[2])
            } else {
                Vec2::new(seat.aim.x, -seat.aim.y)
            };
            controls.aim = Some([position[0] + aim.x, position[1], position[2] + aim.y]);
        }
        seat.pending.sample(controls);
    }
}

/// `--qa-virtual-pads N` (audit only): spawn N virtual pads after the level
/// settles, press Start on each (hot join), push their sticks apart, and
/// write `local-coop.json` (slots, positions, camera) into the autopilot
/// directory. Exercises the real join/drive/camera path without hardware.
#[derive(Resource)]
pub(crate) struct VirtualPads {
    count: usize,
    pads: Vec<Entity>,
    frame: u32,
    report: Option<std::path::PathBuf>,
}
impl VirtualPads {
    pub(crate) fn from_args(args: &[String]) -> Option<Self> {
        let count = arg(args, "--qa-virtual-pads")?.parse().ok()?;
        Some(Self {
            count,
            pads: Vec::new(),
            frame: 0,
            report: arg(args, "--autopilot")
                .map(|d| std::path::Path::new(&d).join("local-coop.json")),
        })
    }
}
pub(super) fn install_virtual_pads(app: &mut App) {
    app.add_play_systems(
        PreUpdate,
        drive_virtual_pads
            .run_if(resource_exists::<VirtualPads>)
            // Backend clears transient edges first. Both input collectors
            // must see the injected frame, including the menu's per-pad Start.
            .after(bevy::input::InputSystems)
            .before(crate::controller::ControllerCollect),
    );
}
pub(super) fn drive_virtual_pads(
    mut commands: Commands,
    mut qa: ResMut<VirtualPads>,
    mut pads: Query<&mut Gamepad>,
    play: Res<Play>,
    coop: Res<LocalCoop>,
) {
    qa.frame += 1;
    let frame = qa.frame;
    if frame == 60 {
        // A menu-hosted chapter restores the same connected devices. Do not
        // inject another party on top of the roster carried through the Tome.
        if coop.seats.is_empty() && coop.pending.is_empty() && coop.client_seats.is_empty() {
            qa.pads = (0..qa.count)
                .map(|_| commands.spawn(Gamepad::default()).id())
                .collect();
        } else {
            qa.pads = coop.seats.iter().map(|seat| seat.pad).collect();
        }
        return;
    }
    for (i, entity) in qa.pads.clone().into_iter().enumerate() {
        let Ok(mut pad) = pads.get_mut(entity) else {
            continue;
        };
        pad.digital_mut().clear();
        let press = 90 + 10 * i as u32;
        if frame == press && !coop.seats.iter().any(|seat| seat.pad == entity) {
            pad.digital_mut().press(GamepadButton::Start);
        } else if frame == press + 1 {
            pad.digital_mut().release(GamepadButton::Start);
        }
        // Leave the first panel visible long enough for a rendered capture,
        // then explicitly confirm every profile/robe/colour and Join row.
        if frame > 130 && frame % 2 == 0 && coop.drafts.front().is_some_and(|d| d.pad == entity) {
            pad.digital_mut().press(GamepadButton::South);
        } else {
            pad.digital_mut().release(GamepadButton::South);
        }
        // Walk the joined players in different directions, then stop.
        let angle = std::f32::consts::TAU * i as f32 / qa.count.max(1) as f32;
        let (x, y) = if (180..260).contains(&frame) {
            (angle.cos(), angle.sin())
        } else {
            (0.0, 0.0)
        };
        pad.analog_mut().set(GamepadAxis::LeftStickX, x);
        pad.analog_mut().set(GamepadAxis::LeftStickY, y);
    }
    if [120, 170, 300].contains(&frame) {
        let Some(path) = &qa.report else { return };
        let world = &play.session.world;
        let players: Vec<_> = world
            .players
            .iter()
            .enumerate()
            .filter_map(|(slot, p)| {
                let p = p.as_ref().filter(|p| p.playing)?;
                Some(serde_json::json!({
                    "slot": slot,
                    "position": world.physics.bodies[p.body].position,
                    "robe": play.session.player_robe(slot),
                }))
            })
            .collect();
        let report = serde_json::json!({
            "frame": frame,
            "players": players,
            "camera_target": world.camera.ground_position(),
            "camera_slots": play.session.camera_slots,
            "join_panel": coop.join_text(),
            "devices": coop.seats.iter().map(|s| serde_json::json!({"pad":s.pad.to_bits(),"slot":s.slot})).collect::<Vec<_>>(),
        });
        let name = path.with_file_name(format!("local-coop-{frame}.json"));
        if let Err(e) = std::fs::write(name, serde_json::to_vec_pretty(&report).unwrap()) {
            warn!("local co-op report: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn app() -> App {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let content = magicka_content::retail_content_dir()
            .canonicalize()
            .unwrap();
        let level = root
            .join("../../scenarios/feature-lab.lvl")
            .canonicalize()
            .unwrap();
        let mut session = PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
        session.disable_scripts();
        session.step(1. / 60.);
        let cache = std::env::temp_dir().join(format!(
            "magicka-coop-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Time>()
            .init_resource::<menu::PauseMenu>()
            .add_plugins(crate::menu_input::MenuInputPlugin)
            .insert_resource(LocalCoop::from_args(&[]).unwrap())
            .insert_resource(Play {
                session,
                content,
                cache,
                #[cfg(feature = "debug-ui")]
                debug_loadout: None,
                level: level.to_string_lossy().into_owned(),
                character: "Wizard".into(),
                scripts: false,
                demo_goblin: false,
                versus: None,
                packs: None,
                pending: None,
                npc_cues: default(),
                npc_seq: 0,
                reloads: 0,
                hide_level: false,
                #[cfg(feature = "debug-ui")]
                scenarios: root.join("../../scenarios"),
                dt: 1. / 60.,
                screen: [1280., 720.],
                menu_hosted: false,
                complete_after: None,
                bindings: default(),
            })
            .add_systems(
                Update,
                (
                    local_coop,
                    |mut play: ResMut<Play>, mut coop: ResMut<LocalCoop>| {
                        // step_session's tick, without the rest of the session step.
                        coop.dispatch_seats(&mut play.session);
                    },
                )
                    .chain(),
            );
        app.world_mut()
            .resource_mut::<crate::controller::LocalDevices>()
            .locked = true;
        app
    }
    fn press(app: &mut App, pad: Entity, button: GamepadButton) {
        // Clear edges exactly as the backend does, then deliver one press frame.
        for mut p in app
            .world_mut()
            .query::<&mut Gamepad>()
            .iter_mut(app.world_mut())
        {
            p.digital_mut().clear();
        }
        app.world_mut()
            .get_mut::<Gamepad>(pad)
            .unwrap()
            .digital_mut()
            .press(button);
        app.update();
        app.world_mut()
            .get_mut::<Gamepad>(pad)
            .unwrap()
            .digital_mut()
            .clear();
        app.world_mut()
            .get_mut::<Gamepad>(pad)
            .unwrap()
            .digital_mut()
            .release(button);
        app.update();
    }
    fn players(app: &App) -> usize {
        app.world()
            .resource::<Play>()
            .session
            .world
            .players
            .iter()
            .flatten()
            .filter(|p| p.playing)
            .count()
    }
    fn confirm(app: &mut App, pad: Entity) {
        for _ in 0..4 {
            press(app, pad, GamepadButton::South);
        }
    }
    #[test]
    fn virtual_pads_reach_menu_edges_after_backend_clear_and_reuse_restored_devices() {
        let mut app = app();
        app.add_plugins(bevy::input::InputPlugin);
        install_virtual_pads(&mut app);
        let args = vec!["--qa-virtual-pads".into(), "3".into()];
        app.insert_resource(VirtualPads::from_args(&args).unwrap());
        for _ in 0..300 {
            app.update();
        }
        assert_eq!(
            players(&app),
            4,
            "all three Start edges must reach the menu-hosted join flow"
        );
        assert!(app.world().resource::<LocalCoop>().drafts.is_empty());
        let pads = app.world().resource::<VirtualPads>().pads.clone();
        let mapping: Vec<_> = app
            .world()
            .resource::<LocalCoop>()
            .seats
            .iter()
            .map(|s| (s.pad, s.slot))
            .collect();
        app.insert_resource(VirtualPads::from_args(&args).unwrap());
        for _ in 0..300 {
            app.update();
        }
        assert_eq!(players(&app), 4);
        assert_eq!(app.world().resource::<VirtualPads>().pads, pads);
        assert_eq!(
            app.world_mut()
                .query::<&Gamepad>()
                .iter(app.world())
                .count(),
            3
        );
        assert_eq!(
            app.world()
                .resource::<LocalCoop>()
                .seats
                .iter()
                .map(|s| (s.pad, s.slot))
                .collect::<Vec<_>>(),
            mapping
        );
        let cache = app.world().resource::<Play>().cache.clone();
        std::fs::remove_dir_all(cache).unwrap();
    }
    #[test]
    fn failed_parked_restore_keeps_saved_player_and_releases_the_local_seat() {
        for hosted in [false, true] {
            let mut app = app();
            let cache = app.world().resource::<Play>().cache.clone();
            let id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
            let store = magicka_runtime::saves::SaveStore::new(cache.join("saves"));
            let mut settings = store.settings().unwrap();
            settings["profiles"] =
                serde_json::json!([{"id":id,"name":"Parked","robe":"Wizard","colour":[1.,0.,0.]}]);
            store.save_settings(&settings).unwrap();
            let row = serde_json::json!({"id":id,"robe":"Wizard","magicks":128,
                "equipment":[[1,"staff_missing_local_restore_fixture"],[0,"weapon_stung"]]});
            app.world_mut()
                .resource_mut::<Play>()
                .session
                .parked_players
                .push(row.clone());
            if hosted {
                let host = {
                    let play = app.world().resource::<Play>();
                    magicka_runtime::network_session::HostedSession::bind(
                        "127.0.0.1:0".parse().unwrap(),
                        &play.level,
                        false,
                        PlayerProfile::default(),
                        &play.session,
                    )
                    .unwrap()
                };
                app.world_mut()
                    .insert_non_send(network::NetworkPlay::Host(Box::new(host)));
            }
            let pad = app.world_mut().spawn(Gamepad::default()).id();
            press(&mut app, pad, GamepadButton::Start);
            confirm(&mut app, pad);
            assert_eq!(players(&app), 1);
            let coop = app.world().resource::<LocalCoop>();
            assert!(coop.seats.is_empty());
            assert!(coop
                .notice
                .as_ref()
                .unwrap()
                .0
                .contains("could not be restored"));
            assert_eq!(
                app.world().resource::<Play>().session.parked_players,
                vec![row]
            );
            if hosted {
                let mut net = app.world_mut().non_send_mut::<network::NetworkPlay>();
                let network::NetworkPlay::Host(host) = &mut *net else {
                    unreachable!()
                };
                assert_eq!(host.transport.free_local_slot(), Some(1));
            }
            std::fs::remove_dir_all(cache).unwrap();
        }
    }
    #[test]
    fn initial_restart_preserves_pad_seats_profiles_and_parked_progress() {
        let mut app = app();
        let first = app.world_mut().spawn(Gamepad::default()).id();
        let second = app.world_mut().spawn(Gamepad::default()).id();
        press(&mut app, first, GamepadButton::Start);
        confirm(&mut app, first);
        press(&mut app, second, GamepadButton::Start);
        confirm(&mut app, second);
        app.world_mut().despawn(second);
        app.update();
        let before = crate::play::local_party(app.world());
        assert_eq!(before.len(), 1);
        let (ids, profiles, parked) = {
            let play = app.world().resource::<Play>();
            (
                play.session.player_ids.clone(),
                play.session.player_profiles.clone(),
                play.session.parked_players.clone(),
            )
        };
        assert_eq!(parked.len(), 1);
        crate::play::reload(&mut app.world_mut().resource_mut::<Play>());
        app.update();
        assert_eq!(players(&app), 2);
        assert_eq!(crate::play::local_party(app.world()), before);
        {
            let play = app.world().resource::<Play>();
            assert_eq!(play.reloads, 1);
            assert_eq!(play.session.player_ids, ids);
            assert_eq!(play.session.player_profiles, profiles);
            assert_eq!(play.session.parked_players, parked);
        }
        app.world_mut()
            .get_mut::<Gamepad>(first)
            .unwrap()
            .analog_mut()
            .set(GamepadAxis::LeftStickX, 1.);
        app.update();
        let slot = app.world().resource::<LocalCoop>().seats[0].slot;
        let play = app.world().resource::<Play>();
        let player = play.session.world.players[slot].as_ref().unwrap();
        assert_eq!(
            play.session.world.physics.bodies[player.body].movement(),
            [1., 0., 0.]
        );
        let cache = play.cache.clone();
        // The chapter intermission serializes this live roster, then the next
        // session loads a save with every extra profile parked until its pad
        // is seated again. Exercise that handoff with the same connected pad.
        let primary_id = "ffffffffffffffffffffffffffffffff";
        let restored = {
            let mut play = app.world_mut().resource_mut::<Play>();
            play.session.player_ids[0] = Some(primary_id.into());
            let save = play.session.disk_save().unwrap();
            PlaySession::load_disk_save_for(&play.content, &save, primary_id, "Wizard").unwrap()
        };
        app.world_mut().resource_mut::<Play>().session = restored;
        let args = vec![
            "--local-players".into(),
            serde_json::to_string(&before).unwrap(),
        ];
        app.insert_resource(LocalCoop::from_args(&args).unwrap());
        app.update();
        assert_eq!(players(&app), 2);
        assert_eq!(crate::play::local_party(app.world()), before);
        assert_eq!(
            app.world().resource::<Play>().session.parked_players,
            parked
        );
        std::fs::remove_dir_all(cache).unwrap();
    }
    #[test]
    fn keyboard_and_late_pad_require_profile_confirmation_then_control_separate_wizards() {
        let mut app = app();
        app.update(); // no pad at session start
        let pad = app.world_mut().spawn(Gamepad::default()).id();
        press(&mut app, pad, GamepadButton::Start);
        assert_eq!(players(&app), 1, "opening profile picker must not spawn");
        assert!(app.world().resource::<LocalCoop>().join_text().is_some());
        assert_eq!(
            app.world()
                .resource::<crate::controller::ControllerInput>()
                .entity,
            None
        );
        // Change colour in the panel; verify it reaches the actual joined avatar profile.
        press(&mut app, pad, GamepadButton::South);
        press(&mut app, pad, GamepadButton::South);
        press(&mut app, pad, GamepadButton::DPadRight);
        press(&mut app, pad, GamepadButton::South);
        assert_eq!(players(&app), 1);
        press(&mut app, pad, GamepadButton::South);
        assert_eq!(players(&app), 2);
        assert!(app.world().resource::<LocalCoop>().join_text().is_none());
        let slot = app.world().resource::<LocalCoop>().seats[0].slot;
        assert!(app.world().resource::<Play>().session.player_ids[slot].is_some());
        app.world_mut()
            .get_mut::<Gamepad>(pad)
            .unwrap()
            .analog_mut()
            .set(GamepadAxis::LeftStickX, 1.);
        app.update();
        let play = app.world().resource::<Play>();
        let extra = play.session.world.players[slot].as_ref().unwrap();
        assert_eq!(
            play.session.world.physics.bodies[extra.body].movement(),
            [1., 0., 0.]
        );
        assert_eq!(play.session.body().movement(), [0.; 3]);
        let color = play.session.player_profiles[slot].as_ref().unwrap().1;
        assert_eq!(color, crate::player_colors::player_color(2));
        let cache = play.cache.clone();
        assert_eq!(
            magicka_runtime::saves::SaveStore::new(cache.join("saves"))
                .settings()
                .unwrap()["profiles"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        app.world_mut().despawn(pad);
        app.update();
        assert_eq!(players(&app), 1);
        assert!(app
            .world()
            .resource::<crate::controller::LocalDevices>()
            .occupied
            .is_empty());
        std::fs::remove_dir_all(cache).unwrap();
    }
    #[test]
    fn two_controllers_keep_primary_ownership_and_cancel_does_not_spawn_or_save() {
        let mut app = app();
        let primary = app.world_mut().spawn(Gamepad::default()).id();
        app.world_mut()
            .resource_mut::<crate::controller::LocalDevices>()
            .primary = Some(primary);
        app.world_mut()
            .get_mut::<Gamepad>(primary)
            .unwrap()
            .analog_mut()
            .set(GamepadAxis::LeftStickY, 1.);
        app.update();
        let joining = app.world_mut().spawn(Gamepad::default()).id();
        press(&mut app, joining, GamepadButton::Start);
        assert_eq!(
            app.world()
                .resource::<crate::controller::ControllerInput>()
                .entity,
            Some(primary)
        );
        press(&mut app, joining, GamepadButton::East);
        assert_eq!(players(&app), 1);
        assert!(app.world().resource::<LocalCoop>().join_text().is_none());
        assert!(!app
            .world()
            .resource::<Play>()
            .cache
            .join("saves/settings.json")
            .exists());
        press(&mut app, joining, GamepadButton::Start);
        confirm(&mut app, joining);
        assert_eq!(players(&app), 2);
        assert_eq!(
            app.world()
                .resource::<crate::controller::ControllerInput>()
                .entity,
            Some(primary)
        );
        let cache = app.world().resource::<Play>().cache.clone();
        press(&mut app, joining, GamepadButton::Select);
        app.world_mut()
            .get_mut::<Gamepad>(joining)
            .unwrap()
            .digital_mut()
            .press(GamepadButton::Select);
        press(&mut app, joining, GamepadButton::Start);
        assert_eq!(players(&app), 1);
        std::fs::remove_dir_all(cache).unwrap();
    }
    #[test]
    fn lock_and_capacity_refuse_picker_and_disconnect_cancels_it() {
        let mut app = app();
        let pad = app.world_mut().spawn(Gamepad::default()).id();
        app.world_mut().resource_mut::<LocalCoop>().hot_join = false;
        press(&mut app, pad, GamepadButton::Start);
        assert!(app.world().resource::<LocalCoop>().join_text().is_none());
        app.world_mut().resource_mut::<LocalCoop>().hot_join = true;
        app.world_mut().resource_mut::<LocalCoop>().max_players = 1;
        press(&mut app, pad, GamepadButton::Start);
        assert!(app.world().resource::<LocalCoop>().join_text().is_none());
        app.world_mut().resource_mut::<LocalCoop>().max_players = 4;
        press(&mut app, pad, GamepadButton::Start);
        assert!(app.world().resource::<LocalCoop>().join_text().is_some());
        app.world_mut().despawn(pad);
        app.update();
        assert!(app.world().resource::<LocalCoop>().join_text().is_none());
        assert_eq!(players(&app), 1);
    }
    #[test]
    fn queued_editors_drop_profiles_claimed_by_another_pad() {
        let mut app = app();
        let cache = app.world().resource::<Play>().cache.clone();
        let store = magicka_runtime::saves::SaveStore::new(cache.join("saves"));
        let mut settings = store.settings().unwrap();
        settings["profiles"] = serde_json::json!([{
            "id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "name":"Shared profile",
            "robe":"Wizard", "colour":[1.,0.,0.]
        }]);
        store.save_settings(&settings).unwrap();
        let first = app.world_mut().spawn(Gamepad::default()).id();
        let second = app.world_mut().spawn(Gamepad::default()).id();
        press(&mut app, first, GamepadButton::Start);
        press(&mut app, second, GamepadButton::Start);
        confirm(&mut app, first);
        confirm(&mut app, second);
        assert_eq!(players(&app), 3);
        let play = app.world().resource::<Play>();
        let ids: Vec<_> = play.session.player_ids.iter().flatten().collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        assert!(app.world().resource::<LocalCoop>().join_text().is_none());
        std::fs::remove_dir_all(cache).unwrap();
    }
    #[test]
    fn admission_changes_while_editing_do_not_spawn_or_save() {
        let mut app = app();
        let pad = app.world_mut().spawn(Gamepad::default()).id();
        press(&mut app, pad, GamepadButton::Start);
        app.world_mut().resource_mut::<LocalCoop>().hot_join = false;
        confirm(&mut app, pad);
        assert_eq!(players(&app), 1);
        assert!(app
            .world()
            .resource::<LocalCoop>()
            .join_text()
            .unwrap()
            .contains("locked"));
        let cache = app.world().resource::<Play>().cache.clone();
        assert!(!cache.join("saves/settings.json").exists());
        {
            let mut coop = app.world_mut().resource_mut::<LocalCoop>();
            coop.hot_join = true;
            coop.max_players = 1;
        }
        press(&mut app, pad, GamepadButton::South);
        assert_eq!(players(&app), 1);
        assert!(app
            .world()
            .resource::<LocalCoop>()
            .join_text()
            .unwrap()
            .contains("full"));
        assert!(!cache.join("saves/settings.json").exists());
        app.world_mut().resource_mut::<LocalCoop>().max_players = 4;
        press(&mut app, pad, GamepadButton::South);
        assert_eq!(players(&app), 2);
        std::fs::remove_dir_all(cache).unwrap();
    }
    #[test]
    fn extra_seats_use_saved_dpad_scheme_and_magick_buttons() {
        let mut pad = Gamepad::default();
        let mut seat = Seat {
            pad: Entity::PLACEHOLDER,
            slot: 1,
            gesture: default(),
            aim: Vec2::ZERO,
            ready: true,
            style: crate::controller::ElementStyle::BumperDpad,
            pending: default(),
        };
        pad.digital_mut().press(GamepadButton::LeftTrigger);
        pad.digital_mut().press(GamepadButton::DPadRight);
        let c = controls(&pad, &mut seat, crate::controller::ElementStyle::BumperDpad);
        assert!(c.elements[7]);
        assert!(!c.magick_next);
        pad.digital_mut().press(GamepadButton::RightTrigger);
        let c = controls(&pad, &mut seat, crate::controller::ElementStyle::BumperDpad);
        assert_eq!(c.elements, [false; 8]);
        assert!(c.magick_next);
        assert!(!c.pad_block);
    }
    #[test]
    fn online_hot_join_waits_for_profile_confirmation_before_opening_connection() {
        exercise_online_hot_join(None, false);
    }
    #[test]
    fn online_hot_join_inherits_the_primary_connections_content_hash() {
        exercise_online_hot_join(Some("authored-mod-content-fixture".into()), false);
    }
    #[test]
    fn primary_disconnect_cancels_join_editors_and_releases_its_local_group() {
        exercise_online_hot_join(None, true);
    }
    fn exercise_online_hot_join(rules_hash: Option<String>, disconnect: bool) {
        use magicka_network::transport::JoinRequest;
        use magicka_runtime::network_session::HostedSession;
        use std::sync::{
            atomic::{AtomicBool, AtomicUsize},
            Arc,
        };
        let live = Arc::new(AtomicBool::new(true));
        let count = Arc::new(AtomicUsize::new(0));
        let names = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let (address_tx, address_rx) = std::sync::mpsc::channel();
        let server_live = live.clone();
        let server_count = count.clone();
        let server_names = names.clone();
        let server_rules = rules_hash.clone();
        let server = std::thread::spawn(move || {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let content = magicka_content::retail_content_dir();
            let level = root
                .join("../../scenarios/feature-lab.lvl")
                .canonicalize()
                .unwrap();
            let mut session =
                PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
            session.disable_scripts();
            session.step(1. / 60.);
            let mut host = HostedSession::bind(
                "127.0.0.1:0".parse().unwrap(),
                level.to_str().unwrap(),
                false,
                PlayerProfile {
                    name: "Host".into(),
                    robe: "Wizard".into(),
                    colour: [1.; 3],
                },
                &session,
            )
            .unwrap();
            host.set_content(server_rules, Vec::new());
            address_tx.send(host.transport.address()).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            while server_live.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
                host.poll(&mut session).unwrap();
                session.step(1. / 60.);
                host.publish(&session).unwrap();
                let frame = host.frame(&session).unwrap();
                server_count.store(frame.players.len(), Ordering::SeqCst);
                *server_names.lock().unwrap() = frame
                    .players
                    .iter()
                    .map(|p| p.profile.name.clone())
                    .collect();
                std::thread::sleep(Duration::from_millis(4));
            }
        });
        let address = address_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let view = network::ClientView::connect(
            &address.to_string(),
            JoinRequest {
                rules_hash,
                name: "Primary client".into(),
                profile: Some(magicka_network::session::ProfileSubmit {
                    profile: PlayerProfile {
                        name: "Primary client".into(),
                        robe: "Wizard".into(),
                        colour: [1.; 3],
                    },
                    profile_id: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
                }),
                ..default()
            },
        )
        .unwrap();
        let mut app = app();
        app.world_mut()
            .insert_non_send(network::NetworkPlay::Client(Box::new(view)));
        app.add_systems(
            Update,
            (|mut play: ResMut<Play>, mut net: NonSendMut<network::NetworkPlay>| {
                if let network::NetworkPlay::Client(view) = &mut *net {
                    view.poll(&mut play, 1. / 60.).unwrap();
                }
            })
            .after(local_coop),
        );
        let pad = app.world_mut().spawn(Gamepad::default()).id();
        press(&mut app, pad, GamepadButton::Start);
        for _ in 0..30 {
            app.update();
            std::thread::sleep(Duration::from_millis(4));
        }
        assert_eq!(
            count.load(Ordering::SeqCst),
            2,
            "profile picker must not open a connection or create an avatar"
        );
        assert!(app.world().resource::<LocalCoop>().join_text().is_some());
        // Four separate confirmations: profile, wizard, colour, Join game.
        confirm(&mut app, pad);
        for _ in 0..500 {
            app.update();
            if count.load(Ordering::SeqCst) == 3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        assert_eq!(count.load(Ordering::SeqCst), 3);
        assert!(names.lock().unwrap().iter().any(|n| n == "Player 3"));
        assert!(app.world().resource::<LocalCoop>().join_text().is_none());
        // Disconnect the extra pad while the primary connection keeps playing.
        // The host must release it promptly, without the ten-second lease.
        app.world_mut().despawn(pad);
        let departed_at = std::time::Instant::now();
        for _ in 0..500 {
            app.update();
            if count.load(Ordering::SeqCst) == 2 {
                break;
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert!(departed_at.elapsed() < magicka_network::transport::PLAYER_LEASE);
        // A replacement pad can choose the same persisted profile and rejoin.
        let replacement = app.world_mut().spawn(Gamepad::default()).id();
        press(&mut app, replacement, GamepadButton::Start);
        confirm(&mut app, replacement);
        for _ in 0..500 {
            app.update();
            if count.load(Ordering::SeqCst) == 3 {
                break;
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        assert_eq!(count.load(Ordering::SeqCst), 3);
        assert!(names.lock().unwrap().iter().any(|n| n == "Player 3"));
        if disconnect {
            let editing = app.world_mut().spawn(Gamepad::default()).id();
            press(&mut app, editing, GamepadButton::Start);
            assert!(app.world().resource::<LocalCoop>().join_text().is_some());
            let cache = app.world().resource::<Play>().cache.clone();
            let store = magicka_runtime::saves::SaveStore::new(cache.join("saves"));
            let profiles = store.settings().unwrap()["profiles"].clone();
            app.world_mut()
                .resource_scope(|world, mut play: Mut<Play>| {
                    let mut net = world.non_send_mut::<network::NetworkPlay>();
                    let network::NetworkPlay::Client(view) = &mut *net else {
                        unreachable!()
                    };
                    view.poll_at(
                        &mut play,
                        1. / 60.,
                        std::time::Instant::now() + magicka_network::transport::PLAYER_LEASE,
                    )
                    .unwrap();
                });
            app.update();
            assert!(
                app.world().resource::<LocalCoop>().join_text().is_none(),
                "disconnect cancels unsaved editors"
            );
            assert!(!app
                .world()
                .resource::<crate::controller::LocalDevices>()
                .occupied
                .contains(&editing));
            for _ in 0..500 {
                app.update();
                if count.load(Ordering::SeqCst) == 1
                    && crate::play::network_departure_finished(app.world())
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(4));
            }
            assert_eq!(
                count.load(Ordering::SeqCst),
                1,
                "all of the disconnected computer's wizards depart"
            );
            press(&mut app, editing, GamepadButton::Start);
            assert!(app.world().resource::<LocalCoop>().join_text().is_none());
            assert!(app
                .world()
                .resource::<LocalCoop>()
                .notice
                .as_ref()
                .unwrap()
                .0
                .contains("Disconnected"));
            assert_eq!(store.settings().unwrap()["profiles"], profiles);
        } else {
            let editing = app.world_mut().spawn(Gamepad::default()).id();
            press(&mut app, editing, GamepadButton::Start);
            assert!(app.world().resource::<LocalCoop>().join_text().is_some());
        }
        // Quitting the client session releases its primary and extra seats;
        // the standalone exit is deferred until their handshakes finish.
        app.add_message::<crate::play::LeaveSession>()
            .add_message::<bevy::app::AppExit>()
            .add_systems(
                Last,
                (network::begin_leave, crate::play::exit_on_leave).chain(),
            );
        app.world_mut()
            .write_message(crate::play::LeaveSession::default());
        app.update();
        assert!(
            disconnect
                || app
                    .world()
                    .resource::<Messages<bevy::app::AppExit>>()
                    .is_empty()
        );
        let quit_at = std::time::Instant::now();
        for _ in 0..500 {
            app.update();
            if count.load(Ordering::SeqCst) == 1
                && crate::play::network_departure_finished(app.world())
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(4));
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(crate::play::network_departure_finished(app.world()));
        assert!(!app
            .world()
            .resource::<Messages<bevy::app::AppExit>>()
            .is_empty());
        assert!(quit_at.elapsed() < magicka_network::transport::PLAYER_LEASE);
        assert!(
            app.world().resource::<LocalCoop>().join_text().is_none(),
            "quitting cancels unfinished join editors"
        );
        let cache = app.world().resource::<Play>().cache.clone();
        live.store(false, Ordering::Relaxed);
        server.join().unwrap();
        std::fs::remove_dir_all(cache).unwrap();
    }
}

/// Opt-in evidence capture for physical and virtual controllers. Never changes input.
#[derive(Resource)]
pub(super) struct JoinAudit {
    dir: PathBuf,
    frame: u64,
    samples: Vec<serde_json::Value>,
    previous: serde_json::Value,
    panel: bool,
    players: usize,
}
impl JoinAudit {
    pub fn from_args(args: &[String]) -> Option<Self> {
        Some(Self {
            dir: PathBuf::from(arg(args, "--local-coop-audit")?),
            frame: 0,
            samples: Vec::new(),
            previous: serde_json::Value::Null,
            panel: false,
            players: 0,
        })
    }
}
pub(super) fn audit_join(
    mut commands: Commands,
    mut audit: ResMut<JoinAudit>,
    coop: Res<LocalCoop>,
    devices: Res<crate::controller::LocalDevices>,
    pads: Query<(Entity, &Gamepad, Option<&Name>)>,
    play: Res<Play>,
    network: Option<NonSend<network::NetworkPlay>>,
) {
    audit.frame += 1;
    let mut controllers:Vec<_>=pads.iter().map(|(e,p,n)|serde_json::json!({
        "entity":e.to_bits(),"name":n.map(|n|n.as_str()),"vendor":p.vendor_id(),"product":p.product_id(),
        "buttons":p.get_pressed().map(|b|format!("{b:?}")).collect::<Vec<_>>(),
        "left_stick":[p.get(GamepadAxis::LeftStickX).unwrap_or(0.),p.get(GamepadAxis::LeftStickY).unwrap_or(0.)],
    })).collect();
    controllers.sort_by_key(|c| c["entity"].as_u64());
    let players:Vec<_>= match network.as_deref() {
        Some(network::NetworkPlay::Client(v))=> v.frame.players.iter().map(|p|serde_json::json!({"slot":p.slot,"position":p.position,"profile":p.profile})).collect(),
        _=> play.session.world.players.iter().enumerate().filter_map(|(slot,p)|{
            let p=p.as_ref().filter(|p|p.playing)?;
            Some(serde_json::json!({"slot":slot,"position":play.session.world.physics.bodies[p.body].position,
                "profile":play.session.player_profiles[slot],"profile_id":play.session.player_ids[slot],"robe":play.session.player_robe(slot)}))
        }).collect(),
    };
    let panel = coop.join_text();
    let sample = serde_json::json!({"controllers":controllers,"primary":devices.primary.map(Entity::to_bits),
        "devices":coop.seats.iter().map(|seat|serde_json::json!({"pad":seat.pad.to_bits(),"slot":seat.slot})).collect::<Vec<_>>(),
        "players":players,"join_panel":panel,
        "reloads":play.reloads,"level":play.session.level_file(),"scene":play.session.scene_name(),
        "camera_target":play.session.world.camera.ground_position(),
        "camera_slots":play.session.camera_slots});
    if sample == audit.previous {
        return;
    }
    audit.previous = sample.clone();
    let frame = audit.frame;
    audit
        .samples
        .push(serde_json::json!({"frame":frame,"state":sample}));
    if audit.samples.len() > 4096 {
        audit.samples.remove(0);
    }
    if let Err(e) = std::fs::create_dir_all(&audit.dir).and_then(|_| {
        std::fs::write(
            audit.dir.join("input-and-seats.json"),
            serde_json::to_vec_pretty(&audit.samples).unwrap(),
        )
    }) {
        warn!("local controller audit: {e}");
        return;
    }
    if (panel.is_some() && !audit.panel) || players.len() != audit.players {
        let name = if panel.is_some() {
            format!("join-panel-{frame}.png")
        } else {
            format!("players-{}-{frame}.png", players.len())
        };
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(audit.dir.join(name)));
    }
    audit.panel = panel.is_some();
    audit.players = players.len();
}
