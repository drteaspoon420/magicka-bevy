//! Run Magicka particle effects headless and dump every frame's particles, so
//! other tools (gta2_magicka's magicka-spritebake) can re-render spell visuals in
//! another game's style.
//!
//!     spell-frames <content-dir> <out-dir> [--fps 30] [--seconds 1.5] [--warmup 0]
//!                  [--area <radius>] [--sheets] <effect-name>...
//!
//! `--velocity S` moves a single effect forward (+Z) at S units/s, as a flying
//! projectile carries its trail; each frame records the emitter `origin`.
//! `--area R` starts each effect as a FullBlast does (blast_visual::bursts): eight
//! radial wedges at 45° steps, horizontal basis scaled by R * 0.1.
//!
//! Writes `<out>/<effect>.json`:
//! `{effect, fps, direction, frames: [[{p, r, radius, life, color, sprite, ab, hsv, aligned, v}]]}`
//! with the effect started at the origin facing +Z. `--sheets` also writes every
//! lifetime slice of EffectTextures/ParticlesA..D as `<out>/sheets/<Name>_<z>.png`
//! (8x8 sprite cells per sheet; sprite index = 64 * sheet + cell).
//! Output is derived from proprietary data: keep it untracked.
use magicka_content::assets::Asset;
use magicka_content::texture::encode_png;
use magicka_content::xna::XnaObject;
use magicka_content::{parse_primary, Content};
use magicka_runtime::particles::{EffectLibrary, EffectSystem};
use std::path::{Path, PathBuf};

fn read_texture3d(path: &Path) -> Result<(usize, usize, usize, Vec<u8>), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let xnb = magicka_xnb::xnb::parse(&bytes).map_err(|e| e.to_string())?;
    let parsed = parse_primary(&xnb)
        .ok_or("null content")?
        .map_err(|e| e.to_string())?;
    let Content::Xna(object) = parsed.content else {
        return Err("expected XNA object".into());
    };
    let XnaObject::Asset(asset) = *object else {
        return Err("expected asset".into());
    };
    let Asset::Texture3D {
        format,
        width,
        height,
        depth,
        mips,
    } = *asset
    else {
        return Err(format!("{}: not a Texture3D", path.display()));
    };
    if format != 1 {
        return Err(format!("{}: surface format {format}", path.display()));
    }
    let (w, h, d) = (width as usize, height as usize, depth as usize);
    let top = mips.first().ok_or("no mips")?;
    if top.len() < w * h * d * 4 {
        return Err("short Texture3D".into());
    }
    // SurfaceFormat.Color is BGRA.
    let rgba = top[..w * h * d * 4]
        .chunks_exact(4)
        .flat_map(|c| [c[2], c[1], c[0], c[3]])
        .collect();
    Ok((w, h, d, rgba))
}

