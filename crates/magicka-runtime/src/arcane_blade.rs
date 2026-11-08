//! ArcaneBlade (RailGunSpell.CastWeapon; research/weapon-enchant.md): a
//! 32-vertex ArcaneEffect ribbon from Equipment[0]'s position along its Up
//! axis × range, older pairs lagging behind, plus per-element
//! `weapon_arcane_*` effects. Killed with the weapon spell, it fades out
//! (alpha −5/s, range × √alpha) and is removed once its last pair collapses.
use magicka_content::skinned::Mat;
use magicka_sim::spells::{Elements as E, Spell};

pub type Vector = [f32; 3];

/// ArcaneBlade.VERTEXCOUNT.
pub const VERTICES: usize = 32;

/// ArcaneBlade.Initialize's CapsuleLight.Radius (Magicka.exe IL_016a).
pub const LIGHT_RADIUS: f32 = 4.0;

/// The native CapsuleLight.Draw inputs; diffuse/ambient are scaled by
/// intensity at draw time. Capsule lights have no specular or shadow pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapsuleLight {
    pub start: Vector,
    pub end: Vector,
    pub radius: f32,
    pub diffuse: Vector,
    pub ambient: Vector,
    pub intensity: f32,
}

/// ELEMENT_EFFECTS by element index (Earth, Arcane, Life, Shield, Ice: 0).
pub const ELEMENT_EFFECTS: [(E, &str); 6] = [
    (E::WATER, "weapon_arcane_water"),
    (E::COLD, "weapon_arcane_cold"),
    (E::FIRE, "weapon_arcane_fire"),
    (E::LIGHTNING, "weapon_arcane_lightning"),
    (E::STEAM, "weapon_arcane_steam"),
    (E::POISON, "weapon_arcane_poison"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct ArcaneBlade {
    /// Avatar-local identity (presentation keys effects by it).
    pub id: u32,
    pub elements: E,
    /// DefaultSpell(elements).GetColor().
    pub color: Vector,
    pub range: f32,
    pub max_range: f32,
    pub alpha: f32,
    pub dead: bool,
    /// CapsuleLight intensity (rises 10/s to 1; follows alpha once dead).
    pub light_intensity: f32,
    pub vertices: [Vector; VERTICES],
    /// The latest item transform with Up scaled by range (effect transform).
    pub effect_transform: Mat,
    initialized: bool,
}

fn up_of(item: &Mat) -> Vector {
    let up = [item[4], item[5], item[6]];
    let l = (up[0] * up[0] + up[1] * up[1] + up[2] * up[2]).sqrt();
    if l > 0.0 {
        up.map(|c| c / l)
    } else {
        [0.0, 1.0, 0.0]
    }
}

impl ArcaneBlade {
    /// ArcaneBlade.Initialize(Equipment[0].Item, spell.Element,
    /// TotalMagnitude·0.5 + 1.5).
    pub fn new(id: u32, elements: E, range: f32) -> Self {
        Self {
            id,
            elements,
            color: magicka_sim::lightning::spell_color(&Spell::default_for(elements)),
            range,
            max_range: range,
            alpha: 1.0,
            dead: false,
            light_intensity: 0.0,
            vertices: [[0.0; 3]; VERTICES],
            effect_transform: magicka_content::skinned::IDENTITY,
            initialized: false,
        }
    }

    /// The `weapon_arcane_*` effects to start (non-beam elements only).
    pub fn element_effects(&self) -> impl Iterator<Item = &'static str> + '_ {
        ELEMENT_EFFECTS
            .iter()
            .filter(|(e, _)| self.elements.has(*e))
            .map(|(_, name)| *name)
    }

    pub fn kill(&mut self) {
        self.dead = true;
    }

    /// Update with Equipment[0]'s transform (the first call is Initialize's
    /// fill of every pair at the current blade).
    pub fn update(&mut self, item: &Mat, dt: f32) {
        if self.dead {
            self.alpha -= dt * 5.0;
            self.range = self.max_range * self.alpha.max(0.0).sqrt();
            self.light_intensity = self.alpha;
        } else {
            self.light_intensity = (self.light_intensity + dt * 10.0).min(1.0);
        }
        let origin = [item[12], item[13], item[14]];
        let up = up_of(item);
        let tip = |range: f32| std::array::from_fn(|k| origin[k] + up[k] * range);
        if !self.initialized {
            self.initialized = true;
            let t = tip(self.range);
            for (i, v) in self.vertices.iter_mut().enumerate() {
                *v = if i % 2 == 0 { origin } else { t };
            }
        }
        self.effect_transform = *item;
        for k in 0..3 {
            self.effect_transform[4 + k] = item[4 + k] * self.range;
        }
        self.vertices[0] = origin;
        self.vertices[1] = tip(self.range);
        let amount = 1.401_298_464_324_817e-45f64.powf(dt as f64) as f32;
        for i in (2..VERTICES).step_by(2) {
            for j in [i, i + 1] {
                let (a, b) = (self.vertices[j - 2], self.vertices[j]);
                self.vertices[j] = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * amount);
            }
        }
    }

    /// IsDead: killed and the last pair within √0.1 of each other.
    pub fn is_dead(&self) -> bool {
        let (a, b) = (self.vertices[30], self.vertices[31]);
        let d2: f32 = (0..3).map(|k| (a[k] - b[k]).powi(2)).sum();
        self.dead && d2 <= 0.1
    }

    /// RenderData colours: centre = colour + 1, edge = colour (alpha 1).
    pub fn colors(&self) -> (Vector, Vector) {
        (self.color.map(|c| c + 1.0), self.color)
    }

    /// The light follows the newest vertex pair, including shrink/fade after
    /// Kill. Non-positive intensity has no visible UNORM light contribution.
    pub fn capsule_light(&self) -> Option<CapsuleLight> {
        (self.initialized && self.light_intensity > 0.0).then_some(CapsuleLight {
            start: self.vertices[0],
            end: self.vertices[1],
            radius: LIGHT_RADIUS,
            diffuse: self.color,
            // CapsuleLight's field starts at zero; ArcaneBlade never sets it.
            ambient: [0.0; 3],
            intensity: self.light_intensity,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item_at(x: f32) -> Mat {
        let mut m = magicka_content::skinned::IDENTITY;
        m[12] = x;
        m
    }

    #[test]
    fn blade_follows_the_item_up_axis_and_fades_after_kill() {
        let mut b = ArcaneBlade::new(1, E::ARCANE | E::FIRE, 2.5);
        assert_eq!(
            b.element_effects().collect::<Vec<_>>(),
            ["weapon_arcane_fire"]
        );
        b.update(&item_at(0.0), 1.0 / 60.0);
        assert_eq!(b.vertices[1], [0.0, 2.5, 0.0]);
        assert_eq!(b.vertices[31], [0.0, 2.5, 0.0]);
        assert!((b.light_intensity - 1.0 / 6.0).abs() < 1e-6);
        assert_eq!(b.effect_transform[5], 2.5);
        for _ in 0..5 {
            b.update(&item_at(1.0), 1.0 / 60.0);
        }
        assert!(b.vertices[30][0] < 1.0 && b.vertices[30][0] > 0.0);
        b.kill();
        let mut frames = 0;
        while !b.is_dead() {
            b.update(&item_at(1.0), 1.0 / 60.0);
            frames += 1;
            assert!(frames < 60, "never collapsed");
        }
        // alpha reaches 0 after 12 frames; the trailing pair then converges.
        assert!(b.range < 2.5 * 0.5);
        assert!(frames >= 10, "{frames}");
    }

    #[test]
    fn capsule_light_tracks_current_endpoints_and_fade_not_the_trailing_pair() {
        let mut blade = ArcaneBlade::new(1, E::ARCANE | E::FIRE, 3.0);
        assert!(blade.capsule_light().is_none());
        blade.update(&item_at(2.0), 0.1);
        let light = blade.capsule_light().unwrap();
        assert_eq!(light.start, [2.0, 0.0, 0.0]);
        assert_eq!(light.end, [2.0, 3.0, 0.0]);
        assert_eq!(light.radius, 4.0);
        assert_eq!(light.diffuse, blade.color);
        assert_eq!(light.ambient, [0.0; 3]);
        assert_eq!(light.intensity, 1.0);
        blade.kill();
        blade.update(&item_at(4.0), 0.1);
        let light = blade.capsule_light().unwrap();
        assert_eq!(light.start, [4.0, 0.0, 0.0]);
        assert!((light.end[1] - 3.0 * 0.5f32.sqrt()).abs() < 1e-6);
        assert_eq!(light.intensity, 0.5);
        assert_ne!(light.start, blade.vertices[30]);
        blade.update(&item_at(4.0), 0.11);
        assert!(blade.capsule_light().is_none());
    }
}
