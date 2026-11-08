//! Read-only retail effect coverage. EffectManager keys Content/Effects XML
//! stems by lowercased custom hash. Level effects: static IDs lowercased but
//! static effect names case-preserved; animated-part IDs and names lowercased.
use magicka_content::{level::AnimatedLevelPart, parse_primary, Content};
use magicka_runtime::{
    effect_action::EffectActionKind,
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
struct LevelFacts {
    effect_ids: Vec<i32>,
    effect_types: Vec<(i32, String)>,
    locators: HashSet<i32>,
    areas: HashSet<i32>,
    animated_locators: HashSet<i32>,
    lights: HashSet<i32>,
}
fn animated(part: &AnimatedLevelPart, facts: &mut LevelFacts) {
    facts
        .animated_locators
        .extend(part.locators.iter().map(|l| custom_hash(&l.name)));
    for e in &part.effects {
        facts
            .effect_ids
            .push(custom_hash(&e.id.to_ascii_lowercase()));
        let name = e.effect.to_ascii_lowercase();
        facts.effect_types.push((custom_hash(&name), name));
    }
    for child in &part.children {
        animated(child, facts);
    }
}
fn model_name(xml: &str) -> Option<String> {
    let start = xml.find("<model>")? + "<model>".len();
    let end = start + xml[start..].find("</model>")?;
    Some(xml[start..end].trim().to_string())
}

#[test]
fn retail_effect_actions_and_level_effects_resolve() {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    if !root.is_dir() {
        eprintln!("SKIP: retail content absent");
        return;
    }
    let mut effect_files = vec![];
    walk(&root.join("Effects"), "xml", &mut effect_files);
    let mut effect_types = HashSet::new();
    for f in &effect_files {
        let stem = f
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_ascii_lowercase();
        assert!(
            effect_types.insert(custom_hash(&stem)),
            "duplicate effect {stem}"
        );
    }
    let mut paths = vec![];
    walk(&root.join("Levels"), "xml", &mut paths);
    paths.sort();
    let mut counts = BTreeMap::<&str, usize>::new();
    let mut findings = BTreeSet::new();
    let mut errors = vec![];
    let mut cache = BTreeMap::<PathBuf, Option<LevelFacts>>::new();
    for path in &paths {
        let bytes = std::fs::read(path).unwrap();
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
        let text = String::from_utf8_lossy(&bytes);
        let Some(model) = model_name(&text) else {
            continue;
        };
        let xnb_path = path.parent().unwrap().join(format!("{model}.xnb"));
        let first_visit = !cache.contains_key(&xnb_path);
        let facts = cache.entry(xnb_path.clone()).or_insert_with(|| {
            let bytes = std::fs::read(&xnb_path).ok()?;
            let xnb = magicka_xnb::xnb::parse(&bytes).ok()?;
            let Content::Level(level) = parse_primary(&xnb)?.ok()?.content else {
                return None;
            };
            let mut facts = LevelFacts {
                effect_ids: level
                    .effects
                    .iter()
                    .map(|e| custom_hash(&e.id.to_ascii_lowercase()))
                    .collect(),
                effect_types: level
                    .effects
                    .iter()
                    .map(|e| (custom_hash(&e.effect), e.effect.clone()))
                    .collect(),
                locators: level
                    .locators
                    .iter()
                    .map(|l| custom_hash(&l.name))
                    .collect(),
                areas: level
                    .trigger_areas
                    .iter()
                    .map(|a| custom_hash(&a.name.to_ascii_lowercase()))
                    .chain([custom_hash("any")])
                    .collect(),
                animated_locators: HashSet::new(),
                // LevelModel.Lights keys: case-preserving name hash.
                lights: level.lights.iter().map(|l| custom_hash(&l.name)).collect(),
            };
            for part in &level.animated_parts {
                animated(part, &mut facts);
            }
            Some(facts)
        });
        let Some(facts) = facts else {
            errors.push(format!("{}: level {model} unreadable", path.display()));
            continue;
        };
        if first_visit {
            *counts.entry("levels").or_default() += 1;
            let mut seen = HashSet::new();
            for id in &facts.effect_ids {
                *counts.entry("level effect").or_default() += 1;
                if !seen.insert(*id) {
                    findings.insert(format!(
                        "{model}: duplicate level effect id {id} (SortedList.Add throws)"
                    ));
                }
            }
            for (hash, name) in &facts.effect_types {
                if !effect_types.contains(hash) {
                    findings.insert(format!("{model}: level effect type '{name}' not loadable"));
                }
            }
        }
        let layout = match SceneScriptLayout::read(&bytes, false) {
            Ok(l) => l,
            Err(e) => {
                errors.push(format!("{}: {e:?}", path.display()));
                continue;
            }
        };
        let level_ids: HashSet<i32> = facts.effect_ids.iter().copied().collect();
        let spawned: HashSet<i32> = layout
            .actions
            .iter()
            .filter_map(|a| match a {
                ActionDefinition::Effect(e) => match e.kind {
                    EffectActionKind::Spawn { name, .. } => Some(name),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let file = path.file_name().unwrap().to_string_lossy().to_string();
        // Interactable IDs: lowercased `locator` attribute hash.
        let interactables: HashSet<i32> = layout
            .deferred_interactables
            .iter()
            .flat_map(|n| n.attributes.iter())
            .filter(|(k, _)| k.eq_ignore_ascii_case("locator"))
            .map(|(_, v)| custom_hash(&v.to_ascii_lowercase()))
            .collect();
        for action in &layout.actions {
            use magicka_runtime::toggle_action::ToggleKind;
            let ActionDefinition::Toggle(t) = action else {
                continue;
            };
            match &t.kind {
                ToggleKind::Light { ids: Some(ids), .. } => {
                    for id in ids {
                        *counts.entry("light id").or_default() += 1;
                        if !facts.lights.contains(id) {
                            findings.insert(format!("{file}: light {id} missing (throws)"));
                        }
                    }
                }
                ToggleKind::Light { ids: None, .. } => {
                    findings.insert(format!("{file}: light action without id"));
                }
                ToggleKind::Poi { id, .. } => {
                    *counts.entry("poi").or_default() += 1;
                    if !interactables.contains(id) {
                        findings.insert(format!("{file}: poi {id} is not an interactable"));
                    }
                }
                _ => *counts.entry("other toggle").or_default() += 1,
            }
        }
        for action in &layout.actions {
            let ActionDefinition::Effect(e) = action else {
                continue;
            };
            match e.kind {
                EffectActionKind::Spawn { effect, area, .. } => {
                    *counts.entry("spawneffect").or_default() += 1;
                    if !effect_types.contains(&effect) {
                        findings.insert(format!(
                            "{file}: spawneffect type {effect} missing (throws)"
                        ));
                    }
                    if !(facts.locators.contains(&area)
                        || facts.areas.contains(&area)
                        || facts.animated_locators.contains(&area))
                    {
                        findings
                            .insert(format!("{file}: spawneffect area {area} missing (throws)"));
                    }
                }
                EffectActionKind::Stop { name } | EffectActionKind::Start { name: Some(name) } => {
                    *counts.entry("stop/start").or_default() += 1;
                    let class = if level_ids.contains(&name) {
                        "stop/start level effect"
                    } else if spawned.contains(&name) {
                        "stop/start spawned name"
                    } else {
                        "stop/start unknown name (no-op)"
                    };
                    *counts.entry(class).or_default() += 1;
                }
                EffectActionKind::Start { name: None } => {
                    *counts.entry("start unnamed (no-op)").or_default() += 1
                }
            }
        }
    }
    eprintln!("effect types: {}", effect_types.len());
    eprintln!("retail effect counts: {counts:?}");
    eprintln!("findings ({}): {findings:#?}", findings.len());
    eprintln!("{} failures", errors.len());
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    // Retail data has one dead reference, which throws natively too. A new one means
    // a resolution check broke. Measured 2026-11-04.
    assert_eq!(findings.len(), 1, "{findings:#?}");
}
