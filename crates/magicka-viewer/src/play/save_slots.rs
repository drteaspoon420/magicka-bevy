use super::*;
use magicka_runtime::saves::{new_player_id, valid_player_id, SaveStore};

#[derive(Resource)]
pub(super) struct SaveBinding {
    store: SaveStore,
    slot: u8,
    last: (u32, u64),
    dirty: bool,
    last_error: Option<String>,
    /// After a failed write, the earliest real time to try again. A save
    /// serialises the whole world, so failures must not retry every frame.
    retry_at: f64,
}
#[derive(Resource)]
pub(super) struct UserVolumes {
    pub music: f32,
    pub sfx: f32,
}
fn slot(args: &[String]) -> Result<Option<u8>, String> {
    arg(args, "--save-slot")
        .map(|s| s.parse::<u8>().map_err(|e| format!("save slot: {e}")))
        .transpose()
}
pub(super) fn restore(
    session: &mut PlaySession,
    args: &[String],
    cache: &std::path::Path,
    content: &std::path::Path,
) -> Result<(), String> {
    let Some(slot) = slot(args)? else {
        return Ok(());
    };
    let store = SaveStore::new(cache.join("saves"));
    if !args.iter().any(|a| a == "--new-campaign") {
        if let Some(state) = store.load_slot(slot)? {
            // Any profile can play the save; absent profiles' players are
            // parked in it until they return (load_disk_save_for).
            *session = match (arg(args, "--profile-id"), arg(args, "--character")) {
                (Some(id), Some(robe)) => {
                    PlaySession::load_disk_save_for(content, &state, &id, &robe)?
                }
                (None, Some(robe)) => {
                    PlaySession::load_disk_save_with_robe(content, &state, &robe)?
                }
                _ => PlaySession::load_disk_save(content, &state)?,
            };
        }
    }
    Ok(())
}
pub(super) fn prepare(
    session: &mut PlaySession,
    args: &[String],
    cache: &std::path::Path,
) -> Result<Option<SaveBinding>, String> {
    let Some(slot) = slot(args)? else {
        return Ok(None);
    };
    let store = SaveStore::new(cache.join("saves"));
    store.slot_path(slot)?;
    if session.player_ids[0].is_none() {
        let id = arg(args, "--profile-id").map_or_else(new_player_id, Ok)?;
        if !valid_player_id(&id) {
            return Err("invalid profile ID".into());
        }
        session.player_ids[0] = Some(id);
    }
    // Capture the initialized level before startup dialogs/hints run. This
    // also gives a new slot a usable restart point during the opening cutscene.
    if args.iter().any(|arg| arg == "--new-campaign") || store.load_slot(slot)?.is_none() {
        store.save_slot(slot, &session.disk_save()?)?;
    }
    Ok(Some(SaveBinding {
        store,
        slot,
        last: (session.scene_changes, session.checkpoint_generation),
        dirty: false,
        last_error: None,
        retry_at: 0.0,
    }))
}
pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Last,
        autosave
            .run_if(resource_exists::<SaveBinding>)
            .in_set(PlaySystems)
            .before(lifecycle::SessionEnd),
    );
}
/// The graphics page's options and the spell wheel, from settings.json.
fn quality_settings(
    settings: &serde_json::Value,
) -> magicka_runtime::global_settings::QualitySettings {
    use magicka_runtime::global_settings::{Quality, QualitySettings};
    let defaults = QualitySettings::default();
    let graphics = &settings["graphics"];
    let quality = |key: &str, default| match graphics[key].as_str() {
        Some("low") => Quality::Low,
        Some("medium") => Quality::Medium,
        Some("high") => Quality::High,
        _ => default,
    };
    QualitySettings {
        shadows: quality("shadows", defaults.shadows),
        decals: quality("decals", defaults.decals),
        particles: quality("particles", defaults.particles),
        particle_lights: graphics["particle_lights"].as_str() != Some("off"),
        spell_wheel: settings["game"]["spell_wheel"].as_str() != Some("off"),
    }
}

/// Before a session loads: decal and particle pools are sized from these as
/// PlayState's constructor does.
pub(super) fn apply_quality(cache: &std::path::Path) -> Result<(), String> {
    let settings = SaveStore::new(cache.join("saves")).settings()?;
    magicka_runtime::global_settings::set_quality(quality_settings(&settings));
    Ok(())
}

pub(super) fn volumes(cache: &std::path::Path) -> Result<UserVolumes, String> {
    let settings = SaveStore::new(cache.join("saves")).settings()?;
    // SubMenuOptionsGame's GlobalSettings, read by the simulation and HUD.
    use magicka_runtime::global_settings::{set_blood_and_gore, set_health_bars, HealthBars};
    set_blood_and_gore(settings["game"]["gore"].as_str() != Some("off"));
    set_health_bars(match settings["game"]["health_bars"].as_str() {
        Some("off") => HealthBars::Off,
        Some("players_only") => HealthBars::PlayersOnly,
        _ => HealthBars::On,
    });
    Ok(UserVolumes {
        music: settings["music"]
            .as_f64()
            .filter(|v| (0.0..=1.0).contains(v))
            .unwrap_or(0.8) as f32,
        sfx: settings["sfx"]
            .as_f64()
            .filter(|v| (0.0..=1.0).contains(v))
            .unwrap_or(1.0) as f32,
    })
}
/// Per session: options may change in the menu between sessions.
pub(super) fn insert(
    world: &mut World,
    binding: Option<SaveBinding>,
    cache: &std::path::Path,
) -> Result<(), String> {
    world.insert_resource(volumes(cache)?);
    if let Some(binding) = binding {
        world.insert_resource(binding);
    }
    Ok(())
}
/// Save the session to its slot now (a campaign level hand-off).
pub(super) fn save_now(world: &mut World) {
    let Some(binding) = world.get_resource::<SaveBinding>() else {
        return;
    };
    let result = world
        .resource::<Play>()
        .session
        .disk_save()
        .and_then(|state| binding.store.save_slot(binding.slot, &state));
    if let Err(e) = result {
        warn!("campaign save: {e}");
    }
}
/// Seconds between attempts while saving keeps failing.
const SAVE_RETRY: f64 = 5.0;

