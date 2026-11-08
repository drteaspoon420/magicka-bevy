//! Minimal dependency-free glTF 2.0 binary (.glb) writer.
//!
//! Conventions when converting from XNA:
//! - XNA matrices are row-vector (`v * M`, translation in M41..M43). Their
//!   row-major storage equals glTF's column-major storage of the equivalent
//!   column-vector matrix, so the 16 floats are copied unchanged.
//! - XNA culls counter-clockwise triangles (front faces are clockwise); glTF
//!   front faces are counter-clockwise, so callers flip winding.
//! - XNA/D3D texture coordinates already have a top-left origin, as in glTF.

use std::fmt::Write as _;

pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o
}

/// Percent-encode a relative URI path (keeps `/`, unreserved characters).
pub fn uri(path: &str) -> String {
    let mut o = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            o.push(b as char);
        } else {
            let _ = write!(o, "%{b:02X}");
        }
    }
    o
}

fn f(x: f32) -> String {
    if x.is_finite() {
        format!("{x}")
    } else {
        "0".into()
    }
}

fn arr(v: &[f32]) -> String {
    format!(
        "[{}]",
        v.iter().map(|x| f(*x)).collect::<Vec<_>>().join(",")
    )
}

#[derive(Default)]
pub struct Builder {
    pub bin: Vec<u8>,
    views: Vec<String>,
    accessors: Vec<String>,
    pub meshes: Vec<String>,
    pub materials: Vec<String>,
    pub textures: Vec<String>,
    pub images: Vec<String>,
    pub nodes: Vec<String>,
    pub animations: Vec<String>,
    samplers_used: bool,
    pub skins: Vec<String>,
}

/// One triangle-list primitive in glTF conventions (CCW winding).
pub struct Primitive {
    pub positions: Vec<[f32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub uvs: Option<Vec<[f32; 2]>>,
    pub indices: Vec<u32>,
    pub material: Option<usize>,
    /// Skinning: up to four joint indices (into the skin's joint list) and
    /// weights per vertex.
    pub joints: Option<Vec<[u8; 4]>>,
    pub weights: Option<Vec<[f32; 4]>>,
    /// Vertex color (COLOR_0) and second UV set (TEXCOORD_1).
    pub colors: Option<Vec<[f32; 4]>>,
    pub uvs1: Option<Vec<[f32; 2]>>,
    /// TANGENT (xyz + bitangent handedness), liquids only.
    pub tangents: Option<Vec<[f32; 4]>>,
}

impl Builder {
    fn view(&mut self, data: &[u8], target: Option<u32>) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let off = self.bin.len();
        self.bin.extend_from_slice(data);
        let t = target.map_or(String::new(), |t| format!(",\"target\":{t}"));
        self.views.push(format!(
            "{{\"buffer\":0,\"byteOffset\":{off},\"byteLength\":{}{t}}}",
            data.len()
        ));
        self.views.len() - 1
    }

    fn floats<const N: usize>(
        &mut self,
        data: &[[f32; N]],
        ty: &str,
        minmax: bool,
        target: Option<u32>,
    ) -> usize {
        let mut bytes = Vec::with_capacity(data.len() * N * 4);
        for v in data {
            for x in v {
                bytes.extend_from_slice(&x.to_le_bytes());
            }
        }
        let view = self.view(&bytes, target);
        let mut mm = String::new();
        if minmax && !data.is_empty() {
            let (mut lo, mut hi) = ([f32::MAX; N], [f32::MIN; N]);
            for v in data {
                for k in 0..N {
                    lo[k] = lo[k].min(v[k]);
                    hi[k] = hi[k].max(v[k]);
                }
            }
            mm = format!(",\"min\":{},\"max\":{}", arr(&lo), arr(&hi));
        }
        self.accessors.push(format!(
            "{{\"bufferView\":{view},\"componentType\":5126,\"count\":{},\"type\":\"{ty}\"{mm}}}",
            data.len()
        ));
        self.accessors.len() - 1
    }

    pub fn scalars(&mut self, data: &[f32], minmax: bool) -> usize {
        let v: Vec<[f32; 1]> = data.iter().map(|x| [*x]).collect();
        self.floats(&v, "SCALAR", minmax, None)
    }

    pub fn vec3(&mut self, data: &[[f32; 3]], minmax: bool) -> usize {
        self.floats(data, "VEC3", minmax, None)
    }

    pub fn vec4(&mut self, data: &[[f32; 4]]) -> usize {
        self.floats(data, "VEC4", false, None)
    }

