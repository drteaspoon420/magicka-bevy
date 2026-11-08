//! Original main page with retained save/profile/options launcher subpages.
use crate::frontend_tome::{MenuId, TomeView};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::{app::AppExit, prelude::*};
use magicka_runtime::saves::{new_player_id, SaveStore};
use magicka_runtime::{
    localization::LocalizationTable,
    main_menu::{MenuCursor, TileId},
    play_content::english_strings,
    scene_loading::custom_hash,
};
use serde_json::{json, Value};
use std::path::PathBuf;
pub(crate) mod book;
mod controls_pages;
mod dlc_page;
mod map_cutscene;
mod native_main;
mod pack_panel;
pub(crate) mod pages;
mod ruleset_pages;
mod settings_pages;
mod sound;
mod splash;

/// SubMenuCampaignSelect_SaveSlotSelect/SubMenuCharacterSelect GameType.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum GameMode {
    #[default]
    Campaign,
    Mythos,
    /// The DLC challenges (Dungeons, The Other Side of the Coin, Vietnam).
    StoryChallenge,
    /// Challenges/*.lvl (Survival).
    Challenge,
    /// Versus/*.lvl, hosted with the chosen mode.
    Versus,
}

#[derive(Resource)]
pub(crate) struct Frontend {
    strings: LocalizationTable,
    cursor: MenuCursor,
    /// The open OptionsMessageBox (quit, notices, deletions).
    popup: Option<pages::MessageBox>,
    content: String,
    cache: String,
    store: SaveStore,
    settings: Value,
    pub(crate) controller_prompt_override: Option<crate::controller::Family>,
    pub(crate) controller_prompt_request: Option<Option<crate::controller::Family>>,
    pub(crate) controller_device_present: bool,
    page: u8,
    slot: u8,
    profile: usize,
    campaign: String,
    /// Robe templates with an exported model (Data/Characters/Wizard*).
    robes: Vec<String>,
    online: bool,
    fresh: bool,
    host: String,
    port: u16,
    message: String,
    launch: Option<Vec<String>>,
    pending_launch: Option<Vec<String>>,
    /// The opening starts once the book has turned to its page.
    intro_pending: bool,
    /// Frames until the menu hides behind a starting session's loading
    /// screen.
    hide_menu_after: Option<u32>,
    /// Each chapter's final map camera (character select's map preview).
    map_previews: std::collections::HashMap<String, map_cutscene::MapEntry>,
    /// SubMenuCutscene.Level/Play for the next map cutscene entry.
    cutscene_level: Option<String>,
    mode: GameMode,
    /// The Versus launch spec (`--versus`) of a Versus session.
    versus: Option<String>,
    dlc: dlc_page::DlcLicences,
    /// PackMan: the lobby's enabled reward packs (`--packs`) of a Challenge
    /// or Versus game.
    packs: Option<String>,
    /// SubMenuEndGame after a completed campaign level: once the camera
    /// settles, continue to the next chapter instead of turning back.
    continue_campaign: bool,
    /// Primary device to restore after the chapter intermission (or "keyboard").
    continue_primary: Option<String>,
    session: Session,
    launch_diagnostics: Vec<String>,
    /// Audit instrumentation (`--menu-audit-repeat N`): relaunch the last
    /// session N times after it returns, proving teardown leaves a reusable world.
    repeat: u32,
    last_launch: Option<Vec<String>>,
    /// The last game frame, shown on the end-of-game page, and the
    /// remaining screen crossfade from it (Transitions.CrossFade, 1 s).
    end_shot: Option<Handle<Image>>,
    end_fade: f32,
    /// `--loading-audit DIR`: session lifecycle events in DIR/host.json.
    audit: Option<PathBuf>,
    audit_events: Vec<Value>,
    /// Lobby local co-op seats handed to play (`--local-players`).
    local_players: Vec<Value>,
    /// Pads seated in the lobby; they never drive the shared menu.
    seated_pads: std::collections::HashSet<Entity>,
    /// The lobby's `--lobby` spec (player limit, hot join, roles).
    lobby_spec: Option<String>,
    /// The role asked for when joining an online game.
    join_role: magicka_network::lobby::Role,
    /// The passphrase typed on the Online page, for hosting and joining.
    /// Kept in memory only: it is never written to the settings file.
    passphrase: Option<String>,
    /// Rewrite extension: the server being joined (shown on the profile
    /// page that precedes every join).
    joining: Option<String>,
    /// `--menu-qa-seats N`: audit captures of a lobby with N extra seats
    /// (no pads attached).
    qa_seats: usize,
    /// `--menu-qa-add-players N` (audit): press "Add local player" N times
    /// once the lobby sees pads.
    qa_add_players: usize,
    /// An online host's own role without a wizard (lobby "You play as").
    host_role: Option<magicka_network::lobby::Role>,
    /// LAN game browser, opened with the Online page.
    lan: Option<magicka_network::discovery::LanBrowser>,
    /// The server address chosen to join (matches its LAN beacon).
    join_entry: Option<String>,
    lan_games: Vec<magicka_network::discovery::LanGame>,
}
impl Frontend {
    fn record(&mut self, event: &str, windows: usize) {
        let Some(dir) = &self.audit else { return };
        self.audit_events
            .push(json!({"event": event, "windows": windows, "page": self.page}));
        if let Err(error) = std::fs::write(
            dir.join("host.json"),
            serde_json::to_vec_pretty(&self.audit_events).unwrap(),
        ) {
            warn!("host audit: {error}");
        }
    }
}
fn record(world: &mut World, event: &str) {
    let windows = world.query::<&Window>().iter(world).count();
    world.resource_mut::<Frontend>().record(event, windows);
}
/// The play session sharing the menu's window, as PlayState shares the
/// original's device: prepared off-thread behind the loading screen, then
/// begun in this world and torn down back to the Tome.
pub(crate) enum Session {
    Idle,
    Preparing(std::thread::JoinHandle<Result<crate::play::Prepared, String>>),
    Playing,
    /// PlayState.Endgame: the menu hidden, one clean frame captured for
    /// the end-of-game crossfade, then the session is torn down.
    Leaving {
        frames: u32,
        leave: crate::play::LeaveSession,
    },
}
/// The captured last game frame (RenderManager.GetScreenShot).
#[derive(Resource, Default)]
struct CapturedShot(Option<Image>);
/// A finished leave, torn down in `Last`: play systems later in the same
/// Update would otherwise run against the removed session (a set's run
/// condition is evaluated once per schedule run).
#[derive(Resource, Default)]
struct PendingEnd(Option<(Option<Image>, crate::play::LeaveSession)>);

