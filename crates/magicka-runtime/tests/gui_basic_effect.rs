use magicka_runtime::gui_basic_effect::{shade, GuiBasicSettings};
#[test]
fn default_hdr_texture_color_and_alpha_are_separate() {
    let settings = GuiBasicSettings {
        color: [1., 2., 3., 0.5],
        texture_enabled: true,
        ..Default::default()
    };
    let value = shade(settings, [1., 0.5, 0., 0.25], [0.; 4], [9.; 4]);
    assert_eq!(value, [0.5, 0.5, 0., 0.125]);
}
#[test]
fn saturation_and_overlay_coverage_follow_pixel_bytecode() {
    let settings = GuiBasicSettings {
        color: [1.; 4],
        texture_enabled: true,
        saturation: 0.,
        scale_to_hdr: false,
        ..Default::default()
    };
    let value = shade(settings, [1., 0., 0., 0.5], [0.; 4], [0.; 4]);
    assert_eq!(value, [0.3086, 0.3086, 0.3086, 0.5]);
    let settings = GuiBasicSettings {
        overlay_enabled: true,
        overlay_tint: [0.5, 0.5, 0.5, 1.],
        saturation: 1.,
        ..settings
    };
    let value = shade(settings, [1., 0., 0., 0.5], [0.2, 0.4, 0.6, 0.5], [0.; 4]);
    assert_eq!(value, [0.55, 0.1, 0.15, 0.75]);
}
