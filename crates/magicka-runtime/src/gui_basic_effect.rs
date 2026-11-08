//! Decoded GUIBasicEffect 2D/3D pixel math; sampling is supplied by the caller.
#[derive(Clone, Copy, Debug)]
pub struct GuiBasicSettings {
    pub color: [f32; 4],
    pub texture_enabled: bool,
    pub vertex_color_enabled: bool,
    pub overlay_enabled: bool,
    pub overlay_tint: [f32; 4],
    pub saturation: f32,
    pub scale_to_hdr: bool,
}
impl Default for GuiBasicSettings {
    fn default() -> Self {
        Self {
            color: [0.; 4],
            texture_enabled: false,
            vertex_color_enabled: false,
            overlay_enabled: false,
            overlay_tint: [0.; 4],
            saturation: 1.,
            scale_to_hdr: true,
        }
    }
}
pub fn shade(
    settings: GuiBasicSettings,
    sample: [f32; 4],
    overlay: [f32; 4],
    vertex: [f32; 4],
) -> [f32; 4] {
    let color = std::array::from_fn::<_, 4, _>(|i| {
        settings.color[i]
            * if settings.vertex_color_enabled {
                vertex[i]
            } else {
                1.
            }
    });
    let mut rgb = std::array::from_fn::<_, 3, _>(|i| {
        color[i]
            * if settings.texture_enabled {
                sample[i]
            } else {
                1.
            }
    });
    let mut coverage = if settings.texture_enabled {
        sample[3]
    } else {
        1.
    };
    if settings.overlay_enabled {
        let alpha = overlay[3] * settings.overlay_tint[3];
        for i in 0..3 {
            let x = overlay[i];
            let tint = settings.overlay_tint[i];
            let blended = if tint >= 0.5 {
                x.sqrt() * (2. * tint - 1.) + 2. * x * (1. - tint)
            } else {
                x * x * (1. - 2. * tint) + 2. * x * tint
            };
            rgb[i] = rgb[i] * coverage + blended * alpha * (1. - coverage);
        }
        coverage = coverage * (1. - alpha) + alpha;
    }
    let luminance = rgb[0] * 0.3086 + rgb[1] * 0.6094 + rgb[2] * 0.082;
    for channel in &mut rgb {
        *channel = *channel * settings.saturation + luminance * (1. - settings.saturation);
        if settings.scale_to_hdr {
            *channel = 1. - (-*channel).exp2();
        }
    }
    [rgb[0], rgb[1], rgb[2], coverage * color[3]]
}