/// The menu's screen-space 2D camera (message boxes, fades, splash), idle
/// while a session plays.
#[derive(Component)]
struct MenuCamera;
pub fn run(args: &[String]) -> Result<(), String> {
    let get = |key: &str| args.windows(2).find(|a| a[0] == key).map(|a| a[1].clone());
    let content = get("--menu").unwrap_or_else(|| "target/Magicka/Content".into());
    let content = std::fs::canonicalize(&content)
        .map_err(|e| format!("{content}: {e}"))?
        .to_string_lossy()
        .into_owned();
    let cache = get("--root").unwrap_or_else(|| "output/.cache".into());
    let store = SaveStore::new(PathBuf::from(&cache).join("saves"));
    let mut settings = store.settings()?;
    if settings["profiles"].as_array().is_none_or(Vec::is_empty) {
        settings["profiles"] =
            json!([{"id":new_player_id()?,"name":"Player","robe":"Wizard","colour":[1.0,1.0,1.0]}]);
        store.save_settings(&settings)?;
    }
    // Play mode needs both the template and its exported glTF.
    let mut robes: Vec<_> = std::fs::read_dir(PathBuf::from(&content).join("Data/Characters"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.path().file_stem()?.to_str()?.to_owned();
            let template = e
                .path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("xnb"))
                && name.to_ascii_lowercase().starts_with("wizard");
            let exported = PathBuf::from(&cache)
                .join(format!("gltf/Data/Characters/{name}.glb"))
                .is_file();
            (template && exported).then_some(name)
        })
        .collect();
    robes.sort_by_key(|r| (r != "Wizard", r.to_ascii_lowercase()));
    let strings = english_strings(std::path::Path::new(&content), &["Player"]);
    for key in magicka_runtime::main_menu::tiles([1.0; 2])
        .iter()
        .filter_map(|tile| tile.label)
        .chain(["#add_menu_rus_quit", "#add_menu_yes", "#add_menu_no"])
    {
        if !strings.try_get_string(custom_hash(key)).found {
            return Err(format!("missing menu localization {key}"));
        }
    }
    let cache = std::fs::canonicalize(&cache)
        .map_err(|e| format!("--root {cache}: {e}"))?
        .to_string_lossy()
        .into_owned();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Magicka rewrite".into(),
                    resolution: (960, 720).into(),
                    ..default()
                }),
                ..default()
            })
            .set(AssetPlugin {
                file_path: cache.clone(),
                ..default()
            }),
    );
    crate::frontend_tome::install(&mut app, &content, args)?;
    // CompanyState; developer entries and audits start at the desk.
    let splash = !args.iter().any(|a| {
        matches!(
            a.as_str(),
            "--no-splash"
                | "--menu-page"
                | "--menu-qa-passphrase"
                | "--menu-join"
                | "--menu-auto-open"
                | "--menu-endgame-preview"
                | "--snapshot"
                | "--loading-audit"
        )
    });
    splash::plugin(&mut app, &content, splash)?;
    app.add_plugins(book::BookPlugin {
        content: content.clone(),
    })
    .init_resource::<pages::PageInput>()
    .add_plugins(sound::plugin)
    .init_resource::<pages::Pages>()
    .add_systems(
        Update,
        (pages::collect_input, pages::run)
            .chain()
            .after(crate::frontend_tome::TomePointerSet)
            .in_set(book::BookDrawSet),
    );
    native_main::install(&mut app, &content)?;
    map_cutscene::plugin(&mut app, &content);
    app.add_systems(Update, settings_pages::apply_window);
    crate::frontend_cinema::install(&mut app, &content, args)?;
    crate::play::install(
        &mut app,
        &PathBuf::from(&content),
        !args.iter().any(|a| a == "--silent"),
    );
    app.insert_resource(ClearColor(Color::BLACK));
    let map_previews = map_cutscene::load_entries(&content);
    app.insert_resource(Frontend {
        strings,
        cursor: MenuCursor::default(),
        popup: None,
        content,
        cache,
        store,
        settings,
        controller_prompt_override: None,
        controller_prompt_request: None,
        controller_device_present: false,
        page: 0,
        intro_pending: false,
        cutscene_level: None,
        // `--menu-page 20` audits the Challenge lobby.
        mode: if get("--menu-page").as_deref() == Some("20") {
            GameMode::Challenge
        } else {
            GameMode::Campaign
        },
        versus: None,
        dlc: dlc_page::DlcLicences::default(),
        packs: None,
        map_previews,
        hide_menu_after: None,
        continue_campaign: false,
        continue_primary: None,
        slot: 0,
        profile: 0,
        // `--menu-campaign LEVEL`: developer/audit entry at a later chapter.
        campaign: get("--menu-campaign").unwrap_or_else(|| "Wizard_Castle.lvl".into()),
        robes,
        online: false,
        fresh: false,
        host: "localhost".into(),
        port: 27015,
        message: String::new(),
        launch: None,
        pending_launch: None,
        session: Session::Idle,
        repeat: get("--menu-audit-repeat")
            .map(|v| v.parse::<u32>().map_err(|e| e.to_string()))
            .transpose()?
            .unwrap_or(0),
        last_launch: None,
        end_shot: None,
        end_fade: 0.0,
        // `--host-audit DIR`: the session lifecycle alone (no loading audit).
        audit: get("--host-audit")
            .or_else(|| get("--loading-audit"))
            .map(PathBuf::from),
        audit_events: Vec::new(),
        local_players: Vec::new(),
        seated_pads: Default::default(),
        lobby_spec: None,
        join_role: magicka_network::lobby::Role::Player,
        passphrase: None,
        joining: None,
        lan: None,
        lan_games: Vec::new(),
        host_role: None,
        join_entry: None,
        qa_add_players: get("--menu-qa-add-players")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        qa_seats: get("--menu-qa-seats")
            .map(|v| v.parse::<usize>().map_err(|e| e.to_string()))
            .transpose()?
            .unwrap_or(0),
        launch_diagnostics: {
            let mut flags = Vec::new();
            if args.iter().any(|a| a == "--silent") {
                flags.push("--silent".into());
            }
            for flag in [
                "--loading-audit",
                "--audit-complete-after",
                "--local-coop-audit",
                "--qa-virtual-pads",
            ] {
                if let Some(value) = get(flag) {
                    flags.extend([flag.into(), value]);
                }
            }
            // Mods chosen at the menu apply to every game it launches.
            for flag in ["--mods", "--enable", "--mods-base"] {
                if let Some(value) = get(flag) {
                    flags.extend([flag.into(), value]);
                }
            }
            flags
        },
    });
    if let Some(page) = get("--menu-page").and_then(|v| v.parse::<u8>().ok()) {
        // Developer/audit entry: open the book and push the page chain
        // instantly (zero-page pushes do not riffle).
        let mut view = app.world_mut().resource_mut::<TomeView>();
        view.auto_open = true;
        let chain: &[MenuId] = match page {
            1 => &[MenuId::SaveSlot],
            5 => &[MenuId::Online],
            6 => &[MenuId::Options],
            9 => &[MenuId::SaveSlot, MenuId::Character],
            10 => &[MenuId::Options, MenuId::Sound],
            13 => &[MenuId::Options, MenuId::GameOptions],
            14 => &[MenuId::Options, MenuId::Graphics],
            16 => &[MenuId::Options, MenuId::Controls, MenuId::Keyboard],
            17 => &[MenuId::Dlc],
            18 => &[MenuId::Challenge],
            19 => &[MenuId::Versus],
            // The Challenge lobby (`--menu-campaign Challenges/<map>.lvl`).
            20 => &[MenuId::Challenge, MenuId::Character],
            _ => &[],
        };
        for menu in chain {
            view.push(*menu, 0);
        }
    }
    // `--menu-qa-passphrase TEXT` (audit): the passphrase box with an entry.
    if let Some(text) = get("--menu-qa-passphrase") {
        app.world_mut().resource_mut::<Frontend>().popup = Some(pages::MessageBox::text_input(
            pages::Purpose::Passphrase,
            "Passphrase for private games (empty for an open game)".into(),
            text,
            vec!["OK".into(), "Cancel".into()],
        ));
    }
    // `--menu-join ADDR` (audit): the join lobby for a server address, as
    // the Online page's Join opens it (the LAN browser keeps listening).
    if let Some(entry) = get("--menu-join") {
        {
            let mut view = app.world_mut().resource_mut::<TomeView>();
            view.auto_open = true;
            view.push(MenuId::Online, 0);
            view.push(MenuId::Character, 0);
        }
        let mut s = app.world_mut().resource_mut::<Frontend>();
        if let Some((host, port)) = entry.rsplit_once(':') {
            s.host = host.trim_matches(['[', ']']).into();
            s.port = port.parse().unwrap_or(s.port);
        }
        s.join_entry = Some(entry.clone());
        s.joining = Some(entry);
        s.lan = Some(magicka_network::discovery::LanBrowser::bind(
            magicka_network::discovery::DISCOVERY_PORT,
        ));
    }
    // `--menu-qa-pads N` (audit): named virtual gamepads for lobby renders.
    if let Some(n) = get("--menu-qa-pads").and_then(|v| v.parse::<usize>().ok()) {
        app.add_systems(Startup, move |mut commands: Commands| {
            for i in 0..n {
                commands.spawn((Gamepad::default(), Name::new(format!("QA pad {}", i + 1))));
            }
        });
    }
    app.add_systems(Startup, |mut commands: Commands| {
        // Screen GUI above the 3D Tome; it must not clear the book.
        commands.spawn((
            Camera2d,
            MenuCamera,
            Camera {
                order: 1,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            // The bloom chain and 1 - 2^-x decode run last, as in play.
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            bevy::camera::visibility::RenderLayers::layer(book::SCREEN_LAYER),
            crate::post::MagickaBloomPass,
            crate::post::MagickaPost::default(),
        ));
    });
    if let Some(path) = get("--snapshot") {
        app.insert_resource(MenuSnapshot {
            path,
            frames: 0,
            taken: false,
            after: get("--snapshot-after")
                .map(|value| value.parse::<f32>().map_err(|e| e.to_string()))
                .transpose()?
                .unwrap_or(0.0),
        });
        app.add_systems(Update, capture_menu.after(native_main::NativeMenuSet));
    }
    if let Some(path) = get("--menu-endgame-preview") {
        let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
        let image = Image::from_buffer(
            &bytes,
            bevy::image::ImageType::Extension("png"),
            bevy::image::CompressedImageFormats::NONE,
            true,
            bevy::image::ImageSampler::linear(),
            bevy::asset::RenderAssetUsages::default(),
        )
        .map_err(|e| format!("{path}: {e}"))?;
        app.world_mut().resource_mut::<TomeView>().auto_open = true;
        app.insert_resource(EndGamePreview(Some(image)))
            .add_systems(Update, preview_end_game.before(book::BookDrawSet));
    }
    if let Some(dir) = get("--screenshots") {
        // Verification captures from the swapchain, independent of the
        // desktop's screen grabbing: DIR/<seconds>.png at each listed time.
        let mut times = get("--screenshot-at")
            .unwrap_or_default()
            .split(',')
            .filter(|v| !v.is_empty())
            .map(|v| {
                v.parse::<f32>()
                    .map_err(|e| format!("--screenshot-at {v}: {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        times.sort_by(f32::total_cmp);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{dir}: {e}"))?;
        app.insert_resource(TimedShots {
            dir: PathBuf::from(dir),
            times,
            next: 0,
        })
        .add_systems(Last, timed_shots);
    }
    app.add_systems(
        Update,
        sync_page
            .after(crate::frontend_tome::TomeSet)
            .before(crate::frontend_cinema::CinemaSet),
    );
    app.add_systems(
        Update,
        (finish_opening, host_session)
            .chain()
            .after(crate::frontend_cinema::CinemaSet),
    )
    .add_systems(
        Last,
        begin_leaving
            .run_if(on_message::<crate::play::LeaveSession>)
            .in_set(crate::play::SessionEnd),
    )
    .add_systems(Last, finish_leaving.after(crate::play::SessionEnd))
    .init_resource::<CapturedShot>()
    .init_resource::<PendingEnd>();
    app.run();
    crate::finish_process(app)
}
/// MainMenu's tile actions: every submenu turns one page.
fn activate_tile(s: &mut Frontend, tome: &mut TomeView, tile: TileId) {
    s.message.clear();
    match tile {
        TileId::Adventure => {
            // An offline campaign; hosting starts from the online page.
            s.online = false;
            s.mode = GameMode::Campaign;
            tome.push(MenuId::SaveSlot, 1);
        }
        TileId::Online => tome.push(MenuId::Online, 1),
        TileId::Dlc => {
            s.online = false;
            tome.push(MenuId::Dlc, 1);
        }
        TileId::Options => tome.push(MenuId::Options, 1),
        TileId::Challenge => {
            s.versus = None;
            tome.push(MenuId::Challenge, 1);
        }
        TileId::Versus => tome.push(MenuId::Versus, 1),
        TileId::Exit => show_quit(s),
        other => {
            let message = match other {
                TileId::Dlc => "DLC campaign selection is not available yet.",
                _ => "This service is unavailable offline.",
            };
            let ok = pages::loc(s, "#add_menu_ok");
            s.popup = Some(pages::MessageBox::new(
                pages::Purpose::Notice,
                message.into(),
                vec![ok],
            ));
        }
    }
}

/// SubMenuMain.ShowRUSure: "#add_menu_rus_quit" with Yes and No.
pub(crate) fn show_quit(s: &mut Frontend) {
    let options = vec![
        pages::loc(s, "#add_menu_yes"),
        pages::loc(s, "#add_menu_no"),
    ];
    s.popup = Some(pages::MessageBox::new(
        pages::Purpose::Quit,
        pages::loc(s, "#add_menu_rus_quit"),
        options,
    ));
    s.record("quit box", 1);
}

pub(crate) fn launch_game(s: &mut Frontend, client: bool, tome: &mut TomeView) {
    let p = &s.settings["profiles"][s.profile];
    // Story challenges, challenges and Versus keep no save.
    let story = matches!(
        s.mode,
        GameMode::StoryChallenge | GameMode::Challenge | GameMode::Versus
    );
    // Any profile can play any save: players whose profile is absent are
    // parked in the save and restored when that profile returns.
    if !client && !s.fresh && !story {
        if let Err(e) = s.store.load_slot(s.slot) {
            s.message = e;
            return;
        }
    }
    let mut args = vec![
        "--play".into(),
        s.content.clone(),
        "--root".into(),
        s.cache.clone(),
        "--level".into(),
        s.campaign.clone(),
        "--character".into(),
        p["robe"].as_str().unwrap_or("Wizard").into(),
        "--name".into(),
        p["name"].as_str().unwrap_or("Player").into(),
        "--profile-id".into(),
        p["id"].as_str().unwrap_or("").into(),
    ];
    let colour: [f32; 3] = serde_json::from_value(p["colour"].clone()).unwrap_or([1.0; 3]);
    args.extend([
        "--robe-colour".into(),
        format!("{},{},{}", colour[0], colour[1], colour[2]),
    ]);
    if client {
        let target = if s.host.contains(':') {
            format!("[{}]:{}", s.host, s.port)
        } else {
            format!("{}:{}", s.host, s.port)
        };
        let address = match magicka_network::transport::resolve(&target) {
            Ok(a) => a,
            Err(e) => {
                s.message = e;
                return;
            }
        };
        args.extend(["--network-join".into(), address.to_string()]);
        if s.join_role != magicka_network::lobby::Role::Player {
            args.extend(["--role".into(), format!("{:?}", s.join_role).to_lowercase()]);
        }
        let entry = if s.host.contains(':') {
            format!("[{}]:{}", s.host, s.port)
        } else {
            format!("{}:{}", s.host, s.port)
        };
        let history = s.settings["history"].as_array_mut().unwrap();
        history.retain(|v| v.as_str() != Some(&entry));
        history.insert(0, json!(entry));
        history.truncate(16);
    } else {
        // Story challenges keep no save.
        if !story {
            args.extend(["--save-slot".into(), s.slot.to_string()]);
            if s.fresh {
                args.push("--new-campaign".into());
            }
        }
        if s.online {
            args.extend(["--network-host".into(), format!("0.0.0.0:{}", s.port)]);
            // A dungeon master/spectator host runs dedicated: no wizard.
            if let Some(role) = s.host_role {
                args.extend(["--host-role".into(), format!("{role:?}").to_lowercase()]);
            }
        }
        if let (GameMode::Versus, Some(spec)) = (s.mode, s.versus.as_ref()) {
            args.extend(["--versus".into(), spec.clone()]);
        }
        // PackMan: the lobby's enabled packs feed Survival/Versus luggage.
        if let (GameMode::Challenge | GameMode::Versus, Some(spec)) = (s.mode, s.packs.as_ref()) {
            args.extend(["--packs".into(), spec.clone()]);
        }
        if let Some(spec) = &s.lobby_spec {
            args.extend(["--lobby".into(), spec.clone()]);
        }
        if !s.local_players.is_empty() && s.host_role.is_none() {
            args.extend([
                "--local-players".into(),
                Value::Array(s.local_players.clone()).to_string(),
            ]);
        }
        if let Some(primary) = s.continue_primary.take() {
            args.extend(["--local-primary".into(), primary]);
        }
    }
    if let Err(e) = s.store.save_settings(&s.settings) {
        s.message = e;
        return;
    }
    if let Some(passphrase) = s.passphrase.as_ref().filter(|_| client || s.online) {
        args.extend(["--passphrase".into(), passphrase.clone()]);
    }
    args.extend(s.launch_diagnostics.clone());
    if !client {
        args.push("--menu-loading".into());
        // SubMenuCharacterSelect: the vanilla campaign's first level opens
        // with SubMenuIntro; any other level with a map (later chapters,
        // Mythos, the Dungeons) with SubMenuCutscene; the rest fade in.
        let chapter = magicka_runtime::end_game::campaign_position(&s.campaign)
            .filter(|(c, _)| *c == magicka_runtime::end_game::Campaign::Vanilla)
            .map(|(_, i)| i);
        if magicka_runtime::frontend_loading::uses_opening_intro(true, chapter, false) {
            // SubMenuCharacterSelect: PushMenu(SubMenuIntro, 8).
            s.pending_launch = Some(args);
            s.intro_pending = true;
            tome.push(MenuId::Intro, 8);
            return;
        }
        if s.map_previews
            .contains_key(&s.campaign.to_ascii_lowercase())
        {
            // PushMenu(SubMenuCutscene, 20) with Play set.
            s.pending_launch = Some(args);
            s.cutscene_level = Some(s.campaign.clone());
            tome.push(MenuId::Cutscene, 20);
            return;
        }
    }
    s.launch = Some(args);
}

#[derive(Resource)]
struct MenuSnapshot {
    path: String,
    frames: u32,
    after: f32,
    taken: bool,
}
fn capture_menu(
    mut commands: Commands,
    mut snapshot: ResMut<MenuSnapshot>,
    state: Res<Frontend>,
    time: Res<Time>,
) {
    use bevy::render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured};
    snapshot.frames += 1;
    if !snapshot.taken && snapshot.frames >= 60 && time.elapsed_secs() >= snapshot.after {
        snapshot.taken = true;
        let record = json!({"page":state.page,"selected":state.cursor.selected.map(TileId::name),"attached":state.cursor.attached,"popup":state.popup.as_ref().map(|p| format!("{:?}", p.purpose))});
        std::fs::write(
            PathBuf::from(&snapshot.path).with_extension("json"),
            serde_json::to_vec_pretty(&record).unwrap(),
        )
        .expect("menu snapshot state");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(snapshot.path.clone()))
            .observe(
                |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                    exit.write(AppExit::Success);
                },
            );
    }
}