    /// Add a mesh made of primitives; returns the mesh index.
    pub fn mesh(&mut self, name: &str, prims: &[Primitive]) -> usize {
        let mut ps = Vec::new();
        for p in prims {
            let pos = self.floats(&p.positions, "VEC3", true, Some(34962));
            let mut attrs = format!("\"POSITION\":{pos}");
            if let Some(n) = &p.normals {
                let a = self.floats(n, "VEC3", false, Some(34962));
                let _ = write!(attrs, ",\"NORMAL\":{a}");
            }
            if let Some(t) = &p.uvs {
                let a = self.floats(t, "VEC2", false, Some(34962));
                let _ = write!(attrs, ",\"TEXCOORD_0\":{a}");
            }
            if let Some(t) = &p.uvs1 {
                let a = self.floats(t, "VEC2", false, Some(34962));
                let _ = write!(attrs, ",\"TEXCOORD_1\":{a}");
            }
            if let Some(c) = &p.colors {
                let a = self.floats(c, "VEC4", false, Some(34962));
                let _ = write!(attrs, ",\"COLOR_0\":{a}");
            }
            if let Some(t) = &p.tangents {
                let a = self.floats(t, "VEC4", false, Some(34962));
                let _ = write!(attrs, ",\"TANGENT\":{a}");
            }
            if let Some(j) = &p.joints {
                let bytes: Vec<u8> = j.iter().flatten().copied().collect();
                let view = self.view(&bytes, Some(34962));
                self.accessors.push(format!(
                    "{{\"bufferView\":{view},\"componentType\":5121,\"count\":{},\"type\":\"VEC4\"}}",
                    j.len()
                ));
                let _ = write!(attrs, ",\"JOINTS_0\":{}", self.accessors.len() - 1);
            }
            if let Some(w) = &p.weights {
                let a = self.floats(w, "VEC4", false, Some(34962));
                let _ = write!(attrs, ",\"WEIGHTS_0\":{a}");
            }
            let mut ib = Vec::with_capacity(p.indices.len() * 4);
            for i in &p.indices {
                ib.extend_from_slice(&i.to_le_bytes());
            }
            let view = self.view(&ib, Some(34963));
            self.accessors.push(format!(
                "{{\"bufferView\":{view},\"componentType\":5125,\"count\":{},\"type\":\"SCALAR\"}}",
                p.indices.len()
            ));
            let idx = self.accessors.len() - 1;
            let mat = p
                .material
                .map_or(String::new(), |m| format!(",\"material\":{m}"));
            ps.push(format!(
                "{{\"attributes\":{{{attrs}}},\"indices\":{idx}{mat}}}"
            ));
        }
        self.meshes.push(format!(
            "{{\"name\":\"{}\",\"primitives\":[{}]}}",
            escape(name),
            ps.join(",")
        ));
        self.meshes.len() - 1
    }

    /// Material with base color and an optional external texture URI.
    pub fn material(
        &mut self,
        name: &str,
        color: [f32; 4],
        texture_uri: Option<&str>,
        alpha_mask: bool,
        extras: Option<&str>,
    ) -> usize {
        let tex = texture_uri.map_or(String::new(), |u| {
            self.images.push(format!("{{\"uri\":\"{}\"}}", escape(u)));
            self.samplers_used = true;
            self.textures.push(format!(
                "{{\"sampler\":0,\"source\":{}}}",
                self.images.len() - 1
            ));
            format!(
                ",\"baseColorTexture\":{{\"index\":{}}}",
                self.textures.len() - 1
            )
        });
        let alpha = if alpha_mask {
            ",\"alphaMode\":\"MASK\""
        } else {
            ""
        };
        let extras = extras.map_or(String::new(), |e| format!(",\"extras\":{e}"));
        self.materials.push(format!(
            "{{\"name\":\"{}\",\"pbrMetallicRoughness\":{{\"baseColorFactor\":{}{tex},\"metallicFactor\":0,\"roughnessFactor\":1}}{alpha}{extras}}}",
            escape(name),
            arr(&color)
        ));
        self.materials.len() - 1
    }

    /// Add a node; `transform` is either a 16-float matrix or TRS JSON
    /// fragment (without braces), e.g. `"matrix":[...]`.
    pub fn node(
        &mut self,
        name: &str,
        transform: &str,
        mesh: Option<usize>,
        children: &[usize],
    ) -> usize {
        let mut s = format!("{{\"name\":\"{}\"", escape(name));
        if !transform.is_empty() {
            let _ = write!(s, ",{transform}");
        }
        if let Some(m) = mesh {
            let _ = write!(s, ",\"mesh\":{m}");
        }
        if !children.is_empty() {
            let c: Vec<String> = children.iter().map(|c| c.to_string()).collect();
            let _ = write!(s, ",\"children\":[{}]", c.join(","));
        }
        s.push('}');
        self.nodes.push(s);
        self.nodes.len() - 1
    }

