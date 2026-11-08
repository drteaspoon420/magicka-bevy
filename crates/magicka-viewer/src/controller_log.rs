//! Opt-in `--controller-log FILE`: JSON lines of the resolved controller state
//! during ordinary play, written whenever it changes. Records keyboard/mouse
//! activity in the same frame, which exposes desktop input injected for a pad
//! (for example Steam's desktop layout through its virtual keyboard/mouse).
use super::*;
use bevy::input::mouse::AccumulatedMouseMotion;
use std::io::Write;

#[derive(Resource)]
struct Log {
    file: std::io::BufWriter<std::fs::File>,
    start: std::time::Instant,
    last: String,
}

pub(super) fn install(app: &mut App) {
    let args: Vec<_> = std::env::args().collect();
    let Some(path) = args
        .windows(2)
        .find(|a| a[0] == "--controller-log")
        .map(|a| a[1].clone())
    else {
        return;
    };
    let file = std::fs::File::create(&path).expect("controller log file");
    app.insert_resource(Log {
        file: std::io::BufWriter::new(file),
        start: std::time::Instant::now(),
        last: String::new(),
    })
    .add_systems(PreUpdate, write.after(ControllerCollect));
}

#[allow(clippy::too_many_arguments)]
fn write(
    mut log: ResMut<Log>,
    state: Res<ControllerInput>,
    pads: Query<(Entity, &Gamepad, Option<&Name>)>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mouse: Option<Res<ButtonInput<MouseButton>>>,
    motion: Option<Res<AccumulatedMouseMotion>>,
    play: Option<Res<crate::play::Play>>,
) {
    let keys: Vec<_> = keys
        .iter()
        .flat_map(|k| k.get_pressed())
        .map(|k| format!("{k:?}"))
        .collect();
    let buttons: Vec<_> = mouse
        .iter()
        .flat_map(|m| m.get_pressed())
        .map(|b| format!("{b:?}"))
        .collect();
    let mouse_motion = motion.map_or(0.0, |m| m.delta.length());
    let pad_list: Vec<_> = pads
        .iter()
        .map(|(e, p, n)| {
            serde_json::json!({"entity": e.to_bits(), "name": n.map(|n| n.as_str()),
                "vendor": p.vendor_id(), "product": p.product_id()})
        })
        .collect();
    let avatar = play.as_ref().filter(|_| state.active).map(|play| {
        let input = state.player_input(&play.session, 16.0 / 9.0);
        serde_json::json!({
            "pad_movement": input.pad_movement, "attack": input.pad_attack,
            "block": input.pad_block, "cast": input.cast_down, "self_cast": input.cast_self_down,
            "area": input.shift_down, "boost": input.boost_down, "interact": input.move_down,
            "elements": input.elements, "magick_prev": input.magick_prev,
            "magick_next": input.magick_next, "inventory": input.inventory,
        })
    });
    let record = serde_json::json!({
        "active": state.active,
        "exclusive": state.exclusive,
        "owner": state.entity.map(|e| e.to_bits()),
        "family": state.family.label(),
        "style": format!("{:?}", state.element_style),
        "in_game": state.in_game,
        "help": state.help,
        "confirm": state.confirm, "back": state.back, "pause": state.pause,
        "direction": state.direction,
        "pads": pad_list,
        "keys": keys, "mouse_buttons": buttons,
        "mouse_moving": mouse_motion > 2.0,
        "player": avatar,
        "body": play.as_ref().map(|p| {
            let b = p.session.body().position;
            [(b[0] * 10.0).round() / 10.0, (b[2] * 10.0).round() / 10.0]
        }),
    })
    .to_string();
    if record == log.last {
        return;
    }
    let t = log.start.elapsed().as_secs_f32();
    let line = format!("{{\"t\":{t:.3},{}", &record[1..]);
    let _ = writeln!(log.file, "{line}");
    let _ = log.file.flush();
    log.last = record;
}
