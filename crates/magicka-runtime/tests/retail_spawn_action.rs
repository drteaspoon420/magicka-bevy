//! Read-only retail Spawn coverage: decode every Spawn and its AIEvents, then
//! classify area/waypoint references against the scene's own level model using
//! the original key rules (locators: case-preserving hash; trigger areas:
//! lowercased hash; Move lookups ignore animated-part locators).
use magicka_content::{level::AnimatedLevelPart, parse_primary, Content};
use magicka_runtime::{
    ai_event::AiEvent,
    scene_loading::custom_hash,
    scene_script::{ActionDefinition, SceneScriptLayout},
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

fn walk(path: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(path).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(&p, ext, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)) {
            out.push(p);
        }
    }
}
fn animated_locators(part: &AnimatedLevelPart, out: &mut HashSet<i32>) {
    out.extend(part.locators.iter().map(|l| custom_hash(&l.name)));
    for child in &part.children {
        animated_locators(child, out);
    }
}
fn model_name(xml: &str) -> Option<String> {
    let start = xml.find("<model>")? + "<model>".len();
    let end = start + xml[start..].find("</model>")?;
    Some(xml[start..end].trim().to_string())
}

#[test]
fn retail_spawn_actions_decode_and_references_classify() {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    if !root.is_dir() {
        eprintln!("SKIP: retail content absent");
        return;
    }
    let mut characters = vec![];
    walk(&root.join("Data/Characters"), "xnb", &mut characters);
    let character_names: BTreeSet<String> = characters
        .iter()
        .map(|p| {
            p.file_stem()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase()
        })
        .collect();
    let mut phys_files = vec![];
    walk(&root.join("Data/PhysicsEntities"), "xnb", &mut phys_files);
    let mut phys_max_hp = BTreeMap::<String, i32>::new();
    for f in &phys_files {
        let bytes = std::fs::read(f).unwrap();
        let xnb = magicka_xnb::xnb::parse(&bytes).unwrap();
        if let Some(Ok(parsed)) = parse_primary(&xnb) {
            if let Content::PhysicsEntity(t) = parsed.content {
                phys_max_hp.insert(
                    f.file_stem()
                        .unwrap()
                        .to_string_lossy()
                        .to_ascii_lowercase(),
                    t.max_hitpoints,
                );
            }
        }
    }
    let mut paths = vec![];
    walk(&root.join("Levels"), "xml", &mut paths);
    paths.sort();
    let mut errors = vec![];
    let mut counts = BTreeMap::<&str, usize>::new();
    let mut unresolved_areas = BTreeSet::new();
    let mut missing_types = BTreeSet::new();
    let mut level_cache =
        BTreeMap::<PathBuf, Option<(HashSet<i32>, HashSet<i32>, HashSet<i32>)>>::new();
    for path in &paths {
        let bytes = std::fs::read(path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        let mut reader = quick_xml::Reader::from_reader(bytes.as_slice());
        let scene = loop {
            match reader.read_event().unwrap() {
                quick_xml::events::Event::Start(n) | quick_xml::events::Event::Empty(n) => {
                    break n.name().as_ref().eq_ignore_ascii_case(b"scene")
                }
                quick_xml::events::Event::Eof => break false,
                _ => {}
            }
        };
        if !scene {
            continue;
        }
        let layout = match SceneScriptLayout::read(&bytes, false) {
            Ok(l) => l,
            Err(e) => {
                errors.push(format!("{}: {e:?}", path.display()));
                continue;
            }
        };
        let spawns: Vec<_> = layout
            .actions
            .iter()
            .filter_map(|a| match a {
                ActionDefinition::Spawn(s) => Some(s),
                _ => None,
            })
            .collect();
        let orders: Vec<_> = layout
            .actions
            .iter()
            .filter_map(|a| match a {
                ActionDefinition::GiveOrder(o) => Some(o),
                _ => None,
            })
            .collect();
        let commands: Vec<_> = layout
            .actions
            .iter()
            .filter_map(|a| match a {
                ActionDefinition::Scene(c) => Some(c.kind),
                _ => None,
            })
            .collect();
        let phys: Vec<_> = layout
            .actions
            .iter()
            .filter_map(|a| match a {
                ActionDefinition::SpawnPhys(p) => Some(p),
                _ => None,
            })
            .collect();
        let moves: Vec<_> = layout
            .actions
            .iter()
            .filter_map(|a| match a {
                ActionDefinition::AvatarMove(m) => Some(m),
                _ => None,
            })
            .collect();
        let entity_commands: Vec<_> = layout
            .actions
            .iter()
            .filter_map(|a| match a {
                ActionDefinition::Entity(e) => Some(e),
                _ => None,
            })
            .collect();
        if spawns.is_empty()
            && entity_commands.is_empty()
            && orders.is_empty()
            && commands.is_empty()
            && phys.is_empty()
            && moves.is_empty()
        {
            continue;
        }
        let Some(model) = model_name(&text) else {
            errors.push(format!("{}: no model", path.display()));
            continue;
        };
        let xnb_path = path.parent().unwrap().join(format!("{model}.xnb"));
        let level = level_cache.entry(xnb_path.clone()).or_insert_with(|| {
            let bytes = std::fs::read(&xnb_path).ok()?;
            let xnb = magicka_xnb::xnb::parse(&bytes).ok()?;
            let Content::Level(level) = parse_primary(&xnb)?.ok()?.content else {
                return None;
            };
            let locators = level
                .locators
                .iter()
                .map(|l| custom_hash(&l.name))
                .collect();
            let mut areas: HashSet<i32> = level
                .trigger_areas
                .iter()
                .map(|a| custom_hash(&a.name.to_ascii_lowercase()))
                .collect();
            areas.insert(custom_hash("any"));
            let mut animated = HashSet::new();
            for part in &level.animated_parts {
                animated_locators(part, &mut animated);
            }
            Some((locators, areas, animated))
        });
        let Some((locators, areas, animated)) = level else {
            errors.push(format!("{}: level {model} unreadable", path.display()));
            continue;
        };
        for kind in &commands {
            use magicka_runtime::scene_command::SceneCommandKind::*;
            let class = match *kind {
                CameraMove { target, .. } if locators.contains(&target) => "cameramove locator",
                CameraMove { target, .. } if animated.contains(&target) => {
                    "cameramove animated locator"
                }
                CameraMove { .. } => "cameramove ground fallback",
                CameraLock { area, .. } if areas.contains(&area) => "cameralock area",
                CameraLock { area, .. } => {
                    unresolved_areas.insert(format!("cameralock {model}:{area}"));
                    "cameralock missing (native throw)"
                }
                CameraShake { area: None, .. } => "camerashake global",
                CameraShake { area: Some(a), .. }
                    if locators.contains(&a) || areas.contains(&a) || animated.contains(&a) =>
                {
                    "camerashake located"
                }
                CameraShake { area: Some(a), .. } => {
                    unresolved_areas.insert(format!("camerashake {model}:{a}"));
                    "camerashake missing (native throw)"
                }
                _ => "scene command other",
            };
            *counts.entry(class).or_default() += 1;
        }
        for e in &entity_commands {
            use magicka_runtime::entity_command::{CharacterSelector, EntityCommandKind::*};
            let resolves =
                |a: &i32| locators.contains(a) || areas.contains(a) || animated.contains(a);
            let class = match &e.kind {
                SetPosition { id: None, .. } => {
                    unresolved_areas.insert(format!("setposition {model}: no id"));
                    "setposition no id (native throw)"
                }
                SetPosition { area, .. } if resolves(area) => "setposition area",
                SetPosition { area, .. } => {
                    unresolved_areas.insert(format!("setposition {model}:{area}"));
                    "setposition area missing (native throw)"
                }
                Remove {
                    target: CharacterSelector::Id(_),
                    ..
                }
                | Kill {
                    target: CharacterSelector::Id(_),
                } => "remove/kill specific",
                Remove {
                    target: CharacterSelector::Area { area, .. },
                    ..
                }
                | Kill {
                    target: CharacterSelector::Area { area, .. },
                } => {
                    if areas.contains(area) {
                        "remove/kill area"
                    } else {
                        unresolved_areas.insert(format!("remove/kill {model}:{area}"));
                        "remove/kill area missing (native throw)"
                    }
                }
                SetDialog { .. } => "setdialog",
                ReleaseGrip { .. } => "releasegrip",
            };
            *counts.entry(class).or_default() += 1;
        }
        for p in &phys {
            *counts.entry("spawnphys").or_default() += 1;
            match p.object.as_deref().and_then(|o| phys_max_hp.get(o)) {
                Some(hp) if *hp > 0 => *counts.entry("spawnphys damageable").or_default() += 1,
                Some(_) => *counts.entry("spawnphys plain").or_default() += 1,
                None => {
                    unresolved_areas.insert(format!("spawnphys object {:?}", p.object));
                }
            }
            if !(locators.contains(&p.area)
                || areas.contains(&p.area)
                || animated.contains(&p.area))
            {
                unresolved_areas.insert(format!("spawnphys {model}:{}", p.area));
            }
        }
        for m in &moves {
            *counts.entry("avatarmove").or_default() += 1;
            let numbered = m
                .node
                .children
                .iter()
                .filter(|c| !c.name.to_ascii_lowercase().starts_with("playerid"))
                .count();
            if numbered > 4 {
                unresolved_areas.insert(format!("avatarmove {model}: {numbered} numbered lists"));
            }
            if m.node
                .children
                .iter()
                .any(|c| c.name.to_ascii_lowercase().starts_with("playerid"))
            {
                *counts.entry("avatarmove with playerid").or_default() += 1;
            }
        }
        let spawned_ids: HashSet<i32> = spawns.iter().map(|s| s.unique_id).collect();
        let mut nodes: Vec<&magicka_runtime::scene_script::XmlElement> = vec![];
        for order in &orders {
            *counts.entry("giveorder").or_default() += 1;
            nodes.push(&order.node);
            match order.id {
                Some(id) if spawned_ids.contains(&id) => {
                    *counts.entry("order id spawned in scene").or_default() += 1
                }
                Some(_) => *counts.entry("order id not spawned in scene").or_default() += 1,
                // GetTriggerArea uses the dictionary indexer: missing keys throw.
                None if areas.contains(&order.area) => {
                    *counts.entry("order area trigger").or_default() += 1
                }
                None => {
                    unresolved_areas.insert(format!("giveorder {}:{}", model, order.area));
                    *counts
                        .entry("order area missing (native throw)")
                        .or_default() += 1
                }
            }
        }
        for spawn in &spawns {
            nodes.push(&spawn.node);
        }
        for node in nodes {
            match AiEvent::read_children(node) {
                Ok(events) => {
                    for (event, child) in events.iter().zip(&node.children) {
                        *counts.entry("ai event").or_default() += 1;
                        if let AiEvent::Move(m) = event {
                            let named = child.attributes.iter().any(|(k, v)| {
                                k.eq_ignore_ascii_case("position") && v.split(',').count() != 3
                            });
                            let class = if !named {
                                "move literal/omitted"
                            } else if locators.contains(&m.waypoint_id) {
                                "move locator"
                            } else if areas.contains(&m.waypoint_id) {
                                "move trigger area"
                            } else {
                                "move unresolved (zero waypoint)"
                            };
                            *counts.entry(class).or_default() += 1;
                        }
                    }
                }
                Err(e) => errors.push(format!("{}: events {e:?}", path.display())),
            }
        }
        for spawn in spawns {
            *counts.entry("spawn").or_default() += 1;
            match &spawn.type_name {
                Some(t) if character_names.contains(t) => {
                    *counts.entry("type found").or_default() += 1
                }
                Some(t) => {
                    missing_types.insert(t.clone());
                }
                None => *counts.entry("type missing").or_default() += 1,
            }
            // GameScene.GetLocator(Matrix) order: any, locator, trigger area, animated parts.
            let class = if spawn.area == custom_hash("any") {
                "area any"
            } else if locators.contains(&spawn.area) {
                "area locator"
            } else if areas.contains(&spawn.area) {
                "area trigger"
            } else if animated.contains(&spawn.area) {
                "area animated"
            } else {
                unresolved_areas.insert(format!("{}:{}", model, spawn.area));
                "area unresolved (native throw)"
            };
            *counts.entry(class).or_default() += 1;
        }
    }
    eprintln!("retail spawn/giveorder counts: {counts:?}");
    eprintln!("missing character types: {missing_types:?}");
    eprintln!(
        "unresolved spawn areas ({}): {unresolved_areas:?}",
        unresolved_areas.len()
    );
    eprintln!("{} failures", errors.len());
    assert!(counts.get("spawn").copied().unwrap_or(0) > 0);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