    /// Reserve a node slot to fill later (children must exist before parents
    /// reference them, but indices can be allocated top-down).
    pub fn set_node(
        &mut self,
        index: usize,
        name: &str,
        transform: &str,
        mesh: Option<usize>,
        children: &[usize],
    ) {
        let n = self.node(name, transform, mesh, children);
        self.nodes.swap(index, n);
        self.nodes.pop();
    }

    pub fn placeholder(&mut self) -> usize {
        self.nodes.push(String::new());
        self.nodes.len() - 1
    }

    /// Attach generated JSON metadata to an already populated node.
    pub fn node_extras(&mut self, index: usize, extras: &str) {
        self.nodes[index].pop(); // closing object brace
        let _ = write!(self.nodes[index], ",\"extras\":{extras}}}");
    }

    /// Inverse bind matrices (XNA row-major = glTF column-major, copied).
    pub fn mat4(&mut self, data: &[[f32; 16]]) -> usize {
        self.floats(data, "MAT4", false, None)
    }

    pub fn skin(
        &mut self,
        name: &str,
        joints: &[usize],
        inverse_bind: usize,
        skeleton: usize,
    ) -> usize {
        let j: Vec<String> = joints.iter().map(|x| x.to_string()).collect();
        self.skins.push(format!(
            "{{\"name\":\"{}\",\"joints\":[{}],\"inverseBindMatrices\":{inverse_bind},\"skeleton\":{skeleton}}}",
            escape(name),
            j.join(",")
        ));
        self.skins.len() - 1
    }

    /// A node with a skinned mesh (its own transform is ignored by glTF).
    pub fn skinned_node(&mut self, name: &str, mesh: usize, skin: usize) -> usize {
        self.nodes.push(format!(
            "{{\"name\":\"{}\",\"mesh\":{mesh},\"skin\":{skin}}}",
            escape(name)
        ));
        self.nodes.len() - 1
    }

    pub fn animation(&mut self, name: &str, channels: &[(usize, &str, usize, usize)]) {
        // channels: (node, path, input accessor, output accessor)
        let mut samplers = Vec::new();
        let mut chans = Vec::new();
        for (i, (node, path, input, output)) in channels.iter().enumerate() {
            samplers.push(format!(
                "{{\"input\":{input},\"output\":{output},\"interpolation\":\"LINEAR\"}}"
            ));
            chans.push(format!(
                "{{\"sampler\":{i},\"target\":{{\"node\":{node},\"path\":\"{path}\"}}}}"
            ));
        }
        self.animations.push(format!(
            "{{\"name\":\"{}\",\"samplers\":[{}],\"channels\":[{}]}}",
            escape(name),
            samplers.join(","),
            chans.join(",")
        ));
    }

    pub fn finish(mut self, scene_nodes: &[usize]) -> Vec<u8> {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let list = |v: &[String]| v.join(",");
        let mut json = format!(
            "{{\"asset\":{{\"version\":\"2.0\",\"generator\":\"magicka-content\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[{}]}}],\"nodes\":[{}]",
            scene_nodes.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(","),
            list(&self.nodes)
        );
        for (key, v) in [
            ("meshes", &self.meshes),
            ("materials", &self.materials),
            ("textures", &self.textures),
            ("images", &self.images),
            ("animations", &self.animations),
            ("skins", &self.skins),
            ("accessors", &self.accessors),
            ("bufferViews", &self.views),
        ] {
            if !v.is_empty() {
                let _ = write!(json, ",\"{key}\":[{}]", list(v));
            }
        }
        if self.samplers_used {
            json.push_str(",\"samplers\":[{\"magFilter\":9729,\"minFilter\":9987,\"wrapS\":10497,\"wrapT\":10497}]");
        }
        if !self.bin.is_empty() {
            let _ = write!(json, ",\"buffers\":[{{\"byteLength\":{}}}]", self.bin.len());
        }
        json.push('}');
        let mut j = json.into_bytes();
        while !j.len().is_multiple_of(4) {
            j.push(b' ');
        }
        let total = 12
            + 8
            + j.len()
            + if self.bin.is_empty() {
                0
            } else {
                8 + self.bin.len()
            };
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(j.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&j);
        if !self.bin.is_empty() {
            out.extend_from_slice(&(self.bin.len() as u32).to_le_bytes());
            out.extend_from_slice(b"BIN\0");
            out.extend_from_slice(&self.bin);
        }
        out
    }
}

pub fn matrix_json(m: &[f32; 16]) -> String {
    format!("\"matrix\":{}", arr(m))
}

pub fn trs_json(t: [f32; 3], r: [f32; 4], s: [f32; 3]) -> String {
    format!(
        "\"translation\":{},\"rotation\":{},\"scale\":{}",
        arr(&t),
        arr(&r),
        arr(&s)
    )
}
