//! Script-driven world state for network clients (protocol 10). Clients run
//! no scene scripts (`network_session::mirror_world` disables them), so the
//! host publishes what its triggers changed and clients copy it into their
//! display world: level-part playback, spawned pickups, tutorial hints and
//! gating, and spell presentation. Protocol 14 adds the per-wizard draw
//! inputs (self-shields, aura halos, weapon trails) and one-shot pulses
//! (zap flashes), applied to the client's mirrored avatars so the viewer's
//! own draw paths run for remote wizards.
use crate::play_session::remote_visuals::{RemoteAvatarVisuals, RemoteBlade};
use crate::play_session::PlaySession;
use crate::tutorial::{DialogHint, Hint};
use magicka_network::world::*;
use magicka_sim::character_health::{SelfShield, SelfShieldType};
use std::collections::{BTreeMap, VecDeque};

/// Character.mZapTimer arming value (magicka-sim character_impact.rs):
/// how long a zap pulse keeps the flash drawn.
pub const ZAP_FLASH_SECONDS: f32 = 0.1;
/// Host seconds a pulse stays in snapshots (loss tolerance).
pub const PULSE_WINDOW_SECONDS: f64 = 1.0;

fn shield_kind(kind: SelfShieldType) -> u8 {
    match kind {
        SelfShieldType::None => 0,
        SelfShieldType::Shield => 1,
        SelfShieldType::Earth => 2,
        SelfShieldType::Ice => 3,
        SelfShieldType::IcedEarth => 4,
    }
}

fn shield_type(kind: u8) -> SelfShieldType {
    match kind {
        1 => SelfShieldType::Shield,
        2 => SelfShieldType::Earth,
        3 => SelfShieldType::Ice,
        4 => SelfShieldType::IcedEarth,
        _ => SelfShieldType::None,
    }
}

/// The host's per-wizard draw inputs (server slots), records with nothing
/// to draw omitted.
pub fn avatar_presentation(session: &PlaySession) -> Vec<AvatarPresentation> {
    session
        .world
        .players
        .iter()
        .enumerate()
        .filter_map(|(slot, p)| {
            let p = p.as_ref()?;
            let shield = &p.character.combat.health.shield;
            let record = AvatarPresentation {
                slot,
                shield: shield.active().then(|| SelfShieldState {
                    kind: shield_kind(shield.kind),
                    health: shield.health,
                    max_health: shield.max_health,
                    magnitude: shield.magnitude,
                    ice_magnitude: shield.ice_magnitude,
                    earth_magnitude: shield.earth_magnitude,
                    noise_offsets: shield.noise_offsets,
                    time_since_damage: shield.time_since_damage,
                    time_sensitive: shield.time_sensitive,
                }),
                halo: p.character.halo_draw,
                ice_blade_trail: p.ice_blade_trail().map(|t| t.to_vec()),
                arcane_blades: p
                    .arcane_blades
                    .iter()
                    .take(MAX_AVATAR_BLADES)
                    .map(|b| ArcaneBladeState {
                        id: b.id,
                        color: b.color,
                        vertices: b.vertices.to_vec(),
                    })
                    .collect(),
            };
            (!record.is_empty()).then_some(record)
        })
        .collect()
}

/// Host-side detector of one-shot per-wizard events, kept briefly so a
/// lost snapshot does not lose them. Serials never repeat within a session.
#[derive(Debug, Default)]
pub struct PulseTracker {
    next_serial: u32,
    recent: VecDeque<AvatarPulse>,
    /// Slots whose zap timer was armed at the last observation.
    zapped: BTreeMap<usize, bool>,
}

impl PulseTracker {
    /// Observe the host simulation at host clock `now` (once per tick).
    pub fn observe(&mut self, session: &PlaySession, now: f64) {
        for (slot, p) in session.world.players.iter().enumerate() {
            let armed = p
                .as_ref()
                .is_some_and(|p| p.character.combat.zap_timer > 0.0);
            let was = self.zapped.insert(slot, armed).unwrap_or(false);
            if armed && !was {
                self.push(slot, PulseKind::Zap, now);
            }
        }
        self.recent
            .retain(|p| p.time <= now && now - p.time <= PULSE_WINDOW_SECONDS);
        while self.recent.len() > MAX_PULSES {
            self.recent.pop_front();
        }
    }

