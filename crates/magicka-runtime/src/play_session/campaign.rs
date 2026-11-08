//! Session side of campaign actions (research/campaign-actions.md): the
//! PlayState equipment sections, AssignItem and Add/RemoveItemFromAvatars.
//! The opening tutorial starts unarmed (wc_s1 Init) until the weapon rack
//! (wc_s3 OnInteract poi_weaponrack) ends the unequip section.
use super::*;
use crate::scene_world::campaign_live::CampaignRequest;

/// PlayState.mUnarmedWeapon.
const UNARMED: &str = "weapon_unarmed";

impl PlaySession {
    /// Credits.Initialize(file) (unless already initialized) and Start.
    fn roll_credits(&mut self, file: &str) -> Result<(), SessionError> {
        if self.credits.is_none() {
            let name = if file.is_empty() { "credits.xml" } else { file };
            let path =
                crate::play_content::resolve(&self.content, &Path::new("Data").join(name))
                    .ok_or_else(|| SessionError::Script(format!("credits file {name} missing")))?;
            let bytes = std::fs::read(&path)
                .map_err(|e| SessionError::Script(format!("{}: {e}", path.display())))?;
            let parsed = crate::credits::CreditsFile::parse(&bytes)
                .map_err(|e| SessionError::Script(format!("credits: {e:?}")))?;
            let strings = crate::play_content::english_strings(&self.content, &["Wizard"]);
            let content = self.content.clone();
            let credits = crate::credits::Credits::initialize(&parsed, &strings, |i| {
                let (_, path, spacing) = crate::credits::FONTS.get(i)?;
                let font = crate::play_content::bitmap_font(&content, path).ok()?;
                let mut m = crate::bitmap_font::FontMetrics::from_content(&font).ok()?;
                m.spacing = *spacing;
                Some(m)
            })
            .map_err(SessionError::Script)?;
            self.credits = Some(credits);
        }
        let height = self.screen[1];
        if let Some(c) = &mut self.credits {
            c.start(height);
        }
        Ok(())
    }

    /// PlayState.Update's Credits.Update; the fade-out begins
    /// Endgame(EndOffGame) and a 1 s fade to black, after which the credits
    /// stop and input is unlimited (OnTransitionEnd).
    pub(super) fn update_credits(&mut self, dt: f32) {
        let height = self.screen[1];
        let Some(c) = &mut self.credits else {
            return;
        };
        if c.fading_out && c.started {
            self.credits_fade = (self.credits_fade + dt).min(1.0);
            if self.credits_fade >= 1.0 {
                c.finish();
                self.world.flags.input_limited = false;
                self.credits_finished = true;
                // OnTransitionEnd pops the PlayState to SubMenuEndGame
                // (CreditsEnd closes the Tome to the main menu) before
                // EndOffGame's own Restart(Checkpoint) could run. Without
                // this the ew2_s2 ending replayed after the credits.
                self.end_game.active = true;
            }
            return;
        }
        if let Some(crate::credits::CreditsEvent::BeginFadeOut) = c.update(dt, height) {
            self.credits_fade = 0.0;
            self.end_game.begin(
                crate::end_game::EndGameCondition::EndOffGame,
                false,
                false,
                0.0,
            );
        }
    }

    /// Apply queued campaign requests in order.
    pub(super) fn drain_campaign_requests(&mut self) {
        for request in std::mem::take(&mut self.world.campaign_requests) {
            if let Err(error) = self.campaign_request(request) {
                self.world.log.push(format!("campaign: {error:?}"));
            }
        }
    }

