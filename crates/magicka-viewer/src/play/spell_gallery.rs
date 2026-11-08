//! QA: a picture of each spell type on each computer. Windows of a networked
//! game cannot be frame-synchronised, so each one watches its own game state
//! and takes its screenshot once a spell type has been showing for a moment
//! (`tools/network-spell-gallery.sh`). With `--autopilot-script spell-gallery`
//! or `spell-observer`.
use super::*;
use magicka_sim::character_health::SelfShieldType;
use std::collections::{BTreeMap, BTreeSet};

/// Frames a spell shows before its picture, so it has grown into view.
const SETTLE_FRAMES: u32 = 10;
/// Frames in which what is playing counts as the level's own.
const BASELINE_FRAMES: u32 = 150;

/// Every spell type this computer's game shows right now.
fn showing(session: &PlaySession, baseline: &BTreeSet<i32>) -> Vec<String> {
    let mut out = Vec::new();
    for beam in &session.beams {
        out.push(format!(
            "beam_{}",
            if beam.color_center[0] > 1.0 {
                "arcane"
            } else {
                "life"
            }
        ));
    }
    if !session.bolts.is_empty() {
        out.push("lightning".into());
    }
    for shield in &session.world.shields {
        out.push(format!("shield_{:?}", shield.shield.kind).to_lowercase());
    }
    if session.world.barriers.iter().next().is_some() {
        out.push("barrier".into());
    }
    if session.world.mines.iter().next().is_some() {
        out.push("mines".into());
    }
    if !session.arcane_blasts.is_empty() {
        out.push("arcane_blast".into());
    }
    for missile in session.world.missiles.iter().filter(|m| !m.body.dead) {
        if let Some(model) = missile.body.model {
            out.push(format!(
                "missile_{}",
                magicka_runtime::missiles::model_name(model)
            ));
        }
    }
    for missile in &session.remote_missiles {
        let name = missile.model.rsplit('/').next().unwrap_or(&missile.model);
        out.push(format!("missile_{name}"));
    }
    for player in session.world.players.iter().flatten() {
        let kind = player.character.combat.health.shield.kind;
        if kind != SelfShieldType::None {
            out.push(format!("armour_{kind:?}"));
        }
    }
    for effect in session.effects.presentation() {
        if !baseline.contains(&effect.definition) {
            out.push(format!("effect_{}", effect.definition));
        }
    }
    out
}

#[derive(Default)]
pub(super) struct Gallery {
    frame: u32,
    baseline: BTreeSet<i32>,
    first_seen: BTreeMap<String, u32>,
    taken: BTreeSet<String>,
}

pub(super) fn spell_gallery(
    mut commands: Commands,
    pilot: Option<ResMut<Autopilot>>,
    // Optional: this runs in the menu's app too, where no game exists yet.
    play: Option<Res<Play>>,
    mut gallery: Local<Gallery>,
    mut network: Option<NonSendMut<network::NetworkPlay>>,
    mut menu: Option<ResMut<menu::PauseMenu>>,
) {
    let (Some(mut pilot), Some(play)) = (pilot, play) else {
        return;
    };
    // By content: a `const` slice has no stable address to compare.
    let first = pilot.script.first().map(|step| (step.0, step.3));
    if first != SPELL_GALLERY_SCRIPT.first().map(|step| (step.0, step.3))
        && first != SPELL_OBSERVER_SCRIPT.first().map(|step| (step.0, step.3))
    {
        return;
    }
    // The script holds at its start until all three wizards are in the game:
    // casts nobody is there to see would prove nothing.
    let everyone_in = match network.as_deref_mut() {
        Some(network::NetworkPlay::Host(host)) => host.transport.seats().len() >= 2,
        Some(network::NetworkPlay::Client(client)) => client.frame.players.len() >= 3,
        None => true,
    };
    if !everyone_in {
        pilot.frame = pilot.started.unwrap_or(0);
    }
    if let Some(started) = pilot.started {
        if script_act(pilot.script, pilot.frame.saturating_sub(started)) == Some(Act::PlayersPage) {
            if let Some(menu) = menu.as_deref_mut() {
                menu.open_players_page();
            }
        }
    }
    gallery.frame += 1;
    let frame = gallery.frame;
    if frame <= BASELINE_FRAMES {
        let playing: Vec<i32> = play
            .session
            .effects
            .presentation()
            .iter()
            .map(|e| e.definition)
            .collect();
        gallery.baseline.extend(playing);
        return;
    }
    let baseline = std::mem::take(&mut gallery.baseline);
    let now = showing(&play.session, &baseline);
    gallery.baseline = baseline;
    let mut due = Vec::new();
    for kind in now {
        if gallery.taken.contains(&kind) {
            continue;
        }
        let first = *gallery.first_seen.entry(kind.clone()).or_insert(frame);
        if frame >= first + SETTLE_FRAMES && !due.contains(&kind) {
            due.push(kind);
        }
    }
    // One picture a frame: several types can become due together.
    if let Some(kind) = due.into_iter().next() {
        let path = pilot.dir.join(format!("spell-{kind}.png"));
        commands
            .spawn(Screenshot::image(pilot.target.clone()))
            .observe(save_to_disk(path));
        gallery.taken.insert(kind);
    }
}