#[derive(Resource)]
struct TimedShots {
    dir: PathBuf,
    times: Vec<f32>,
    next: usize,
}
/// `--screenshots DIR [--screenshot-at T,...]`: captures at the listed
/// times (exiting after the last), and for every `DIR/NAME.request` a test
/// driver creates, `DIR/NAME.png` (the request file is removed).
fn timed_shots(mut commands: Commands, mut shots: ResMut<TimedShots>, time: Res<Time>) {
    use bevy::render::view::screenshot::save_to_disk;
    for entry in std::fs::read_dir(&shots.dir)
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "request") && std::fs::remove_file(&path).is_ok() {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path.with_extension("png")));
        }
    }
    let Some(&at) = shots.times.get(shots.next) else {
        return;
    };
    if time.elapsed_secs() < at {
        return;
    }
    shots.next += 1;
    let path = shots.dir.join(format!("{at:06.2}.png"));
    let last = shots.next == shots.times.len();
    let mut shot = commands.spawn(Screenshot::primary_window());
    shot.observe(save_to_disk(path));
    if last {
        shot.observe(
            |_: On<ScreenshotCaptured>, mut exit: MessageWriter<AppExit>| {
                exit.write(AppExit::Success);
            },
        );
    }
}

fn finish_opening(
    mut state: ResMut<Frontend>,
    mut intro: ResMut<crate::frontend_cinema::IntroControl>,
) {
    let Some(completion) = intro.completed.take() else {
        return;
    };
    let Some(mut args) = state.pending_launch.take() else {
        return;
    };
    if let Some(index) = completion.loading_tip {
        args.extend(["--loading-intro-tip".into(), index.to_string()]);
    }
    state.launch = Some(args);
}