fn autosave(
    play: Res<Play>,
    mut binding: ResMut<SaveBinding>,
    mut exits: MessageReader<AppExit>,
    mut leaving: MessageReader<lifecycle::LeaveSession>,
    time: Res<Time<Real>>,
) {
    let quitting = exits.read().next().is_some() | leaving.read().next().is_some();
    let current = (
        play.session.scene_changes,
        play.session.checkpoint_generation,
    );
    if current != binding.last {
        binding.dirty = true;
    }
    // Retain the last stable save during a cutscene: dialog playback itself
    // is not in the disk snapshot yet. Retry the deferred checkpoint after it ends.
    if play.session.world.flags.cutscene
        || !play.session.world.tutorial.dialog_done()
        || play
            .session
            .world
            .dialogs
            .as_ref()
            .is_some_and(|dialogs| dialogs.active().iter().any(Option::is_some))
    {
        return;
    }
    if !binding.dirty && !quitting {
        return;
    }
    let now = time.elapsed_secs_f64();
    if !quitting && now < binding.retry_at {
        return;
    }
    let result = play
        .session
        .disk_save()
        .and_then(|state| binding.store.save_slot(binding.slot, &state));
    match result {
        Ok(()) => {
            binding.last = current;
            binding.dirty = false;
            binding.last_error = None;
            binding.retry_at = 0.0;
        }
        Err(e) => {
            binding.retry_at = now + SAVE_RETRY;
            if binding.last_error.as_ref() != Some(&e) {
                warn!("campaign save: {e}");
                binding.last_error = Some(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn occupied_slot_uses_selected_profile_robe_and_direct_load_uses_saved_robe() {
        let content = magicka_content::retail_content_dir();
        if !content.is_dir() {
            eprintln!("retail content absent; skipping");
            return;
        }
        let cache =
            std::env::temp_dir().join(format!("magicka-robe-slot-{}", new_player_id().unwrap()));
        let store = SaveStore::new(cache.join("saves"));
        let id = new_player_id().unwrap();
        let mut original = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
        original.player_ids[0] = Some(id.clone());
        let saved = original.disk_save().unwrap();
        store.save_slot(0, &saved).unwrap();
        // Compare file bytes across restoration, avoiding JSON float-parser
        // rounding differences between in-memory and disk snapshots.
        let stored_bytes = std::fs::read(store.slot_path(0).unwrap()).unwrap();
        let args = vec![
            "--save-slot".into(),
            "0".into(),
            "--profile-id".into(),
            id,
            "--character".into(),
            "Wizard_Necromancer".into(),
        ];
        restore(&mut original, &args, &cache, &content).unwrap();
        assert_eq!(original.player_robe(0), Some("Wizard_Necromancer"));
        assert_eq!(
            std::fs::read(store.slot_path(0).unwrap()).unwrap(),
            stored_bytes,
            "loading never rewrites a slot"
        );
        let selected = original.disk_save().unwrap();
        assert_eq!(selected["players"][0]["robe"], json!("Wizard_Necromancer"));
        store.save_slot(0, &selected).unwrap();
        let mut direct = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
        restore(
            &mut direct,
            &["--save-slot".into(), "0".into()],
            &cache,
            &content,
        )
        .unwrap();
        // Play uses this same restored name for the model path and Play resource.
        assert_eq!(direct.player_robe(0), Some("Wizard_Necromancer"));
        assert_eq!(
            direct.avatar().character.definition.id,
            original.avatar().character.definition.id
        );
        std::fs::remove_dir_all(cache).unwrap();
    }
}

#[cfg(test)]
mod quality_tests {
    use super::quality_settings;
    use magicka_runtime::global_settings::{Quality, QualitySettings};

    #[test]
    fn missing_settings_are_the_constructor_defaults() {
        assert_eq!(
            quality_settings(&serde_json::json!({})),
            QualitySettings::default()
        );
    }

    #[test]
    fn every_menu_value_is_read() {
        let q = quality_settings(&serde_json::json!({
            "game": {"spell_wheel": "off"},
            "graphics": {"shadows": "high", "decals": "low", "particles": "medium",
                         "particle_lights": "off"},
        }));
        assert_eq!(q.shadows, Quality::High);
        assert_eq!(q.decals, Quality::Low);
        assert_eq!(q.particles, Quality::Medium);
        assert!(!q.particle_lights && !q.spell_wheel);
    }
}
