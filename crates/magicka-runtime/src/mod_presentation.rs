//! Client-side mod presentation (protocol 12): one-shot particles and
//! sounds the host announced, played locally once each. Never gameplay.
use crate::play_session::PlaySession;
use crate::scene_loading::custom_hash;
use magicka_definitions::{DefinitionId, Definitions};
use magicka_network::session::{GameFrame, PresentationEventState};
use std::collections::BTreeSet;

/// Plays each host event once. Events already in the first frame a client
/// sees are history (a late joiner does not replay them).
#[derive(Default)]
pub struct RemoteEvents {
    session: u64,
    primed: bool,
    seen: BTreeSet<u64>,
}
impl RemoteEvents {
    pub fn take(&mut self, frame: &GameFrame) -> Vec<PresentationEventState> {
        if self.session != frame.physics.session {
            *self = Self {
                session: frame.physics.session,
                ..Default::default()
            };
        }
        let fresh: Vec<_> = frame
            .events
            .iter()
            .filter(|e| self.seen.insert(e.id))
            .cloned()
            .collect();
        // Forget IDs no longer in snapshots (they never return).
        let live: BTreeSet<u64> = frame.events.iter().map(|e| e.id).collect();
        let oldest = live.iter().next().copied().unwrap_or(u64::MAX);
        self.seen.retain(|id| *id >= oldest || live.contains(id));
        if !self.primed {
            self.primed = true;
            return Vec::new();
        }
        fresh
    }
}

/// A presentation name: a registry effect ID (resolved to its asset), a
/// `content:` path, or a retail effect/cue name.
pub fn resolve(defs: Option<&Definitions>, name: &str) -> String {
    let asset = DefinitionId::parse(name, "base")
        .ok()
        .filter(|_| name.contains(':') && !name.starts_with("content:"))
        .and_then(|id| defs?.effects.get(&id).map(|e| e.asset.clone()))
        .unwrap_or_else(|| name.to_string());
    asset
        .strip_prefix("content:")
        .unwrap_or(&asset)
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim_end_matches(".xml")
        .to_string()
}

/// `Bank/cue` (bank by Magicka.Audio.Banks name) or a Misc-bank cue.
fn sound(name: &str) -> (u16, String) {
    use crate::audio::banks::*;
    let full = name.strip_prefix("content:").unwrap_or(name);
    match full.split_once('/') {
        Some((bank, cue)) => {
            let bank = match bank.to_ascii_lowercase().as_str() {
                "wave" | "wavebank" => WAVE_BANK,
                "music" => MUSIC,
                "ambience" => AMBIENCE,
                "ui" => UI,
                "spells" => SPELLS,
                "characters" => CHARACTERS,
                "footsteps" => FOOTSTEPS,
                "weapons" => WEAPONS,
                "additional" => ADDITIONAL,
                _ => MISC,
            };
            (bank, cue.to_string())
        }
        None => (MISC, full.to_string()),
    }
}

/// Play events on this computer's session (effects and audio only).
pub fn play(
    session: &mut PlaySession,
    defs: Option<&Definitions>,
    events: &[PresentationEventState],
) {
    for e in events {
        if e.sound {
            let name = defs
                .and_then(|d| {
                    let id = DefinitionId::parse(&e.name, "base").ok()?;
                    d.effects.get(&id).map(|x| x.asset.clone())
                })
                .unwrap_or_else(|| e.name.clone());
            let (bank, cue) = sound(&name);
            session.world.play_cue(bank, &cue, e.position);
        } else if let Some(p) = e.position {
            let effect = resolve(defs, &e.name);
            let hash = custom_hash(&effect.to_ascii_lowercase());
            if session
                .effects
                .start_at(hash, p, [0.0, 0.0, -1.0])
                .is_none()
            {
                session
                    .world
                    .log
                    .push(format!("mod effect {effect} unavailable"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magicka_network::{PhysicsFrame, PROTOCOL_VERSION};

    fn frame(session: u64, ids: &[u64]) -> GameFrame {
        GameFrame {
            physics: PhysicsFrame {
                version: PROTOCOL_VERSION,
                session,
                epoch: 1,
                tick: 1,
                scene: "lab".into(),
                bodies: vec![],
            },
            level: "x.lvl".into(),
            players: vec![],
            npcs: vec![],
            effects: vec![],
            rules: None,
            lobby: Default::default(),
            participants: vec![],
            world: Default::default(),
            mod_ui: vec![],
            time: 0.0,
            events: ids
                .iter()
                .map(|&id| PresentationEventState {
                    id,
                    sound: false,
                    name: "fx".into(),
                    position: Some([0.0; 3]),
                    owner: None,
                    tick: 1,
                })
                .collect(),
        }
    }

    #[test]
    fn events_play_once_and_late_joiners_skip_history() {
        let mut r = RemoteEvents::default();
        assert!(r.take(&frame(1, &[4, 5])).is_empty(), "history on join");
        let ids = |v: Vec<PresentationEventState>| v.into_iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(ids(r.take(&frame(1, &[4, 5, 6]))), [6]);
        assert_eq!(ids(r.take(&frame(1, &[5, 6, 7]))), [7]);
        assert_eq!(ids(r.take(&frame(1, &[7]))), Vec::<u64>::new());
        // A new host session starts over.
        assert!(r.take(&frame(2, &[1])).is_empty());
        assert_eq!(ids(r.take(&frame(2, &[1, 2]))), [2]);
    }

    #[test]
    fn names_resolve_through_registry_content_paths_and_banks() {
        assert_eq!(
            resolve(None, "content:Effects/fire_blast.xml"),
            "fire_blast"
        );
        assert_eq!(resolve(None, "spark"), "spark");
        assert_eq!(
            sound("spells/storm"),
            (crate::audio::banks::SPELLS, "storm".into())
        );
        assert_eq!(sound("ding").0, crate::audio::banks::MISC);
    }
}