fn set_menu_camera(world: &mut World, active: bool) {
    let mut cameras = world.query_filtered::<&mut Camera, With<MenuCamera>>();
    for mut camera in cameras.iter_mut(world) {
        camera.is_active = active;
    }
}

/// Follow the Tome's settled page; start the opening once its page is up.
fn sync_page(
    mut state: ResMut<Frontend>,
    mut tome: ResMut<TomeView>,
    mut intro: ResMut<crate::frontend_cinema::IntroControl>,
) {
    if !matches!(state.session, Session::Idle) {
        return;
    }
    let previous = state.page;
    state.page = match tome.current() {
        None | Some(MenuId::Main) => 0,
        Some(MenuId::SaveSlot) => 1,
        Some(MenuId::Online) => 5,
        Some(MenuId::Options) => 6,
        Some(MenuId::Intro) => 7,
        Some(MenuId::Character) => 9,
        Some(MenuId::Sound) => 10,
        Some(MenuId::EndGame) => 11,
        Some(MenuId::Cutscene) => 12,
        Some(MenuId::GameOptions) => 13,
        Some(MenuId::Graphics) => 14,
        Some(MenuId::Controls) => 15,
        Some(MenuId::Keyboard) => 16,
        Some(MenuId::Dlc) => 17,
        Some(MenuId::Challenge) => 18,
        Some(MenuId::Versus) => 19,
    };
    if state.page != previous || (state.audit_events.is_empty() && tome.interactive()) {
        let page = format!("page {}", state.page);
        state.record(&page, 1);
    }
    if std::mem::take(&mut intro.riffle) {
        if let Some(t) = tome.tome.as_mut() {
            t.riffle(true, 1);
        }
    }
    // SubMenuIntro sets the Tome's light targets as the story progresses.
    if let Some([intensity, amount, speed]) = intro.light.take() {
        if let Some(t) = tome.tome.as_mut() {
            t.light.target = intensity;
            t.light.target_variation_amount = amount;
            t.light.target_variation_speed = speed;
        }
    }
    if state.page == 7 && state.intro_pending && tome.interactive() {
        state.intro_pending = false;
        intro.volumes = [
            state.settings["music"].as_f64().unwrap_or(1.0) as f32,
            state.settings["sfx"].as_f64().unwrap_or(1.0) as f32,
        ];
        intro.requested = true;
    }
}

