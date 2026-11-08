//! Scene host for `campaign_action` (research/campaign-actions.md). Avatar
//! equipment changes need the session's content and rigs, so they are queued
//! as `CampaignRequest`s and applied by PlaySession.
use super::*;
use crate::campaign_action::{CampaignAction, CampaignCommand as C, Operation};

/// Session-side work requested by campaign actions, in execution order.
#[derive(Clone, Debug, PartialEq)]
pub enum CampaignRequest {
    /// PlayState.UnequipWeaponsSection = begin (End plays `animation`).
    UnequipWeaponsSection {
        begin: bool,
        animation: u16,
    },
    /// PlayState.DefaultAvatarSection.
    DefaultAvatarSection(bool),
    /// Level.GoToScene's first scene with Level.NoItems: every avatar gets
    /// weapon_unarmed in both slots before the scene's triggers run.
    NoItems,
    /// AssignItem (player 0 = all, or the first avatar in `area`).
    AssignItem {
        item: String,
        player: usize,
        area: i32,
        animation: u16,
        quick: bool,
    },
    AddItemToAvatars {
        item: String,
        bone: String,
    },
    RemoveItemFromAvatars(String),
    /// RollCredits with its credits file.
    RollCredits(String),
}

/// AnimatedLevelPart / PhysicsEntity.Highlight(time) requests.
#[derive(Clone, Debug, PartialEq)]
pub enum Highlight {
    Anim { path: Vec<i32>, time: f32 },
    Phys { id: i32, time: f32 },
}

