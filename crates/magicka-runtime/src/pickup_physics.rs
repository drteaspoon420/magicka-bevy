//! Avian adapter for detached pickups. The runtime owns motion, not the viewer.
use crate::pickups::PickupBounds;
use magicka_content::level::LevelModel;
use magicka_physics::{BodyKey, BodyMode, BodyShape, BodyState, PhysicsError, RigidWorld};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct PickupBody {
    pub key: BodyKey,
    pub model: String,
    pub transform: [f32; 16],
    pub bounds: PickupBounds,
    pub book: bool,
    pub immovable: bool,
    pub mass: f32,
    pub solid: bool,
    pub shape: Option<BodyShape>,
}
struct Tracked {
    source: PickupBody,
    center: [f32; 3],
}
pub struct PickupPhysics {
    world: RigidWorld,
    tracked: BTreeMap<u16, Tracked>,
    /// Kinematic trimesh per animated level part (pre-order, as
    /// `SceneHit::animated_owner`), posed at the part's live world matrix.
    parts: Vec<Option<BodyKey>>,
    /// AnimatedLevelPart.mCollidingEntities for rigid bodies:
    /// (handle, part) -> remaining contact time.
    carried: BTreeMap<(u16, u32), f32>,
}

/// One animated level part's live motion for [`PickupPhysics::carry_on_parts`].
#[derive(Clone, Copy, Debug)]
pub struct PartMotion {
    pub world: [f32; 16],
    /// `inverse(previous world) * world` of the last part update.
    pub delta: [f32; 16],
    pub playing: bool,
}

/// Reserved key space for part colliders; entity handles never reach it.
const PART_HANDLE: u16 = u16::MAX;
/// AnimatedLevelPart.PostCollision keeps a supported entity this long.
const CONTACT_TIME: f32 = 0.25;
/// PostCollision's floor test (DirToBody.Y > 0.7).
const FLOOR_NORMAL_Y: f32 = 0.7;

fn clean_triangles(
    vertices: &[[f32; 3]],
    source: &[[i32; 3]],
) -> Result<Vec<[u32; 3]>, PhysicsError> {
    let mut triangles = Vec::new();
    for triangle in source {
        if triangle
            .iter()
            .any(|i| *i < 0 || *i as usize >= vertices.len())
        {
            return Err(PhysicsError::InvalidMesh);
        }
        let [a, b, c] = triangle.map(|i| vertices[i as usize]);
        let u: [f32; 3] = std::array::from_fn(|i| b[i] - a[i]);
        let v: [f32; 3] = std::array::from_fn(|i| c[i] - a[i]);
        let cross = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        // Decoded meshes may contain zero-area triangles. They do not
        // contribute collision; exclude them from the modern solver.
        if cross.iter().map(|v| v * v).sum::<f32>() > f32::EPSILON {
            triangles.push(triangle.map(|i| i as u32));
        }
    }
    Ok(triangles)
}

/// Rotate `v` by the unit quaternion `q` (x, y, z, w).
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let t = cross(u, v).map(|x| 2.0 * x);
    let c = cross(u, t);
    std::array::from_fn(|i| v[i] + q[3] * t[i] + c[i])
}

fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}
impl PickupPhysics {
    pub fn new(level: &LevelModel) -> Result<Self, PhysicsError> {
        let mut world = RigidWorld::new();
        // PhysicsManager initializes the simulator with Down * 9.82 * 2.
        world.set_gravity(magicka_sim::character_physics::GRAVITY)?;
        for mesh in &level.collision_layers {
            let triangles = clean_triangles(&mesh.vertices, &mesh.triangles)?;
            if !triangles.is_empty() {
                world.add_mesh(&mesh.vertices, &triangles)?;
            }
        }
        // Animated parts collide too (their skins are in the same JigLibX
        // collision system); they start at identity until the first
        // carry_on_parts poses them.
        fn walk(
            parts: &[magicka_content::level::AnimatedLevelPart],
            world: &mut RigidWorld,
            out: &mut Vec<Option<BodyKey>>,
        ) -> Result<(), PhysicsError> {
            for part in parts {
                let index = out.len() as u64;
                let mut key = None;
                if let Some((_, mesh)) = &part.collision {
                    let triangles = clean_triangles(&mesh.vertices, &mesh.triangles)?;
                    if !triangles.is_empty() {
                        let k = BodyKey {
                            handle: PART_HANDLE,
                            generation: index,
                        };
                        world.spawn_shape(
                            k,
                            BodyMode::Kinematic,
                            &BodyShape::Mesh {
                                vertices: mesh.vertices.clone(),
                                triangles,
                            },
                            1.0,
                            BodyState::at([0.0; 3]),
                            true,
                        )?;
                        key = Some(k);
                    }
                }
                out.push(key);
                walk(&part.children, world, out)?;
            }
            Ok(())
        }
        let mut parts = Vec::new();
        walk(&level.animated_parts, &mut world, &mut parts)?;
        Ok(Self {
            world,
            tracked: BTreeMap::new(),
            parts,
            carried: BTreeMap::new(),
        })
    }

    /// AnimatedLevelPart.Update for rigid bodies, run after the scene's part
    /// update and before integration: pose each part collider at its live
    /// world matrix, then transform every retained body by its part's
    /// `inverse(old) * new` delta (velocity kept, body woken as SetActive
    /// does) and age the contact, dropping it after this transform as the
    /// native loop does.
    pub fn carry_on_parts(&mut self, parts: &[PartMotion], dt: f32) -> Result<(), PhysicsError> {
        for (key, motion) in self.parts.iter().zip(parts) {
            if let Some(key) = key {
                self.world
                    .set_state(*key, BodyState::from_transform(motion.world, [0.0; 3]))?;
            }
        }
        let carried = std::mem::take(&mut self.carried);
        for ((handle, part), remaining) in carried {
            let (Some(tracked), Some(motion)) =
                (self.tracked.get(&handle), parts.get(part as usize))
            else {
                continue;
            };
            let key = tracked.source.key;
            let Some(state) = self.world.state(key) else {
                continue;
            };
            let m = magicka_content::skinned::mul(&state.model_transform([0.0; 3]), &motion.delta);
            let mut moved = BodyState::from_transform(m, [0.0; 3]);
            moved.linear_velocity = state.linear_velocity;
            moved.angular_velocity = state.angular_velocity;
            moved.sleeping = false;
            self.world.set_state(key, moved)?;
            let remaining = remaining - dt;
            if remaining > 0.0 {
                self.carried.insert((handle, part), remaining);
            }
        }
        Ok(())
    }

    /// AnimatedLevelPart.PostCollision for rigid bodies, after integration:
    /// a dynamic body resting on a playing part (`probe` returns the part
    /// and floor normal Y of the scene hit straight below its lowest point)
    /// is retained for at least 0.25 s. INFERRED: the downward probe stands
    /// in for JigLibX's contact list, which the modern solver does not
    /// expose.
    pub fn refresh_part_contacts(
        &mut self,
        playing: &[bool],
        mut probe: impl FnMut([f32; 3], f32) -> Option<(u32, f32)>,
    ) {
        for (&handle, tracked) in &self.tracked {
            if tracked.source.immovable {
                continue;
            }
            let Some(state) = self.world.state(tracked.source.key) else {
                continue;
            };
            let below = Self::extent_below(&tracked.source, tracked.center, state.rotation);
            let Some((part, normal_y)) = probe(state.position, below + 0.1) else {
                continue;
            };
            if normal_y > FLOOR_NORMAL_Y && playing.get(part as usize).copied().unwrap_or(false) {
                let t = self.carried.entry((handle, part)).or_insert(0.0);
                *t = t.max(CONTACT_TIME);
            }
        }
    }

