//! Headless rewrite server: no renderer, display or retail executable.
use magicka_network::session::PlayerProfile;
use magicka_runtime::{network_session::HostedSession, play_session::PlaySession};
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
fn security_options(args: &[String]) -> Result<magicka_network::security::HostSecurity, String> {
    use magicka_network::security::{secret_from, HostSecurity};
    let value = |flag: &str| -> Result<Option<&str>, String> {
        match args.iter().position(|a| a == flag) {
            Some(i) => args
                .get(i + 1)
                .map(|s| Some(s.as_str()))
                .ok_or_else(|| format!("{flag} requires a value")),
            None => Ok(None),
        }
    };
    let mut security = match secret_from(value("--passphrase-file")?, "MAGICKA_PASSPHRASE")? {
        Some(passphrase) => HostSecurity::private(&passphrase)?,
        None => HostSecurity::default(),
    };
    security.dm_credential = secret_from(value("--dm-key-file")?, "MAGICKA_DM_KEY")?;
    if let Some(list) = value("--ban")? {
        security.banned = list
            .split(',')
            .map(|ip| ip.trim().parse().map_err(|e| format!("--ban {ip}: {e}")))
            .collect::<Result<_, String>>()?;
    }
    if let Some(limit) = value("--connections-per-address")? {
        security.connections_per_address = limit
            .parse()
            .map_err(|e| format!("--connections-per-address: {e}"))?;
    }
    Ok(security)
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help") {
        println!("magicka-server --content <Content> --level <level.lvl> --bind <127.0.0.1:17700> [--no-scripts] [--demo-goblin] [--max-ticks N] [--lobby players=8,hotjoin=off,spectators=on,dm=on,invaders=on] [--mods DIR [--enable a,b] [--mods-base DIR]]");
        println!("  [--passphrase-file FILE | env MAGICKA_PASSPHRASE]  private game: joining needs the passphrase, traffic is encrypted");
        println!("  [--dm-key-file FILE | env MAGICKA_DM_KEY]          the dungeon master role needs this credential");
        println!("  [--ban IP,IP] [--connections-per-address N]");
        return Ok(());
    }
    let value = |flag: &str| -> Result<Option<&str>, String> {
        match args.iter().position(|a| a == flag) {
            Some(i) => args
                .get(i + 1)
                .map(|s| Some(s.as_str()))
                .ok_or_else(|| format!("{flag} requires a value")),
            None => Ok(None),
        }
    };
    let content = PathBuf::from(value("--content")?.ok_or("--content required")?);
    let level = value("--level")?.ok_or("--level required")?;
    let address = value("--bind")?
        .unwrap_or("127.0.0.1:17700")
        .parse()
        .map_err(|e| format!("bind address: {e}"))?;
    let limit: Option<u64> = value("--max-ticks")?
        .map(|s| s.parse().map_err(|e| format!("max-ticks: {e}")))
        .transpose()?;
    let mut session =
        PlaySession::load(&content, level, "Wizard").map_err(|e| format!("level load: {e:?}"))?;
    if args.iter().any(|a| a == "--no-scripts") {
        session.disable_scripts();
    }
    if args.iter().any(|a| a == "--demo-goblin") {
        for _ in 0..30 {
            session.step(1. / 60.);
        }
        session.spawn_npc(
            "Goblin_pirate",
            value("--demo-goblin-order")?.unwrap_or("attack"),
            7.,
        )?;
    }
    // Rewrite extension: the original allowed four players and no joining
    // after the game began; four stays the default.
    let lobby = match value("--lobby")? {
        Some(spec) => magicka_network::lobby::LobbySettings::default().parse_onto(spec)?,
        None => Default::default(),
    };
    let security = security_options(&args)?;
    let public = !std::net::SocketAddr::ip(&address).is_loopback();
    for warning in security.warnings(public, lobby.dungeon_masters) {
        eprintln!("warning: {warning}");
    }
    let private = security.key.is_private();
    let mut host = HostedSession::bind_secured(
        address,
        level,
        true,
        PlayerProfile::default(),
        &session,
        lobby,
        security,
    )?;
    // Rewrite extension: server-side mods (definitions + Lua).
    let mut mods = match value("--mods")? {
        Some(dir) => {
            let enabled: Option<Vec<String>> = value("--enable")?
                .map(|list| list.split(',').map(|s| s.trim().to_string()).collect());
            let set = magicka_mods::ModSet::load(
                value("--mods-base")?.map(std::path::Path::new),
                std::path::Path::new(dir),
                enabled.as_deref(),
            )
            .map_err(|e| format!("mods: {e}"))?;
            let mods = magicka_runtime::mod_runtime::ModRuntime::from_set(set, host.identity)?;
            host.set_content(mods.content.rules_hash.clone(), mods.content.mods.clone());
            mods.attach(&mut session);
            println!("Mods: {}", mods.content.mods.join(", "));
            Some(mods)
        }
        None => None,
    };
    println!(
        "Headless Magicka rewrite server listening on {} (players {}, hot join {}, {} game)",
        host.transport.address(),
        lobby.max_players,
        if lobby.hot_join { "open" } else { "locked" },
        if private { "private" } else { "open" }
    );
    let tick_duration = Duration::from_secs_f64(1. / 60.);
    let mut tick = 0;
    while limit.is_none_or(|limit| tick < limit) {
        let started = Instant::now();
        host.poll(&mut session)?;
        if host.has_players() {
            if session.end_game.restart {
                host.restart(&mut session)?;
            }
            if let Some(mods) = mods.as_mut() {
                host.serve_mod_requests(mods, &mut session);
                mods.step(&mut session, 1. / 60.);
            }
            session.step(1. / 60.);
        }
        host.capture(&session);
        if let Some(mods) = mods.as_mut() {
            host.publish_mods(mods);
        }
        // An unpublishable frame must not end the game for everyone: clients
        // that get no frames report it themselves.
        if let Err(e) = host.publish(&session) {
            eprintln!("publish: {e}");
        }
        tick += 1;
        if let Some(remaining) = tick_duration.checked_sub(started.elapsed()) {
            thread::sleep(remaining);
        }
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("magicka-server: {error}");
        std::process::exit(1);
    }
}