impl<C2: SceneContent> SceneWorld<C2> {
    pub(super) fn campaign(&mut self, a: &CampaignAction, quick: bool) -> Result<(), WorldError> {
        match &a.command {
            // TutorialUtils telemetry goes to Steam only.
            C::Telemetry => {}
            C::FoundMoose(id) => self.profile_events.push(("moose".into(), id.clone())),
            C::KilledAPony(id) => self.profile_events.push(("pony".into(), id.clone())),
            C::ConditionalAchievement {
                achievement,
                victim,
            } => {
                let award = match achievement.as_str() {
                    "thisismagicka" | "therecanbeonlyone" => self.attacked_last_by_avatar(*victim),
                    "drivenmad" | "breezedthrough" | "handlingthefrustration" => {
                        !self.died_in_level_flag
                    }
                    other => {
                        return Err(format!("conditional achievement {other} not implemented"))
                    }
                };
                if award {
                    self.flags.achievements.insert(achievement.clone());
                }
            }
            C::SetVolume { id, volume } => {
                // ChangeAmbientSoundVolume: located cue's Volume variable,
                // else a global sound's stored volume.
                let key = if let Some((key, _)) = self.sounds.located.get(id) {
                    Some(*key)
                } else if let Some(g) = self.sounds.global.get_mut(id) {
                    g.3 = *volume;
                    Some(g.0)
                } else {
                    None
                };
                if let Some(key) = key {
                    self.audio_requests
                        .push(crate::audio::AudioRequest::Replay {
                            key,
                            volume_db: *volume,
                        });
                }
            }
            C::SetMusicFocus {
                locator,
                radius,
                clear_to,
            } => {
                let m = self.scene_locator(*locator)?;
                self.music_focus = Some(([m[12], m[13], m[14]], *radius, *clear_to));
            }
            C::CameraBias { bias, time } => self.camera.set_bias(*bias, *time),
            C::CameraFollow(id) => {
                if let Some(e) = self.by_unique_id(*id) {
                    let (handle, position) = (e.handle, e.position);
                    self.camera.follow(position);
                    self.camera_follow = Some(handle);
                }
            }
            C::CameraPosition {
                target,
                offset,
                magnification,
            } => {
                let base = self
                    .locator_info
                    .get(target)
                    .map(|(m, _)| [m[12], m[13], m[14]])
                    .unwrap_or_else(|| self.camera.ground_position());
                let p = std::array::from_fn(|i| base[i] + offset[i]);
                self.camera.set_position(p, true);
                if *magnification > 0.0 {
                    self.camera.set_magnification(*magnification);
                }
            }
            C::Flash(duration) => self.flash(*duration),
            C::HighlightAnim { path, time } => self.highlights.push(Highlight::Anim {
                path: path.clone(),
                time: *time,
            }),
            C::HighlightPhys { id, time } => self.highlights.push(Highlight::Phys {
                id: *id,
                time: *time,
            }),
            C::DisplayTitle {
                title,
                subtitle,
                display_time,
                fade_in,
                fade_out,
                no_translation,
                align,
            } => {
                // DisplayTitleInLevel.Execute, then Level.DisplayTitle, which
                // ignores a request while a title is displaying.
                let present = (!no_translation && (!title.is_empty() || !subtitle.is_empty()))
                    || (!title.is_empty() && !subtitle.is_empty());
                if present && self.level_title.is_none() {
                    let mut r = crate::title_renderer::TitleRenderer::default();
                    let display_time = if *display_time < 0.0 {
                        2.0
                    } else {
                        *display_time
                    };
                    r.set_titles(title, subtitle, display_time, *fade_in, *fade_out);
                    r.start();
                    let kind = crate::title_renderer::TitleKind::Level {
                        no_translation: *no_translation,
                        align: *align,
                    };
                    self.level_title = Some((r, kind));
                }
            }
            C::SetUi(enabled) => self.ui_enabled = *enabled,
            C::SpawnCloneWizards(node) => self.spawn_clone_wizards(&node.0)?,
            C::RollCredits(file) => self
                .campaign_requests
                .push(CampaignRequest::RollCredits(file.clone())),
            C::ScrollScore {
                value,
                area,
                offset,
                ttl,
            } => {
                let base = match self.random_area_point(*area) {
                    Some(p) => p,
                    None => crate::scene_command::SceneCommandHost::try_locator(self, *area)?
                        .map_or([0.0; 3], |m| [m[12], m[13], m[14]]),
                };
                let at = std::array::from_fn(|i| base[i] + offset[i]);
                self.floating_text
                    .add_colored(*value, at, *ttl, false, [1.0; 3]);
            }
            C::StaffLight(on) => {
                if let Some(on) = on {
                    self.flags.staff_light = *on;
                }
            }
            C::CameraEffect { effect, add } => {
                // RemoveCameraEffect with no effect removes them all.
                let effect = if *add || *effect != 0 { *effect } else { 0 };
                self.camera_effect_requests.push((effect, *add));
            }
            C::Overkill {
                id,
                type_hash,
                area,
                factions,
            } => {
                if let Some(id) = id {
                    if let Some(handle) = self.by_unique_id(*id).map(|e| e.handle) {
                        self.overkill(handle);
                    }
                    return Ok(());
                }
                let any = crate::campaign_action::any_id();
                let present =
                    crate::entity_command::EntityCommandHost::present_characters(self, *area)?
                        .ok_or_else(|| format!("overkill: no trigger area {area}"))?;
                for c in present {
                    if *type_hash == any
                        || c.type_id == *type_hash
                        || c.original_faction & factions != 0
                    {
                        self.overkill(c.handle);
                    }
                }
            }
            C::SetFaction {
                player,
                id,
                faction,
                area,
                operation,
                type_hash,
            } => {
                let apply = |current: i32| match operation {
                    Operation::Add => current | faction,
                    Operation::Remove => current & !faction,
                    Operation::Set => *faction,
                };
                let targets: Vec<u16> = if let Some(slot) = player {
                    self.players[*slot]
                        .as_ref()
                        .filter(|p| p.playing)
                        .map(|p| p.handle)
                        .into_iter()
                        .collect()
                } else if let Some(id) = id {
                    self.by_unique_id(*id)
                        .filter(|e| {
                            matches!(e.kind, EntityKind::Npc { .. } | EntityKind::Avatar { .. })
                        })
                        .map(|e| e.handle)
                        .into_iter()
                        .collect()
                } else {
                    let any = crate::campaign_action::any_id();
                    crate::entity_command::EntityCommandHost::present_characters(self, *area)?
                        .ok_or_else(|| format!("setfaction: no trigger area {area}"))?
                        .into_iter()
                        .filter(|c| *type_hash == any || c.type_id == *type_hash)
                        .map(|c| c.handle)
                        .collect()
                };
                for handle in targets {
                    if let Some(p) = self
                        .players
                        .iter_mut()
                        .flatten()
                        .find(|p| p.handle == handle)
                    {
                        p.faction = apply(p.faction);
                    }
                    if let Some(e) = self.entities.iter_mut().find(|e| e.handle == handle) {
                        e.faction = apply(e.faction);
                    }
                }
            }
            C::RemoveAllStatusEffects => self.remove_all_status_effects(),
            C::Teleport {
                id,
                target,
                effect,
                sound,
            } => self.teleport_script(*id, *target, *effect, *sound)?,
            C::Magick {
                magick,
                id,
                position,
            } => self.script_magick(*magick, *id, *position),
            C::UnequipWeaponsSection { begin, animation } => {
                self.campaign_requests
                    .push(CampaignRequest::UnequipWeaponsSection {
                        begin: *begin,
                        animation: *animation,
                    })
            }
            C::DefaultAvatarSection(on) => self
                .campaign_requests
                .push(CampaignRequest::DefaultAvatarSection(*on)),
            C::AssignItem {
                item,
                player,
                area,
                animation,
            } => self.campaign_requests.push(CampaignRequest::AssignItem {
                item: item.clone(),
                player: *player,
                area: *area,
                animation: *animation,
                quick,
            }),
            C::AddItemToAvatars { item, bone } => {
                self.campaign_requests
                    .push(CampaignRequest::AddItemToAvatars {
                        item: item.clone(),
                        bone: bone.clone(),
                    })
            }
            C::RemoveItemFromAvatars(item) => self
                .campaign_requests
                .push(CampaignRequest::RemoveItemFromAvatars(item.clone())),
        }
        Ok(())
    }

