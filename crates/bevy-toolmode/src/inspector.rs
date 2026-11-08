//! Reflection-based entity inspector: pick an entity (inspect tool, entity
//! browser, or [`Inspector::entity`] from game code), then view and edit its
//! reflected components, add registered components with a default value,
//! and remove components.
//!
//! Cost: nothing while the window is closed. While open, only the selected
//! entity's *expanded* components are walked. Edits go through
//! `Mut::bypass_change_detection` and the component is marked changed only
//! when a value was actually edited, so an open inspector does not make
//! `Changed<T>` systems run every frame.
use crate::{tools, Selectable, ToolButtons, ToolMode, ToolRay};
use bevy::{
    camera::primitives::Aabb,
    ecs::world::EntityMut,
    picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings},
    prelude::*,
    reflect::{
        enums::{DynamicEnum, DynamicVariant, VariantInfo},
        ReflectMut, ReflectRef, TypeInfo,
    },
};
use bevy_egui::{egui, EguiContext, PrimaryEguiContext};

#[derive(Resource, Default)]
pub struct Inspector {
    pub open: bool,
    pub entity: Option<Entity>,
    browser_filter: String,
    add_filter: String,
    /// Components expanded in the window, by type name.
    expanded: Vec<String>,
}

/// Distance along `ray` to an oriented box (local `aabb` under `transform`).
pub fn ray_obb(ray: Ray3d, aabb: &Aabb, transform: &GlobalTransform) -> Option<f32> {
    let inverse = transform.affine().inverse();
    let origin = inverse.transform_point3(ray.origin);
    let dir = inverse.transform_vector3(*ray.direction);
    let min = Vec3::from(aabb.min());
    let max = Vec3::from(aabb.max());
    let (mut near, mut far) = (f32::MIN, f32::MAX);
    for i in 0..3 {
        if dir[i].abs() < 1e-9 {
            if origin[i] < min[i] || origin[i] > max[i] {
                return None;
            }
            continue;
        }
        let a = (min[i] - origin[i]) / dir[i];
        let b = (max[i] - origin[i]) / dir[i];
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    // The local direction keeps the world parameterisation (same t).
    (near <= far && far >= 0.0).then(|| near.max(0.0))
}

/// Inspect tool: a [`Selectable`] under the cursor wins (characters and
/// props, whose meshes may be skinned); otherwise the first mesh triangle
/// the ray hits. Bounding boxes alone are not used: large level meshes
/// would swallow every click. A hit on an unnamed glTF primitive selects
/// its named node.
pub(crate) fn pick(
    mode: Res<ToolMode>,
    buttons: Res<ToolButtons>,
    ray: Res<ToolRay>,
    mut inspector: ResMut<Inspector>,
    mut meshes: MeshRayCast,
    selectables: Query<(Entity, &Selectable, &GlobalTransform)>,
    parents: Query<&ChildOf>,
    names: Query<(), With<Name>>,
) {
    if !mode.active || mode.tool != tools::INSPECT || !buttons.primary_pressed {
        return;
    }
    let Some(ray) = ray.ray else {
        return;
    };
    let selectable = selectables
        .iter()
        .filter_map(|(e, s, g)| {
            let center = g.translation() + Vec3::Y * s.center_height;
            tools::ray_sphere(ray, center, s.radius).map(|t| (t, e))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, e)| e);
    let picked = selectable.or_else(|| {
        let hit = meshes
            .cast_ray(ray, &MeshRayCastSettings::default())
            .first()
            .map(|(e, _)| *e)?;
        Some(match parents.get(hit) {
            Ok(parent) if !names.contains(hit) && names.contains(parent.parent()) => {
                parent.parent()
            }
            _ => hit,
        })
    });
    if let Some(e) = picked {
        inspector.entity = Some(e);
        inspector.open = true;
    }
}

const MAX_DEPTH: usize = 10;
const MAX_ELEMENTS: usize = 64;

fn number<T: egui::emath::Numeric>(ui: &mut egui::Ui, v: &mut T) -> bool {
    ui.add(egui::DragValue::new(v).speed(0.05)).changed()
}

/// Opaque leaf editors; returns Some(changed) when the type is known.
fn leaf(ui: &mut egui::Ui, value: &mut dyn PartialReflect) -> Option<bool> {
    macro_rules! numbers {
        ($($t:ty),*) => {
            $(if let Some(v) = value.try_downcast_mut::<$t>() {
                return Some(number(ui, v));
            })*
        };
    }
    numbers!(f32, f64, i8, i16, i32, i64, u8, u16, u32, u64, usize, isize);
    if let Some(v) = value.try_downcast_mut::<bool>() {
        return Some(ui.checkbox(v, "").changed());
    }
    if let Some(v) = value.try_downcast_mut::<String>() {
        return Some(ui.text_edit_singleline(v).changed());
    }
    if let Some(q) = value.try_downcast_mut::<Quat>() {
        let (y, x, z) = q.to_euler(EulerRot::YXZ);
        let mut d = [y.to_degrees(), x.to_degrees(), z.to_degrees()];
        let mut changed = false;
        ui.horizontal(|ui| {
            for (label, v) in ["yaw", "pitch", "roll"].iter().zip(d.iter_mut()) {
                ui.label(*label);
                changed |= ui
                    .add(egui::DragValue::new(v).speed(0.5).suffix("°"))
                    .changed();
            }
        });
        if changed {
            *q = Quat::from_euler(
                EulerRot::YXZ,
                d[0].to_radians(),
                d[1].to_radians(),
                d[2].to_radians(),
            );
        }
        return Some(changed);
    }
    if let Some(c) = value.try_downcast_mut::<Color>() {
        let s = c.to_srgba();
        let mut rgba = [s.red, s.green, s.blue, s.alpha];
        let changed = ui.color_edit_button_rgba_unmultiplied(&mut rgba).changed();
        if changed {
            *c = Color::srgba(rgba[0], rgba[1], rgba[2], rgba[3]);
        }
        return Some(changed);
    }
    None
}

/// Edit any reflected value; returns whether something changed.
pub fn edit_value(ui: &mut egui::Ui, value: &mut dyn PartialReflect, depth: usize) -> bool {
    if let Some(changed) = leaf(ui, value) {
        return changed;
    }
    if depth >= MAX_DEPTH {
        ui.label("…");
        return false;
    }
    let mut changed = false;
    // Enum variant switching needs the type info before borrowing mutably.
    let info = value.get_represented_type_info();
    match value.reflect_mut() {
        ReflectMut::Struct(s) => {
            for i in 0..s.field_len() {
                let name = s.name_at(i).unwrap_or("?").to_string();
                if let Some(field) = s.field_at_mut(i) {
                    changed |= labelled(ui, &name, field, depth);
                }
            }
        }
        ReflectMut::TupleStruct(s) => {
            for i in 0..s.field_len() {
                if let Some(field) = s.field_mut(i) {
                    changed |= labelled(ui, &i.to_string(), field, depth);
                }
            }
        }
        ReflectMut::Tuple(t) => {
            for i in 0..t.field_len() {
                if let Some(field) = t.field_mut(i) {
                    changed |= labelled(ui, &i.to_string(), field, depth);
                }
            }
        }
        ReflectMut::List(l) => {
            ui.label(format!("{} elements", l.len()));
            for i in 0..l.len().min(MAX_ELEMENTS) {
                if let Some(e) = l.get_mut(i) {
                    changed |= labelled(ui, &format!("[{i}]"), e, depth);
                }
            }
        }
        ReflectMut::Array(a) => {
            for i in 0..a.len().min(MAX_ELEMENTS) {
                if let Some(e) = a.get_mut(i) {
                    changed |= labelled(ui, &format!("[{i}]"), e, depth);
                }
            }
        }
        ReflectMut::Enum(e) => {
            let current = e.variant_name().to_string();
            let mut switch_to = None;
            if let Some(TypeInfo::Enum(info)) = info {
                // Only unit variants can be switched to without field data.
                let units: Vec<&str> = (0..info.variant_len())
                    .filter_map(|i| match info.variant_at(i) {
                        Some(VariantInfo::Unit(u)) => Some(u.name()),
                        _ => None,
                    })
                    .collect();
                egui::ComboBox::from_id_salt(ui.next_auto_id())
                    .selected_text(&current)
                    .show_ui(ui, |ui| {
                        for name in units {
                            if ui.selectable_label(name == current, name).clicked()
                                && name != current
                            {
                                switch_to = Some(name.to_string());
                            }
                        }
                    });
            } else {
                ui.label(&current);
            }
            for i in 0..e.field_len() {
                let name = e.name_at(i).map_or_else(|| i.to_string(), str::to_string);
                if let Some(field) = e.field_at_mut(i) {
                    changed |= labelled(ui, &name, field, depth);
                }
            }
            if let Some(name) = switch_to {
                e.apply(&DynamicEnum::new(name, DynamicVariant::Unit));
                changed = true;
            }
        }
        ReflectMut::Map(m) => {
            ui.label(format!("map, {} entries (read-only)", m.len()));
        }
        ReflectMut::Set(s) => {
            ui.label(format!("set, {} entries (read-only)", s.len()));
        }
        ReflectMut::Opaque(v) => {
            ui.label(egui::RichText::new(format!("{v:?}")).weak());
        }
        #[allow(unreachable_patterns)]
        _ => {
            ui.label("(unsupported)");
        }
    }
    changed
}

fn labelled(ui: &mut egui::Ui, name: &str, value: &mut dyn PartialReflect, depth: usize) -> bool {
    let simple = matches!(value.reflect_ref(), ReflectRef::Opaque(_))
        || value.try_downcast_ref::<Quat>().is_some();
    if simple {
        ui.horizontal(|ui| {
            ui.label(name);
            edit_value(ui, value, depth + 1)
        })
        .inner
    } else {
        egui::CollapsingHeader::new(name)
            .id_salt(ui.next_auto_id())
            .show(ui, |ui| edit_value(ui, value, depth + 1))
            .body_returned
            .unwrap_or(false)
    }
}

fn entity_label(world: &World, e: Entity) -> String {
    match world.get::<Name>(e) {
        Some(name) => format!("{name} ({e})"),
        None => format!("{e}"),
    }
}

/// Exclusive: draws the inspector window with full world access.
pub fn window(world: &mut World) {
    if !world.resource::<Inspector>().open {
        return;
    }
    let Ok(mut ctx) = world
        .query_filtered::<&mut EguiContext, With<PrimaryEguiContext>>()
        .single_mut(world)
        .map(|mut c| c.get_mut().clone())
    else {
        return;
    };
    let mut inspector = world.remove_resource::<Inspector>().unwrap();
    let mut open = true;
    egui::Window::new("Entity inspector")
        .id(egui::Id::new("toolmode_inspector"))
        .open(&mut open)
        .default_pos([60.0, 80.0])
        .default_size([380.0, 560.0])
        .vscroll(true)
        .show(&mut ctx, |ui| contents(ui, world, &mut inspector));
    inspector.open = open;
    world.insert_resource(inspector);
}

fn contents(ui: &mut egui::Ui, world: &mut World, inspector: &mut Inspector) {
    egui::CollapsingHeader::new("Entity browser").show(ui, |ui| {
        ui.add(
            egui::TextEdit::singleline(&mut inspector.browser_filter)
                .hint_text("Filter by name..."),
        );
        let filter = inspector.browser_filter.to_ascii_lowercase();
        let mut named: Vec<(Entity, String)> = world
            .query::<(Entity, &Name)>()
            .iter(world)
            .filter(|(_, n)| filter.is_empty() || n.as_str().to_ascii_lowercase().contains(&filter))
            .map(|(e, n)| (e, n.to_string()))
            .take(300)
            .collect();
        named.sort_by(|a, b| a.1.cmp(&b.1));
        egui::ScrollArea::vertical()
            .id_salt("inspector_browser")
            .max_height(180.0)
            .show(ui, |ui| {
                for (e, name) in named {
                    if ui
                        .selectable_label(inspector.entity == Some(e), format!("{name} ({e})"))
                        .clicked()
                    {
                        inspector.entity = Some(e);
                    }
                }
            });
    });
    let Some(entity) = inspector.entity.filter(|e| world.get_entity(*e).is_ok()) else {
        ui.label("No entity. Use the Inspect tool on something, or pick one in the browser.");
        return;
    };
    ui.heading(entity_label(world, entity));
    ui.horizontal_wrapped(|ui| {
        if let Some(parent) = world.get::<ChildOf>(entity).map(ChildOf::parent) {
            if ui
                .button(format!("parent: {}", entity_label(world, parent)))
                .clicked()
            {
                inspector.entity = Some(parent);
            }
        }
        if ui.button("despawn").clicked() {
            world.despawn(entity);
            inspector.entity = None;
        }
    });
    if inspector.entity.is_none() {
        return;
    }
    if let Some(children) = world.get::<Children>(entity) {
        let children: Vec<Entity> = children.iter().collect();
        egui::CollapsingHeader::new(format!("children ({})", children.len())).show(ui, |ui| {
            for c in children.into_iter().take(MAX_ELEMENTS) {
                if ui.small_button(entity_label(world, c)).clicked() {
                    inspector.entity = Some(c);
                }
            }
        });
    }
    ui.separator();
    let registry = world.resource::<AppTypeRegistry>().clone();
    let registry = registry.read();
    let components: Vec<(
        bevy::ecs::component::ComponentId,
        String,
        Option<std::any::TypeId>,
        bool,
    )> = world
        .inspect_entity(entity)
        .map(|infos| {
            infos
                .map(|i| (i.id(), i.name().to_string(), i.type_id(), i.mutable()))
                .collect()
        })
        .unwrap_or_default();
    let mut remove: Option<(bevy::ecs::component::ComponentId, String)> = None;
    for (id, name, type_id, mutable) in components {
        let short = name.rsplit("::").next().unwrap_or(&name).to_string();
        let reflect = type_id
            .and_then(|t| registry.get(t))
            .and_then(|r| r.data::<ReflectComponent>());
        let expanded = inspector.expanded.contains(&name);
        let header = egui::CollapsingHeader::new(if reflect.is_some() {
            egui::RichText::new(&short)
        } else {
            egui::RichText::new(&short).weak()
        })
        .id_salt(&name)
        .open(Some(expanded));
        let response = header.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(&name).small().weak());
                if ui.small_button("remove").clicked() {
                    remove = Some((id, short.clone()));
                }
            });
            let Some(reflect) = reflect else {
                ui.label("not reflected (register the type to edit it)");
                return;
            };
            if mutable {
                if let Some(mut value) =
                    reflect.reflect_mut(EntityMut::from(world.entity_mut(entity)))
                {
                    let changed = edit_value(
                        ui,
                        value.bypass_change_detection().as_partial_reflect_mut(),
                        0,
                    );
                    if changed {
                        value.set_changed();
                    }
                }
            } else if let Some(value) = reflect.reflect(world.entity(entity)) {
                ui.label("immutable component (read-only)");
                ui.label(
                    egui::RichText::new(format!("{value:?}"))
                        .monospace()
                        .small(),
                );
            }
        });
        if response.header_response.clicked() {
            if expanded {
                inspector.expanded.retain(|n| *n != name);
            } else {
                inspector.expanded.push(name.clone());
            }
        }
    }
    if let Some((id, short)) = remove {
        world.entity_mut(entity).remove_by_id(id);
        ui.label(format!("removed {short}"));
    }
    ui.separator();
    ui.collapsing("Add component", |ui| {
        ui.add(egui::TextEdit::singleline(&mut inspector.add_filter).hint_text("Type name..."));
        let filter = inspector.add_filter.to_ascii_lowercase();
        if filter.len() < 2 {
            ui.label("Type at least two letters.");
            return;
        }
        let mut addable: Vec<(&str, std::any::TypeId)> = registry
            .iter()
            .filter(|r| {
                r.data::<ReflectComponent>().is_some() && r.data::<ReflectDefault>().is_some()
            })
            .map(|r| (r.type_info().type_path(), r.type_id()))
            .filter(|(path, _)| path.to_ascii_lowercase().contains(&filter))
            .collect();
        addable.sort();
        let mut add = None;
        egui::ScrollArea::vertical()
            .id_salt("inspector_add")
            .max_height(200.0)
            .show(ui, |ui| {
                for (path, type_id) in addable.into_iter().take(100) {
                    if ui.small_button(path).clicked() {
                        add = Some(type_id);
                    }
                }
            });
        if let Some(type_id) = add {
            let registration = registry.get(type_id).unwrap();
            let value = registration.data::<ReflectDefault>().unwrap().default();
            registration.data::<ReflectComponent>().unwrap().insert(
                &mut world.entity_mut(entity),
                value.as_partial_reflect(),
                &registry,
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ray_obb_respects_rotation_and_scale() {
        let aabb = Aabb::from_min_max(Vec3::splat(-1.0), Vec3::splat(1.0));
        let t = GlobalTransform::from(
            Transform::from_xyz(10.0, 0.0, 0.0)
                .with_scale(Vec3::new(2.0, 1.0, 1.0))
                .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
        );
        // Rotated 90 degrees: the x-scaled extent now lies along world z.
        let along_x = Ray3d::new(Vec3::ZERO, Dir3::X);
        assert!((ray_obb(along_x, &aabb, &t).unwrap() - 9.0).abs() < 1e-4);
        let along_z = Ray3d::new(Vec3::new(10.0, 0.0, -10.0), Dir3::Z);
        assert!((ray_obb(along_z, &aabb, &t).unwrap() - 8.0).abs() < 1e-4);
        let miss = Ray3d::new(Vec3::new(0.0, 3.0, 0.0), Dir3::X);
        assert!(ray_obb(miss, &aabb, &t).is_none());
    }

    #[derive(Reflect, Default)]
    struct Sample {
        speed: f32,
        enabled: bool,
        name: String,
    }

    #[test]
    fn edit_value_walks_struct_fields_without_reporting_changes() {
        let mut value = Sample::default();
        let ctx = egui::Context::default();
        let mut changed = true;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            changed = edit_value(ui, &mut value, 0);
        });
        // egui panics when texture deltas drop unapplied; this test has no renderer.
        output.textures_delta.clear();
        assert!(!changed, "drawing alone must not count as an edit");
    }
}
