//! Replace the original ButtonChar atlas cells without changing text metrics.
//! Covers native bitmap text in menus, dialogue and inventory as well as hints.
use super::{glyphs, ControllerInput, Family};
use crate::native_text::{NativeBitmapText, OverlayFontMaterial};
use bevy::{
    image::{ImageSampler, ImageType},
    prelude::*,
};
use std::collections::HashMap;

#[derive(Resource, Default)]
pub(super) struct FontPrompts {
    atlases: HashMap<AssetId<Image>, [Handle<Image>; 5]>,
}
pub(super) fn button_glyph(code: u16) -> Option<usize> {
    Some(match code {
        0x320 => 16,
        0x321 => 14,
        0x322 => 15,
        0x323 => 13,
        0x324 => 0,
        0x325 => 1,
        0x326 => 2,
        0x327 => 3,
        0x328 => 10,
        0x329 => 11,
        _ => return None,
    })
}
fn composite(target: &mut Image, rect: [u32; 4], source: &Image) {
    let [x, y, w, h] = rect;
    let tw = target.width();
    let th = target.height();
    let sw = source.width();
    let sh = source.height();
    let Some(dst) = target.data.as_mut() else {
        return;
    };
    let Some(src) = source.data.as_ref() else {
        return;
    };
    if w == 0 || h == 0 || x + w > tw || y + h > th {
        return;
    }
    // Preserve the native glyph cell and fit each device's silhouette inside it.
    let scale = (w as f32 / sw as f32).min(h as f32 / sh as f32);
    let rw = (sw as f32 * scale).round().max(1.) as u32;
    let rh = (sh as f32 * scale).round().max(1.) as u32;
    for j in 0..h {
        for i in 0..w {
            let out = ((y + j) * tw + x + i) as usize * 4;
            dst[out..out + 4].fill(0);
            let ix = i as i32 - ((w - rw) / 2) as i32;
            let iy = j as i32 - ((h - rh) / 2) as i32;
            if ix < 0 || iy < 0 || ix >= rw as i32 || iy >= rh as i32 {
                continue;
            }
            // Average source coverage; small font glyphs retain smooth outlines.
            let x0 = ix as u32 * sw / rw;
            let x1 = ((ix as u32 + 1) * sw / rw).max(x0 + 1).min(sw);
            let y0 = iy as u32 * sh / rh;
            let y1 = ((iy as u32 + 1) * sh / rh).max(y0 + 1).min(sh);
            let mut sums = [0u32; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let pos = (sy * sw + sx) as usize * 4;
                    for c in 0..4 {
                        sums[c] += src[pos + c] as u32;
                    }
                }
            }
            for c in 0..4 {
                dst[out + c] = (sums[c] / ((x1 - x0) * (y1 - y0))) as u8;
            }
        }
    }
}
pub(super) fn update(
    state: Res<ControllerInput>,
    mut registry: ResMut<FontPrompts>,
    mut images: Option<ResMut<Assets<Image>>>,
    mut colors: Option<ResMut<Assets<ColorMaterial>>>,
    mut overlays: Option<ResMut<Assets<OverlayFontMaterial>>>,
    texts: Query<(
        &NativeBitmapText,
        Option<&MeshMaterial2d<ColorMaterial>>,
        Option<&MeshMaterial2d<OverlayFontMaterial>>,
    )>,
) {
    let Some(images) = images.as_mut() else {
        return;
    };
    let mut sources = Vec::new();
    for (text, color, overlay) in &texts {
        let handle = color
            .and_then(|m| colors.as_ref()?.get(&m.0)?.texture.clone())
            .or_else(|| overlay.and_then(|m| Some(overlays.as_ref()?.get(&m.0)?.texture.clone())));
        let Some(handle) = handle else { continue };
        if registry
            .atlases
            .values()
            .any(|row| row.iter().any(|h| h == &handle))
        {
            continue;
        }
        if !sources.iter().any(|(h, _)| *h == handle) {
            sources.push((handle, text.font.clone()));
        }
    }
    for (handle, font) in sources {
        let Some(original) = images.get(&handle).cloned() else {
            continue;
        };
        let row = std::array::from_fn(|i| {
            if i == 0 {
                return handle.clone();
            }
            let mut atlas = original.clone();
            for code in 0x320..=0x329 {
                let Some(index) = button_glyph(code) else {
                    continue;
                };
                let Ok(cell) = font.glyph(code) else { continue };
                if cell.character as u16 != code {
                    continue;
                }
                let png = Image::from_buffer(
                    glyphs::PNGS[i - 1][index],
                    ImageType::Extension("png"),
                    bevy::image::CompressedImageFormats::NONE,
                    false,
                    ImageSampler::default(),
                    default(),
                )
                .expect("Kenney glyph");
                composite(
                    &mut atlas,
                    [
                        cell.origin[0] as u32,
                        cell.origin[1] as u32,
                        cell.size[0] as u32,
                        cell.size[1] as u32,
                    ],
                    &png,
                );
            }
            images.add(atlas)
        });
        registry.atlases.insert(handle.id(), row);
    }
    let selected = if state.active {
        1 + Family::ALL.iter().position(|f| *f == state.family).unwrap()
    } else {
        0
    };
    let replace = |texture: &mut Handle<Image>| {
        if let Some(row) = registry
            .atlases
            .values()
            .find(|row| row.iter().any(|h| h == texture))
        {
            if *texture != row[selected] {
                *texture = row[selected].clone();
            }
        }
    };
    if let Some(colors) = colors.as_mut() {
        for (_, material) in colors.iter_mut() {
            if let Some(texture) = &mut material.texture {
                replace(texture);
            }
        }
    }
    if let Some(overlays) = overlays.as_mut() {
        for (_, material) in overlays.iter_mut() {
            replace(&mut material.texture);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_text_atlas_switches_device_glyphs_and_restores_keyboard_bytes() {
        let root = magicka_content::retail_content_dir();
        let font =
            magicka_runtime::play_content::bitmap_font(&root, "Languages/eng/Font/Maiandra14")
                .unwrap();
        let mut app = App::new();
        let mut images = Assets::<Image>::default();
        let mut colors = Assets::<ColorMaterial>::default();
        let (font, material) =
            crate::native_text::upload_font(&font, &mut images, &mut colors).unwrap();
        let original = colors.get(&material).unwrap().texture.clone().unwrap();
        let original_bytes = images.get(&original).unwrap().data.clone();
        let metrics = font.metrics.measure("\u{324} Continue", true).unwrap();
        app.insert_resource(images)
            .insert_resource(colors)
            .init_resource::<FontPrompts>()
            .init_resource::<ControllerInput>()
            .add_systems(Update, update);
        app.world_mut().spawn((
            NativeBitmapText::new(font.clone(), "\u{324} Continue", default()),
            MeshMaterial2d(material.clone()),
        ));
        let mut handles = Vec::new();
        for family in Family::ALL {
            {
                let mut state = app.world_mut().resource_mut::<ControllerInput>();
                state.active = true;
                state.family = family;
            }
            app.update();
            let texture = app
                .world()
                .resource::<Assets<ColorMaterial>>()
                .get(&material)
                .unwrap()
                .texture
                .clone()
                .unwrap();
            assert_ne!(texture, original);
            assert_ne!(
                app.world()
                    .resource::<Assets<Image>>()
                    .get(&texture)
                    .unwrap()
                    .data,
                original_bytes
            );
            assert_eq!(
                font.metrics.measure("\u{324} Continue", true).unwrap(),
                metrics
            );
            handles.push(texture);
        }
        handles.dedup();
        assert_eq!(handles.len(), 4);
        app.world_mut().resource_mut::<ControllerInput>().active = false;
        app.update();
        assert_eq!(
            app.world()
                .resource::<Assets<ColorMaterial>>()
                .get(&material)
                .unwrap()
                .texture
                .as_ref(),
            Some(&original)
        );
        assert_eq!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&original)
                .unwrap()
                .data,
            original_bytes
        );
    }
    #[test]
    fn original_button_char_positions_map_to_the_same_physical_buttons() {
        assert_eq!(
            (0x324..=0x329).map(button_glyph).collect::<Vec<_>>(),
            vec![Some(0), Some(1), Some(2), Some(3), Some(10), Some(11)]
        );
        assert_eq!(button_glyph(b'A' as u16), None);
    }
}
