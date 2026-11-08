//! Do spells look right on the other computers? A real host and two real
//! `ClientView`s over UDP, casting with real inputs, in the three directions:
//! host to client, client to host, client to another client.
//!
//! Each row states how much of the spell each computer has and how far it is
//! from the wizard who cast it *as that computer shows that wizard*, so an
//! effect on the wrong wizard or at the wrong place fails.
//!
//! `cargo test --release -p magicka-viewer spell_sync -- --nocapture --test-threads=1`
use super::*;
use magicka_network::transport::NetConditions;
use magicka_runtime::avatar::ELEMENT_KEYS;
use magicka_runtime::network_world::mirror_slot;
use magicka_sim::character_health::SelfShieldType as Armour;
use magicka_sim::spells::Elements as E;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Cast {
    Force,
    Own,
    Area,
}
#[derive(Clone, Copy, PartialEq, Debug)]
enum Sees {
    Beams,
    Bolts,
    Shields,
    Barriers,
    Mines,
    Missiles,
    Blasts,
    Emitters,
    Armour(Armour),
}
struct Spell {
    name: &'static str,
    elements: &'static [E],
    cast: Cast,
    hold: u32,
    sees: Sees,
    /// How far from its caster this spell may legitimately be.
    reach: f32,
}
const fn spell(
    name: &'static str,
    elements: &'static [E],
    cast: Cast,
    hold: u32,
    sees: Sees,
    reach: f32,
) -> Spell {
    Spell {
        name,
        elements,
        cast,
        hold,
        sees,
        reach,
    }
}
const SPELLS: &[Spell] = &[
    spell(
        "arcane beam",
        &[E::ARCANE],
        Cast::Force,
        60,
        Sees::Beams,
        3.0,
    ),
    spell("life beam", &[E::LIFE], Cast::Force, 60, Sees::Beams, 3.0),
    spell(
        "lightning",
        &[E::LIGHTNING],
        Cast::Force,
        40,
        Sees::Bolts,
        30.0,
    ),
    spell(
        "shield disc",
        &[E::SHIELD],
        Cast::Force,
        30,
        Sees::Shields,
        10.0,
    ),
    spell(
        "shield ring",
        &[E::SHIELD],
        Cast::Area,
        30,
        Sees::Shields,
        10.0,
    ),
    spell(
        "earth barrier",
        &[E::SHIELD, E::EARTH],
        Cast::Force,
        30,
        Sees::Barriers,
        25.0,
    ),
    spell(
        "mines",
        &[E::SHIELD, E::ARCANE],
        Cast::Force,
        30,
        Sees::Mines,
        25.0,
    ),
    spell(
        "mine ring",
        &[E::SHIELD, E::ARCANE],
        Cast::Area,
        30,
        Sees::Mines,
        25.0,
    ),
    spell(
        "fire spray",
        &[E::FIRE],
        Cast::Force,
        60,
        Sees::Emitters,
        25.0,
    ),
    spell(
        "water spray",
        &[E::WATER],
        Cast::Force,
        60,
        Sees::Emitters,
        25.0,
    ),
    spell(
        "cold spray",
        &[E::COLD],
        Cast::Force,
        60,
        Sees::Emitters,
        25.0,
    ),
    spell(
        "steam spray",
        &[E::WATER, E::FIRE],
        Cast::Force,
        60,
        Sees::Emitters,
        25.0,
    ),
    spell("rock", &[E::EARTH], Cast::Force, 40, Sees::Missiles, 200.0),
    spell(
        "ice shards",
        &[E::WATER, E::COLD],
        Cast::Force,
        40,
        Sees::Missiles,
        200.0,
    ),
    spell(
        "shield armour",
        &[E::SHIELD],
        Cast::Own,
        15,
        Sees::Armour(Armour::Shield),
        0.0,
    ),
    spell(
        "earth armour",
        &[E::SHIELD, E::EARTH],
        Cast::Own,
        15,
        Sees::Armour(Armour::Earth),
        0.0,
    ),
    spell(
        "ice armour",
        &[E::SHIELD, E::WATER, E::COLD],
        Cast::Own,
        15,
        Sees::Armour(Armour::Ice),
        0.0,
    ),
    spell(
        "iced earth armour",
        &[E::SHIELD, E::EARTH, E::WATER, E::COLD],
        Cast::Own,
        15,
        Sees::Armour(Armour::IcedEarth),
        0.0,
    ),
    spell(
        "arcane blast",
        &[E::ARCANE],
        Cast::Area,
        20,
        Sees::Blasts,
        8.0,
    ),
    spell(
        "fire blast",
        &[E::FIRE],
        Cast::Area,
        20,
        Sees::Emitters,
        25.0,
    ),
    spell(
        "earth-fire blast",
        &[E::EARTH, E::FIRE],
        Cast::Area,
        20,
        Sees::Emitters,
        25.0,
    ),
];

