//! AnimatedPhysicsEntity, the entity SpawnAnimProp creates for a template
//! with clips and a skeleton (research/anim-prop.md): a radius box body,
//! an optional damage gate and one XNAnimation clip clock.
use magicka_content::physics::{CollisionBox, PhysicsEntityTemplate};

/// The template has an animated part with a skeleton and clip actions
/// (SpawnAnimProp's AnimatedPhysicsEntity test).
pub fn is_animated(template: &PhysicsEntityTemplate) -> bool {
    template.animated.as_ref().is_some_and(|a| {
        a.skeleton_model.is_some() && a.animation_sets.iter().any(|s| !s.is_empty())
    })
}

/// mRadius, with a non-positive template radius read as 1.
pub fn radius(template: &PhysicsEntityTemplate) -> f32 {
    match template.animated.as_ref().map(|a| a.radius) {
        Some(r) if r > 0. => r,
        _ => 1.,
    }
}

/// CreateBody: one box with its corner at (-r, 0, -r) and sides 2r, so it
/// rests on the entity origin. The template's own boxes are unused
/// (CorrectWithTemplateBox is empty).
pub fn collision_box(template: &PhysicsEntityTemplate) -> Option<CollisionBox> {
    is_animated(template).then(|| {
        let r = radius(template);
        CollisionBox {
            name: String::new(),
            position: [-r, 0., -r],
            sides: [2. * r; 3],
            orientation: [0., 0., 0., 1.],
        }
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimProp {
    /// IsDamageable: false deflects every damage collection, unpushed
    /// damage and every status effect.
    pub damageable: bool,
    pub pushable: bool,
    pub gravity: bool,
    /// mCurrentAnimation (Animations name); idle when the slot is empty.
    pub animation: String,
    /// The started clip; `None` when ForceAnimation found an empty slot
    /// (the controller then has no clip and counts as finished).
    pub clip: Option<String>,
    pub speed: f32,
    pub looping: bool,
    pub time: f32,
    /// Clip duration; `None` when the skeleton is unavailable.
    pub duration: Option<f32>,
}

impl AnimProp {
    /// ForceAnimation(animation) against clip set 0 (WeaponClass.Default).
    pub fn force(
        template: &PhysicsEntityTemplate,
        animation: Option<&str>,
        damageable: bool,
        gravity: bool,
        clip_duration: impl FnOnce(&str, &str) -> Option<f32>,
    ) -> Self {
        let action = animation.and_then(|name| {
            template
                .animated
                .as_ref()?
                .animation_sets
                .first()?
                .iter()
                .find(|a| a.animation.eq_ignore_ascii_case(name))
        });
        let mut prop = Self {
            damageable,
            pushable: template.pushable,
            gravity,
            animation: "idle".into(),
            clip: None,
            speed: 1.,
            looping: false,
            time: 0.,
            duration: None,
        };
        if let Some(a) = action {
            prop.animation = a.animation.to_ascii_lowercase();
            prop.clip = Some(a.clip.clone());
            prop.speed = a.speed;
            prop.looping = a.looping;
            prop.duration = template
                .animated
                .as_ref()
                .and_then(|p| p.skeleton_model.as_deref())
                .and_then(|s| clip_duration(s, &a.clip));
        }
        prop
    }

    /// AnimationController.HasFinished: no clip, or a one-shot at its end.
    pub fn finished(&self) -> bool {
        match (&self.clip, self.duration) {
            (None, _) => true,
            (Some(_), Some(d)) => !self.looping && self.time >= d,
            (Some(_), None) => false,
        }
    }

    /// Update: the clock runs while alive and not finished.
    pub fn update(&mut self, dt: f32, dead: bool) {
        if dead || self.finished() {
            return;
        }
        self.time += dt * self.speed;
        if let Some(d) = self.duration.filter(|d| *d > 0.) {
            if self.looping {
                self.time %= d;
            } else {
                self.time = self.time.min(d);
            }
        }
    }
}

/// A template asset reference resolved from Data/AnimatedProps, without
/// extension ("..\..\Models\X" -> "Models/X").
pub fn content_path(reference: &str) -> String {
    let mut parts = vec!["Data", "AnimatedProps"];
    for p in reference.split(['\\', '/']) {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}