    /// ConditionalUnlockAchievement: the victim NPC's LastAttacker is a
    /// playing local avatar (NonPlayerCharacter.LastAttacker ≈ the most
    /// recent attacked_by entry).
    fn attacked_last_by_avatar(&self, victim: i32) -> bool {
        let Some(handle) = self.by_unique_id(victim).map(|e| e.handle) else {
            return false;
        };
        let Some(npc) = self.npcs.get(&handle) else {
            return false;
        };
        npc.combat.attacked_by.last().is_some_and(|(attacker, _)| {
            self.players
                .iter()
                .flatten()
                .any(|p| p.handle == *attacker && p.playing)
        })
    }

    /// RemoveAllStatusEffects: Nullify's per-character clearing without the
    /// charm end, summon kills, magick clearing or effects.
    fn remove_all_status_effects(&mut self) {
        use magicka_sim::status::Status as S;
        const STATUSES: u16 = S::Burning as u16
            | S::Wet as u16
            | S::Frozen as u16
            | S::Cold as u16
            | S::Poisoned as u16
            | S::Healing as u16
            | S::Greased as u16
            | S::Steamed as u16;
        for p in self.players.iter_mut().flatten() {
            let body = &mut self.physics.bodies[p.body];
            nullify_character(&mut p.character, STATUSES);
            p.stop_current_spell();
            p.conjure.queue.clear();
            stop_levitate(&mut p.character, body);
        }
        for npc in self.npcs.values_mut() {
            let body = &mut self.physics.bodies[npc.body];
            nullify_character(&mut npc.character, STATUSES);
            stop_levitate(&mut npc.character, body);
        }
    }

    /// Teleport action: appear effect at the entity and at the locator,
    /// ground probe 4 m down, lift by the capsule half height + radius + 0.1,
    /// MoveTo with the locator orientation, desired direction = Forward, cue.
    fn teleport_script(
        &mut self,
        id: i32,
        target: i32,
        effect: i32,
        sound: i32,
    ) -> Result<(), WorldError> {
        if id == 0 {
            return Ok(());
        }
        let effect = if effect == 0 {
            custom_hash("magick_teleport_appear")
        } else {
            effect
        };
        let sound = if sound == 0 {
            custom_hash("magick_teleportb")
        } else {
            sound
        };
        let Some(handle) = self.by_unique_id(id).map(|e| e.handle) else {
            return Ok(());
        };
        let Some(body) = self.character_body(handle) else {
            return Ok(());
        };
        let from = self.physics.bodies[body].position;
        let locator = self.scene_locator(target)?;
        let mut at = [locator[12], locator[13], locator[14]];
        for (instance_effect, position) in [(effect, from), (effect, at)] {
            self.effect_serial = self.effect_serial.wrapping_add(1);
            let mut transform = magicka_content::skinned::IDENTITY;
            transform[12..15].copy_from_slice(&position);
            self.effect_commands.push(SceneEffectCommand::Start {
                instance: self.effect_serial,
                effect: instance_effect,
                transform,
            });
        }
        if let Some(hit) = self.collision.collision().segment_intersect(
            Segment {
                origin: at,
                delta: [0.0, -4.0, 0.0],
            },
            false,
        ) {
            at = hit.hit.position;
        }
        let b = &mut self.physics.bodies[body];
        at[1] += b.radius + b.length * 0.5 + 0.1;
        let forward = [-locator[8], -locator[9], -locator[10]];
        let mut orientation = locator;
        orientation[12] = 0.0;
        orientation[13] = 0.0;
        orientation[14] = 0.0;
        let o = magicka_sim::character_physics::Orientation {
            right: [orientation[0], orientation[1], orientation[2]],
            up: [orientation[4], orientation[5], orientation[6]],
            backward: [orientation[8], orientation[9], orientation[10]],
        };
        b.move_to(at, o);
        b.set_desired_direction(forward);
        let key = self.backend().next_key();
        self.audio_requests.push(crate::audio::AudioRequest::Start {
            key,
            bank: crate::audio::banks::SPELLS,
            cue: sound,
            volume_db: 0.0,
            source: crate::audio::SoundSource::Emitter(at),
            magnitude: None,
        });
        Ok(())
    }