fn near(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
}
fn feet_flat(a: [f32; 3], b: [f32; 3]) -> f32 {
    near([a[0], 0.0, a[2]], [b[0], 0.0, b[2]])
}

/// One computer's picture of the spell, this tick.
#[derive(Clone, Debug, Default)]
struct Picture {
    count: usize,
    /// Distance of the farthest piece from the caster as shown here.
    farthest: f32,
    /// Armour on a wizard who did not cast it.
    armour_on_others: usize,
    /// Where the caster is, on this computer.
    at: Option<[f32; 3]>,
    /// What this spell looks like: compared between computers.
    look: Vec<String>,
    /// Particles alive, decals on the ground, damage numbers: what a spell
    /// leaves behind, which the lists above say nothing about.
    particles: usize,
    decals: usize,
    numbers: usize,
}
fn tenth(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}
fn picture(session: &PlaySession, caster: usize, sees: Sees, baseline: &[i32]) -> Picture {
    let caster_at = session
        .world
        .players
        .get(caster)
        .and_then(Option::as_ref)
        .map(|p| session.world.physics.bodies[p.body].position);
    let places: Vec<[f32; 3]> = match sees {
        Sees::Beams => session.beams.iter().map(|b| b.position).collect(),
        Sees::Bolts => session
            .bolts
            .iter()
            .map(|_| caster_at.unwrap_or_default())
            .collect(),
        Sees::Shields => session
            .world
            .shields
            .iter()
            .map(|s| s.shield.position)
            .collect(),
        Sees::Barriers => session
            .world
            .barriers
            .iter()
            .map(|(_, b)| b.position)
            .collect(),
        Sees::Mines => session
            .world
            .mines
            .iter()
            .map(|(_, m)| m.position)
            .collect(),
        Sees::Blasts => session.arcane_blasts.iter().map(|b| b.position).collect(),
        Sees::Missiles => session
            .world
            .missiles
            .iter()
            .filter(|m| !m.body.dead)
            .map(|m| m.body.position)
            .chain(session.remote_missiles.iter().map(|m| m.position))
            .collect(),
        Sees::Emitters => session
            .effects
            .presentation()
            .iter()
            .filter(|e| !baseline.contains(&e.definition))
            .map(|e| [e.transform[12], e.transform[13], e.transform[14]])
            .collect(),
        Sees::Armour(kind) => session
            .world
            .players
            .get(caster)
            .and_then(Option::as_ref)
            .filter(|p| p.character.combat.health.shield.kind == kind)
            .map(|_| caster_at.unwrap_or_default())
            .into_iter()
            .collect(),
    };
    let armour_on_others = match sees {
        Sees::Armour(_) => session
            .world
            .players
            .iter()
            .enumerate()
            .filter(|(slot, p)| {
                *slot != caster
                    && p.as_ref()
                        .is_some_and(|p| p.character.combat.health.shield.active())
            })
            .count(),
        _ => 0,
    };
    let look: Vec<String> = match sees {
        Sees::Beams => session
            .beams
            .iter()
            .map(|b| {
                format!(
                    "beam {:?} {:?} spirals {}",
                    b.color_center, b.color_edge, b.spirals
                )
            })
            .collect(),
        Sees::Shields => session
            .world
            .shields
            .iter()
            .map(|s| {
                format!(
                    "shield {:?} radius {}",
                    s.shield.kind,
                    tenth(s.shield.radius)
                )
            })
            .collect(),
        Sees::Blasts => session
            .arcane_blasts
            .iter()
            .map(|b| {
                format!(
                    "blast radius {} colour {:?} elements {}",
                    tenth(b.max_radius),
                    b.color,
                    b.elements
                )
            })
            .collect(),
        Sees::Missiles => session
            .world
            .missiles
            .iter()
            .filter(|m| !m.body.dead)
            .filter_map(|m| {
                m.body.model.map(|model| {
                    format!(
                        "Models/Missiles/{}",
                        magicka_runtime::missiles::model_name(model)
                    )
                })
            })
            .chain(session.remote_missiles.iter().map(|m| m.model.clone()))
            .collect(),
        Sees::Emitters => session
            .effects
            .presentation()
            .iter()
            .filter(|e| !baseline.contains(&e.definition))
            .map(|e| format!("effect {}", e.definition))
            .collect(),
        Sees::Armour(_) => session
            .world
            .players
            .get(caster)
            .and_then(Option::as_ref)
            .map(|p| {
                let a = &p.character.combat.health.shield;
                format!(
                    "armour {:?} max {} magnitude {}",
                    a.kind,
                    tenth(a.max_health),
                    tenth(a.magnitude)
                )
            })
            .into_iter()
            .collect(),
        _ => Vec::new(),
    };
    Picture {
        at: caster_at,
        particles: session
            .effects
            .particles
            .active()
            .iter()
            .filter(|p| p.radius > 0. && p.normalized_lifetime < 1.)
            .count(),
        decals: session.world.decals.live().count(),
        numbers: session.world.floating_numbers.len(),
        look,
        count: places.len(),
        farthest: caster_at.map_or(0.0, |at| {
            places.iter().map(|p| feet_flat(*p, at)).fold(0.0, f32::max)
        }),
        armour_on_others,
    }
}

