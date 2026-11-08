use magicka_runtime::text_box_background::*;
#[test]
fn grid_retains_native_inner_bounds_border_uv_and_half_pixel() {
    let v = vertices([100., 80.], [200., 40.], 1.);
    assert_eq!(v[0].pixel_position, [-32.5, 27.5]);
    assert_eq!(v[5].pixel_position, [-0.5, 59.5]);
    assert_eq!(v[10].pixel_position, [199.5, 99.5]);
    assert_eq!(v[15].pixel_position, [231.5, 131.5]);
    assert_eq!(v[5].uv, [0.25, 0.25]);
    assert_eq!(v[10].uv, [0.75, 0.75]);
    assert!(INDICES.iter().all(|&i| i < 16));
    assert_eq!(INDICES.len() / 3, 18);
    let half = vertices([100., 80.], [200., 40.], 0.5);
    assert_eq!(half[0].pixel_position, [33.5, 53.5]);
    assert!(vertices([100., 80.], [200., 40.], 0.)
        .iter()
        .all(|v| v.pixel_position == [99.5, 79.5]));
}
#[test]
fn hdr_pixel_mapping_preserves_alpha_and_does_not_clamp_inputs() {
    assert_eq!(shade([1., 2., 0., 0.25], [1.; 4]), [0.5, 0.75, 0., 0.25]);
    assert_eq!(
        shade([1., 2., 0., 0.25], [2., 0.5, 1., 0.5]),
        [0.75, 0.5, 0., 0.125]
    );
    assert_eq!(shade([-1., 0., 0., 2.], [1.; 4]), [-1., 0., 0., 2.]);
}
