//! The CPU Beam vertex stage against ArcaneEffect's bytecode, executed by
//! output/evidence/tools/check_arcane_beam.py (preshader + vs_2_0) into
//! output/fixtures/arcane_beam/vs_cases.json. Skips when the fixture (derived
//! from user-supplied content) is absent.
use magicka_runtime::beam_mesh::{rail_draws, strip_triangles, vertex, BeamDraw};
use magicka_sim::railgun::RailRender;
use std::path::Path;

fn v3(v: &serde_json::Value) -> [f32; 3] {
    std::array::from_fn(|i| v[i].as_f64().unwrap() as f32)
}

#[test]
fn cpu_beam_vertices_match_the_executed_shader() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures/arcane_beam/vs_cases.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("fixture absent; skipping");
        return;
    };
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let (mut checked, mut worst_pos, mut worst_uv) = (0, 0.0f32, 0.0f32);
    for case in json["cases"].as_array().unwrap() {
        let p = &case["params"];
        let f = |k: &str| p[k].as_f64().unwrap() as f32;
        let draw = BeamDraw {
            origin: v3(&p["origin"]),
            direction: v3(&p["direction"]),
            length: f("length"),
            cut: f("cut"),
            time: f("time"),
            clockwise: p["clockwise"].as_bool().unwrap(),
            wave_scale: f("wave_scale"),
            min_radius: f("min_radius"),
            max_radius: f("max_radius"),
            start_length: f("start_length"),
            dropoff: f("dropoff"),
            ray_radius: f("ray_radius"),
            texture_scale: f("texture_scale"),
            alpha: f("alpha"),
            color_center: [0.0; 3],
            color_edge: [0.0; 3],
        };
        let camera = v3(&p["camera"]);
        for v in case["vertices"].as_array().unwrap() {
            let index = v["index"].as_u64().unwrap() as f32;
            let side = v["side"].as_f64().unwrap() as f32;
            let x = (index / 255.0) * (index / 255.0);
            let tv = if side > 0.0 { 0.5 } else { 0.625 };
            let got = vertex(&draw, camera, x, side, tv);
            let want = v3(&v["position"]);
            let scale = 1.0f32.max(want.iter().fold(0.0f32, |m, c| m.max(c.abs())));
            let e = (0..3).fold(0.0f32, |m, i| m.max((got.position[i] - want[i]).abs())) / scale;
            worst_pos = worst_pos.max(e);
            let uv = [
                v["uv"][0].as_f64().unwrap() as f32,
                v["uv"][1].as_f64().unwrap() as f32,
            ];
            let eu =
                ((got.uv[0] - uv[0]).abs() / uv[0].abs().max(1.0)).max((got.uv[1] - uv[1]).abs());
            worst_uv = worst_uv.max(eu);
            assert!(e < 2e-5, "position {got:?} vs {want:?} ({p})");
            assert!(eu < 2e-5, "uv {:?} vs {uv:?}", got.uv);
            assert!((got.alpha - v["alpha"].as_f64().unwrap() as f32).abs() < 1e-6);
            checked += 1;
        }
    }
    eprintln!("{checked} vertices; worst relative position {worst_pos:e}, uv {worst_uv:e}");
    assert!(checked > 800);
}

#[test]
fn render_data_draw_sequence_and_strip_indices() {
    let r = RailRender {
        rail: 0,
        position: [0.0; 3],
        direction: [1.0, 0.0, 0.0],
        length: 10.0,
        cut: 0.0,
        time: 3.0,
        color_center: [0.666; 3],
        color_edge: [0.333; 3],
        spirals: 4,
        branch: false,
        additional: magicka_sim::spells::Elements::NONE,
        light: ([0.0; 3], [0.0; 3]),
        light_color: [1.0; 3],
    };
    let draws = rail_draws(&r);
    assert_eq!(draws.len(), 5);
    assert_eq!(
        (
            draws[0].min_radius,
            draws[0].max_radius,
            draws[0].ray_radius
        ),
        (0.0, 0.0, 0.333)
    );
    // Second spiral: num = 0.75.
    let root = 0.75f32.sqrt();
    let d = draws[2];
    assert!(!d.clockwise && draws[1].clockwise);
    assert_eq!(d.time, (3.0 + 7.5) * root);
    assert_eq!((d.wave_scale, d.max_radius), (4.0 * root, 1.25 * root));
    assert_eq!(d.ray_radius, 0.1 / root);
    let tris = strip_triangles();
    assert_eq!(tris.len(), 510 * 3);
    assert_eq!(&tris[..6], &[0, 1, 2, 2, 1, 3]);
}
