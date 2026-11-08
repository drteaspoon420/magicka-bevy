//! Per-bone bind-pose diagnostics for one skinned model file.
use magicka_content::assets::Asset;
use magicka_content::skinned::{mul, resolve, IDENTITY};
use magicka_content::xna::XnaObject;
use magicka_content::{parse_primary, Content};
use magicka_xnb::xnb;
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let x = xnb::parse(&std::fs::read(path).unwrap()).unwrap();
    let p = parse_primary(&x).unwrap().unwrap();
    let Content::Xna(o) = p.content else { return };
    let XnaObject::Asset(a) = *o else { return };
    let Asset::SkinnedModel(sm) = *a else { return };
    let r = resolve(&sm, p.shared).unwrap();
    let abs = r.skeleton.bind_absolute();
    for (i, (b, m)) in r.skeleton.bones.iter().zip(&abs).enumerate() {
        let e = mul(&b.inverse_bind, m)
            .iter()
            .zip(IDENTITY.iter())
            .map(|(x, y)| (x - y).abs())
            .fold(0f32, f32::max);
        if e > 1e-3 {
            println!(
                "{i:3} {:<24} parent {:?} err {e:.4} t {:?} q {:?} s {:?}",
                b.name, b.parent, b.translation, b.orientation, b.scale
            );
        }
    }
    println!("{} bones", abs.len());
    // Hypothesis: the pipeline baked inverse binds with S·R (scale first).
    let mut abs2: Vec<[f32; 16]> = Vec::new();
    let mut worst = 0f32;
    for b in &r.skeleton.bones {
        let rot = magicka_content::skinned::pose_matrix(b.translation, b.orientation, [1.0; 3]);
        let mut s = IDENTITY;
        s[0] = b.scale[0];
        s[5] = b.scale[1];
        s[10] = b.scale[2];
        let local = mul(&s, &rot);
        let m = match b.parent {
            Some(p) => mul(&local, &abs2[p]),
            None => local,
        };
        let e = mul(&b.inverse_bind, &m)
            .iter()
            .zip(IDENTITY.iter())
            .map(|(x, y)| (x - y).abs())
            .fold(0f32, f32::max);
        worst = worst.max(e);
        abs2.push(m);
    }
    println!("S·R hypothesis: worst error {worst:e}");
    let pm = magicka_content::skinned::pose_matrix;
    let err =
        |name: &str,
         local_of: &dyn Fn(&magicka_content::skinned::Bone) -> [f32; 16],
         inherit_of: &dyn Fn(&magicka_content::skinned::Bone, &[f32; 16]) -> [f32; 16]| {
            let mut abs: Vec<[f32; 16]> = Vec::new();
            let mut worst = 0f32;
            for b in &r.skeleton.bones {
                let local = local_of(b);
                let m = match b.parent {
                    Some(p) => mul(&local, &inherit_of(&r.skeleton.bones[p], &abs[p])),
                    None => local,
                };
                worst = worst.max(
                    mul(&b.inverse_bind, &m)
                        .iter()
                        .zip(IDENTITY.iter())
                        .map(|(x, y)| (x - y).abs())
                        .fold(0f32, f32::max),
                );
                abs.push(m);
            }
            println!("{name}: worst error {worst:e}");
        };
    err(
        "no scale at all",
        &|b| pm(b.translation, b.orientation, [1.0; 3]),
        &|_, a| *a,
    );
    // Segment scale compensate: child sees the parent's absolute with the
    // parent's own local scale divided out (S^-1 · abs_parent).
    err(
        "R·S, scale not inherited",
        &|b| pm(b.translation, b.orientation, b.scale),
        &|pb, a| {
            let mut inv = IDENTITY;
            inv[0] = 1.0 / pb.scale[0];
            inv[5] = 1.0 / pb.scale[1];
            inv[10] = 1.0 / pb.scale[2];
            mul(&inv, a)
        },
    );
}