    /// Distance from the body centre down to its lowest collision point.
    fn extent_below(source: &PickupBody, center: [f32; 3], rotation: [f32; 4]) -> f32 {
        match &source.shape {
            Some(BodyShape::Mesh { vertices, .. }) => vertices
                .iter()
                .map(|v| -rotate(rotation, std::array::from_fn(|i| v[i] - center[i]))[1])
                .fold(0.0, f32::max),
            Some(BodyShape::Box { size, rotation: r }) => {
                let q = quat_mul(rotation, *r);
                (0..3)
                    .map(|a| {
                        let mut axis = [0.0; 3];
                        axis[a] = size[a] * 0.5;
                        rotate(q, axis)[1].abs()
                    })
                    .sum()
            }
            None => {
                let mut size: [f32; 3] =
                    std::array::from_fn(|i| source.bounds.max[i] - source.bounds.min[i]);
                if source.book {
                    size[1] *= 2.0;
                }
                (0..3)
                    .map(|a| {
                        let mut axis = [0.0; 3];
                        axis[a] = size[a] * 0.5;
                        rotate(rotation, axis)[1].abs()
                    })
                    .sum()
            }
        }
    }

    /// Host-owned physics state with model origins, independent of Bevy IDs.
    pub fn network_snapshot(
        &self,
    ) -> Result<Vec<magicka_network::RemoteBody>, magicka_network::FrameError> {
        self.tracked
            .values()
            .map(|tracked| {
                let state = self
                    .world
                    .state(tracked.source.key)
                    .ok_or(magicka_network::FrameError::InvalidBody)?;
                Ok(magicka_network::RemoteBody::new(
                    tracked.source.key,
                    magicka_network::canonical_asset(&tracked.source.model)?,
                    tracked.center,
                    state,
                ))
            })
            .collect()
    }
    pub fn remove(&mut self, handle: u16) {
        if let Some(old) = self.tracked.remove(&handle) {
            self.world.remove(old.source.key);
        }
    }
    /// Dynamic tracked bodies: (handle, state, mass).
    pub fn dynamic_bodies(&self) -> Vec<(u16, BodyState, f32)> {
        self.tracked
            .iter()
            .filter(|(_, t)| !t.source.immovable)
            .filter_map(|(&h, t)| Some((h, self.world.state(t.source.key)?, t.source.mass)))
            .collect()
    }
    /// Generation-stamped bodies for Earthquake's EntityManager query. Radius
    /// comes from decoded collision bounds rather than a prop-size constant.
    pub fn earthquake_targets(&self) -> Vec<crate::scene_world::earthquake_live::RigidTarget> {
        self.tracked
            .values()
            .filter_map(|tracked| {
                let source = &tracked.source;
                let state = self.world.state(source.key)?;
                let radius = match &source.shape {
                    Some(BodyShape::Mesh { vertices, .. }) => vertices
                        .iter()
                        .map(|p| p.iter().map(|v| v * v).sum::<f32>().sqrt())
                        .fold(0.0f32, f32::max),
                    Some(BodyShape::Box { size, .. }) => {
                        size.iter().map(|v| v * v).sum::<f32>().sqrt() * 0.5
                    }
                    None => {
                        let mut sides: [f32; 3] =
                            std::array::from_fn(|i| source.bounds.max[i] - source.bounds.min[i]);
                        if source.book {
                            sides[1] *= 2.0;
                        }
                        sides.iter().map(|v| v * v).sum::<f32>().sqrt() * 0.5
                    }
                };
                Some(crate::scene_world::earthquake_live::RigidTarget {
                    key: source.key,
                    position: state.position,
                    radius,
                    mass: source.mass,
                })
            })
            .collect()
    }

