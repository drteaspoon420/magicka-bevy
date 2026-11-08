//! EntityManager's wrapped query grid. Indices preserve entity identity and list
//! order; the world supplies synchronous shield intersection outputs.
use crate::{impact_motion::Vector, spray_pulse::Segment};

#[derive(Clone, Copy, Debug)]
pub struct QueryEntity {
    pub position: Vector,
    pub radius: f32,
    pub dead: bool,
    pub shield: bool,
    pub damageable: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryError {
    InvalidGeometry,
    CellOverflow,
}

fn cell(value: f32) -> Result<i32, QueryError> {
    let value = (value as f64).floor();
    if !value.is_finite() {
        return Err(QueryError::InvalidGeometry);
    }
    if value < i32::MIN as f64 || value > i32::MAX as f64 {
        return Err(QueryError::CellOverflow);
    }
    Ok(value as i32)
}
fn validate(position: Vector, radius: f32) -> Result<(), QueryError> {
    if position.iter().any(|x| !x.is_finite()) || !radius.is_finite() || radius < 0.0 {
        return Err(QueryError::InvalidGeometry);
    }
    Ok(())
}
fn distance_squared(a: Vector, b: Vector) -> f32 {
    let d = std::array::from_fn::<_, 3, _>(|i| a[i] - b[i]);
    d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
}

pub struct EntityQueryGrid {
    cells: [Vec<usize>; 256],
}
impl EntityQueryGrid {
    pub fn build(entities: &[QueryEntity]) -> Result<Self, QueryError> {
        let mut result = Self {
            cells: std::array::from_fn(|_| vec![]),
        };
        for (index, entity) in entities.iter().enumerate() {
            validate(entity.position, entity.radius)?;
            let x = entity.position[0] * 0.125;
            let z = entity.position[2] * 0.125;
            let radius = entity.radius * 0.125;
            let extent = radius.min(8.0);
            for cx in cell(x - extent)?..=cell(x + extent)? {
                let left = cx as f32 - x;
                let right = cx.wrapping_add(1) as f32 - x;
                let d = if left.abs() < right.abs() {
                    left
                } else {
                    right
                };
                let mut depth = ((radius * radius - d * d) as f64).sqrt() as f32;
                if depth.is_nan() {
                    depth = radius;
                }
                let depth = depth.min(extent);
                for cz in cell(z - depth)?..=cell(z + depth)? {
                    result.cells[(cx.rem_euclid(16) * 16 + cz.rem_euclid(16)) as usize].push(index);
                }
            }
        }
        Ok(result)
    }

    /// Grid snapshot and entity order must match. Geometry may reflect current
    /// world positions, as in the original separate grid-refresh/query operations.
    /// Shield callback returns both success and the out-position, even on misses.
    pub fn query(
        &self,
        entities: &[QueryEntity],
        mut center: Vector,
        radius: f32,
        ignore_protected: bool,
        ignore_y: bool,
        mut shield_segment: impl FnMut(usize, Segment) -> (bool, Vector),
    ) -> Result<Vec<usize>, QueryError> {
        validate(center, radius)?;
        if ignore_y {
            center[1] = 0.0;
        }
        let (mut x0, mut x1) = (
            cell((center[0] - radius) * 0.125)?,
            cell((center[0] + radius) * 0.125)?,
        );
        let (mut z0, mut z1) = (
            cell((center[2] - radius) * 0.125)?,
            cell((center[2] + radius) * 0.125)?,
        );
        if i64::from(x1) - i64::from(x0) > 16 {
            x0 = 0;
            x1 = 15;
        }
        if i64::from(z1) - i64::from(z0) > 16 {
            z0 = 0;
            z1 = 15;
        }
        let mut found = vec![];
        for x in x0..=x1 {
            for z in z0..=z1 {
                for &index in &self.cells[(x.rem_euclid(16) * 16 + z.rem_euclid(16)) as usize] {
                    let entity = &entities[index];
                    if entity.dead {
                        continue;
                    }
                    let mut position = entity.position;
                    if ignore_y {
                        position[1] = 0.0;
                    }
                    let extent = radius + entity.radius;
                    if distance_squared(position, center) <= extent * extent
                        && !found.contains(&index)
                    {
                        found.push(index);
                    }
                }
            }
        }
        if ignore_protected {
            let mut outer = 0;
            while outer < found.len() {
                let shield = found[outer];
                if entities[shield].shield {
                    let mut inner = 0;
                    while inner < found.len() {
                        if inner == outer {
                            inner += 1;
                            continue;
                        }
                        let target = &entities[found[inner]];
                        let segment = Segment {
                            origin: center,
                            delta: std::array::from_fn(|i| target.position[i] - center[i]),
                        };
                        let (hit, position) = shield_segment(shield, segment);
                        // Original bitwise OR evaluates the intersection even
                        // for non-damageable targets, then consumes its out value.
                        if (!target.damageable || hit) && distance_squared(position, center) > 1e-6
                        {
                            if inner < outer {
                                outer -= 1;
                            }
                            found.remove(inner);
                        } else {
                            inner += 1;
                        }
                    }
                }
                outer += 1;
            }
        }
        Ok(found)
    }
}
