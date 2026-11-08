//! Fixed-size Shield WALL mesh, reconstructed from Shield::.ctor.
//! See research/shield-wall.md. Local Z is the long axis; X is thickness.
use crate::impact_motion::Vector;

#[derive(Debug)]
pub struct WallMesh {
    pub positions: Vec<Vector>,
    pub normals: Vec<Vector>,
    pub uv: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

pub fn mesh() -> WallMesh {
    let mut mesh = WallMesh {
        positions: vec![[0.0; 3]; 242],
        normals: vec![[0.0; 3]; 242],
        uv: vec![[0.0; 2]; 242],
        indices: Vec::with_capacity(1200),
    };
    for face in 0..2 {
        let base = face * 121;
        let sign = if face == 0 { 1.0 } else { -1.0 };
        mesh.positions[base + 120][0] = sign * 0.2;
        mesh.normals[base + 120][0] = sign;
        for sample in 0..20 {
            let z = -((sample as f32 / 20.0) * std::f32::consts::PI).cos();
            let y = (1.0 - (z * z) as f64).powf(0.15) as f32;
            let uv_len = (z * z + y * y).sqrt();
            for (ring, (scale, thickness)) in [(1.0, 0.0), (0.666, 0.1332), (0.333, 0.17999999)]
                .into_iter()
                .enumerate()
            {
                for half in 0..2 {
                    let mirror = if half == 0 { 1.0 } else { -1.0 };
                    let i = base + ring * 40 + half * 20 + sample;
                    mesh.positions[i] = [
                        sign * thickness,
                        mirror * y * 3.0 * scale,
                        mirror * z * 6.0 * scale,
                    ];
                    mesh.normals[i] = [sign, 0.0, 0.0];
                    mesh.uv[i] = [mirror * z / uv_len * scale, mirror * y / uv_len * scale];
                }
            }
        }
        for n in 0..40 {
            let next = (n + 1) % 40;
            let triangles = [
                [n, n + 40, next],
                [n + 40, next + 40, next],
                [n + 40, n + 80, next + 40],
                [n + 80, next + 80, next + 40],
                [n + 80, 120, next + 80],
            ];
            for mut triangle in triangles {
                if face == 1 {
                    triangle.swap(1, 2);
                }
                mesh.indices.extend(triangle.map(|i| (base + i) as u32));
            }
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    #[test]
    fn wall_has_native_fixed_profile_and_opposite_faces() {
        let mesh = super::mesh();
        assert_eq!(mesh.positions.len(), 242);
        assert_eq!(mesh.indices.len(), 1200);
        assert!(mesh.indices.iter().all(|&i| i < 242));
        assert_eq!(mesh.positions[0], [0.0, 0.0, -6.0]);
        assert_eq!(mesh.positions[20], [0.0, -0.0, 6.0]);
        assert_eq!(mesh.positions[120], [0.2, 0.0, 0.0]);
        assert_eq!(mesh.positions[241], [-0.2, 0.0, 0.0]);
        assert!((mesh.positions[10][1] - 3.0).abs() < 1e-6);
        for i in 0..120 {
            assert_eq!(mesh.positions[i][0], -mesh.positions[i + 121][0]);
            assert_eq!(mesh.positions[i][1..], mesh.positions[i + 121][1..]);
            assert_eq!(mesh.uv[i], mesh.uv[i + 121]);
        }
    }
}
