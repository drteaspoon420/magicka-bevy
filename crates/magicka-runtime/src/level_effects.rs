//! Level-placed visual effects (torch and candle flames, dust, waterfalls…):
//! `LevelModel.GetAllEffects`, `AnimatedLevelPart.GetAllEffects` and the
//! effect loop of `GameScene.Update` (research/level-effects.md).
//!
//! Each placement's transform is `Matrix.CreateWorld(position, forward, Up)`;
//! effects of animated parts are additionally multiplied by the part's
//! absolute transform every frame. A positive Range casts a segment of that
//! length along the transform's Forward and scales the 3×3 part of the
//! transform by the hit fraction (MagickaMath.UniformMatrixScale).

use crate::animated_hierarchy::AnimatedPartState;
use crate::particles::{create_world, EffectRef, EffectSystem};
use crate::scene_loading::custom_hash;
use magicka_content::level::{AnimatedLevelPart, EffectPlacement, LevelModel};
use magicka_content::skinned::{mul, Mat};

#[derive(Clone, Debug)]
pub struct LevelEffect {
    pub id: i32,
    pub hash: i32,
    pub animation: Option<i32>,
    pub effect: EffectRef,
    /// The stored CreateWorld transform (relative to its part, if any).
    pub local: Mat,
    pub range: f32,
    /// Child-index path of the owning animated part (decoded order).
    pub part: Option<Vec<usize>>,
}

fn placement_transform(p: &EffectPlacement) -> Mat {
    create_world(p.position, p.forward, [0.0, 1.0, 0.0])
}

/// Decode the native named GetAllEffects entries. PlaySession registers them
/// in GameScene.mEffects before the first script update.
pub fn placements(level: &LevelModel) -> Vec<LevelEffect> {
    let mut out = Vec::new();
    fn add(
        p: &EffectPlacement,
        part: Option<Vec<usize>>,
        animation: Option<i32>,
        out: &mut Vec<LevelEffect>,
    ) {
        out.push(LevelEffect {
            id: custom_hash(&p.id.to_lowercase()),
            hash: custom_hash(&p.effect),
            animation,
            effect: EffectRef::default(),
            local: placement_transform(p),
            range: p.range,
            part,
        });
    }
    for p in &level.effects {
        add(p, None, None, &mut out);
    }
    fn walk(parts: &[AnimatedLevelPart], path: &mut Vec<usize>, out: &mut Vec<LevelEffect>) {
        for (i, part) in parts.iter().enumerate() {
            path.push(i);
            for p in &part.effects {
                add(p, Some(path.clone()), Some(custom_hash(&part.name)), out);
            }
            walk(&part.children, path, out);
            path.pop();
        }
    }
    walk(&level.animated_parts, &mut Vec::new(), &mut out);
    out
}

fn part_world<'a>(parts: &'a [AnimatedPartState], path: &[usize]) -> Option<&'a Mat> {
    let (first, rest) = path.split_first()?;
    let part = parts.get(*first)?;
    if rest.is_empty() {
        Some(&part.world)
    } else {
        part_world(&part.children, rest)
    }
}

/// GameScene.Update for one effect: the world transform with the Range clip.
/// `intersect(origin, delta)` returns the scene hit fraction, if any.
pub fn world_transform(
    e: &LevelEffect,
    parts: &[AnimatedPartState],
    intersect: impl Fn([f32; 3], [f32; 3]) -> Option<f32>,
) -> Mat {
    let mut m = match &e.part {
        Some(path) => match part_world(parts, path) {
            Some(world) => mul(&e.local, world),
            None => e.local,
        },
        None => e.local,
    };
    if e.range > 0.0 {
        let origin = [m[12], m[13], m[14]];
        // Matrix.Forward = −(M31, M32, M33).
        let forward = [-m[8], -m[9], -m[10]];
        let delta = forward.map(|v| v * e.range);
        let scale = intersect(origin, delta).unwrap_or(1.0);
        for v in &mut m[..3] {
            *v *= scale;
        }
        for v in &mut m[4..7] {
            *v *= scale;
        }
        for v in &mut m[8..11] {
            *v *= scale;
        }
    }
    m
}

/// Apply the frame's transforms (only to active effects, as GameScene does).
pub fn update(
    list: &mut [LevelEffect],
    effects: &mut EffectSystem,
    parts: &[AnimatedPartState],
    intersect: impl Fn([f32; 3], [f32; 3]) -> Option<f32>,
) {
    for e in list {
        if !effects.is_active(&e.effect) {
            continue;
        }
        let m = world_transform(e, parts, &intersect);
        effects.update_orientation(&mut e.effect, m);
    }
}

/// AnimatedLevelPart.TryGetLocator over the scene's parts (GameScene
/// .GetLocator's last fallback): a part's own locator times its current
/// world transform (mOldTransform), else its children in order. Dictionary
/// insertion order is the content order.
pub fn part_locator(
    parts: &[AnimatedLevelPart],
    states: &[AnimatedPartState],
    id: i32,
) -> Option<Mat> {
    for (part, state) in parts.iter().zip(states) {
        if let Some(l) = part.locators.iter().find(|l| custom_hash(&l.name) == id) {
            return Some(mul(&l.transform, &state.world));
        }
        if let Some(m) = part_locator(&part.children, &state.children, id) {
            return Some(m);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn effect(range: f32) -> LevelEffect {
        let p = EffectPlacement {
            id: "fx".into(),
            position: [1.0, 2.0, 3.0],
            forward: [0.0, -1.0, 0.0001],
            range,
            effect: "ambient_fire_torch".into(),
        };
        LevelEffect {
            id: custom_hash(&p.id.to_lowercase()),
            hash: custom_hash(&p.effect),
            animation: None,
            effect: EffectRef::default(),
            local: placement_transform(&p),
            range,
            part: None,
        }
    }

    #[test]
    fn range_scales_the_rotation_by_the_hit_fraction() {
        let e = effect(4.0);
        let m = world_transform(&e, &[], |origin, delta| {
            assert_eq!(origin, [1.0, 2.0, 3.0]);
            assert!((delta[1] + 4.0).abs() < 1e-3);
            Some(0.25)
        });
        let len = (m[8] * m[8] + m[9] * m[9] + m[10] * m[10]).sqrt();
        assert!((len - 0.25).abs() < 1e-5);
        assert_eq!([m[12], m[13], m[14]], [1.0, 2.0, 3.0]);
        // A miss keeps the stored transform.
        assert_eq!(world_transform(&e, &[], |_, _| None), e.local);
    }

    #[test]
    fn zero_range_skips_the_segment_test() {
        let e = effect(0.0);
        let m = world_transform(&e, &[], |_, _| panic!("no segment for Range 0"));
        assert_eq!(m, e.local);
    }
}
