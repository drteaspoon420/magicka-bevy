//! Item.Update's attached-item effects (research/weapon-enchant.md):
//! `swordelementeffect_<element>` for every element of the item's charged
//! spell queue, oriented along effect0 → effect1, and the item definition's
//! own (passive) effects, started at attach0 and kept at effect0.
use crate::character_pose::EquipmentAttachment;
use magicka_content::skinned::{mul, Mat};
use magicka_sim::spells::Elements as E;

pub type Vector = [f32; 3];

/// Item.ChantEffects by element index (Earth … Steam, Poison → cold).
pub const CHANT_EFFECTS: [&str; 11] = [
    "swordelementeffect_earth",
    "swordelementeffect_water",
    "swordelementeffect_cold",
    "swordelementeffect_fire",
    "swordelementeffect_lightning",
    "swordelementeffect_arcane",
    "swordelementeffect_life",
    "swordelementeffect_shield",
    "swordelementeffect_ice",
    "swordelementeffect_steam",
    "swordelementeffect_cold",
];

/// One attached item's world points this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemFrame {
    /// mAttach0AbsoluteTransform.
    pub attach0: Mat,
    pub attach1: Mat,
    pub item: Mat,
    /// mEffect0AbsolutePos / mEffect1AbsolutePos.
    pub effect0: Vector,
    pub effect1: Vector,
}

fn point(p: Vector, m: &Mat) -> Vector {
    std::array::from_fn(|k| p[0] * m[k] + p[1] * m[4 + k] + p[2] * m[8 + k] + m[12 + k])
}

fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalized(v: Vector) -> Vector {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|c| c / l)
}

impl ItemFrame {
    /// Item.Update with the held orientation `bind · skinned[bone]`.
    pub fn new(a: &EquipmentAttachment, skinned: &[Mat]) -> Self {
        let item = a.item_transform(skinned);
        Self {
            attach0: mul(&a.attach0, &item),
            attach1: mul(&a.attach1, &item),
            item,
            effect0: point(a.effect0, &item),
            effect1: point(a.effect1, &item),
        }
    }

    pub fn attach0_position(&self) -> Vector {
        [self.attach0[12], self.attach0[13], self.attach0[14]]
    }

    /// Matrix.Forward of mAttach0AbsoluteTransform (−row 3).
    pub fn attach0_forward(&self) -> Vector {
        [-self.attach0[8], -self.attach0[9], -self.attach0[10]]
    }

    /// effect1 − effect0 normalised (attach0 Forward when shorter than 1e-3).
    pub fn direction(&self) -> Vector {
        let d: Vector = std::array::from_fn(|k| self.effect1[k] - self.effect0[k]);
        if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] <= 1e-6 {
            self.attach0_forward()
        } else {
            normalized(d)
        }
    }

    /// The chant-effect transform: rows Up×dir, dir×that, dir, effect0.
    pub fn chant_transform(&self) -> Mat {
        let d = self.direction();
        let right = cross([0.0, 1.0, 0.0], d);
        let up = normalized(cross(d, right));
        let right = normalized(right);
        [
            right[0],
            right[1],
            right[2],
            0.0,
            up[0],
            up[1],
            up[2],
            0.0,
            d[0],
            d[1],
            d[2],
            0.0,
            self.effect0[0],
            self.effect0[1],
            self.effect0[2],
            1.0,
        ]
    }
}

/// (element index, effect name) for each element in `elements`.
pub fn chant_effects(elements: E) -> impl Iterator<Item = (usize, &'static str)> {
    E::BY_INDEX
        .into_iter()
        .enumerate()
        .filter(move |(_, e)| elements.has(*e))
        .map(|(i, _)| (i, CHANT_EFFECTS[i]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chant_transform_points_along_the_blade_from_effect0() {
        let f = ItemFrame {
            attach0: magicka_content::skinned::IDENTITY,
            attach1: magicka_content::skinned::IDENTITY,
            item: magicka_content::skinned::IDENTITY,
            effect0: [1.0, 2.0, 3.0],
            effect1: [1.0, 2.0, 5.0],
        };
        let m = f.chant_transform();
        assert_eq!(&m[8..11], &[0.0, 0.0, 1.0]);
        assert_eq!(&m[12..15], &[1.0, 2.0, 3.0]);
        // Right = Up × dir = (1, 0, 0); up = dir × right = (0, 1, 0).
        assert_eq!(&m[0..3], &[1.0, 0.0, 0.0]);
        assert_eq!(&m[4..7], &[0.0, 1.0, 0.0]);
        // Coincident points fall back to attach0 Forward (−Z for identity).
        let g = ItemFrame {
            effect1: f.effect0,
            ..f
        };
        assert_eq!(g.direction(), [0.0, 0.0, -1.0]);
    }

    #[test]
    fn chant_effects_follow_the_element_index_table() {
        let names: Vec<_> = chant_effects(E::FIRE | E::ARCANE | E::POISON).collect();
        assert_eq!(
            names,
            [
                (3, "swordelementeffect_fire"),
                (5, "swordelementeffect_arcane"),
                (10, "swordelementeffect_cold"),
            ]
        );
    }
}