#[inline(never)]
fn log_capture(image: &Image) {
    let format = format!("{:?}", image.texture_descriptor.format);
    let bytes = image.data.as_ref().map_or(0, Vec::len);
    info!(
        "end of game capture {}x{} {format} {bytes} bytes",
        image.width(),
        image.height()
    );
}

fn finish_leaving(world: &mut World) {
    if let Some((shot, leave)) = world.resource_mut::<PendingEnd>().0.take() {
        end_session(world, shot, leave);
    }
}

fn begin_leaving(mut leave: MessageReader<crate::play::LeaveSession>, mut state: ResMut<Frontend>) {
    let Some(message) = leave.read().last().copied() else {
        return;
    };
    if matches!(state.session, Session::Playing) {
        state.session = Session::Leaving {
            frames: 0,
            leave: message,
        };
    }
}

/// Start requested sessions and begin them once prepared. One window and
/// one event loop serve the whole application lifetime.
fn host_session(world: &mut World) {
    if let Session::Leaving { frames, leave } = world.resource::<Frontend>().session {
        world.resource_mut::<Frontend>().session = Session::Leaving {
            frames: frames + 1,
            leave,
        };
        match frames {
            0 => crate::play::hide_pause_menu(world),
            // Let the hidden menu leave the frame before capturing it.
            2 => {
                world.resource_mut::<CapturedShot>().0 = None;
                world.spawn(Screenshot::primary_window()).observe(
                    |shot: On<ScreenshotCaptured>, mut captured: ResMut<CapturedShot>| {
                        captured.0 = Some(shot.image.clone());
                    },
                );
            }
            _ => {}
        }
        if !crate::play::network_departure_finished(world) {
            return;
        }
        let shot = world.resource_mut::<CapturedShot>().0.take();
        // Without a readback (no surface) the return still happens.
        if shot.is_some() || frames > 120 {
            world.resource_mut::<PendingEnd>().0 = Some((shot, leave));
        }
        return;
    }
    // One session at a time: a launch waits until the last one is torn down.
    let idle = matches!(world.resource::<Frontend>().session, Session::Idle);
    let request = idle
        .then(|| world.resource_mut::<Frontend>().launch.take())
        .flatten();
    if let Some(args) = request {
        world.resource_mut::<Frontend>().last_launch = Some(args.clone());
        crate::play::capture_baseline(world);
        // The menu (with any fade's black) stays up until the loading
        // screen's camera has drawn: see `hide_menu_after`.
        world.resource_mut::<Frontend>().hide_menu_after = Some(2);
        let content = PathBuf::from(&world.resource::<Frontend>().content);
        if let Err(error) = crate::play::present_loading(world, &content, &args) {
            fail_session(world, error);
            return;
        }
        let client = match crate::play::connect(&args) {
            Ok(client) => client,
            Err(error) => {
                fail_session(world, error);
                return;
            }
        };
        let joined = client.as_ref().map(|c| c.level().to_owned());
        if let Some(client) = client {
            world.insert_non_send(PendingClient(client));
        }
        let worker = std::thread::spawn(move || {
            crate::play::prepare(&content.to_string_lossy(), &args, joined.as_deref())
        });
        let mut state = world.resource_mut::<Frontend>();
        state.session = Session::Preparing(worker);
        state.page = 8;
        record(world, "launch");
        return;
    }
    let hide = world
        .resource_mut::<Frontend>()
        .hide_menu_after
        .as_mut()
        .map(|frames| {
            *frames = frames.saturating_sub(1);
            *frames
        });
    if (hide == Some(0) || matches!(world.resource::<Frontend>().session, Session::Playing))
        && world
            .resource_mut::<Frontend>()
            .hide_menu_after
            .take()
            .is_some()
    {
        set_menu_camera(world, false);
        world.resource_mut::<TomeView>().visible = false;
    }
    let finished = matches!(
        &world.resource::<Frontend>().session,
        Session::Preparing(worker) if worker.is_finished()
    );
    if !finished {
        return;
    }
    let Session::Preparing(worker) = std::mem::replace(
        &mut world.resource_mut::<Frontend>().session,
        Session::Playing,
    ) else {
        unreachable!()
    };
    let result = worker
        .join()
        .unwrap_or_else(|_| Err("session preparation panicked".into()))
        .and_then(|prepared| {
            let client = world.remove_non_send::<PendingClient>().map(|c| c.0);
            crate::play::begin(world, prepared, client)
        });
    match result {
        Ok(()) => {
            if world
                .resource_mut::<Frontend>()
                .hide_menu_after
                .take()
                .is_some()
            {
                set_menu_camera(world, false);
                world.resource_mut::<TomeView>().visible = false;
            }
            record(world, "begin")
        }
        Err(error) => fail_session(world, error),
    }
}

