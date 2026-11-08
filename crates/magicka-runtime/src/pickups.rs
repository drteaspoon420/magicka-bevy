//! Pickup selection and the represented portion of native Item.Swap.
//! Spatial ordering and side effects (triggers, physics, visuals) belong to the host.
use crate::{animation_controller::WEAPON_CLASS_STAFF, avatar::AvatarItem};
use magicka_sim::{box_intersection::OrientedBox, spray_pulse::Segment};

/// Model-space collision bounds for a detached pickup. Books double Y size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickupBounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}
impl PickupBounds {
    pub fn world_box(&self, transform: &[f32; 16], book: bool) -> OrientedBox {
        let center: [f32; 3] = std::array::from_fn(|i| (self.min[i] + self.max[i]) * 0.5);
        let mut side_lengths = std::array::from_fn(|i| self.max[i] - self.min[i]);
        if book {
            side_lengths[1] *= 2.0;
        }
        OrientedBox {
            center: std::array::from_fn(|i| {
                center[0] * transform[i]
                    + center[1] * transform[4 + i]
                    + center[2] * transform[8 + i]
                    + transform[12 + i]
            }),
            axes: [
                [transform[0], transform[1], transform[2]],
                [transform[4], transform[5], transform[6]],
                [transform[8], transform[9], transform[10]],
            ],
            side_lengths,
        }
    }
}

/// KeyboardMouseController.FindPickUp: query around the cursor plane point
/// (radius 6, including Y), then the first pickable collision box hit in host
/// enumeration order. This deliberately does not sort by ray-hit distance.
pub fn mouse_pickup(
    candidates: impl IntoIterator<Item = (PickupCandidate, OrientedBox)>,
    segment: Segment,
    plane: [f32; 3],
) -> Option<u16> {
    candidates.into_iter().find_map(|(item, bounds)| {
        let dx = item.position[0] - plane[0];
        let dy = item.position[1] - plane[1];
        let dz = item.position[2] - plane[2];
        (item.pickable
            && !item.dead
            && dx * dx + dy * dy + dz * dz <= (6.0 + item.radius).powi(2)
            && bounds.segment_intersect(segment).is_some())
        .then_some(item.handle)
    })
}

/// Avatar.FindPickUp queries EntityManager with this radius, including Y.
pub const PICKUP_QUERY_RADIUS: f32 = 2.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickupCandidate {
    pub handle: u16,
    pub position: [f32; 3],
    pub radius: f32,
    pub pickable: bool,
    pub dead: bool,
}

/// Both FindPickUp overloads: preserve host query order, with optional exact
/// target matching. Keyboard/mouse passes check_direction=false; pads check
/// the forward hemisphere unless the pickup lies within the avatar radius.
pub fn find_pickup(
    candidates: impl IntoIterator<Item = PickupCandidate>,
    position: [f32; 3],
    direction: [f32; 3],
    avatar_radius: f32,
    check_direction: bool,
    target: Option<u16>,
) -> Option<u16> {
    for item in candidates {
        if item.dead || !item.pickable || target.is_some_and(|h| h != item.handle) {
            continue;
        }
        let d = std::array::from_fn::<_, 3, _>(|i| item.position[i] - position[i]);
        let query = PICKUP_QUERY_RADIUS + item.radius;
        let in_query = d.iter().map(|v| v * v).sum::<f32>() <= query * query;
        if !in_query {
            continue;
        }
        let horizontal = (d[0] * d[0] + d[2] * d[2]).sqrt();
        if horizontal <= avatar_radius
            || !check_direction
            || (direction[0] * d[0] + direction[2] * d[2]) / horizontal > 0.0
        {
            return Some(item.handle);
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquipmentExchange {
    Bound,
    Swapped,
}

/// Native InternalPickUp: Staff goes to equipment index 1, all others to 0.
pub fn equipment_slot(item: &AvatarItem) -> i32 {
    if item.definition.weapon_class == WEAPON_CLASS_STAFF {
        1
    } else {
        0
    }
}

/// Exchange the item definition, combat/spell state and cooldowns without
/// replacing the avatar. Frames remain owned by their spatial hosts and are
/// recomputed after attachment geometry changes. The host must stop old item
/// effects, move/enable the dropped item, update attachments and animation
/// classes, run OnPickup (even before a bound rejection), and request the pickup
/// animation. Those side effects are deliberately not hidden here.
pub fn exchange_equipment(held: &mut AvatarItem, ground: &mut AvatarItem) -> EquipmentExchange {
    if held.definition.bound {
        return EquipmentExchange::Bound;
    }
    let held_frame = held.frame.take();
    let ground_frame = ground.frame.take();
    let held_attached = held.attached;
    let ground_attached = ground.attached;
    std::mem::swap(held, ground);
    held.attached = held_attached;
    ground.attached = ground_attached;
    held.frame = held_frame;
    ground.frame = ground_frame;
    EquipmentExchange::Swapped
}

/// Detached item state shared by scripted and developer spawning. PickupPhysics
/// owns rigid bodies and writes motion back to these placement records.
pub struct WorldItemPickup {
    /// Changes when a scripted placeholder is populated again.
    pub generation: u64,
    pub item: Option<AvatarItem>,
    pub transform: [f32; 16],
    pub on_pickup: i32,
    pub immovable: bool,
    pub ignore_tractor_pull: bool,
    pub body_enabled: bool,
    pub animated_part: Option<i32>,
    /// Item.mDespawnTime (0: permanent).
    pub despawn: f32,
}
impl Default for WorldItemPickup {
    fn default() -> Self {
        Self {
            generation: 0,
            item: None,
            transform: magicka_content::skinned::IDENTITY,
            on_pickup: 0,
            immovable: true,
            ignore_tractor_pull: false,
            body_enabled: false,
            animated_part: None,
            despawn: 0.0,
        }
    }
}
