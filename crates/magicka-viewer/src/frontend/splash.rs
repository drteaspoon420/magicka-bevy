//! CompanyState: the Paradox, Arrowhead, "pieces" and Magicka screens before
//! the menu. Each is a centred quad on black; a skip moves to the next.
use super::book::{Atlas, PageDraw, Widget};
use crate::frontend_tome::TomeView;
use bevy::prelude::*;

const SCREENS: [&str; 4] = [
    "UI/Company/paradox",
    "UI/Company/arrowhead",
    "UI/Company/pieces",
    "UI/Company/magicka",
];

#[derive(Resource)]
pub(crate) struct Splash {
    screen: usize,
    countdown: f32,
    alpha: f32,
    /// Fade target and rate (RenderableQuad.Fade(seconds)).
    fade: Option<(f32, f32)>,
    sizes: [[f32; 2]; 4],
    done: bool,
}

pub(crate) fn plugin(app: &mut App, content: &str, enabled: bool) -> Result<(), String> {
    let mut sizes = [[0.0; 2]; 4];
    for (size, name) in sizes.iter_mut().zip(SCREENS) {
        let path = magicka_runtime::play_content::resolve(
            std::path::Path::new(content),
            std::path::Path::new(&format!("{name}.xnb")),
        )
        .ok_or_else(|| format!("missing {name}"))?;
        let magicka_content::assets::Asset::Texture2D(texture) =
            crate::text_box_visual::read_asset(&path)?
        else {
            return Err(format!("expected texture {name}"));
        };
        *size = [texture.width as f32, texture.height as f32];
    }
    app.insert_resource(Splash {
        screen: 0,
        countdown: 3.0,
        // Initialize: the Paradox quad starts visible.
        alpha: 1.0,
        fade: None,
        sizes,
        done: !enabled,
    })
    .add_systems(Update, run.in_set(super::book::BookDrawSet));
    if enabled {
        app.add_systems(Startup, |mut tome: ResMut<TomeView>| tome.visible = false);
    }
    Ok(())
}

fn run(
    mut splash: ResMut<Splash>,
    mut tome: ResMut<TomeView>,
    mut draw: ResMut<PageDraw>,
    input: Res<super::pages::PageInput>,
    time: Res<Time>,
) {
    if splash.done {
        return;
    }
    let dt = time.delta_secs();
    if let Some((target, rate)) = splash.fade {
        splash.alpha += (target - splash.alpha).signum() * dt * rate;
        if (target - splash.alpha).abs() <= dt * rate {
            splash.alpha = target;
            splash.fade = None;
        }
    }
    let skip = input.confirm || input.back || input.released;
    splash.countdown -= dt;
    let last = splash.screen == SCREENS.len() - 1;
    if splash.countdown <= 0.0 || skip {
        let visible = splash.alpha > 0.0;
        if last {
            // UpdateMagicka: fade out (0.2 s when skipped), then the menu.
            if splash.fade.is_none() && visible && splash.alpha >= 1.0 {
                splash.fade = Some((0.0, if skip { 5.0 } else { 1.0 }));
            }
            if !visible {
                splash.done = true;
                // CompanyState.OnExit → MenuState: the Tome wakes up.
                tome.visible = true;
                return;
            }
        } else if visible && !skip {
            if splash.fade.is_none() {
                splash.fade = Some((0.0, 2.0));
            }
        } else {
            splash.screen += 1;
            splash.countdown = 3.0;
            splash.alpha = 0.0;
            splash.fade = Some((1.0, 1.0));
        }
    }
    let [w, h] = splash.sizes[splash.screen];
    let screen = input.screen;
    // The Magicka logo shrinks to fit narrow resolutions.
    let scale = if last { (screen[0] / w).min(1.0) } else { 1.0 };
    let size = [w * scale, h * scale];
    let mut out = draw.screen();
    out.push(Widget::fill([0.0, 0.0], screen, [0.0, 0.0, 0.0, 1.0]));
    out.push(
        Widget::image(
            Atlas::Content(SCREENS[splash.screen].into()),
            [0.0, 0.0, w, h],
            [(screen[0] - size[0]) * 0.5, (screen[1] - size[1]) * 0.5],
            size,
        )
        .color([1.0, 1.0, 1.0, splash.alpha.clamp(0.0, 1.0)]),
    );
}
