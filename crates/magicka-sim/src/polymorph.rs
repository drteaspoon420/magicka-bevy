//! Polymorph selection, replacement arithmetic and effect lifetime.
//! Runtime actor reconstruction and AI event storage are separate adapters.
//! Evidence: research/polymorph.md; MethodDefs 0x06000b41..0x06000b49.
use crate::impact_motion::Vector;

pub const QUERY_RADIUS: f32 = 6.0;
pub const DURATION: f32 = 20.0;
pub const AVATAR_FORMS: [&str; 4] = [
    "polymorph_horse",
    "polymorph_imp",
    "polymorph_goblin",
    "polymorph_troll",
];
pub const NPC_FORMS: [&str; 4] = ["horse", "imp", "goblin_bomber", "troll_forest"];

/// Execute(owner) uses the supplied direction without renormalizing it.
pub fn query_center(position: Vector, direction: Vector) -> Vector {
    std::array::from_fn(|i| position[i] + direction[i] * 3.0)
}

#[derive(Clone, Copy, Debug)]
pub struct Candidate<'a> {
    pub handle: u16,
    pub position: Vector,
    pub character: bool,
    pub ethereal: bool,
    pub name: &'a str,
}

/// Consume EntityManager query results in their existing order. The caller
/// queries radius 6 with ignoreProtected=false and ignoreY=true; ranking still
/// uses all three axes. Dead filtering and body-radius reach belong to that
/// query, not an additional center-distance or health check here.
pub fn find_closest<'a>(
    center: Vector,
    owner: Option<u16>,
    candidates: impl IntoIterator<Item = Candidate<'a>>,
) -> Option<u16> {
    let mut closest = None;
    let mut distance = f32::MAX;
    for candidate in candidates {
        if !candidate.character
            || Some(candidate.handle) == owner
            || candidate.ethereal
            || ["boss_vlad_swamp", "vlad_vietnam", "vlad_future"]
                .iter()
                .any(|name| candidate.name.eq_ignore_ascii_case(name))
        {
            continue;
        }
        let d: Vector = std::array::from_fn(|i| candidate.position[i] - center[i]);
        let squared = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if squared < distance {
            distance = squared;
            closest = Some(candidate.handle);
        }
    }
    closest
}

/// Both replacement paths preserve the current health fraction, including on
/// reversal. This is not restoration of the health saved at casting time.
pub fn transferred_hit_points(hp: f32, old_max: f32, new_max: f32) -> f32 {
    hp / old_max * new_max
}

/// MoveTo preserves orientation, clears velocity and keeps the feet in place.
/// The caller supplies current old-body dimensions and new-template dimensions.
pub fn replacement_position(
    mut position: Vector,
    old_radius: f32,
    old_length: f32,
    new_radius: f32,
    new_length: f32,
) -> Vector {
    position[1] += (new_radius + new_length * 0.5) - (old_radius + old_length * 0.5);
    position
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PolymorphEffect {
    /// NPC replacement changes the handle; avatars retain their identity.
    /// Native allocation failure/client execution can register a null target.
    pub target: Option<u16>,
    /// Avatar Template.ID or NPC logical Type, as captured by its native path.
    pub original_type: i32,
    pub ttl: f32,
}
impl PolymorphEffect {
    pub fn new(target: Option<u16>, original_type: i32) -> Self {
        Self {
            target,
            original_type,
            ttl: DURATION,
        }
    }
    pub fn update(&mut self, dt: f32, hit_points: Option<f32>) {
        self.ttl -= dt;
        // Update uses bgt.un: NaN health does not force TTL to zero.
        if self.target.is_none() || hit_points.is_none_or(|hp| hp <= 0.0) {
            self.ttl = 0.0;
        }
    }
    pub fn is_dead(&self) -> bool {
        self.ttl <= 0.0
    }
    /// OnRemove restores only a living target with a valid playstate/level.
    /// The host must rebuild from current state and then discard its AI storage,
    /// even if replacement allocation fails. NaN health does not restore.
    pub fn restoration(
        &self,
        hit_points: Option<f32>,
        level_available: bool,
    ) -> Option<(u16, i32)> {
        if level_available && hit_points.is_some_and(|hp| hp > 0.0) {
            self.target.map(|target| (target, self.original_type))
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PolymorphEffects {
    active: Vec<PolymorphEffect>,
}
impl PolymorphEffects {
    pub fn active(&self) -> &[PolymorphEffect] {
        &self.active
    }
    /// Call after transformation with the resulting target identity.
    pub fn add(&mut self, effect: PolymorphEffect) {
        self.active.push(effect);
    }
    /// Recasting removes/restores the matching effect immediately. The caller
    /// returns success without choosing a new form or playing new effects.
    pub fn take_for_recast(&mut self, target: u16) -> Option<PolymorphEffect> {
        let i = self.active.iter().position(|e| e.target == Some(target))?;
        Some(self.active.remove(i))
    }
    /// Return removals in insertion order for host OnRemove processing.
    pub fn update(
        &mut self,
        dt: f32,
        mut hit_points: impl FnMut(u16) -> Option<f32>,
    ) -> Vec<PolymorphEffect> {
        let mut removed = Vec::new();
        let mut i = 0;
        while i < self.active.len() {
            let effect = &mut self.active[i];
            let hp = effect.target.and_then(&mut hit_points);
            effect.update(dt, hp);
            if effect.is_dead() {
                removed.push(self.active.remove(i));
            } else {
                i += 1;
            }
        }
        removed
    }
    /// Nullify/ClearEffects runs the same restoration gate as ordinary removal.
    pub fn clear(&mut self) -> Vec<PolymorphEffect> {
        std::mem::take(&mut self.active)
    }
}
