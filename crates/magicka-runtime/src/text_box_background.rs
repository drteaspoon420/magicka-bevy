//! TextBoxEffect's sixteen-vertex grid and decoded pixel-shader equation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackgroundVertex {
    pub pixel_position: [f32; 2],
    pub uv: [f32; 2],
}
pub const BORDER_SIZE: f32 = 32.0;
pub const INDICES: [u16; 54] = [
    0, 1, 4, 1, 5, 4, 1, 2, 5, 2, 6, 5, 2, 3, 6, 3, 7, 6, 4, 5, 8, 5, 9, 8, 5, 6, 9, 6, 10, 9, 6,
    7, 10, 7, 11, 10, 8, 9, 12, 9, 13, 12, 9, 10, 13, 10, 14, 13, 10, 11, 14, 11, 15, 14,
];
/// Native vertex shader outputs pixel coordinates with its explicit -.5 offset.
/// D3D9 versus modern raster-center equivalence is a separate renderer concern.
pub fn vertices(center: [f32; 2], size: [f32; 2], scale: f32) -> [BackgroundVertex; 16] {
    let base = [-0.5, -0.5, 0.5, 0.5];
    let normal = [-1.0, 0.0, 0.0, 1.0];
    let uv = [0.0, 0.25, 0.75, 1.0];
    std::array::from_fn(|i| {
        let x = i % 4;
        let y = i / 4;
        BackgroundVertex {
            pixel_position: [
                (base[x] * size[0] + normal[x] * BORDER_SIZE) * scale + center[0] - 0.5,
                (base[y] * size[1] + normal[y] * BORDER_SIZE) * scale + center[1] - 0.5,
            ],
            uv: [uv[x], uv[y]],
        }
    })
}
/// texld/mul/exp/add from TextBoxEffect's pixel shader, without framebuffer clamp.
pub fn shade(sample: [f32; 4], color: [f32; 4]) -> [f32; 4] {
    let product = std::array::from_fn::<_, 4, _>(|i| sample[i] * color[i]);
    [
        1.0 - (-product[0]).exp2(),
        1.0 - (-product[1]).exp2(),
        1.0 - (-product[2]).exp2(),
        product[3],
    ]
}
