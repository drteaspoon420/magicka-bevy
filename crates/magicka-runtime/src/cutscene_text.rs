//! Native CutsceneText constructor/Initialize and fixed-screen PreRenderUpdate.
use crate::{
    localization::LocalizationTable,
    text_box::{TextBoxError, TextBoxState},
    text_box_initialize::{
        TextBoxFonts, TextBoxInit, TextBoxInitError, TextBoxInitHost, TextBoxInitTarget,
        TextBoxWidget,
    },
};
pub const CUTSCENE_TEXT_WIDTH: i32 = 640;
pub const CUTSCENE_BLACKBAR_SIZE: f32 = 0.1;
pub struct CutsceneTextWidget {
    pub widget: TextBoxWidget,
}
impl Default for CutsceneTextWidget {
    fn default() -> Self {
        let widget = TextBoxWidget {
            name_characters: vec![0; 32],
            ..Default::default()
        };
        Self { widget }
    }
}
impl TextBoxInitTarget for CutsceneTextWidget {
    fn state_mut(&mut self) -> &mut TextBoxState {
        &mut self.widget.state
    }
    fn initialize_target<H: TextBoxInitHost>(
        &mut self,
        init: TextBoxInit<'_>,
        strings: &LocalizationTable,
        fonts: TextBoxFonts<'_>,
        host: &mut H,
        budget: usize,
    ) -> Result<(), TextBoxInitError<H::Error>> {
        // Derived constructor replaces both Text/OwnerName with Maiandra16.
        self.widget.initialize(
            init,
            strings,
            TextBoxFonts {
                body: fonts.owner_name,
                owner_name: fonts.owner_name,
            },
            host,
            budget,
        )?;
        self.widget.state.size[0] = CUTSCENE_TEXT_WIDTH;
        Ok(())
    }
}
/// Ignores world/screen position, matrix and force flag. Clamp order and unscaled
/// dimensions follow CutSceneRenderData.PreRenderUpdate, including oversized boxes.
pub fn pre_render(
    state: &mut TextBoxState,
    channel: usize,
    dt: f32,
    screen: [f32; 2],
) -> Result<[f32; 2], TextBoxError> {
    let render = *state
        .channels
        .get(channel)
        .ok_or(TextBoxError::InvalidChannel(channel))?;
    if render.scale > 0.99 {
        state.typing.update(dt)?;
    }
    let inset = [0., (screen[1] * CUTSCENE_BLACKBAR_SIZE).floor()];
    let mut position = [screen[0] * 0.5, screen[1] * 0.8];
    for i in 0..2 {
        let half = (render.size[i] as f32 + 32.) * 0.5;
        position[i] = position[i].max(half + inset[i]);
        position[i] = position[i].min(screen[i] - half - inset[i]);
    }
    Ok(position)
}