    fn push(&mut self, slot: usize, kind: PulseKind, time: f64) {
        self.next_serial = self.next_serial.wrapping_add(1).max(1);
        self.recent.push_back(AvatarPulse {
            serial: self.next_serial,
            slot,
            kind,
            time,
        });
    }

    /// The pulses still inside the window, oldest first.
    pub fn recent(&self) -> Vec<AvatarPulse> {
        self.recent.iter().copied().collect()
    }
}

/// Host seconds a changed decal stays in snapshots (loss tolerance).
pub const DECAL_WINDOW_SECONDS: f64 = 0.5;
/// Alpha change that counts: a growing scorch mark changes it every tick.
const DECAL_ALPHA_STEP: f32 = 0.04;

/// Which ground decals changed recently. Decals are many and mostly still, so
/// only those travel, and each client keeps and ages its own copy.
#[derive(Debug, Default)]
pub struct DecalTracker {
    /// Slot to (generation, alpha and position when last sent, host time).
    seen: BTreeMap<usize, (u64, f32, [f32; 3], f64)>,
    recent: Vec<DecalState>,
}
impl DecalTracker {
    pub fn observe(&mut self, session: &PlaySession, now: f64) {
        let live: Vec<_> = session.world.decals.live().collect();
        self.seen
            .retain(|index, _| live.iter().any(|(r, _)| r.index == *index));
        let mut recent = Vec::new();
        for (r, p) in live {
            let at = [p.transform[12], p.transform[13], p.transform[14]];
            let entry = self.seen.entry(r.index).or_insert((0, 0.0, at, now));
            if entry.0 != r.generation
                || (entry.1 - p.color[3]).abs() >= DECAL_ALPHA_STEP
                || entry.2 != at
            {
                *entry = (r.generation, p.color[3], at, now);
            }
            if now - entry.3 <= DECAL_WINDOW_SECONDS && now >= entry.3 {
                recent.push((
                    entry.3,
                    DecalState {
                        index: r.index as u16,
                        generation: r.generation,
                        transform: p.transform,
                        args: p.args,
                        color: p.color,
                        animated_owner: p.animated_owner,
                    },
                ));
            }
        }
        // Newest first: what a frame cannot carry is what most clients have.
        recent.sort_by(|a, b| b.0.total_cmp(&a.0));
        self.recent = recent.into_iter().map(|(_, d)| d).collect();
    }
    pub fn recent(&self) -> Vec<DecalState> {
        self.recent.clone()
    }
}