/// The running host, on its own thread: `ClientView::connect` blocks until the
/// host has sent it a wizard.
#[derive(Default)]
struct HostShare {
    controls: Option<Controls>,
    watch: Option<(usize, Sees)>,
    picture: Option<Picture>,
    baseline: Vec<i32>,
    conditions: Option<NetConditions>,
    /// The host refuses all input, as during a cutscene.
    lock_input: bool,
    stop: bool,
    address: Option<std::net::SocketAddr>,
    error: Option<String>,
}
fn level() -> std::path::PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scenarios/feature-lab.lvl")
        .canonicalize()
        .unwrap()
}
fn content() -> std::path::PathBuf {
    magicka_content::retail_content_dir()
}
fn run_host(share: Arc<Mutex<HostShare>>) {
    let level = level();
    let mut session = PlaySession::load(&content(), level.to_str().unwrap(), "Wizard").unwrap();
    session.disable_scripts();
    session.step(1. / 60.);
    {
        let s = share.lock().unwrap();
        session.world.flags.input_limited = s.lock_input;
    }
    let mut host = HostedSession::bind_lobby(
        "127.0.0.1:0".parse().unwrap(),
        level.to_str().unwrap(),
        false,
        PlayerProfile::default(),
        &session,
        Default::default(),
    )
    .unwrap();
    session.network_mode = magicka_runtime::play_session::NetworkMode::Server;
    host.transport
        .set_conditions(share.lock().unwrap().conditions);
    share.lock().unwrap().address = Some(host.transport.address());
    loop {
        let started = Instant::now();
        let (controls, watch, stop) = {
            let mut s = share.lock().unwrap();
            (s.controls.take(), s.watch, s.stop)
        };
        if stop {
            return;
        }
        if let Err(e) = host.poll(&mut session) {
            share.lock().unwrap().error = Some(e);
        }
        if let Some(controls) = controls {
            session.handle_remote_input(0, controls).ok();
        }
        session.step(1. / 60.);
        host.capture(&session);
        host.publish(&session).ok();
        if let Some((caster, sees)) = watch {
            let baseline = share.lock().unwrap().baseline.clone();
            share.lock().unwrap().picture = Some(picture(&session, caster, sees, &baseline));
        }
        std::thread::sleep(Duration::from_millis(16).saturating_sub(started.elapsed()));
    }
}

fn play() -> Play {
    let level = level();
    let content = content();
    let mut session = PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
    session.disable_scripts();
    session.network_mode = magicka_runtime::play_session::NetworkMode::Client;
    Play {
        session,
        content,
        cache: std::env::temp_dir().join(format!("magicka-spell-sync-{}", std::process::id())),
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
        scenarios: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scenarios"),
        dt: 1. / 60.,
        screen: [1280., 720.],
        menu_hosted: false,
        complete_after: None,
        bindings: default(),
    }
}