    /// Magick action: Effect.Execute(position) without an ID, otherwise
    /// Execute(character) for the entity with that unique ID.
    fn script_magick(&mut self, magick: u8, id: Option<i32>, position: [f32; 3]) {
        use magicka_sim::spells::MagickType as M;
        let Ok(kind) = M::try_from(magick) else {
            return;
        };
        let owner = id.and_then(|id| self.by_unique_id(id).map(|e| e.handle));
        let at = owner
            .and_then(|h| self.character_body(h))
            .map_or(position, |b| self.physics.bodies[b].position);
        match kind {
            M::Nullify => self.nullify(),
            M::Rain => {
                self.rain_from(owner);
            }
            M::Blizzard => {
                if !self.scene_indoors {
                    if self.blizzard_cue.is_none() {
                        self.blizzard_cue = Some(self.play_cue(16, "magick_blizzard", None));
                    }
                    self.ability_effects.blizzard(owner, self.play_time);
                }
            }
            M::MeteorS => {
                self.meteor_shower(owner, at);
            }
            M::Corporealize => self.corporealize(at),
            M::Etherealize => {
                if let Some(h) = owner {
                    self.etherealize(h);
                }
            }
            other => self
                .log
                .push(format!("script magick {other:?} not reconstructed")),
        }
    }

    /// CameraFollow keeps the camera on the followed entity's position.
    /// SpawnCloneWizards.Execute (not on clients): for each playing, living
    /// player i, an NPC of the avatar's template with mesh i at locator
    /// `{Area}{i}`, unique ID "clonewizard{i}", the player's colour and the
    /// avatar's MaxHitPoints; otherwise Spawn's order/AI-event setup, with
    /// the action's Animation as the special idle.
    fn spawn_clone_wizards(
        &mut self,
        node: &crate::scene_script::XmlElement,
    ) -> Result<(), WorldError> {
        if self.role == NetworkRole::Client {
            return Ok(());
        }
        let slots: Vec<(usize, String, f32)> = self
            .players
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                let p = p.as_ref()?;
                (p.playing && !p.dead()).then(|| {
                    (
                        i,
                        p.character.definition.id.clone(),
                        p.character.combat.health.max_hit_points,
                    )
                })
            })
            .collect();
        for (i, template, max_hit_points) in slots {
            let mut spawn = node.clone();
            spawn.name = "spawn".into();
            spawn.attributes = node
                .attributes
                .iter()
                .filter_map(|(k, v)| match k.to_ascii_lowercase().as_str() {
                    "area" => Some((k.clone(), format!("{v}{i}"))),
                    "animation" => Some(("idleanimation".into(), v.clone())),
                    // Spawn-only properties the clone action does not have.
                    "delay"
                    | "type"
                    | "id"
                    | "meshid"
                    | "nr"
                    | "health"
                    | "snaptonavmesh"
                    | "forcedraw"
                    | "forcecamera"
                    | "forcenavmesh"
                    | "cannotdiewithoutexplicitkill" => None,
                    _ => Some((k.clone(), v.clone())),
                })
                .chain([
                    ("type".into(), template),
                    ("id".into(), format!("clonewizard{i}")),
                    ("meshid".into(), i.to_string()),
                ])
                .collect();
            let spawn = crate::spawn_action::SpawnAction::read(&spawn)
                .map_err(|e| format!("spawnclonewizards: {e:?}"))?;
            let handle = self.spawn_generic_boss(&spawn)?;
            self.entity_mut(handle)?.hit_points = max_hit_points;
            if let Some(n) = self.npcs.get_mut(&handle) {
                n.character.combat.health.hit_points = max_hit_points;
            }
            self.clone_wizards.insert(handle, i);
        }
        Ok(())
    }

    /// Level.Update's title (mHasTitleDisplaying).
    pub(super) fn update_level_title(&mut self, dt: f32) {
        self.shown_level_title = None;
        let Some((renderer, kind)) = &mut self.level_title else {
            return;
        };
        match renderer.update(dt) {
            Some(alpha) => {
                self.shown_level_title = Some(crate::title_renderer::ShownTitle::of(
                    renderer, alpha, *kind,
                ))
            }
            None => self.level_title = None,
        }
    }

    pub(super) fn update_camera_follow(&mut self) {
        let Some(handle) = self.camera_follow else {
            return;
        };
        if !self.camera.is_following() {
            self.camera_follow = None;
            return;
        }
        match self
            .entities
            .iter()
            .find(|e| e.handle == handle && !e.removed)
        {
            Some(e) => self.camera.set_followed_position(Some(e.position)),
            None => self.camera_follow = None,
        }
    }
}