fn main() -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut take = |flag: &str, default: &str| -> String {
        match args.iter().position(|a| a == flag) {
            Some(i) => {
                let v = args.remove(i + 1);
                args.remove(i);
                v
            }
            None => default.into(),
        }
    };
    let fps: f32 = take("--fps", "30").parse().map_err(|_| "--fps")?;
    let seconds: f32 = take("--seconds", "1.5").parse().map_err(|_| "--seconds")?;
    let warmup: f32 = take("--warmup", "0").parse().map_err(|_| "--warmup")?;
    let area: Option<f32> = match take("--area", "") {
        s if s.is_empty() => None,
        s => Some(s.parse().map_err(|_| "--area")?),
    };
    let speed: f32 = take("--velocity", "0").parse().map_err(|_| "--velocity")?;
    let sheets = args.iter().any(|a| a == "--sheets");
    args.retain(|a| a != "--sheets");
    if args.len() < 2 {
        return Err("usage: spell-frames <content-dir> <out-dir> [--fps n] [--seconds s] [--warmup s] [--sheets] <effect>...".into());
    }
    let content = PathBuf::from(&args[0]);
    let out = PathBuf::from(&args[1]);
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    if sheets {
        let dir = out.join("sheets");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        for name in ["ParticlesA", "ParticlesB", "ParticlesC", "ParticlesD"] {
            let (w, h, d, rgba) =
                read_texture3d(&content.join(format!("EffectTextures/{name}.xnb")))?;
            for z in 0..d {
                let slice = &rgba[z * w * h * 4..(z + 1) * w * h * 4];
                std::fs::write(dir.join(format!("{name}_{z}.png")), encode_png(w, h, slice))
                    .map_err(|e| e.to_string())?;
            }
            println!("{name}: {w}x{h}, {d} lifetime slices");
        }
    }
    let library =
        EffectLibrary::load_directory(&content.join("Effects")).map_err(|e| format!("{e:?}"))?;
    let dt = 1.0 / fps;
    for name in &args[2..] {
        let hash = magicka_runtime::scene_loading::custom_hash(&name.to_lowercase());
        let definition = library
            .effects
            .get(&hash)
            .ok_or_else(|| format!("effect {name} not found"))?;
        let (kind, duration) = (format!("{:?}", definition.kind), definition.duration);
        let mut system = EffectSystem::new(library.clone());
        let mut moving = None;
        match area {
            None => {
                moving = Some(
                    system
                        .start_at(hash, [0.0; 3], [0.0, 0.0, 1.0])
                        .ok_or("start failed")?,
                );
            }
            Some(radius) => {
                for segment in 0..8 {
                    let [cos, _, minus_sin] = magicka_runtime::blast_visual::direction(segment);
                    let scale = radius * 0.1;
                    let m = [
                        cos * scale,
                        0.0,
                        minus_sin * scale,
                        0.0,
                        0.0,
                        1.0,
                        0.0,
                        0.0,
                        -minus_sin * scale,
                        0.0,
                        cos * scale,
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        1.0,
                    ];
                    system.start(hash, m).ok_or("start failed")?;
                }
            }
        }
        let mut origin = [0.0f32; 3];
        let mut step = |system: &mut EffectSystem, origin: &mut [f32; 3]| {
            if speed != 0.0 {
                origin[2] += speed * dt;
                if let Some(r) = moving.as_mut() {
                    system.update_position_direction(r, *origin, [0.0, 0.0, 1.0]);
                }
            }
            system.update(dt);
            system.update_particles(dt);
        };
        let mut t = 0.0;
        while t < warmup {
            step(&mut system, &mut origin);
            t += dt;
        }
        let mut frames = Vec::new();
        let n = (seconds * fps).round() as usize;
        let mut peak = 0;
        let mut origins = Vec::new();
        for _ in 0..n {
            step(&mut system, &mut origin);
            origins.push(origin);
            let active = system.particles.active();
            peak = peak.max(active.len());
            frames.push(serde_json::Value::Array(
                active
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "p": p.position, "r": p.rotation, "radius": p.radius,
                            "life": p.normalized_lifetime, "color": p.color, "sprite": p.sprite,
                            "ab": p.alpha_blended, "hsv": p.hsv_colorize, "aligned": p.rotation_aligned,
                            "v": p.velocity,
                        })
                    })
                    .collect(),
            ));
        }
        let doc = serde_json::json!({
            "effect": name, "kind": kind, "duration": duration, "fps": fps, "warmup": warmup,
            "direction": [0.0, 0.0, 1.0], "area_radius": area, "velocity": speed, "origins": origins, "frames": frames,
        });
        let path = out.join(format!("{name}.json"));
        std::fs::write(&path, serde_json::to_string(&doc).unwrap()).map_err(|e| e.to_string())?;
        println!(
            "{name}: {kind}, duration {duration:.2}s, {n} frames, peak {peak} particles -> {}",
            path.display()
        );
    }
    Ok(())
}