    fn campaign_request(&mut self, request: CampaignRequest) -> Result<(), SessionError> {
        match request {
            CampaignRequest::RollCredits(file) => self.roll_credits(&file)?,
            CampaignRequest::UnequipWeaponsSection { begin, animation } => {
                // PlayState.UnequipWeaponsSection setter (no-op when equal).
                if self.unequip_weapons_section != begin {
                    self.unequip_weapons_section = begin;
                    if begin {
                        self.remove_all_weapons()?;
                    } else {
                        // Default-avatar and selected weapons are the same
                        // template defaults in the rewrite (one character).
                        self.change_all_weapons_to_template()?;
                    }
                }
                // EndUnequipWeaponsSection plays its animation regardless.
                if !begin && animation != 0 {
                    for p in self.world.players.iter_mut().flatten() {
                        p.motion.attack(animation, true);
                    }
                }
            }
            CampaignRequest::DefaultAvatarSection(on) => {
                // PlayState.DefaultAvatarSection setter: avatars change to the
                // profile's default avatar (the rewrite's single character),
                // then weapons follow the unequip section.
                if self.default_avatar_section != on {
                    self.default_avatar_section = on;
                    if self.unequip_weapons_section {
                        self.remove_all_weapons()?;
                    } else {
                        self.change_all_weapons_to_template()?;
                    }
                }
            }
            CampaignRequest::NoItems => self.remove_all_weapons()?,
            CampaignRequest::AssignItem {
                item,
                player,
                area,
                animation,
                quick,
            } => self.assign_item(&item, player, area, animation, quick)?,
            CampaignRequest::AddItemToAvatars { item, bone } => {
                // Equipment slots without an attach index get the item at
                // `bone`; the rewrite's avatars always use both slots.
                self.world
                    .log
                    .push(format!("additemtoavatars {item} at {bone}: no free slot"));
            }
            CampaignRequest::RemoveItemFromAvatars(item) => {
                // Equipment holding this item type reverts to the template.
                let template = self.template_item_names()?;
                for player in 0..self.world.players.len() {
                    let Some(avatar) = self.world.players[player].as_ref() else {
                        continue;
                    };
                    if !avatar.playing {
                        continue;
                    }
                    let hits: Vec<i32> = avatar
                        .items
                        .iter()
                        .filter(|(_, i)| i.definition.name.eq_ignore_ascii_case(&item))
                        .map(|(s, _)| *s)
                        .collect();
                    for slot in hits {
                        if let Some(name) = template.get(&slot) {
                            self.set_equipment(player, slot, name)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// PlayState.RemoveAllWeapons: both slots become weapon_unarmed.
    pub(super) fn remove_all_weapons(&mut self) -> Result<(), SessionError> {
        for player in 0..self.world.players.len() {
            if self.world.players[player].is_none() {
                continue;
            }
            self.set_equipment(player, 1, UNARMED)?;
            self.set_equipment(player, 0, UNARMED)?;
        }
        Ok(())
    }

    /// ChangeAllWeaponsToDefault / ToSelected: the template's equipment.
    fn change_all_weapons_to_template(&mut self) -> Result<(), SessionError> {
        let template = self.template_item_names()?;
        for player in 0..self.world.players.len() {
            if self.world.players[player].is_none() {
                continue;
            }
            for (slot, name) in &template {
                self.set_equipment(player, *slot, name)?;
            }
        }
        Ok(())
    }

    /// Template equipment item names by slot (Data/Characters-relative
    /// paths reduced to the item name).
    fn template_item_names(&self) -> Result<BTreeMap<i32, String>, SessionError> {
        let template =
            load_character(&self.content, &self.character).map_err(SessionError::Content)?;
        Ok(template
            .equipment
            .iter()
            .filter_map(|(slot, a)| {
                let path = a.item.as_deref()?;
                let name = path.rsplit(['/', '\\']).next()?.to_string();
                Some((*slot, name))
            })
            .collect())
    }

    /// AssignItem.Execute: staff_default / weapon_default restore the
    /// template's item, other items go to slot 1 (Staff class) or 0; with
    /// an animation (not on QuickExecute) the avatar plays it.
    fn assign_item(
        &mut self,
        item: &str,
        player: usize,
        area: i32,
        animation: u16,
        quick: bool,
    ) -> Result<(), SessionError> {
        let template = self.template_item_names()?;
        let targets: Vec<usize> = if area == crate::campaign_action::any_id() && player == 0 {
            (0..self.world.players.len())
                .filter(|&i| {
                    self.world.players[i]
                        .as_ref()
                        .is_some_and(|p| p.playing && !p.dead())
                })
                .collect()
        } else if player == 0 {
            // The first avatar present in the area.
            use crate::entity_command::EntityCommandHost;
            let present = EntityCommandHost::present_characters(&mut self.world, area)
                .map_err(SessionError::World)?
                .unwrap_or_default();
            present
                .iter()
                .find_map(|c| {
                    (0..self.world.players.len()).find(|&i| {
                        self.world.players[i]
                            .as_ref()
                            .is_some_and(|p| p.handle == c.handle && !p.dead())
                    })
                })
                .into_iter()
                .collect()
        } else {
            (player - 1..player)
                .filter(|&i| self.world.players[i].as_ref().is_some_and(|p| !p.dead()))
                .collect()
        };
        let (slot, name) = match item {
            "staff_default" => (1, template.get(&1).cloned()),
            "weapon_default" => (0, template.get(&0).cloned()),
            other => {
                let class = crate::play_content::item_weapon_class(&self.content, other)
                    .ok_or_else(|| SessionError::Template(format!("missing item {other}")))?;
                (
                    if class == crate::animation_controller::WEAPON_CLASS_STAFF {
                        1
                    } else {
                        0
                    },
                    Some(other.to_string()),
                )
            }
        };
        let Some(name) = name else {
            return Ok(());
        };
        for player in targets {
            self.set_equipment(player, slot, &name)?;
            if animation != 0 && !quick {
                if let Some(p) = self.world.players[player].as_mut() {
                    p.motion.attack(animation, true);
                }
            }
        }
        Ok(())
    }

    /// Item.GetCachedWeapon(name, avatar.Equipment[slot].Item): replace an
    /// equipped item by template name (bound items too), rebuilding the rig
    /// attachments and weapon animation classes and stopping that slot's
    /// item effects.
    pub(super) fn set_equipment(
        &mut self,
        player: usize,
        slot: i32,
        name: &str,
    ) -> Result<(), SessionError> {
        let character = self
            .remote_characters
            .get(&player)
            .unwrap_or(&self.character)
            .clone();
        let mut template =
            load_character(&self.content, &character).map_err(SessionError::Content)?;
        let avatar = self.player_avatar(player)?;
        if avatar
            .items
            .iter()
            .any(|(s, i)| *s == slot && i.definition.name.eq_ignore_ascii_case(name))
        {
            return Ok(());
        }
        for (s, attachment) in &mut template.equipment {
            let n = if *s == slot {
                name.to_string()
            } else if let Some((_, item)) = avatar.items.iter().find(|(i, _)| i == s) {
                item.definition.name.clone()
            } else {
                continue;
            };
            attachment.item = Some(format!("../items/Wizard/{n}"));
        }
        let mut items = crate::play_content::avatar_items(&self.content, &template);
        let incoming = items
            .iter()
            .position(|(s, _)| *s == slot)
            .map(|i| items.swap_remove(i).1)
            .ok_or_else(|| SessionError::Template(format!("missing item {name}")))?;
        let rig = crate::play_content::load_rig(&self.content, &template)
            .map_err(SessionError::Content)?;
        let animations = Arc::new(AnimationSet::with_weapons(
            &template.animation_sets,
            &crate::play_content::weapon_classes(rig.as_ref()),
            rig.as_ref()
                .map(|r| r.skeleton.clip_durations())
                .unwrap_or_default(),
        ));
        let avatar = self.world.players[player]
            .as_mut()
            .ok_or_else(|| SessionError::World("equipment player unavailable".into()))?;
        let owner = avatar.handle;
        match avatar.items.iter_mut().find(|(s, _)| *s == slot) {
            Some((_, held)) => {
                let attached = held.attached;
                *held = incoming;
                held.attached = attached;
            }
            None => avatar.items.push((slot, incoming)),
        }
        avatar.rig = rig;
        avatar.motion.animations = animations;
        let stopped: Vec<_> = self
            .item_effects
            .keys()
            .filter(|(h, s, _)| *h == owner && *s == slot)
            .copied()
            .collect();
        for key in stopped {
            if let Some(mut r) = self.item_effects.remove(&key) {
                self.effects.stop(&mut r);
            }
        }
        Ok(())
    }
}
