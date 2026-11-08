//! Physical gamepad probe through the viewer's own Bevy/gilrs input stack.
//!
//! Prints one JSON line per connection, button and axis change, plus the
//! family the viewer would pick. No window, no game content.
//!
//! cargo run --locked -j 2 -p magicka-viewer --example gamepad_probe -- 30
use bevy::app::ScheduleRunnerPlugin;
use bevy::input::gamepad::{
    GamepadAxisChangedEvent, GamepadButtonChangedEvent, GamepadConnection, GamepadConnectionEvent,
};
use bevy::prelude::*;
use std::time::{Duration, Instant};

#[derive(Resource)]
struct Probe {
    start: Instant,
    seconds: f32,
    axes: std::collections::HashMap<(Entity, String), f32>,
}

fn main() {
    let seconds = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20.0);
    App::new()
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(4))),
            bevy::input::InputPlugin,
            bevy::gilrs::GilrsPlugin,
        ))
        .insert_resource(Probe {
            start: Instant::now(),
            seconds,
            axes: Default::default(),
        })
        .add_systems(Update, report)
        .run();
}

fn report(
    mut probe: ResMut<Probe>,
    mut connections: MessageReader<GamepadConnectionEvent>,
    mut buttons: MessageReader<GamepadButtonChangedEvent>,
    mut axes: MessageReader<GamepadAxisChangedEvent>,
    mut exit: MessageWriter<AppExit>,
) {
    let t = probe.start.elapsed().as_secs_f32();
    for c in connections.read() {
        match &c.connection {
            GamepadConnection::Connected {
                name,
                vendor_id,
                product_id,
            } => println!(
                "{}",
                serde_json::json!({"t": t, "event": "connected", "entity": c.gamepad.to_bits(),
                    "name": name, "vendor": vendor_id, "product": product_id,
                    "viewer_family": family(name, *vendor_id)})
            ),
            GamepadConnection::Disconnected => println!(
                "{}",
                serde_json::json!({"t": t, "event": "disconnected", "entity": c.gamepad.to_bits()})
            ),
        }
    }
    for b in buttons.read() {
        println!(
            "{}",
            serde_json::json!({"t": t, "event": "button", "button": format!("{:?}", b.button),
                "state": format!("{:?}", b.state), "value": b.value})
        );
    }
    for a in axes.read() {
        // Report each axis at 0.05 resolution to keep stick sweeps readable.
        let key = (a.entity, format!("{:?}", a.axis));
        let q = (a.value * 20.0).round() / 20.0;
        if probe.axes.get(&key) != Some(&q) {
            println!(
                "{}",
                serde_json::json!({"t": t, "event": "axis", "axis": key.1, "value": a.value})
            );
            probe.axes.insert(key, q);
        }
    }
    if t >= probe.seconds {
        exit.write(AppExit::Success);
    }
}

/// Mirrors `Family::detect` in src/controller.rs (kept private there).
fn family(name: &str, vendor: Option<u16>) -> &'static str {
    let name = name.to_lowercase();
    if name.contains("steam deck") || name.contains("neptune") {
        "Steam Deck"
    } else if name.contains("steam controller") {
        "Steam Controller"
    } else if vendor == Some(0x054c)
        || name.contains("dualsense")
        || name.contains("playstation")
        || name.contains("ps5")
    {
        "PS5"
    } else {
        "Xbox"
    }
}
