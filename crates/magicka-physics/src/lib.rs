//! Modern rigid-body backend. No renderer, target assets or retail networking.
//! Body keys belong to gameplay; Bevy entity IDs never leave this host.
use avian3d::prelude::*;
use bevy::{prelude::*, time::TimeUpdateStrategy};
use std::{collections::BTreeMap, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BodyKey {
    pub handle: u16,
    pub generation: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyMode {
    Static,
    Kinematic,
    Dynamic,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyState {
    pub position: [f32; 3],
    /// Quaternion x,y,z,w.
    pub rotation: [f32; 4],
    pub linear_velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub sleeping: bool,
}
impl BodyState {
    /// Retail row-vector matrices have the same storage as column-vector
    /// matrices here. Strip scale: rigid-body orientation is a rotation.
    pub fn from_transform(transform: [f32; 16], center: [f32; 3]) -> Self {
        let axes = Mat3::from_cols(
            Vec3::new(transform[0], transform[1], transform[2]).normalize(),
            Vec3::new(transform[4], transform[5], transform[6]).normalize(),
            Vec3::new(transform[8], transform[9], transform[10]).normalize(),
        );
        let rotation = Quat::from_mat3(&axes).normalize();
        let position = Vec3::new(transform[12], transform[13], transform[14])
            + rotation * Vec3::from_array(center);
        Self {
            position: position.to_array(),
            rotation: rotation.to_array(),
            ..Self::at([0.0; 3])
        }
    }
    pub fn model_transform(&self, center: [f32; 3]) -> [f32; 16] {
        let rotation = Quat::from_array(self.rotation);
        let position = Vec3::from_array(self.position) - rotation * Vec3::from_array(center);
        Mat4::from_rotation_translation(rotation, position).to_cols_array()
    }
    pub fn at(position: [f32; 3]) -> Self {
        Self {
            position,
            rotation: [0.0, 0.0, 0.0, 1.0],
            linear_velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
            sleeping: false,
        }
    }
    fn valid(&self) -> bool {
        self.position
            .iter()
            .chain(&self.rotation)
            .chain(&self.linear_velocity)
            .chain(&self.angular_velocity)
            .all(|v| v.is_finite())
            && {
                let norm = self.rotation.iter().map(|v| v * v).sum::<f32>();
                norm.is_finite() && norm > 0.0
            }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicsError {
    DuplicateKey,
    InvalidBody,
    InvalidMesh,
    MissingBody,
    InvalidDelta,
}

/// Collision data independent of Bevy entity identities.
#[derive(Clone, Debug, PartialEq)]
pub enum BodyShape {
    Box {
        size: [f32; 3],
        rotation: [f32; 4],
    },
    Mesh {
        vertices: Vec<[f32; 3]>,
        triangles: Vec<[u32; 3]>,
    },
}

pub struct RigidWorld {
    world: World,
    bodies: BTreeMap<BodyKey, Entity>,
    pub ticks: u64,
}
impl Default for RigidWorld {
    fn default() -> Self {
        Self::new()
    }
}
impl RigidWorld {
    pub fn new() -> Self {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, PhysicsPlugins::new(Update)))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
        app.finish();
        app.cleanup();
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .set_max_delta(Duration::MAX);
        // Initialize Bevy time without consuming the caller's first delta.
        app.update();
        Self {
            world: std::mem::take(app.world_mut()),
            bodies: BTreeMap::new(),
            ticks: 0,
        }
    }
    /// Host-selected world acceleration, shared by all dynamic bodies.
    pub fn set_gravity(&mut self, acceleration: [f32; 3]) -> Result<(), PhysicsError> {
        if acceleration.iter().any(|v| !v.is_finite()) {
            return Err(PhysicsError::InvalidBody);
        }
        self.world
            .insert_resource(Gravity(Vec3::from_array(acceleration)));
        Ok(())
    }

    pub fn spawn_box(
        &mut self,
        key: BodyKey,
        mode: BodyMode,
        size: [f32; 3],
        mass: f32,
        state: BodyState,
    ) -> Result<(), PhysicsError> {
        self.spawn_shape(
            key,
            mode,
            &BodyShape::Box {
                size,
                rotation: [0., 0., 0., 1.],
            },
            mass,
            state,
            true,
        )
    }
    /// Dynamic meshes use a convex hull; static meshes retain their triangles.
    pub fn spawn_shape(
        &mut self,
        key: BodyKey,
        mode: BodyMode,
        shape: &BodyShape,
        mass: f32,
        state: BodyState,
        solid: bool,
    ) -> Result<(), PhysicsError> {
        if self.bodies.contains_key(&key) {
            return Err(PhysicsError::DuplicateKey);
        }
        if !state.valid() || !mass.is_finite() || mass <= 0.0 {
            return Err(PhysicsError::InvalidBody);
        }
        let collider = match shape {
            BodyShape::Box { size, rotation } => {
                let rotation = Quat::from_array(*rotation);
                if size.iter().any(|v| !v.is_finite() || *v <= 0.)
                    || !rotation.is_finite()
                    || rotation.length_squared() <= f32::EPSILON
                {
                    return Err(PhysicsError::InvalidBody);
                }
                Collider::compound(vec![(
                    Vec3::ZERO,
                    rotation.normalize(),
                    Collider::cuboid(size[0], size[1], size[2]),
                )])
            }
            BodyShape::Mesh {
                vertices,
                triangles,
            } => {
                if vertices.is_empty()
                    || triangles.is_empty()
                    || vertices.iter().flatten().any(|v| !v.is_finite())
                    || triangles
                        .iter()
                        .flatten()
                        .any(|i| *i as usize >= vertices.len())
                {
                    return Err(PhysicsError::InvalidMesh);
                }
                let vertices = vertices.iter().copied().map(Vec3::from_array).collect();
                if mode == BodyMode::Dynamic {
                    Collider::convex_hull(vertices).ok_or(PhysicsError::InvalidMesh)?
                } else {
                    Collider::trimesh(vertices, triangles.clone())
                }
            }
        };
        let mode = match mode {
            BodyMode::Static => RigidBody::Static,
            BodyMode::Kinematic => RigidBody::Kinematic,
            BodyMode::Dynamic => RigidBody::Dynamic,
        };
        let entity = self
            .world
            .spawn((
                mode,
                collider,
                Mass(mass),
                Position(Vec3::from_array(state.position)),
                Rotation(Quat::from_array(state.rotation).normalize()),
                Transform::from_translation(Vec3::from_array(state.position))
                    .with_rotation(Quat::from_array(state.rotation).normalize()),
                LinearVelocity(Vec3::from_array(state.linear_velocity)),
                AngularVelocity(Vec3::from_array(state.angular_velocity)),
            ))
            .id();
        if !solid {
            self.world.entity_mut(entity).insert(Sensor);
        }
        if state.sleeping {
            self.world.entity_mut(entity).insert(Sleeping);
        }
        self.bodies.insert(key, entity);
        Ok(())
    }
    /// Static collision from authored or decoded scene triangles.
    pub fn add_mesh(
        &mut self,
        vertices: &[[f32; 3]],
        triangles: &[[u32; 3]],
    ) -> Result<(), PhysicsError> {
        if vertices.iter().flatten().any(|v| !v.is_finite())
            || triangles.is_empty()
            || triangles
                .iter()
                .flatten()
                .any(|i| *i as usize >= vertices.len())
        {
            return Err(PhysicsError::InvalidMesh);
        }
        for t in triangles {
            let a = Vec3::from_array(vertices[t[0] as usize]);
            let b = Vec3::from_array(vertices[t[1] as usize]);
            let c = Vec3::from_array(vertices[t[2] as usize]);
            if (b - a).cross(c - a).length_squared() <= f32::EPSILON {
                return Err(PhysicsError::InvalidMesh);
            }
        }
        self.world.spawn((
            RigidBody::Static,
            Collider::trimesh(
                vertices.iter().copied().map(Vec3::from_array).collect(),
                triangles.to_vec(),
            ),
        ));
        Ok(())
    }
    pub fn remove(&mut self, key: BodyKey) -> bool {
        self.bodies
            .remove(&key)
            .is_some_and(|entity| self.world.despawn(entity))
    }
    pub fn state(&self, key: BodyKey) -> Option<BodyState> {
        let entity = self.world.get_entity(*self.bodies.get(&key)?).ok()?;
        Some(BodyState {
            position: entity.get::<Position>()?.0.to_array(),
            rotation: entity.get::<Rotation>()?.0.to_array(),
            linear_velocity: entity.get::<LinearVelocity>()?.0.to_array(),
            angular_velocity: entity.get::<AngularVelocity>()?.0.to_array(),
            sleeping: entity.contains::<Sleeping>(),
        })
    }
    /// Stable ordered snapshots suitable for an authoritative network host.
    pub fn snapshot(&self) -> Vec<(BodyKey, BodyState)> {
        self.bodies
            .keys()
            .filter_map(|key| self.state(*key).map(|state| (*key, state)))
            .collect()
    }
    pub fn set_state(&mut self, key: BodyKey, state: BodyState) -> Result<(), PhysicsError> {
        if !state.valid() {
            return Err(PhysicsError::InvalidBody);
        }
        let entity = *self.bodies.get(&key).ok_or(PhysicsError::MissingBody)?;
        let mut e = self.world.entity_mut(entity);
        e.insert((
            Position(Vec3::from_array(state.position)),
            Rotation(Quat::from_array(state.rotation).normalize()),
            Transform::from_translation(Vec3::from_array(state.position))
                .with_rotation(Quat::from_array(state.rotation).normalize()),
            LinearVelocity(Vec3::from_array(state.linear_velocity)),
            AngularVelocity(Vec3::from_array(state.angular_velocity)),
        ));
        if state.sleeping {
            e.insert(Sleeping);
        } else {
            e.remove::<Sleeping>();
        }
        Ok(())
    }
    /// Caller owns timestep policy; zero delta leaves time and bodies untouched.
    pub fn step(&mut self, dt: f32) -> Result<(), PhysicsError> {
        if !dt.is_finite() || dt < 0.0 {
            return Err(PhysicsError::InvalidDelta);
        }
        if dt == 0.0 {
            return Ok(());
        }
        let duration = Duration::try_from_secs_f32(dt).map_err(|_| PhysicsError::InvalidDelta)?;
        self.world
            .insert_resource(TimeUpdateStrategy::ManualDuration(duration));
        self.world.run_schedule(bevy::app::Main);
        self.ticks += 1;
        Ok(())
    }
}