/// Who casts: the host's wizard, or the first client's.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Caster {
    Host,
    ClientOne,
}
#[derive(Debug, Default, Clone)]
struct Seen {
    peak: usize,
    farthest: f32,
    wrong_wizard: usize,
    /// Every distinct look shown over the cast.
    looks: std::collections::BTreeSet<String>,
    particles: usize,
    decals: usize,
    numbers: usize,
}
impl Seen {
    fn note(&mut self, picture: &Picture) {
        if picture.count > self.peak {
            self.peak = picture.count;
        }
        if picture.count > 0 {
            self.farthest = self.farthest.max(picture.farthest);
        }
        self.wrong_wizard = self.wrong_wizard.max(picture.armour_on_others);
        self.looks.extend(picture.look.iter().cloned());
        self.particles = self.particles.max(picture.particles);
        self.decals = self.decals.max(picture.decals);
        self.numbers = self.numbers.max(picture.numbers);
    }
}

/// Run one spell from one caster; returns (host, client 1, client 2).
fn run_case(spell: &Spell, caster: Caster) -> Result<[Seen; 3], String> {
    let share = Arc::new(Mutex::new(HostShare::default()));
    let host_thread = {
        let share = share.clone();
        std::thread::spawn(move || run_host(share))
    };
    let address = loop {
        if let Some(a) = share.lock().unwrap().address {
            break a;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let join = |name: &str| {
        ClientView::connect(
            &address.to_string(),
            JoinRequest {
                name: name.into(),
                profile: Some(magicka_network::session::ProfileSubmit {
                    profile: PlayerProfile {
                        name: name.into(),
                        ..default()
                    },
                    profile_id: None,
                }),
                ..default()
            },
        )
    };
    let mut views = [join("One")?, join("Two")?];
    let mut plays = [play(), play()];
    let slots = [views[0].own_slot().unwrap(), views[1].own_slot().unwrap()];
    let host_caster_slot = match caster {
        Caster::Host => 0,
        Caster::ClientOne => slots[0],
    };
    share.lock().unwrap().watch = Some((host_caster_slot, spell.sees));

    let mut baseline: Vec<i32> = Vec::new();
    let pump = |views: &mut [ClientView; 2],
                plays: &mut [Play; 2],
                baseline: &[i32]|
     -> Result<[Picture; 2], String> {
        let mut pictures = [Picture::default(), Picture::default()];
        for i in 0..2 {
            views[i].poll(&mut plays[i], 1. / 60.)?;
            let local = mirror_slot(host_caster_slot, Some(slots[i]));
            pictures[i] = picture(&plays[i].session, local, spell.sees, &baseline);
        }
        std::thread::sleep(Duration::from_millis(16));
        Ok(pictures)
    };
    // Everyone has everyone's wizard before anything is cast.
    for _ in 0..90 {
        pump(&mut views, &mut plays, &baseline)?;
    }
    // What is already playing is the level's, not the spell's.
    for play in &plays {
        baseline.extend(
            play.session
                .effects
                .presentation()
                .iter()
                .map(|e| e.definition),
        );
    }
    share.lock().unwrap().baseline = baseline.clone();
    let mut seen = [Seen::default(), Seen::default(), Seen::default()];
    let send = |views: &mut [ClientView; 2], controls: Controls| -> Result<(), String> {
        match caster {
            Caster::Host => {
                share.lock().unwrap().controls = Some(controls);
                Ok(())
            }
            Caster::ClientOne => views[0].send_controls(controls),
        }
    };
    let aim = |views: &[ClientView; 2]| {
        let at = views[0]
            .frame
            .players
            .iter()
            .find(|p| p.slot == host_caster_slot)
            .map_or([0.0; 3], |p| p.position);
        [at[0] + 3.0, at[1], at[2] + 3.0]
    };
    let mut phases: Vec<Controls> = Vec::new();
    for element in spell.elements {
        let at = ELEMENT_KEYS.iter().position(|k| k == element).unwrap();
        let mut press = Controls::default();
        press.elements[at] = true;
        phases.push(press);
        phases.push(Controls::default());
        phases.push(Controls::default());
    }
    for _ in 0..spell.hold {
        phases.push(Controls {
            cast_down: spell.cast != Cast::Own,
            cast_self_down: spell.cast == Cast::Own,
            shift_down: spell.cast == Cast::Area,
            ..Controls::default()
        });
    }
    phases.extend(std::iter::repeat_n(Controls::default(), 60));
    for controls in phases {
        let controls = Controls {
            aim: Some(aim(&views)),
            ..controls
        };
        send(&mut views, controls)?;
        let pictures = pump(&mut views, &mut plays, &baseline)?;
        seen[1].note(&pictures[0]);
        seen[2].note(&pictures[1]);
        if let Some(picture) = share.lock().unwrap().picture.clone() {
            seen[0].note(&picture);
        }
    }
    share.lock().unwrap().stop = true;
    host_thread.join().map_err(|_| "the host thread panicked")?;
    if let Some(e) = share.lock().unwrap().error.take() {
        return Err(e);
    }
    Ok(seen)
}

#[test]
fn every_spell_is_seen_correctly_in_every_direction() {
    if !content().is_dir() {
        return;
    }
    let only = std::env::var("MAGICKA_SPELL").ok();
    println!(
        "{:<20} {:<9} {:>12} {:>12} {:>12}   (count, farthest from caster in m)",
        "spell", "caster", "host", "client 1", "client 2"
    );
    let mut problems = Vec::new();
    for spell in SPELLS {
        if only.as_deref().is_some_and(|o| !spell.name.contains(o)) {
            continue;
        }
        for caster in [Caster::Host, Caster::ClientOne] {
            let seen = match run_case(spell, caster) {
                Ok(seen) => seen,
                Err(e) => {
                    println!("{:<20} {caster:?}: FAILED TO RUN: {e}", spell.name);
                    problems.push(format!("{} {caster:?}: {e}", spell.name));
                    continue;
                }
            };
            let show = |s: &Seen| format!("{} {:.1}", s.peak, s.farthest);
            println!(
                "    particles {} / {} / {}, decals {} / {} / {}, numbers {} / {} / {}",
                seen[0].particles,
                seen[1].particles,
                seen[2].particles,
                seen[0].decals,
                seen[1].decals,
                seen[2].decals,
                seen[0].numbers,
                seen[1].numbers,
                seen[2].numbers,
            );
            println!(
                "{:<20} {:<9} {:>12} {:>12} {:>12}",
                spell.name,
                format!("{caster:?}"),
                show(&seen[0]),
                show(&seen[1]),
                show(&seen[2])
            );
            let names = ["host", "client 1", "client 2"];
            if seen[0].peak == 0 {
                problems.push(format!(
                    "{} {caster:?}: the cast never happened on the host",
                    spell.name
                ));
                continue;
            }
            for (i, s) in seen.iter().enumerate() {
                if s.peak == 0 {
                    problems.push(format!(
                        "{} {caster:?}: {} never shows it",
                        spell.name, names[i]
                    ));
                }
                if spell.reach > 0.0 && s.farthest > spell.reach {
                    problems.push(format!(
                        "{} {caster:?}: {} shows it {:.1} m from the caster",
                        spell.name, names[i], s.farthest
                    ));
                }
                if s.looks != seen[0].looks && !s.looks.is_superset(&seen[0].looks) {
                    let missing: Vec<_> = seen[0].looks.difference(&s.looks).collect();
                    problems.push(format!(
                        "{} {caster:?}: {} never shows {missing:?}; it shows {:?}",
                        spell.name, names[i], s.looks
                    ));
                }
                // Particles are random and one computer samples them later, so
                // this asks for the same order of magnitude, not equality.
                if seen[0].particles >= 20 && s.particles * 2 < seen[0].particles {
                    problems.push(format!(
                        "{} {caster:?}: {} peaks at {} particles, the host at {}",
                        spell.name, names[i], s.particles, seen[0].particles
                    ));
                }
                if seen[0].decals > 0 && s.decals == 0 {
                    problems.push(format!(
                        "{} {caster:?}: {} shows no decals, the host {}",
                        spell.name, names[i], seen[0].decals
                    ));
                }
                if seen[0].numbers > 0 && s.numbers == 0 {
                    problems.push(format!(
                        "{} {caster:?}: {} shows no damage numbers",
                        spell.name, names[i]
                    ));
                }
                if s.wrong_wizard > 0 {
                    problems.push(format!(
                        "{} {caster:?}: {} shows it on another wizard",
                        spell.name, names[i]
                    ));
                }
            }
        }
    }
    assert!(problems.is_empty(), "\n{}\n", problems.join("\n"));
}

// ---- does the owned wizard feel right? ----

#[derive(Default)]
struct Walk {
    /// Mean distance of the drawn wizard from where the host really has it,
    /// corrected for what the host will have by then.
    mean_error: f32,
    /// Biggest single-frame move and how much moves vary frame to frame.
    biggest_move: f32,
    jitter: f32,
    samples: usize,
}
fn walk(predict: bool, locked: bool) -> Result<Walk, String> {
    let share = Arc::new(Mutex::new(HostShare::default()));
    let conditions = NetConditions {
        latency: Duration::from_millis(50),
        ..Default::default()
    };
    share.lock().unwrap().conditions = Some(conditions);
    share.lock().unwrap().lock_input = locked;
    let host_thread = {
        let share = share.clone();
        std::thread::spawn(move || run_host(share))
    };
    let address = loop {
        if let Some(a) = share.lock().unwrap().address {
            break a;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let mut view = ClientView::connect(
        &address.to_string(),
        JoinRequest {
            name: "Walker".into(),
            profile: Some(magicka_network::session::ProfileSubmit {
                profile: PlayerProfile {
                    name: "Walker".into(),
                    ..default()
                },
                profile_id: None,
            }),
            conditions: Some(conditions),
            ..default()
        },
    )?;
    view.predict = predict;
    let mut play = play();
    let slot = view.own_slot().unwrap();
    share.lock().unwrap().watch = Some((slot, Sees::Beams));
    for _ in 0..60 {
        view.poll(&mut play, 1. / 60.)?;
        std::thread::sleep(Duration::from_millis(16));
    }
    let mut out = Walk::default();
    let mut previous: Option<[f32; 3]> = None;
    let mut previous_move: Option<f32> = None;
    let mut total_error = 0.0;
    let mut total_jitter = 0.0;
    for tick in 0..300u32 {
        // Change direction every second: starts and stops are where it shows.
        let angle = (tick / 60) as f32 * 2.1;
        let at = play.session.world.physics.bodies[play.session.avatar().body].position;
        view.send_controls(Controls {
            aim: Some([at[0] + 8.0 * angle.cos(), at[1], at[2] + 8.0 * angle.sin()]),
            move_down: tick % 120 < 100,
            ..Controls::default()
        })?;
        view.poll(&mut play, 1. / 60.)?;
        let drawn = play.session.world.physics.bodies[play.session.avatar().body].position;
        if let Some(previous) = previous {
            let step = feet_flat(drawn, previous);
            out.biggest_move = out.biggest_move.max(step);
            if let Some(last) = previous_move {
                total_jitter += (step - last).abs();
            }
            previous_move = Some(step);
        }
        previous = Some(drawn);
        if let Some(host_at) = share.lock().unwrap().picture.as_ref().and_then(|p| p.at) {
            total_error += feet_flat(drawn, host_at);
            out.samples += 1;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    share.lock().unwrap().stop = true;
    host_thread.join().map_err(|_| "the host thread panicked")?;
    out.mean_error = total_error / out.samples.max(1) as f32;
    out.jitter = total_jitter / 299.0;
    Ok(out)
}

#[test]
fn prediction_keeps_the_owned_wizard_close_and_smooth() {
    if !content().is_dir() {
        return;
    }
    for (name, locked) in [("walking", false), ("input locked by the host", true)] {
        let off = walk(false, locked).unwrap();
        let on = walk(true, locked).unwrap();
        for (label, w) in [("off", &off), ("on", &on)] {
            println!(
                "{name:<26} prediction {label:<3}: error {:.3} m, biggest frame move {:.3} m, jitter {:.4} m ({} samples)",
                w.mean_error, w.biggest_move, w.jitter, w.samples
            );
        }
        if locked {
            assert!(
                on.mean_error < 0.2,
                "a locked wizard must stay put: {} m",
                on.mean_error
            );
        } else {
            assert!(
                on.mean_error <= off.mean_error,
                "{} vs {}",
                on.mean_error,
                off.mean_error
            );
            assert!(
                on.jitter <= off.jitter * 1.5,
                "{} vs {}",
                on.jitter,
                off.jitter
            );
        }
    }
}