/// A joined server's view, kept on this thread while the session prepares.
struct PendingClient(crate::play::Client);

fn fail_session(world: &mut World, error: String) {
    error!("play session: {error}");
    world.remove_non_send::<PendingClient>();
    world.resource_mut::<Frontend>().hide_menu_after = None;
    crate::play::tear_down(world);
    set_menu_camera(world, true);
    world.resource_mut::<TomeView>().visible = true;
    let mut state = world.resource_mut::<Frontend>();
    state.session = Session::Idle;
    let ok = pages::loc(&state, "#add_menu_ok");
    state.popup = Some(pages::MessageBox::new(
        pages::Purpose::Notice,
        error.clone(),
        vec![ok],
    ));
    state.message.clear();
}

/// PlayState pops back to the Tome after a quit or the credits:
/// OnTransitionEnd/ScreenCaptureCallback play the Zoomed_In camera, push
/// SubMenuEndGame (showing the captured frame) and crossfade from it.
fn end_session(world: &mut World, shot: Option<Image>, leave: crate::play::LeaveSession) {
    match &shot {
        Some(image) => log_capture(image),
        None => warn!("end of game: no screen capture"),
    }
    // LevelComplete: the save moves on to the next chapter first.
    let next = leave
        .level_complete
        .then(|| crate::play::advance_campaign(world))
        .flatten();
    let local_players = crate::play::local_party(world);
    let primary = world
        .get_resource::<crate::controller::LocalDevices>()
        .and_then(|d| d.primary)
        .map(|e| e.to_bits().to_string())
        .unwrap_or_else(|| "keyboard".into());
    crate::play::tear_down(world);
    set_menu_camera(world, true);
    if let Some(level) = next {
        let mut state = world.resource_mut::<Frontend>();
        state.seated_pads = local_players
            .iter()
            .filter_map(|p| p["pad"].as_u64())
            .map(Entity::from_bits)
            .collect();
        state.local_players = local_players;
        state.continue_primary = Some(primary);
        state.campaign = level;
        state.fresh = false;
        state.continue_campaign = true;
    }
    enter_end_game(world, shot, leave.credits);
    let mut state = world.resource_mut::<Frontend>();
    state.session = Session::Idle;
    state.page = 0;
    record(world, "end");
    let mut state = world.resource_mut::<Frontend>();
    if state.repeat > 0 {
        state.repeat -= 1;
        state.launch = state.last_launch.clone();
    }
}

