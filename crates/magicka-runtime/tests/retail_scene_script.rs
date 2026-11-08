use magicka_runtime::scene_script::{ActionDefinition, ConditionDefinition, SceneScriptLayout};
use quick_xml::{events::Event, Reader};
use std::path::{Path, PathBuf};
fn walk(path: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(path).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xml")) {
            out.push(p);
        }
    }
}
#[test]
fn retail_scene_trigger_layouts_load_in_both_loop_modes() {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    if !root.is_dir() {
        eprintln!("SKIP: retail content absent");
        return;
    }
    let mut paths = vec![];
    walk(&root.join("Levels"), &mut paths);
    paths.sort();
    let (mut scenes, mut triggers, mut unknown_conditions, mut unknown_actions) = (0, 0, 0, 0);
    let mut errors = vec![];
    let mut control_actions = 0;
    let mut presence_conditions = 0;
    let mut dialog_conditions = 0;
    let mut dialog_actions = 0;
    let mut spawn_actions = 0;
    let mut order_actions = 0;
    let mut scene_commands = 0;
    let mut unknown: std::collections::BTreeMap<String, usize> = Default::default();
    for path in &paths {
        let bytes = std::fs::read(path).unwrap();
        let mut reader = Reader::from_reader(bytes.as_slice());
        let scene = loop {
            match reader.read_event().unwrap() {
                Event::Start(n) | Event::Empty(n) => {
                    break n.name().as_ref().eq_ignore_ascii_case(b"scene")
                }
                Event::Eof => break false,
                _ => {}
            }
        };
        if !scene {
            continue;
        }
        scenes += 1;
        for looped in [false, true] {
            match SceneScriptLayout::read(&bytes, looped) {
                Ok(layout) => {
                    if !looped {
                        triggers += layout.triggers.len();
                        dialog_actions += layout
                            .actions
                            .iter()
                            .filter(|a| matches!(a, ActionDefinition::Dialog(_)))
                            .count();
                        scene_commands += layout
                            .actions
                            .iter()
                            .filter(|a| matches!(a, ActionDefinition::Scene(_)))
                            .count();
                        order_actions += layout
                            .actions
                            .iter()
                            .filter(|a| matches!(a, ActionDefinition::GiveOrder(_)))
                            .count();
                        spawn_actions += layout
                            .actions
                            .iter()
                            .filter(|a| matches!(a, ActionDefinition::Spawn(_)))
                            .count();
                        dialog_conditions += layout
                            .conditions
                            .iter()
                            .filter(|c| matches!(c, ConditionDefinition::Dialog(_)))
                            .count();
                        presence_conditions += layout
                            .conditions
                            .iter()
                            .filter(|c| matches!(c, ConditionDefinition::Presence(_)))
                            .count();
                        control_actions += layout
                            .actions
                            .iter()
                            .filter(|a| matches!(a, ActionDefinition::Control(_)))
                            .count();
                        for c in &layout.conditions {
                            if let ConditionDefinition::Unsupported(n) = c {
                                *unknown.entry(format!("if:{}", n.name)).or_default() += 1;
                            }
                        }
                        for a in &layout.actions {
                            if let ActionDefinition::Unsupported(n) = a {
                                *unknown.entry(format!("then:{}", n.name)).or_default() += 1;
                            }
                        }
                        unknown_conditions += layout
                            .conditions
                            .iter()
                            .filter(|d| matches!(d, ConditionDefinition::Unsupported(_)))
                            .count();
                        unknown_actions += layout
                            .actions
                            .iter()
                            .filter(|d| matches!(d, ActionDefinition::Unsupported(_)))
                            .count();
                    }
                }
                Err(error) => errors.push(format!("{} looped={looped}: {error:?}", path.display())),
            }
        }
    }
    eprintln!("script layouts: {scenes} scenes, both loop modes; {triggers} nonloop triggers, {unknown_conditions} unsupported conditions, {unknown_actions} unsupported actions; {} failures",errors.len());
    eprintln!("unsupported tags: {unknown:?}");
    eprintln!("supported trigger-control actions in nonloop scene layouts: {control_actions}");
    eprintln!("supported presence conditions in nonloop scene layouts: {presence_conditions}");
    eprintln!("supported dialog conditions in nonloop scene layouts: {dialog_conditions}");
    eprintln!("supported dialog actions in nonloop scene layouts: {dialog_actions}");
    eprintln!("supported spawn actions in nonloop scene layouts: {spawn_actions}");
    eprintln!("supported giveorder actions in nonloop scene layouts: {order_actions}");
    eprintln!(
        "supported cutscene/input/fade/camera actions in nonloop scene layouts: {scene_commands}"
    );
    assert!(scenes > 0);
    // Only scenes some .lvl lists are loaded natively; havindr_s5HBF.xml is
    // unlisted and its OnInteract `repeatitions` attribute would throw.
    let mut listed = std::collections::BTreeSet::new();
    fn walk_lvl(path: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(path).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk_lvl(&p, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lvl")) {
                out.push(p);
            }
        }
    }
    let mut lvls = vec![];
    walk_lvl(&root.join("Levels"), &mut lvls);
    assert!(!lvls.is_empty());
    for lvl in &lvls {
        let text = String::from_utf8_lossy(&std::fs::read(lvl).unwrap()).to_lowercase();
        for part in text.split("<scene>").skip(1) {
            if let Some(end) = part.find("</scene>") {
                let name = part[..end].trim().replace('\\', "/");
                if let Some(file) = name.rsplit('/').next() {
                    listed.insert(file.to_string());
                }
            }
        }
    }
    assert!(listed.contains("havindr_s2.xml"), "{listed:?}");
    let errors: Vec<_> = errors
        .into_iter()
        .filter(|e| {
            let file = e.split(" looped=").next().unwrap_or(e);
            let file = file.rsplit('/').next().unwrap_or(file).to_lowercase();
            listed.contains(&file)
        })
        .collect();
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
