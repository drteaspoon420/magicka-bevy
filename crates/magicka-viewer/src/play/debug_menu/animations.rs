//! Pose-only player preview: freeze simulation and never execute clip actions.
use super::{egui, Play};
use bevy::prelude::*;
use magicka_runtime::animation_controller::{animation_name, AnimationController, ClipCue};
use std::collections::BTreeSet;

#[derive(Resource)]
pub(in crate::play) struct AnimationPreview {
    pub enabled: bool,
    playing: bool,
    selected: u16,
    speed: f32,
    seek: Option<f32>,
    started: bool,
    saved: Option<(usize, AnimationController)>,
}
impl Default for AnimationPreview {
    fn default() -> Self {
        Self {
            enabled: false,
            playing: true,
            selected: 1,
            speed: 1.0,
            seek: None,
            started: false,
            saved: None,
        }
    }
}

impl AnimationPreview {
    pub(super) fn from_argument(name: Option<&str>) -> Self {
        let mut preview = Self::default();
        if let Some(id) = name.and_then(magicka_runtime::animation_controller::animation_value) {
            preview.enabled = true;
            preview.selected = id;
        }
        preview
    }
}

pub(super) fn section(ui: &mut egui::Ui, preview: &mut AnimationPreview, play: &Play) {
    ui.checkbox(
        &mut preview.enabled,
        "Preview player animation (freeze gameplay)",
    );
    ui.label("Pose preview skips animation actions: no damage, spells or script events.");
    let avatar = play.session.avatar();
    let set = &avatar.motion.animations;
    let ids: BTreeSet<_> = set
        .actions
        .keys()
        .chain(set.weapon_sets.iter().flat_map(|s| s.keys()))
        .copied()
        .collect();
    egui::ComboBox::from_label("Animation")
        .selected_text(animation_name(preview.selected))
        .height(400.0)
        .show_ui(ui, |ui| {
            for id in ids {
                if ui
                    .selectable_value(&mut preview.selected, id, animation_name(id))
                    .changed()
                {
                    preview.started = false;
                }
            }
        });
    ui.checkbox(&mut preview.playing, "Play");
    ui.add(egui::Slider::new(&mut preview.speed, 0.1..=3.0).text("Playback speed"));
    if preview.enabled {
        let controller = &avatar.motion.controller;
        ui.label(format!(
            "Clip: {}",
            controller.clip_name().unwrap_or("none")
        ));
        let mut position = controller.time;
        if ui
            .add(
                egui::Slider::new(&mut position, 0.0..=controller.clip_duration().max(0.001))
                    .text("Time (seconds)"),
            )
            .changed()
        {
            preview.seek = Some(position);
            preview.playing = false;
        }
        if ui.button("Restart clip").clicked() {
            preview.started = false;
        }
        if ui.button("Return to gameplay").clicked() {
            preview.enabled = false;
        }
    }
}

pub(in crate::play) fn step(preview: &mut AnimationPreview, play: &mut Play, dt: f32) -> bool {
    let world = &mut play.session.world;
    let Some(avatar) = world.players[0].as_mut() else {
        return false;
    };
    let identity = std::sync::Arc::as_ptr(&avatar.motion.animations) as usize;
    if !preview.enabled {
        if let Some((saved_identity, controller)) = preview.saved.take() {
            if identity == saved_identity {
                avatar.motion.controller = controller;
                if let Some(clip) = avatar.motion.controller.clip_name() {
                    play.pending = Some(ClipCue {
                        clip: clip.into(),
                        fade: 0.0,
                        looping: avatar.motion.controller.is_looping(),
                    });
                }
            }
        }
        preview.started = false;
        return false;
    }
    if preview
        .saved
        .as_ref()
        .is_none_or(|(key, _)| *key != identity)
    {
        preview.saved = Some((identity, avatar.motion.controller.clone()));
        preview.started = false;
    }
    let controller = &mut avatar.motion.controller;
    controller.speed = 1.0;
    if !preview.started {
        controller.force_animation(&avatar.motion.animations, preview.selected);
        play.pending = controller.cues.pop();
        controller.cues.clear();
        preview.started = true;
    }
    if let Some(position) = preview.seek.take() {
        controller.time = position.clamp(0.0, controller.clip_duration());
    } else if preview.playing {
        controller.update(dt * preview.speed);
    }
    controller.signals.clear();
    controller.cues.clear();
    avatar.finish(
        &mut world.physics.bodies[avatar.body],
        world.flags.cutscene,
        0.0,
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use magicka_runtime::play_session::PlaySession;
    use std::{collections::HashMap, path::Path};

    #[test]
    fn preview_advances_and_scrubs_pose_without_gameplay_then_restores_controller() {
        let content = magicka_content::retail_content_dir();
        if !content.is_dir() {
            return;
        }
        let mut session = PlaySession::load(&content, "Havindr.lvl", "Wizard").unwrap();
        session.disable_scripts();
        let mut play = Play {
            session,
            content,
            cache: Default::default(),
            debug_loadout: None,
            level: "Havindr.lvl".into(),
            character: "Wizard".into(),
            scripts: false,
            demo_goblin: false,
            versus: None,
            packs: None,
            pending: None,
            npc_cues: HashMap::new(),
            npc_seq: 0,
            reloads: 0,
            hide_level: false,
            scenarios: std::path::PathBuf::new(),
            dt: 0.0,
            screen: [1280.0, 720.0],
            menu_hosted: false,
            complete_after: None,
            bindings: Default::default(),
        };
        let original = play.session.avatar().motion.controller.clone();
        let position = play.session.body().position;
        let hp = play.session.avatar().character.combat.health.hit_points;
        let effects = play.session.effects.active_effects();
        let mut preview = AnimationPreview::from_argument(Some("charge_force"));
        for _ in 0..30 {
            assert!(step(&mut preview, &mut play, 1.0 / 60.0));
        }
        assert!(play.session.avatar().motion.controller.time > 0.1);
        assert_eq!(
            play.session.avatar().motion.controller.clip_name(),
            Some("cast_force_charge_build")
        );
        assert_eq!(play.session.body().position, position);
        assert_eq!(play.session.avatar().character.combat.health.hit_points, hp);
        assert!(play.session.avatar().current_spell.is_none());
        assert_eq!(play.session.effects.active_effects(), effects);
        preview.seek = Some(0.25);
        preview.playing = false;
        step(&mut preview, &mut play, 1.0);
        assert_eq!(play.session.avatar().motion.controller.time, 0.25);
        step(&mut preview, &mut play, 1.0);
        assert_eq!(play.session.avatar().motion.controller.time, 0.25);
        preview.enabled = false;
        assert!(!step(&mut preview, &mut play, 1.0));
        assert_eq!(
            play.session.avatar().motion.controller.clip_name(),
            original.clip_name()
        );
        assert_eq!(play.session.avatar().motion.controller.time, original.time);
    }
}