/// The host's presentation of its current scene.
pub fn world_presentation(session: &PlaySession) -> WorldPresentation {
    let world = &session.world;
    let pickups = world
        .entities
        .iter()
        .filter(|e| e.added && !e.removed && !e.dead)
        .filter_map(|e| {
            if let Some(book) = world.book_pickups.get(&e.handle) {
                return (!book.dead).then_some(PickupState {
                    handle: e.handle,
                    generation: book.generation,
                    kind: PickupKind::Book {
                        magick: book.magick,
                    },
                    transform: book.transform,
                });
            }
            let pickup = world.item_pickups.get(&e.handle)?;
            Some(PickupState {
                handle: e.handle,
                generation: pickup.generation,
                kind: PickupKind::Item {
                    name: pickup.item.as_ref()?.definition.name.clone(),
                },
                transform: pickup.transform,
            })
        })
        .collect();
    let tutorial = &world.tutorial;
    let mut cast_types: Vec<i32> = world
        .flags
        .tutorial_cast_types_disabled
        .iter()
        .copied()
        .collect();
    cast_types.truncate(16);
    WorldPresentation {
        animations: world
            .collision
            .playback_states()
            .into_iter()
            .map(|p| PartPlaybackState {
                time: p.time,
                start: p.start,
                end: p.end,
                speed: p.speed,
                looping: p.looping,
                playing: p.playing,
                resting_timer: p.resting_timer,
            })
            .collect(),
        pickups,
        tutorial: TutorialState {
            hint: tutorial.target_hint().map(|h| HintState {
                id: h.id,
                text: h.text.clone(),
                controller_text: h.controller_text.clone(),
                animation: h.animation,
                position: h.position,
            }),
            dialog: tutorial.dialog.as_ref().map(|d| DialogState {
                serial: tutorial.dialog_serial,
                text: d.text.clone(),
                element: d.element,
                magick: d.magick,
                scale: d.scale,
                size: d.size,
            }),
            dark: tutorial.dark_target(),
        },
        gating: GatingState {
            elements_disabled: world.flags.tutorial_elements_disabled,
            cast_types_disabled: cast_types,
            push_disabled: world.flags.tutorial_push_disabled,
            magicks_allowed: session.magicks_allowed(),
            ui_enabled: world.ui_enabled,
        },
        spells: SpellPresentation {
            beams: session.beams.clone(),
            bolts: session
                .bolts
                .iter()
                .map(|b| BoltState {
                    serial: b.serial,
                    ttl: b.ttl,
                    visual: b.visual,
                })
                .collect(),
            shields: world
                .shields
                .iter()
                .map(|s| (s.handle, s.shield.clone()))
                .collect(),
            barriers: world.barriers.iter().map(|(s, b)| (s, b.clone())).collect(),
            mines: world.mines.iter().map(|(s, m)| (s, m.clone())).collect(),
            arcane_blasts: session
                .arcane_blasts
                .iter()
                .map(|b| ArcaneBlastState {
                    position: b.position,
                    max_radius: b.max_radius,
                    elements: b.elements,
                    color: b.color,
                    ttl: b.ttl,
                    rotation: b.rotation,
                })
                .collect(),
            missiles: world
                .missiles
                .iter()
                .filter(|m| !m.body.dead)
                .filter_map(|m| {
                    let model = match (m.body.model, &m.body.item_model) {
                        (Some(model), _) => {
                            format!("Models/Missiles/{}", crate::missiles::model_name(model))
                        }
                        (None, Some(item)) => item.clone(),
                        (None, None) => return None,
                    };
                    Some(MissileState {
                        id: m.body.id,
                        model,
                        position: m.body.position,
                        velocity: m.body.velocity,
                    })
                })
                .collect(),
            lights: session
                .spell_point_lights()
                .map(|l| LightState {
                    position: l.position,
                    diffuse: l.diffuse,
                    ambient: l.ambient,
                    capsule_end: None,
                    intensity: l.draw_intensity(),
                    radius: l.radius,
                    specular: l.specular,
                })
                .chain(
                    session
                        .spell_capsule_lights()
                        .into_iter()
                        .map(|l| LightState {
                            position: l.start,
                            diffuse: l.diffuse,
                            ambient: l.ambient,
                            capsule_end: Some(l.end),
                            intensity: l.intensity,
                            radius: l.radius,
                            specular: 0.0,
                        }),
                )
                .collect(),
        },
        avatars: avatar_presentation(session),
        // The host's `PulseTracker` and `DecalTracker` fill these
        // (network_session.rs).
        pulses: Vec::new(),
        decals: Vec::new(),
        numbers: world
            .floating_numbers
            .iter()
            .map(|n| NumberState {
                position: n.world_position,
                scale: n.scale,
                value: n.value,
                is_xp: n.is_xp,
                color: n.color,
            })
            .collect(),
    }
}

/// A client's copy of the host's scripted world state.
#[derive(Default)]
pub struct RemoteWorld {
    /// Host pickup handle → this world's handle.
    pickups: BTreeMap<u16, u16>,
    /// The host dialog (serial) shown here, and the one dismissed here.
    shown_dialog: Option<u32>,
    dismissed_dialog: Option<u32>,
    /// Newest pulse serial applied here (late joiners skip history).
    pulse_serial: Option<u32>,
}

/// This machine's slot for a host slot: the own wizard lives in slot 0,
/// the host's slot 0 takes the own slot (or the observer view slot).
pub fn mirror_slot(host: usize, own: Option<usize>) -> usize {
    match own {
        Some(own) if host == own => 0,
        Some(own) if host == 0 => own,
        None if host == 0 => crate::network_session::OBSERVER_VIEW_SLOT,
        _ => host,
    }
}