/// ScreenCaptureCallback: the Zoomed_In camera, SubMenuEndGame (pushed
/// instantly with 9 riffles) showing the capture, and the crossfade from it.
fn enter_end_game(world: &mut World, shot: Option<Image>, credits: bool) {
    let shot = shot.and_then(|image| {
        // The swapchain capture is BGRA and its alpha is not coverage. A
        // fresh RGBA texture (the readback image's descriptor is not one a
        // material can sample) drawn opaque.
        use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
        let bgra = matches!(
            image.texture_descriptor.format,
            TextureFormat::Bgra8UnormSrgb | TextureFormat::Bgra8Unorm
        );
        let (width, height) = (image.width(), image.height());
        let mut data = image.data?;
        for pixel in data.as_chunks_mut::<4>().0 {
            if bgra {
                pixel.swap(0, 2);
            }
            pixel[3] = 255;
        }
        let mut image = Image::new(
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::default(),
        );
        image.sampler = bevy::image::ImageSampler::linear();
        Some(world.resource_mut::<Assets<Image>>().add(image))
    });
    let mut state = world.resource_mut::<Frontend>();
    state.end_fade = if shot.is_some() { 1.0 } else { 0.0 };
    state.end_shot = shot;
    let mut tome = world.resource_mut::<TomeView>();
    tome.visible = true;
    if let Some(t) = tome.tome.as_mut() {
        // Back from play the candle burns as it does on the main page.
        t.light.target = 1.0;
        t.light.target_variation_amount = 0.2;
        t.light.target_variation_speed = 6.0;
        if t.current() == Some(MenuId::Intro) {
            t.pop_instant();
        }
        t.push_instant(MenuId::EndGame, 9);
        t.play_camera(magicka_runtime::frontend_tome::CameraClip::ZoomedIn);
        // SubMenuEndGame.OnEnter.
        t.cross_fade_camera(magicka_runtime::frontend_tome::CameraClip::Idle, 1.0);
        if credits {
            t.close_to_back();
        }
    }
}

