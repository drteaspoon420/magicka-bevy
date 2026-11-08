//! Disk restoration uses initialized content and the checkpoint startup replay.
use super::*;
use serde_json::{json, Value};

fn playback(states: &[crate::animated_pose::PartPlayback]) -> Value {
    json!(states
        .iter()
        .map(|s| (
            s.time,
            s.start,
            s.end,
            s.speed,
            s.looping,
            s.playing,
            s.resting_timer
        ))
        .collect::<Vec<_>>())
}
fn read_playback(v: &Value) -> Result<Vec<crate::animated_pose::PartPlayback>, String> {
    let rows: Vec<(f32, f32, f32, f32, bool, bool, f32)> =
        serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    if rows
        .iter()
        .any(|r| ![r.0, r.1, r.2, r.3, r.6].iter().all(|v| v.is_finite()))
    {
        return Err("non-finite animation state".into());
    }
    Ok(rows
        .into_iter()
        .map(
            |(time, start, end, speed, looping, playing, resting_timer)| {
                crate::animated_pose::PartPlayback {
                    time,
                    start,
                    end,
                    speed,
                    looping,
                    playing,
                    resting_timer,
                }
            },
        )
        .collect())
}
impl PlaySession {
    pub fn disk_save(&self) -> Result<Value, String> {
        if self.world.flags.cutscene || !self.world.tutorial.dialog_done() {
            return Err("save deferred until the cutscene or tutorial dialog ends".into());
        }
        let dialogs = self
            .world
            .dialogs
            .as_ref()
            .map(|d| d.disk_boundary())
            .transpose()?;
        if self.world.role == magicka_sim::damage_delivery::NetworkRole::Client {
            return Err("campaign saves are owned by the host".into());
        }
        // Four entries as before; larger parties append further slots.
        let mut matrices = vec![magicka_content::skinned::IDENTITY; 4];
        let mut players = Vec::new();
        for (slot, p) in self
            .world
            .players
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.as_ref().filter(|p| p.playing).map(|p| (i, p)))
        {
            // Slot 0 always has a profile; anonymous network joiners (no
            // profile ID) are not part of the campaign save.
            let Some(id) = self.player_ids[slot]
                .as_ref()
                .filter(|id| crate::saves::valid_player_id(id))
            else {
                if slot == 0 {
                    return Err("playing player has no persistent profile ID".into());
                }
                continue;
            };
            let body = &self.world.physics.bodies[p.body];
            let o = body.orientation;
            if matrices.len() <= slot {
                matrices.resize(slot + 1, magicka_content::skinned::IDENTITY);
            }
            matrices[slot] = [
                o.right[0],
                o.right[1],
                o.right[2],
                0.,
                o.up[0],
                o.up[1],
                o.up[2],
                0.,
                o.backward[0],
                o.backward[1],
                o.backward[2],
                0.,
                body.position[0],
                body.position[1] - body.radius - 0.5 * body.length,
                body.position[2],
                1.,
            ];
            let robe = if slot == 0 {
                &self.character
            } else {
                self.remote_characters
                    .get(&slot)
                    .ok_or("missing player robe")?
            };
            players.push(json!({"slot":slot,"id":id,"robe":robe,"equipment":p.items.iter().map(|(slot,item)|(*slot,item.definition.name.clone())).collect::<Vec<_>>(),"magicks":self.unlocked_magicks[slot]}));
        }
        // Parked players keep their rows; their slot numbers follow the
        // playing ones so they never collide.
        let mut next_slot = players
            .iter()
            .filter_map(|p| p["slot"].as_u64())
            .max()
            .map_or(1, |s| s + 1);
        for row in &self.parked_players {
            if players.iter().any(|p| p["id"] == row["id"]) {
                continue;
            }
            let mut row = row.clone();
            row["slot"] = json!(next_slot);
            row["parked"] = json!(true);
            next_slot += 1;
            players.push(row);
        }
        let cp=self.checkpoint.as_ref().map(|cp|json!({"scene":cp.scene,"matrices":cp.matrices,"ignored":cp.ignored,"scripts":cp.runtime.disk_state(),"animations":playback(&cp.animations),"players":cp.players,"dialogs":cp.dialogs,"physics_version":1,"physics":physics(&cp.physics),"pickables":pickables(&cp.pickables),"camera":camera(&cp.camera),"music":cp.music,"music_focus":cp.music_focus_value,"unequipped":cp.unequip_weapons_section,"default_avatar":cp.default_avatar_section}));
        Ok(
            json!({"version":crate::saves::VERSION,"level":self.level_file,"scene":self.scene_name,"players":players,"matrices":matrices,"scripts":self.runtime.disk_state(),"animations":playback(&self.world.collision.playback_states()),"checkpoint":cp,"unequipped":self.unequip_weapons_section,"default_avatar":self.default_avatar_section,"staff_light":self.world.flags.staff_light,"flags":flags(&self.world.flags),"ui_enabled":self.world.ui_enabled,"play_time":self.play_time,"dialogs":dialogs,"physics_version":1,"physics":physics(&self.world.checkpoint_physics()),"pickables":pickables(&self.world.checkpoint_pickables()),"camera":camera(&self.world.camera.state()),"died":self.world.died_in_level_flag,"mods":self.mod_state}),
        )
    }
    /// Load a save for the profile `id` wearing `robe` (rewrite extension).
    /// The profile takes slot 0 with its saved equipment and magicks. Every
    /// other saved player is parked: kept verbatim, written back on save and
    /// restored by [`PlaySession::claim_parked`] when that profile joins. A
    /// profile new to the save starts fresh, with its robe's equipment and
    /// the magicks the saved party had learned, and the previous owner's
    /// player is parked rather than lost.
    pub fn load_disk_save_for(
        content: &Path,
        v: &Value,
        id: &str,
        robe: &str,
    ) -> Result<Self, String> {
        if !crate::saves::valid_player_id(id) {
            return Err("invalid profile ID".into());
        }
        let (prepared, parked) = assign_save_players(v, id, robe)?;
        let mut session = Self::load_disk_save(content, &prepared)?;
        session.parked_players.extend(parked);
        Ok(session)
    }

    /// The playing slot currently using this persistent profile.
    pub fn profile_slot(&self, id: &str) -> Option<usize> {
        self.world.players.iter().enumerate().find_map(|(slot, p)| {
            (p.as_ref().is_some_and(|p| p.playing) && self.player_ids[slot].as_deref() == Some(id))
                .then_some(slot)
        })
    }

    /// A joining profile reclaims its parked player: saved equipment (worn
    /// with the robe the profile now chose) and unlocked magicks. Returns
    /// whether a parked player existed. Joiners always get the profile ID
    /// so later saves include them.
    pub fn claim_parked(
        &mut self,
        slot: usize,
        id: &str,
        robe: &str,
    ) -> Result<bool, SessionError> {
        if !crate::saves::valid_player_id(id) {
            return Ok(false);
        }
        self.player_avatar(slot)?;
        if self.profile_slot(id).is_some_and(|owner| owner != slot) {
            return Err(SessionError::World(
                "that profile is already playing".into(),
            ));
        }
        let Some(index) = self.parked_players.iter().position(|row| row["id"] == id) else {
            self.player_ids[slot] = Some(id.to_owned());
            return Ok(false);
        };
        // Keep the saved row and current owner until all required content loads.
        let row = self.parked_players[index].clone();
        if row["fresh"] == true {
            self.reset_player_equipment(slot, robe)?;
        } else {
            let equipment: Vec<(i32, String)> = serde_json::from_value(row["equipment"].clone())
                .map_err(|e| SessionError::World(format!("parked equipment: {e}")))?;
            let item = |s: i32| {
                equipment
                    .iter()
                    .find(|(k, _)| *k == s)
                    .map(|(_, n)| n.clone())
            };
            let (Some(staff), Some(weapon)) = (item(1), item(0)) else {
                return Err(SessionError::World("parked equipment is incomplete".into()));
            };
            self.set_player_loadout(slot, robe, &staff, &weapon)?;
        }
        self.parked_players.remove(index);
        self.player_ids[slot] = Some(id.to_owned());
        self.unlocked_magicks[slot] |= row["magicks"].as_u64().unwrap_or(0);
        Ok(true)
    }

    /// Keep a departing profile's player so a later save, or a rejoin,
    /// still has it.
    pub fn park_player(&mut self, slot: usize) {
        let Some(id) = self.player_ids[slot].clone() else {
            return;
        };
        let Some(p) = self.world.players.get(slot).and_then(Option::as_ref) else {
            return;
        };
        let robe = if slot == 0 {
            self.character.clone()
        } else {
            match self.remote_characters.get(&slot) {
                Some(r) => r.clone(),
                None => return,
            }
        };
        let row = json!({"id":id,"robe":robe,"equipment":p.items.iter().map(|(slot,item)|(*slot,item.definition.name.clone())).collect::<Vec<_>>(),"magicks":self.unlocked_magicks[slot]});
        self.parked_players.retain(|r| r["id"] != row["id"]);
        self.parked_players.push(row);
        self.player_ids[slot] = None;
    }

    /// A profile's selected robe overrides the campaign's previous robe.
    /// Campaign equipment, magicks and checkpoint progress still belong to the
    /// save. Update both player snapshots so death does not revert the choice.
    /// This operates on a copy; a failed load never changes the stored save.
    pub fn load_disk_save_with_robe(content: &Path, v: &Value, robe: &str) -> Result<Self, String> {
        let mut selected = v.clone();
        fn select_player(state: &mut Value, robe: &str) {
            if let Some(rows) = state.get_mut("players").and_then(Value::as_array_mut) {
                for row in rows.iter_mut().filter(|row| row["slot"] == 0) {
                    row["robe"] = json!(robe);
                }
            }
        }
        select_player(&mut selected, robe);
        if let Some(checkpoint) = selected.get_mut("checkpoint") {
            select_player(checkpoint, robe);
        }
        Self::load_disk_save(content, &selected)
    }

    pub fn load_disk_save(content: &Path, v: &Value) -> Result<Self, String> {
        if v["version"] != crate::saves::VERSION {
            return Err("unsupported campaign save version".into());
        }
        let text = |key: &str| v[key].as_str().ok_or_else(|| format!("missing save {key}"));
        let level = text("level")?;
        let scene = text("scene")?;
        let rows = v["players"].as_array().ok_or("missing saved players")?;
        let first = rows
            .iter()
            .find(|p| p["slot"] == 0)
            .ok_or("save has no primary player")?;
        let robe = first["robe"].as_str().ok_or("missing saved robe")?;
        let all: Vec<[f32; 16]> =
            serde_json::from_value(v["matrices"].clone()).map_err(|e| e.to_string())?;
        if all.len() < 4
            || all.len() > magicka_network::lobby::PLAYER_SLOT_CAP
            || all.iter().flatten().any(|v| !v.is_finite())
        {
            return Err("invalid saved placement".into());
        }
        let matrices: [[f32; 16]; 4] = std::array::from_fn(|i| all[i]);
        let mut next = Self::load_at(
            content,
            level,
            robe,
            Some(custom_hash(&scene.to_ascii_lowercase())),
            Some(matrices),
        )
        .map_err(|e| format!("save scene: {e:?}"))?;
        next.runtime.restore_disk_state(&v["scripts"])?;
        let physics = next.read_disk_physics(v)?;
        let f = &v["flags"];
        let unequip_weapons_section = v["unequipped"]
            .as_bool()
            .ok_or("missing equipment section")?;
        let default_avatar_section = v["default_avatar"]
            .as_bool()
            .ok_or("missing avatar section")?;
        let cp = Checkpoint {
            scene: custom_hash(&scene.to_ascii_lowercase()),
            matrices,
            ignored: vec![],
            runtime: next.runtime.clone(),
            animations: read_playback(&v["animations"])?,
            players: v["players"].clone(),
            dialogs: (!v["dialogs"].is_null()).then(|| v["dialogs"].clone()),
            physics,
            // Inactive scenes are not represented in the current disk schema.
            saved_scenes: BTreeMap::new(),
            // Older saves (no camera/pickables) keep the fresh scene's camera.
            camera: read_camera(&v["camera"], &next.world.camera.state())?,
            music: read_music(&f["music"])?,
            music_focus_value: read_focus(&f["music_focus_value"])?,
            pickables: read_pickables(&v["pickables"])?,
            unequip_weapons_section,
            default_avatar_section,
        };
        next.checkpoint = Some(cp);
        next.restart_at_checkpoint()
            .map_err(|e| format!("save replay: {e:?}"))?;
        if next
            .world
            .log
            .iter()
            .any(|s| s.starts_with("checkpoint replay") || s.starts_with("checkpoint animations"))
        {
            return Err("saved scene replay failed".into());
        }
        if let Some(dialogs) = next.world.dialogs.as_mut() {
            dialogs.restore_disk_boundary(&v["dialogs"])?;
        } else if !v["dialogs"].is_null() {
            return Err("saved dialog host missing".into());
        }
        let mut used = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for row in rows {
            if row["parked"] == true {
                let mut parked = row.clone();
                if let Some(o) = parked.as_object_mut() {
                    o.remove("parked");
                    o.remove("slot");
                }
                if parked["id"]
                    .as_str()
                    .is_some_and(crate::saves::valid_player_id)
                {
                    next.parked_players.push(parked);
                }
                continue;
            }
            let slot = row["slot"]
                .as_u64()
                .filter(|s| (*s as usize) < magicka_network::lobby::PLAYER_SLOT_CAP)
                .ok_or("invalid saved player slot")? as usize;
            let id = row["id"]
                .as_str()
                .filter(|id| crate::saves::valid_player_id(id))
                .ok_or("invalid player identity")?;
            if !used.insert(slot) || !ids.insert(id.to_owned()) {
                return Err("duplicate saved player".into());
            }
            let robe = row["robe"].as_str().ok_or("missing robe")?;
            if slot != 0 && next.world.players[slot].is_none() {
                let (mut body, make) = Self::prepare_player(content, robe, slot)
                    .map_err(|e| format!("saved player: {e:?}"))?;
                let (feet, o) = locator_parts(&super::slot_matrix(&all, slot));
                let mut p = feet;
                p[1] += body.radius + 0.5 * body.length;
                body.move_to(p, o);
                next.world.add_player(slot, body, make);
                next.remote_characters.insert(slot, robe.into());
            }
            if row["fresh"] == true {
                // A profile new to this save starts with its robe's defaults.
                next.reset_player_equipment(slot, robe)
                    .map_err(|e| format!("fresh equipment: {e:?}"))?;
            } else {
                let equipment: Vec<(i32, String)> =
                    serde_json::from_value(row["equipment"].clone()).map_err(|e| e.to_string())?;
                let name = |slot| {
                    equipment
                        .iter()
                        .find(|(s, _)| *s == slot)
                        .map(|(_, n)| n.as_str())
                        .ok_or("missing saved equipment")
                };
                next.set_player_loadout(slot, robe, name(1)?, name(0)?)
                    .map_err(|e| format!("saved equipment: {e:?}"))?;
            }
            let index = next.world.players[slot].as_ref().unwrap().body;
            let body = &mut next.world.physics.bodies[index];
            let (mut position, orientation) = locator_parts(&super::slot_matrix(&all, slot));
            position[1] += body.radius + 0.5 * body.length;
            body.move_to(position, orientation);
            next.player_ids[slot] = Some(id.into());
            next.unlocked_magicks[slot] = row["magicks"].as_u64().ok_or("missing magicks")?;
        }
        next.unequip_weapons_section = v["unequipped"]
            .as_bool()
            .ok_or("missing equipment section")?;
        next.default_avatar_section = v["default_avatar"]
            .as_bool()
            .ok_or("missing avatar section")?;
        next.world.flags.staff_light = v["staff_light"]
            .as_bool()
            .ok_or("missing staff-light flag")?;
        next.world.died_in_level_flag = v["died"].as_bool().ok_or("missing death flag")?;
        let f = &v["flags"];
        macro_rules! read_flag {
            ($key:literal) => {
                serde_json::from_value(f[$key].clone())
                    .map_err(|e| format!("saved flag {}: {e}", $key))?
            };
        }
        next.world.flags.achievements = read_flag!("achievements");
        next.world.flags.lights_disabled = read_flag!("lights_disabled");
        next.world.flags.interactables_disabled = read_flag!("interactables_disabled");
        next.world.flags.tutorial_elements_disabled = read_flag!("tutorial_elements_disabled");
        next.world.flags.tutorial_cast_types_disabled = read_flag!("tutorial_cast_types_disabled");
        next.world.flags.tutorial_push_disabled = read_flag!("tutorial_push_disabled");
        next.world.flags.input_limited = read_flag!("input_limited");
        next.world.flags.music = read_flag!("music");
        next.world.flags.music_focus_value = read_focus(&f["music_focus_value"])?;
        next.world.ui_enabled = v["ui_enabled"].as_bool().ok_or("missing UI flag")?;
        next.play_time = v["play_time"]
            .as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .ok_or("invalid save time")?;
        next.checkpoint = if v["checkpoint"].is_null() {
            None
        } else {
            let cp = &v["checkpoint"];
            let scene = cp["scene"]
                .as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .ok_or("invalid checkpoint scene")?;
            let matrices =
                serde_json::from_value(cp["matrices"].clone()).map_err(|e| e.to_string())?;
            let mut original = Self::load_at(content, level, robe, Some(scene), Some(matrices))
                .map_err(|e| format!("checkpoint content: {e:?}"))?;
            original.runtime.restore_disk_state(&cp["scripts"])?;
            let physics = original.read_disk_physics(cp)?;
            let section = |key: &str, current: bool| -> Result<bool, String> {
                match &cp[key] {
                    Value::Null => Ok(current),
                    Value::Bool(b) => Ok(*b),
                    _ => Err(format!("invalid checkpoint {key} section")),
                }
            };
            Some(Checkpoint {
                scene,
                matrices,
                ignored: serde_json::from_value(cp["ignored"].clone())
                    .map_err(|e| e.to_string())?,
                runtime: original.runtime,
                animations: read_playback(&cp["animations"])?,
                players: cp["players"].clone(),
                dialogs: (!cp["dialogs"].is_null()).then(|| cp["dialogs"].clone()),
                physics,
                saved_scenes: BTreeMap::new(),
                camera: read_camera(&cp["camera"], &original.world.camera.state())?,
                music: read_music(&cp["music"])?,
                music_focus_value: read_focus(&cp["music_focus"])?,
                pickables: read_pickables(&cp["pickables"])?,
                unequip_weapons_section: section("unequipped", next.unequip_weapons_section)?,
                default_avatar_section: section("default_avatar", next.default_avatar_section)?,
            })
        };
        // Rewrite extension: mod state travels with the save.
        next.mod_state = Some(v["mods"].clone()).filter(|m| !m.is_null());
        Ok(next)
    }

    /// Old rewrite saves could resave missing props as an empty snapshot while
    /// retaining positive SpawnPhys health in their checkpoint. Reconcile only
    /// unversioned lists with that existing, narrowly scoped legacy recovery.
    /// Versioned lists, including empty ones, are authoritative.
    fn read_disk_physics(
        &mut self,
        boundary: &Value,
    ) -> Result<Vec<crate::scene_world::saved_entities::PhysicsSnapshot>, String> {
        if let Some(version) = boundary.get("physics_version") {
            if version != 1 {
                return Err("unsupported saved physics version".into());
            }
            if !boundary["physics"].is_array() {
                return Err("missing versioned physics list".into());
            }
            return read_physics(&boundary["physics"]);
        }
        let mut states = read_physics(&boundary["physics"])?;
        for recovered in self.world.legacy_checkpoint_physics(&self.runtime) {
            if !states
                .iter()
                .any(|s| s.unique_id() == recovered.unique_id())
            {
                states.push(recovered);
            }
        }
        Ok(states)
    }
}

