//! Play-session lifecycle inside a longer-lived application. The original
//! pushes PlayState over the menu's Tome on one device and window, and pops it
//! back on quit or after the credits. A standalone `--play` process starts one
//! session at startup; the menu host begins and ends any number of sessions
//! in its own window.
use super::Play;
use bevy::{
    ecs::{
        intern::Interned,
        resource::IsResource,
        schedule::{ScheduleConfigs, ScheduleLabel},
        system::{ScheduleSystem, SystemIdMarker},
    },
    prelude::*,
    window::Monitor,
};
use std::collections::HashSet;

/// Every per-frame play system. It runs only while a session exists.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PlaySystems;
/// Play-module `Startup` systems, run when a session begins.
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PlayStartup;
/// Play-module `PostStartup` systems, run after [`PlayStartup`].
#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PlayPostStartup;

/// The session asks to return to the front end (pause-menu quit, or the end
/// of the credits). Autosave observes it before the session is torn down.
#[derive(Message, Clone, Copy, Debug, Default)]
pub(crate) struct LeaveSession {
    /// The credits ended (the Tome then closes to its back cover).
    pub credits: bool,
    /// A menu-hosted campaign level is complete: the host continues with
    /// the next chapter (SubMenuEndGame, then SubMenuCutscene).
    pub level_complete: bool,
}

/// Ordering anchor in `Last`: saving precedes it; leaving the session
/// (process exit or host teardown) belongs to it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SessionEnd;

/// Session-scoped resource resets, applied as each session begins.
#[derive(Resource, Default)]
struct SessionResets(Vec<fn(&mut World)>);

/// Entities and resources that belong to the host rather than to a session,
/// captured when the host hands the window to a session.
#[derive(Resource)]
pub(crate) struct HostBaseline {
    entities: HashSet<Entity>,
    resources: HashSet<bevy::ecs::component::ComponentId>,
}

const GATED: [fn() -> Interned<dyn ScheduleLabel>; 5] = [
    || PreUpdate.intern(),
    || Update.intern(),
    || PostUpdate.intern(),
    || Last.intern(),
    || FixedUpdate.intern(),
];

pub(crate) fn configure(app: &mut App) {
    app.add_message::<LeaveSession>()
        .init_resource::<SessionResets>()
        .init_schedule(PlayStartup)
        .init_schedule(PlayPostStartup);
    for label in GATED {
        app.configure_sets(label(), PlaySystems.run_if(resource_exists::<Play>));
    }
}

pub(crate) trait PlayAppExt {
    /// `add_systems` for the play module: startup work moves to the session
    /// schedules and frame work joins [`PlaySystems`].
    fn add_play_systems<M>(
        &mut self,
        schedule: impl ScheduleLabel,
        systems: impl IntoScheduleConfigs<ScheduleSystem, M>,
    ) -> &mut Self;
    /// `init_resource` whose value is rebuilt for every session.
    fn init_play_resource<R: Resource + Default>(&mut self) -> &mut Self;
    /// Clear host-lifetime state (caches survive) when a session begins/ends.
    fn add_session_reset(&mut self, reset: fn(&mut World)) -> &mut Self;
}
impl PlayAppExt for App {
    fn add_play_systems<M>(
        &mut self,
        schedule: impl ScheduleLabel,
        systems: impl IntoScheduleConfigs<ScheduleSystem, M>,
    ) -> &mut Self {
        let label = schedule.intern();
        if label == Startup.intern() {
            self.add_systems(PlayStartup, systems)
        } else if label == PostStartup.intern() {
            self.add_systems(PlayPostStartup, systems)
        } else {
            let systems: ScheduleConfigs<ScheduleSystem> =
                systems.into_configs().in_set(PlaySystems);
            self.add_systems(label, systems)
        }
    }
    fn init_play_resource<R: Resource + Default>(&mut self) -> &mut Self {
        self.init_resource::<R>();
        self.add_session_reset(|world| world.insert_resource(R::default()))
    }
    fn add_session_reset(&mut self, reset: fn(&mut World)) -> &mut Self {
        self.world_mut()
            .resource_mut::<SessionResets>()
            .0
            .push(reset);
        self
    }
}

pub(crate) fn capture_baseline(world: &mut World) {
    let entities = world.iter_entities().map(|e| e.id()).collect();
    let resources = world.iter_resources().map(|(info, _)| info.id()).collect();
    world.insert_resource(HostBaseline {
        entities,
        resources,
    });
}

/// Reset session-scoped resources and run the session startup schedules.
pub(crate) fn run_startup(world: &mut World) {
    reset(world);
    world.run_schedule(PlayStartup);
    world.run_schedule(PlayPostStartup);
}
fn reset(world: &mut World) {
    let resets = world.resource::<SessionResets>().0.clone();
    for reset in resets {
        reset(world);
    }
}

/// Despawn everything the session created and drop its resources, returning
/// the window to the host captured in [`HostBaseline`].
pub(crate) fn tear_down(world: &mut World) {
    let Some(baseline) = world.remove_resource::<HostBaseline>() else {
        return;
    };
    let doomed: Vec<Entity> = world
        .iter_entities()
        .filter(|e| {
            !baseline.entities.contains(&e.id())
                && !e.contains::<IsResource>()
                && !e.contains::<Window>()
                && !e.contains::<Monitor>()
                // Devices connected during play still belong to the input
                // backend after returning to the menu or entering a chapter.
                && !e.contains::<Gamepad>()
                && !e.contains::<SystemIdMarker>()
        })
        .map(|e| e.id())
        .collect();
    for entity in doomed {
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }
    // Session resources are play-module types (plus the few crate-level
    // scene descriptors the play module inserts per session).
    let doomed: Vec<_> = world
        .iter_resources()
        .filter(|(info, _)| {
            let name = info.name().to_string();
            !baseline.resources.contains(&info.id())
                && (name.starts_with("magicka_viewer::play::")
                    || name.starts_with("magicka_viewer::terrain::"))
        })
        .map(|(info, _)| info.id())
        .collect();
    for id in doomed {
        world.remove_resource_by_id(id);
    }
    world.remove_non_send::<super::network::NetworkPlay>();
    reset(world);
    // The pause menu stops virtual time; the host's animations use it too.
    let mut time = world.resource_mut::<Time<Virtual>>();
    time.unpause();
    time.set_relative_speed(1.0);
    world.insert_resource(ClearColor(Color::BLACK));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teardown_keeps_controllers_connected_after_the_session_started() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        configure(&mut app);
        let existing = app.world_mut().spawn(Gamepad::default()).id();
        capture_baseline(app.world_mut());
        let mut pad = Gamepad::default();
        pad.analog_mut().set(GamepadAxis::LeftStickX, 0.7);
        let hot_joiner = app.world_mut().spawn(pad).id();
        let scene_entity = app.world_mut().spawn_empty().id();
        app.world_mut().resource_mut::<Time<Virtual>>().pause();
        tear_down(app.world_mut());
        assert!(app.world().get::<Gamepad>(existing).is_some());
        assert_eq!(
            app.world()
                .get::<Gamepad>(hot_joiner)
                .unwrap()
                .get(GamepadAxis::LeftStickX),
            Some(0.7)
        );
        assert!(app.world().get_entity(scene_entity).is_err());
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
    }
}