    /// Ignore requests from an earlier lifetime of a reused entity handle.
    pub fn add_velocity_for(&mut self, key: BodyKey, dv: [f32; 3]) {
        if self
            .tracked
            .get(&key.handle)
            .is_some_and(|t| t.source.key == key)
        {
            self.add_velocity(key.handle, dv);
        }
    }
    /// Adds `dv` to a body's linear velocity and wakes it.
    pub fn add_velocity(&mut self, handle: u16, dv: [f32; 3]) {
        let Some(t) = self.tracked.get(&handle) else {
            return;
        };
        let key = t.source.key;
        if let Some(mut state) = self.world.state(key) {
            for (v, d) in state.linear_velocity.iter_mut().zip(dv) {
                *v += d;
            }
            state.sleeping = false;
            let _ = self.world.set_state(key, state);
        }
    }
    /// Teleport a tracked body to a model transform, at rest (editor
    /// placement). Untracked handles are ignored.
    pub fn place(&mut self, handle: u16, transform: [f32; 16]) {
        let Some(t) = self.tracked.get(&handle) else {
            return;
        };
        let mut state = BodyState::from_transform(transform, t.center);
        state.linear_velocity = [0.0; 3];
        state.angular_velocity = [0.0; 3];
        state.sleeping = false;
        let _ = self.world.set_state(t.source.key, state);
    }
    pub fn state(&self, handle: u16) -> Option<BodyState> {
        self.world.state(self.tracked.get(&handle)?.source.key)
    }
    pub fn step(
        &mut self,
        bodies: Vec<PickupBody>,
        dt: f32,
    ) -> Result<Vec<(u16, [f32; 16], BodyState)>, PhysicsError> {
        self.step_with_release_velocities(bodies, dt, &[])
    }

    /// Apply animation release velocities after body registration and before
    /// integration. Generation keys prevent requests affecting reused handles.
    pub fn step_with_release_velocities(
        &mut self,
        bodies: Vec<PickupBody>,
        dt: f32,
        velocities: &[(BodyKey, [f32; 3])],
    ) -> Result<Vec<(u16, [f32; 16], BodyState)>, PhysicsError> {
        let live: BTreeSet<_> = bodies.iter().map(|b| b.key.handle).collect();
        let removed: Vec<_> = self
            .tracked
            .keys()
            .filter(|h| !live.contains(h))
            .copied()
            .collect();
        for h in removed {
            self.remove(h);
        }
        for body in bodies {
            let changed = self.tracked.get(&body.key.handle).is_none_or(|old| {
                old.source.key != body.key
                    || old.source.model != body.model
                    || old.source.immovable != body.immovable
                    || old.source.transform != body.transform
                    || old.source.mass != body.mass
                    || old.source.solid != body.solid
                    || old.source.shape != body.shape
                    || old.source.bounds != body.bounds
            });
            if !changed {
                continue;
            }
            self.remove(body.key.handle);
            let center = std::array::from_fn(|i| (body.bounds.min[i] + body.bounds.max[i]) * 0.5);
            let mut size = std::array::from_fn(|i| body.bounds.max[i] - body.bounds.min[i]);
            if body.book {
                size[1] *= 2.0;
            }
            self.world.spawn_shape(
                body.key,
                if body.immovable {
                    BodyMode::Static
                } else {
                    BodyMode::Dynamic
                },
                body.shape.as_ref().unwrap_or(&BodyShape::Box {
                    size,
                    rotation: [0., 0., 0., 1.],
                }),
                body.mass,
                BodyState::from_transform(body.transform, center),
                body.solid,
            )?;
            self.tracked.insert(
                body.key.handle,
                Tracked {
                    source: body,
                    center,
                },
            );
        }
        for &(key, velocity) in velocities {
            if self
                .tracked
                .get(&key.handle)
                .is_some_and(|t| t.source.key == key)
            {
                if let Some(mut state) = self.world.state(key) {
                    state.linear_velocity = velocity;
                    state.sleeping = false;
                    self.world.set_state(key, state)?;
                }
            }
        }
        if self.tracked.is_empty() {
            return Ok(Vec::new());
        }
        self.world.step(dt)?;
        let mut poses = Vec::new();
        for (&handle, tracked) in &mut self.tracked {
            let state = self
                .world
                .state(tracked.source.key)
                .ok_or(PhysicsError::MissingBody)?;
            let transform = state.model_transform(tracked.center);
            tracked.source.transform = transform;
            poses.push((handle, transform, state));
        }
        Ok(poses)
    }
}