/// Everything the spell sync test covers, cast one after another.
pub(super) const SPELL_GALLERY_SCRIPT: &[Step] = &[
    (300, [0., 0.5], Act::Idle, "joined"),
    // Beams.
    (2, [0., 0.5], Act::Arcane, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (90, [0., 0.5], Act::Cast, "beam_arcane"),
    (50, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Life, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (90, [0., 0.5], Act::Cast, "beam_life"),
    (50, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Lightning, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (60, [0., 0.5], Act::Cast, "lightning"),
    (50, [0., 0.5], Act::Idle, "e"),
    // Sprays.
    (2, [0., 0.5], Act::Fire, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (80, [0., 0.5], Act::Cast, "spray_fire"),
    (50, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Water, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (80, [0., 0.5], Act::Cast, "spray_water"),
    (50, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Cold, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (80, [0., 0.5], Act::Cast, "spray_cold"),
    (50, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Water, "e"),
    (6, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Fire, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (80, [0., 0.5], Act::Cast, "spray_steam"),
    (50, [0., 0.5], Act::Idle, "e"),
    // Projectiles.
    (2, [0., 0.5], Act::Earth, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (60, [0., 0.5], Act::Cast, "rock"),
    (90, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Water, "e"),
    (6, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Cold, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (60, [0., 0.5], Act::Cast, "ice"),
    (90, [0., 0.5], Act::Idle, "e"),
    // Blasts.
    (2, [0., 0.5], Act::Arcane, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (30, [0., 0.5], Act::AreaCast, "arcane_blast"),
    (60, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Fire, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (30, [0., 0.5], Act::AreaCast, "fire_blast"),
    (60, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Earth, "e"),
    (6, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Fire, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (30, [0., 0.5], Act::AreaCast, "earth_fire_blast"),
    (60, [0., 0.5], Act::Idle, "e"),
    // Shields, then what stays on the ground.
    (2, [0., 0.5], Act::Shield, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (30, [0., 0.5], Act::Cast, "shield_disc"),
    (60, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Shield, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (30, [0., 0.5], Act::AreaCast, "shield_ring"),
    (60, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Shield, "e"),
    (6, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Earth, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (30, [0., 0.5], Act::Cast, "barrier"),
    (60, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Shield, "e"),
    (6, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Arcane, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (30, [0., 0.5], Act::Cast, "mines"),
    (60, [0., 0.5], Act::Idle, "e"),
    // Self-cast armour.
    (2, [0., 0.5], Act::Shield, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (12, [0., 0.5], Act::SelfCast, "armour_shield"),
    (50, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Shield, "e"),
    (6, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Earth, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (12, [0., 0.5], Act::SelfCast, "armour_earth"),
    (50, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Shield, "e"),
    (6, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Water, "e"),
    (6, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Cold, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (12, [0., 0.5], Act::SelfCast, "armour_ice"),
    (50, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Shield, "e"),
    (6, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Earth, "e"),
    (6, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Water, "e"),
    (6, [0., 0.5], Act::Idle, "e"),
    (2, [0., 0.5], Act::Cold, "e"),
    (14, [0., 0.5], Act::Idle, "e"),
    (12, [0., 0.5], Act::SelfCast, "armour_iced_earth"),
    (90, [0., 0.5], Act::Idle, "e"),
];
/// A host with players connected, showing the pause menu's Players page.
pub(super) const PLAYERS_PAGE_SCRIPT: &[Step] = &[
    (300, [0., 0.5], Act::Idle, "joined"),
    (60, [0., 0.5], Act::PlayersPage, "players_page"),
];
/// A window that watches: the shell ends it when the caster is done.
pub(super) const SPELL_OBSERVER_SCRIPT: &[Step] = &[(100_000, [0., 0.5], Act::Idle, "watching")];