/// Rewrite a save so `id` is the slot-0 player and every other saved player
/// is parked (see [`PlaySession::load_disk_save_for`]).
fn assign_save_players(v: &Value, id: &str, robe: &str) -> Result<(Value, Vec<Value>), String> {
    let rows = v["players"].as_array().ok_or("missing saved players")?;
    let union = rows
        .iter()
        .filter_map(|r| r["magicks"].as_u64())
        .fold(0, |a, m| a | m);
    let own = rows.iter().find(|r| r["id"] == id).cloned();
    // A parked row has no placement or checkpoint row of its own: the
    // returning profile stands where slot 0 stood.
    let own_slot = own
        .as_ref()
        .filter(|r| r["parked"] != true)
        .and_then(|r| r["slot"].as_u64());
    let mut parked = Vec::new();
    for row in rows {
        if row["id"] == id {
            continue;
        }
        let mut row = row.clone();
        if let Some(o) = row.as_object_mut() {
            o.remove("parked");
            o.remove("slot");
        }
        if row["id"]
            .as_str()
            .is_some_and(crate::saves::valid_player_id)
        {
            parked.push(row);
        }
    }
    let primary = match own {
        Some(mut row) => {
            if let Some(o) = row.as_object_mut() {
                o.remove("parked");
            }
            row["slot"] = json!(0);
            row["robe"] = json!(robe);
            row
        }
        None => json!({"slot":0,"id":id,"robe":robe,"equipment":[],"magicks":union,"fresh":true}),
    };
    let reslot = |state: &mut Value, rows: Vec<Value>| -> Result<(), String> {
        let from = own_slot.unwrap_or(0) as usize;
        if let Some(m) = state.get_mut("matrices").and_then(Value::as_array_mut) {
            if let Some(matrix) = m.get(from).cloned() {
                m[0] = matrix;
            }
            m.truncate(4);
        }
        state["players"] = Value::Array(rows);
        Ok(())
    };
    let mut prepared = v.clone();
    reslot(&mut prepared, vec![primary.clone()])?;
    if prepared.get("checkpoint").is_some_and(|c| !c.is_null()) {
        let cp_rows = v["checkpoint"]["players"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let cp_primary = match own_slot {
            Some(s) => cp_rows
                .into_iter()
                .find(|r| r["slot"].as_u64() == Some(s))
                .map(|mut r| {
                    r["slot"] = json!(0);
                    r["robe"] = json!(robe);
                    r
                })
                .unwrap_or_else(|| primary.clone()),
            None => primary.clone(),
        };
        reslot(&mut prepared["checkpoint"], vec![cp_primary])?;
    }
    Ok((prepared, parked))
}

fn flags(f: &crate::scene_world::SceneFlags) -> Value {
    json!({"achievements":f.achievements,"lights_disabled":f.lights_disabled,"interactables_disabled":f.interactables_disabled,"tutorial_elements_disabled":f.tutorial_elements_disabled,"tutorial_cast_types_disabled":f.tutorial_cast_types_disabled,"tutorial_push_disabled":f.tutorial_push_disabled,"input_limited":f.input_limited,"music":f.music,"music_focus_value":f.music_focus_value})
}

impl PlaySession {
    pub(super) fn checkpoint_players(&self) -> Value {
        json!(self.world.players.iter().enumerate().filter_map(|(slot,p)| {
            let p=p.as_ref().filter(|p|p.playing)?;
            let robe=if slot==0 {self.character.as_str()}else{self.remote_characters.get(&slot)?.as_str()};
            Some(json!({"slot":slot,"id":self.player_ids[slot],"robe":robe,"equipment":p.items.iter().map(|(slot,item)|(*slot,item.definition.name.clone())).collect::<Vec<_>>(),"magicks":self.unlocked_magicks[slot],"fairy":self.world.spawn_fairies[slot]}))
        }).collect::<Vec<_>>())
    }
    pub(super) fn apply_checkpoint_players(&mut self, v: &Value) -> Result<(), SessionError> {
        let error = |s: &str| SessionError::World(format!("checkpoint players: {s}"));
        let rows = v.as_array().ok_or_else(|| error("missing snapshot"))?;
        for row in rows {
            if row["parked"] == true {
                continue;
            }
            let saved_slot = row["slot"]
                .as_u64()
                .filter(|n| (*n as usize) < magicka_network::lobby::PLAYER_SLOT_CAP)
                .ok_or_else(|| error("invalid slot"))? as usize;
            // A checkpoint does not own the current party roster. Match named
            // profiles even after they move slots; never spawn a departed player
            // or apply their inventory to a new owner of the same slot.
            let slot = if let Some(id) = row["id"].as_str() {
                self.world.players.iter().enumerate().find_map(|(slot, p)| {
                    (p.as_ref().is_some_and(|p| p.playing)
                        && self.player_ids[slot].as_deref() == Some(id))
                    .then_some(slot)
                })
            } else {
                // Legacy snapshots have only slot identity. An explicitly
                // anonymous snapshot cannot supply a named joiner's inventory.
                self.world
                    .players
                    .get(saved_slot)
                    .and_then(Option::as_ref)
                    .filter(|p| {
                        p.playing
                            && (row.get("id").is_none() || self.player_ids[saved_slot].is_none())
                    })
                    .map(|_| saved_slot)
            };
            let Some(slot) = slot else { continue };
            // State.mSpawnFairies (absent in older snapshots: unchanged).
            if let Some(fairy) = row["fairy"].as_bool() {
                self.world.spawn_fairies[slot] = fairy;
            }
            let robe = self
                .player_robe(slot)
                .ok_or_else(|| error("missing robe"))?
                .to_owned();
            if row["fresh"] == true {
                self.reset_player_equipment(slot, &robe)?;
                self.unlocked_magicks[slot] = row["magicks"].as_u64().unwrap_or(0);
                continue;
            }
            let equipment: Vec<(i32, String)> =
                serde_json::from_value(row["equipment"].clone()).map_err(|_| error("equipment"))?;
            let name = |slot| {
                equipment
                    .iter()
                    .find(|(s, _)| *s == slot)
                    .map(|(_, n)| n.as_str())
                    .ok_or_else(|| error("missing item"))
            };
            self.set_player_loadout(slot, &robe, name(1)?, name(0)?)?;
            self.unlocked_magicks[slot] = row["magicks"]
                .as_u64()
                .ok_or_else(|| error("missing magicks"))?;
        }
        Ok(())
    }
}

fn read_physics(
    v: &Value,
) -> Result<Vec<crate::scene_world::saved_entities::PhysicsSnapshot>, String> {
    if v.is_null() {
        return Ok(Vec::new());
    }
    v.as_array()
        .ok_or("invalid saved physics list")?
        .iter()
        .map(crate::scene_world::saved_entities::PhysicsSnapshot::from_disk)
        .collect()
}

fn physics(states: &[crate::scene_world::saved_entities::PhysicsSnapshot]) -> Value {
    Value::Array(states.iter().map(|s| s.disk_state()).collect())
}

fn pickables(states: &[crate::scene_world::saved_entities::PickableSnapshot]) -> Value {
    Value::Array(states.iter().map(|s| s.disk_state()).collect())
}

/// Absent in older saves: no permanent pickables were recorded.
fn read_pickables(
    v: &Value,
) -> Result<Vec<crate::scene_world::saved_entities::PickableSnapshot>, String> {
    if v.is_null() {
        return Ok(Vec::new());
    }
    v.as_array()
        .ok_or("invalid saved pickable list")?
        .iter()
        .map(crate::scene_world::saved_entities::PickableSnapshot::from_disk)
        .collect()
}

/// MagickCamera.State.Write: magnification, target magnification, time,
/// behaviour byte, target and current positions (box and entity dropped).
fn camera(state: &crate::camera::CameraState) -> Value {
    let s = state.persisted();
    json!({"magnification":s.magnification,"target_magnification":s.target_magnification,
        "time":s.time,"behaviour":s.behaviour.to_byte(),"target":s.target,"current":s.current})
}

/// Absent in older saves: the freshly built scene's camera (`fallback`).
fn read_camera(
    v: &Value,
    fallback: &crate::camera::CameraState,
) -> Result<crate::camera::CameraState, String> {
    if v.is_null() {
        return Ok(fallback.clone());
    }
    let number = |key: &str| -> Result<f32, String> {
        v[key]
            .as_f64()
            .map(|n| n as f32)
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("invalid saved camera {key}"))
    };
    let vector = |key: &str| -> Result<[f32; 3], String> {
        let p: [f32; 3] = serde_json::from_value(v[key].clone())
            .map_err(|e| format!("saved camera {key}: {e}"))?;
        if p.iter().any(|n| !n.is_finite()) {
            return Err(format!("invalid saved camera {key}"));
        }
        Ok(p)
    };
    Ok(crate::camera::CameraState {
        magnification: number("magnification")?,
        target_magnification: number("target_magnification")?,
        time: number("time")?,
        behaviour: v["behaviour"]
            .as_u64()
            .and_then(|b| u8::try_from(b).ok())
            .and_then(crate::camera::CameraBehaviour::from_byte)
            .ok_or("invalid saved camera behaviour")?,
        following: None,
        target: vector("target")?,
        current: vector("current")?,
        camera_box: None,
    })
}

fn read_music(v: &Value) -> Result<Option<(u16, i32)>, String> {
    serde_json::from_value(v.clone()).map_err(|e| format!("saved music: {e}"))
}

fn read_focus(v: &Value) -> Result<Option<f32>, String> {
    let focus: Option<f32> =
        serde_json::from_value(v.clone()).map_err(|e| format!("saved music focus: {e}"))?;
    if focus.is_some_and(|f| !f.is_finite()) {
        return Err("invalid saved music focus".into());
    }
    Ok(focus)
}