impl RemoteWorld {
    /// This world's handle for a host pickup.
    pub fn local_pickup(&self, host: u16) -> Option<u16> {
        self.pickups.get(&host).copied()
    }
    /// The host handle of one of this world's pickups.
    pub fn host_pickup(&self, local: u16) -> Option<u16> {
        self.pickups
            .iter()
            .find(|(_, l)| **l == local)
            .map(|(h, _)| *h)
    }
    /// Host pickup → local handle pairs.
    pub fn pickup_pairs(&self) -> impl Iterator<Item = (u16, u16)> + '_ {
        self.pickups.iter().map(|(h, l)| (*h, *l))
    }

    /// Copy the host's presentation into the client's display world and
    /// advance the tutorial fades and keyboard HUD by `dt`. Only a client
    /// with its own wizard (`player`) shows the modal dialog hints.
    pub fn apply(
        &mut self,
        state: &WorldPresentation,
        session: &mut PlaySession,
        dt: f32,
        player: bool,
    ) -> Result<(), String> {
        let playback: Vec<_> = state
            .animations
            .iter()
            .map(|p| crate::animated_pose::PartPlayback {
                time: p.time,
                start: p.start,
                end: p.end,
                speed: p.speed,
                looping: p.looping,
                playing: p.playing,
                resting_timer: p.resting_timer,
            })
            .collect();
        if !playback.is_empty() && session.world.collision.playback_states() != playback {
            session
                .world
                .collision
                .restore_playback(&playback, &magicka_content::skinned::IDENTITY)
                .map_err(|e| format!("mirror level animations: {e:?}"))?;
        }
        self.apply_pickups(&state.pickups, session)?;
        self.apply_tutorial(&state.tutorial, session, dt, player);
        let flags = &mut session.world.flags;
        flags.tutorial_elements_disabled = state.gating.elements_disabled;
        flags.tutorial_cast_types_disabled =
            state.gating.cast_types_disabled.iter().copied().collect();
        flags.tutorial_push_disabled = state.gating.push_disabled;
        session.mirrored_magicks_allowed = Some(state.gating.magicks_allowed);
        // PlayState.Update's HUDManager step, which clients do not run.
        session.world.ui_enabled = state.gating.ui_enabled;
        if session.world.ui_enabled {
            session
                .keyboard_hud
                .set_disabled_elements(state.gating.elements_disabled as u32);
            session.keyboard_hud.update(dt);
        }
        let spells = &state.spells;
        session.beams = spells.beams.clone();
        session.bolts = spells
            .bolts
            .iter()
            .map(|b| crate::play_session::LiveBolt {
                visual: b.visual,
                ttl: b.ttl,
                serial: b.serial,
            })
            .collect();
        session.world.shields = spells
            .shields
            .iter()
            .map(|(handle, shield)| crate::scene_world::LiveShield {
                handle: *handle,
                shield: shield.clone(),
            })
            .collect();
        session.world.barriers = magicka_sim::barrier::BarrierPool::mirror(spells.barriers.clone());
        session.world.mines = magicka_sim::spell_mine::MinePool::mirror(spells.mines.clone());
        session.arcane_blasts = spells
            .arcane_blasts
            .iter()
            .map(|b| crate::arcane_blast::ArcaneBlast {
                position: b.position,
                max_radius: b.max_radius,
                elements: b.elements,
                color: b.color,
                ttl: b.ttl,
                rotation: b.rotation,
            })
            .collect();
        session.remote_missiles = spells.missiles.clone();
        session.dynamic_lights = spells
            .lights
            .iter()
            .filter(|l| l.capsule_end.is_none())
            .map(|l| {
                let mut light = crate::arcane_blast::DynamicLight::mirrored(
                    l.position,
                    l.diffuse,
                    l.intensity,
                    l.radius,
                    l.specular,
                );
                light.ambient = l.ambient;
                light
            })
            .collect();
        session.remote_capsule_lights = Some(
            spells
                .lights
                .iter()
                .filter_map(|l| {
                    l.capsule_end.map(|end| crate::arcane_blade::CapsuleLight {
                        start: l.position,
                        end,
                        radius: l.radius,
                        diffuse: l.diffuse,
                        ambient: l.ambient,
                        intensity: l.intensity,
                    })
                })
                .collect(),
        );
        for decal in &state.decals {
            session.world.decals.mirror(
                crate::decals::Reference {
                    index: usize::from(decal.index),
                    generation: decal.generation,
                },
                crate::decals::Projection {
                    transform: decal.transform,
                    inverse: crate::live_collision::inverse(&decal.transform)
                        .unwrap_or([f32::NAN; 16]),
                    args: decal.args,
                    color: decal.color,
                    animated_owner: decal.animated_owner,
                },
            );
        }
        // Clients do not step the world, so decals age here.
        session.world.decals.update(dt);
        session.world.floating_numbers = state
            .numbers
            .iter()
            .map(|n| crate::floating_text::NumberInstance {
                world_position: n.position,
                scale: n.scale,
                value: n.value,
                is_xp: n.is_xp,
                color: n.color,
            })
            .collect();
        Ok(())
    }

    fn apply_pickups(
        &mut self,
        pickups: &[PickupState],
        session: &mut PlaySession,
    ) -> Result<(), String> {
        let world = &mut session.world;
        let matches =
            |world: &crate::scene_world::SceneWorld<_>, local: u16, kind: &PickupKind| match kind {
                PickupKind::Book { magick } => world
                    .book_pickups
                    .get(&local)
                    .is_some_and(|b| b.magick == *magick),
                PickupKind::Item { name } => world
                    .item_pickups
                    .get(&local)
                    .and_then(|p| p.item.as_ref())
                    .is_some_and(|i| i.definition.name == *name),
            };
        // Mappings survive only while the local pickup still matches (scene
        // changes rebuild the world and its handles).
        let host: BTreeMap<u16, &PickupState> = pickups.iter().map(|p| (p.handle, p)).collect();
        self.pickups
            .retain(|h, l| host.get(h).is_some_and(|p| matches(world, *l, &p.kind)));
        for p in pickups {
            if self.pickups.contains_key(&p.handle) {
                continue;
            }
            let taken = |l: u16| self.pickups.values().any(|v| *v == l);
            // Level-placed pickups load with the same handles on both sides.
            let local = if matches(world, p.handle, &p.kind) && !taken(p.handle) {
                p.handle
            } else {
                world
                    .mirror_pickup(&p.kind, p.transform)
                    .map_err(|e| format!("mirror pickup: {e:?}"))?
            };
            self.pickups.insert(p.handle, local);
        }
        let live: BTreeMap<u16, &PickupState> =
            self.pickups.iter().map(|(h, l)| (*l, host[h])).collect();
        for entity in &mut world.entities {
            let book = world.book_pickups.get_mut(&entity.handle);
            let item = world.item_pickups.get_mut(&entity.handle);
            if book.is_none() && item.is_none() {
                continue;
            }
            match live.get(&entity.handle) {
                Some(p) => {
                    entity.added = true;
                    entity.removed = false;
                    entity.dead = false;
                    entity.orientation = p.transform;
                    entity.position = [p.transform[12], p.transform[13], p.transform[14]];
                    if let Some(book) = book {
                        book.transform = p.transform;
                        book.generation = p.generation;
                        book.dead = false;
                    }
                    if let Some(item) = item {
                        item.transform = p.transform;
                        item.generation = p.generation;
                    }
                }
                None => {
                    entity.removed = true;
                    entity.dead = true;
                }
            }
        }
        Ok(())
    }

    fn apply_tutorial(
        &mut self,
        state: &TutorialState,
        session: &mut PlaySession,
        dt: f32,
        player: bool,
    ) {
        let tutorial = &mut session.world.tutorial;
        // Without a wizard there is nothing to teach: no modal dialog.
        if !player && tutorial.dialog.is_some() {
            tutorial.dismiss_dialog();
            tutorial.dialog = None;
            self.shown_dialog = None;
        }
        let serial = state.dialog.as_ref().map(|d| d.serial).filter(|_| player);
        if let Some(d) = state.dialog.as_ref().filter(|_| player) {
            if self.shown_dialog != serial && self.dismissed_dialog != serial {
                tutorial.set_dialog(DialogHint {
                    text: d.text.clone(),
                    element: d.element,
                    magick: d.magick,
                    // The host runs the continuation trigger.
                    trigger: 0,
                    scale: d.scale,
                    size: d.size,
                });
                self.shown_dialog = serial;
            }
        }
        let dismissed_here = serial.is_some() && self.dismissed_dialog == serial;
        tutorial.mirror(
            state.hint.as_ref().map(|h| Hint {
                id: h.id,
                text: h.text.clone(),
                controller_text: h.controller_text.clone(),
                animation: h.animation,
                position: h.position,
            }),
            state.dark && player && !dismissed_here,
        );
        tutorial.update(dt);
    }

    /// Copy the host's per-wizard draw inputs onto this machine's mirrored
    /// avatars (`mirror_slot`) and arm their zap timers from the pulses, so
    /// the viewer's self-shield, halo, trail and zap draw paths run for
    /// them. `time` is the host clock of the frame. Returns the pulses not
    /// applied before (each once); the first call primes and returns none.
    pub fn apply_avatars(
        &mut self,
        state: &WorldPresentation,
        session: &mut PlaySession,
        own_slot: Option<usize>,
        time: f64,
    ) -> Vec<AvatarPulse> {
        let records: BTreeMap<usize, &AvatarPresentation> = state
            .avatars
            .iter()
            .map(|a| (mirror_slot(a.slot, own_slot), a))
            .collect();
        let mut zap: BTreeMap<usize, f32> = BTreeMap::new();
        for p in &state.pulses {
            if p.kind == PulseKind::Zap {
                let left = (ZAP_FLASH_SECONDS - (time - p.time).max(0.0) as f32).max(0.0);
                let slot = mirror_slot(p.slot, own_slot);
                let entry = zap.entry(slot).or_default();
                *entry = entry.max(left);
            }
        }
        let fresh: Vec<AvatarPulse> = match self.pulse_serial {
            None => Vec::new(),
            Some(last) => state
                .pulses
                .iter()
                .filter(|p| p.serial > last)
                .copied()
                .collect(),
        };
        self.pulse_serial = Some(
            state
                .pulses
                .iter()
                .map(|p| p.serial)
                .chain(self.pulse_serial)
                .max()
                .unwrap_or(0),
        );
        // Observers' dormant own avatar (slot 0) mirrors nobody.
        let dormant = own_slot.is_none();
        for (slot, p) in session.world.players.iter_mut().enumerate() {
            let Some(p) = p.as_mut() else { continue };
            if slot == 0 && dormant {
                continue;
            }
            let record = records.get(&slot);
            let combat = &mut p.character.combat;
            combat.health.shield = match record.and_then(|r| r.shield) {
                Some(s) => SelfShield {
                    kind: shield_type(s.kind),
                    spell: combat.health.shield.spell,
                    health: s.health,
                    max_health: s.max_health,
                    magnitude: s.magnitude,
                    ice_magnitude: s.ice_magnitude,
                    earth_magnitude: s.earth_magnitude,
                    noise_offsets: s.noise_offsets,
                    time_since_damage: s.time_since_damage,
                    time_sensitive: s.time_sensitive,
                },
                None => SelfShield::default(),
            };
            combat.zap_timer = zap.get(&slot).copied().unwrap_or(0.0);
            p.character.halo_draw = record.and_then(|r| r.halo);
            let visuals = record.map(|r| RemoteAvatarVisuals {
                ice_blade_trail: r
                    .ice_blade_trail
                    .as_ref()
                    .and_then(|t| <[[f32; 3]; TRAIL_VERTICES]>::try_from(t.as_slice()).ok())
                    .map(Box::new),
                arcane_blades: r
                    .arcane_blades
                    .iter()
                    .filter_map(|b| {
                        Some(RemoteBlade {
                            id: b.id,
                            color: b.color,
                            vertices: Box::new(
                                <[[f32; 3]; TRAIL_VERTICES]>::try_from(b.vertices.as_slice())
                                    .ok()?,
                            ),
                        })
                    })
                    .collect(),
            });
            match visuals.filter(|v| v.ice_blade_trail.is_some() || !v.arcane_blades.is_empty()) {
                Some(v) => {
                    session.remote_avatars.insert(p.handle, v);
                }
                None => {
                    session.remote_avatars.remove(&p.handle);
                }
            }
        }
        fresh
    }

    /// Dismiss this machine's copy of the dialog (its own players' input
    /// only; the host's dismissal runs the trigger). False during holdoff.
    pub fn dismiss_dialog(&mut self, session: &mut PlaySession) -> bool {
        if session.world.tutorial.dismiss_dialog().is_some() {
            self.dismissed_dialog = self.shown_dialog;
            return true;
        }
        false
    }
}