/// `--menu-endgame-preview IMAGE`: once the book is open, enter
/// SubMenuEndGame as if a session had just ended on IMAGE.
#[derive(Resource)]
struct EndGamePreview(Option<Image>);
fn preview_end_game(world: &mut World) {
    if !world.resource::<TomeView>().interactive() {
        return;
    }
    if let Some(image) = world.resource_mut::<EndGamePreview>().0.take() {
        enter_end_game(world, Some(image), false);
    }
}

impl Frontend {
    pub(crate) fn refresh_profiles(&mut self) {
        match self.store.settings() {
            Ok(settings) => self.settings["profiles"] = settings["profiles"].clone(),
            Err(e) => warn!("reload hot-join profiles: {e}"),
        }
    }
    pub(crate) fn controller_element_style(&self) -> crate::controller::ElementStyle {
        crate::controller::ElementStyle::parse(self.settings["controller_element_style"].as_str())
    }
    pub(crate) fn set_controller_element_style(&mut self, style: crate::controller::ElementStyle) {
        self.settings["controller_element_style"] = style.key().into();
        if let Err(error) = self.store.save_settings(&self.settings) {
            self.message = error;
        }
    }
    pub(crate) fn controller_prompt_family(&self) -> Option<crate::controller::Family> {
        self.controller_prompt_override
    }
}
