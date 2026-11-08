//! Mod UI panels (rewrite extension, plan H5): declarative `ui/*.kv`
//! panels drawn from replicated mod net tables. F4 shows or hides them;
//! the number keys choose a listed entry, which becomes a mod request (a
//! reliable `ModRequest` from clients, a direct call on the host). The
//! server decides every outcome; the panel only displays and asks.
use super::*;
use magicka_runtime::mod_runtime::ModRuntime;

#[derive(Component)]
pub(super) struct ModPanelOverlay;

#[derive(Default)]
pub(super) struct PanelState {
    visible: bool,
    status: String,
    shown: String,
    next_request: u64,
}

const DIGITS: [KeyCode; 9] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];

#[allow(clippy::type_complexity)]
pub(super) fn mod_panel(
    mut play: ResMut<Play>,
    (mut network, mut mods): (
        Option<NonSendMut<network::NetworkPlay>>,
        Option<NonSendMut<ModRuntime>>,
    ),
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut text: Query<
        (&mut NativeBitmapText, &mut Transform, &mut Visibility),
        With<ModPanelOverlay>,
    >,
    mut state: Local<PanelState>,
) {
    if state.next_request == 0 {
        // Request IDs stay unique across reconnects of this client.
        state.next_request = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_millis() as u64)
            << 8;
    }
    if keys.as_ref().is_some_and(|k| k.just_pressed(KeyCode::F4)) {
        state.visible = !state.visible;
    }
    let pressed = keys
        .as_ref()
        .and_then(|k| DIGITS.iter().position(|d| k.just_pressed(*d)));
    // Panels, replicated entries and this player's slot.
    let (panels, localization, entries, slot) = match (network.as_deref_mut(), mods.as_deref()) {
        (Some(network::NetworkPlay::Client(c)), _) => {
            if let Some(reply) = c.take_mod_reply() {
                state.status = reply;
            }
            let Some(defs) = c.definitions.clone() else {
                return;
            };
            let Some(slot) = c.own_slot() else { return };
            (
                defs.panels.clone(),
                defs.localization.clone(),
                c.frame.mod_ui.clone(),
                slot,
            )
        }
        (_, Some(m)) => (
            m.definitions.panels.clone(),
            m.definitions.localization.clone(),
            magicka_runtime::mod_ui::entries(&m.scripts.net_tables()),
            0,
        ),
        _ => return,
    };
    let views: Vec<_> = panels
        .iter()
        .map(|p| magicka_runtime::mod_ui::render(p, &entries, slot, &localization))
        .collect();
    if let (true, Some(choice)) = (state.visible, pressed) {
        // The number keys pick from the first panel with a list.
        if let Some((view, (_, item))) = views
            .iter()
            .find_map(|v| v.choices.get(choice).map(|c| (v, c)))
        {
            let action = view.action.clone().unwrap_or_default();
            state.next_request += 1;
            let id = state.next_request;
            let data = serde_json::json!({ "choice": choice, "item": item });
            state.status = match (network.as_deref_mut(), mods.as_deref_mut()) {
                (Some(network::NetworkPlay::Client(c)), _) => {
                    match c.mod_request(id, &action, &data) {
                        Ok(()) => "...".into(),
                        Err(e) => e,
                    }
                }
                (_, Some(m)) => describe(&m.request(&mut play.session, slot, id, &action, &data)),
                _ => String::new(),
            };
        }
    }
    let mut lines = Vec::new();
    if state.visible {
        for v in &views {
            lines.push(v.title.clone());
            lines.extend(v.lines.iter().cloned());
            for (i, (text, _)) in v.choices.iter().enumerate() {
                lines.push(format!("{}. {text}", i + 1));
            }
        }
        if !state.status.is_empty() {
            lines.push(state.status.clone());
        }
    } else if !views.is_empty() {
        lines.push("F4: mod panel".into());
    }
    let lines = lines.join("\n");
    for (mut text, mut transform, mut visibility) in &mut text {
        *visibility = if lines.is_empty() {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
        transform.translation =
            Vec3::new(play.screen[0] * 0.5 - 520., play.screen[1] * 0.5 - 72., 10.);
        if state.shown != lines {
            text.typing.set_text(Some(&lines));
            state.shown = lines.clone();
        }
        if !text.typing.dirty {
            text.typing.finish();
        }
    }
}

/// A request result as one status line.
pub(super) fn describe(result: &serde_json::Value) -> String {
    match result.get("error").and_then(|e| e.as_str()) {
        Some(e) => e.to_string(),
        None => "Done".into(),
    }
}
