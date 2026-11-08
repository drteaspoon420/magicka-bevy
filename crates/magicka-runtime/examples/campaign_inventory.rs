//! Read-only inventory of the root campaign levels using the live parser.
//! Output is derived from user-supplied content; keep it in private evidence.
use magicka_runtime::{
    boss_action::BossCommand,
    play_content::{read_level, resolve},
    scene_script::{ActionDefinition, ConditionDefinition, SceneScriptLayout},
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn variant(value: &impl std::fmt::Debug) -> String {
    format!("{value:?}")
        .split(['(', '{', ' '])
        .next()
        .unwrap_or("")
        .to_owned()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("usage: campaign_inventory <Content>")?,
    );
    let levels = root.join("Levels");
    let mut paths: Vec<_> = std::fs::read_dir(&levels)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lvl")))
        .collect();
    paths.sort();
    let mut report = Vec::new();
    for path in paths {
        let start = read_level(&path).map_err(|e| format!("{}: {e:?}", path.display()))?;
        let mut scenes = Vec::new();
        for relative in &start.scenes {
            let scene = resolve(&levels, Path::new(&relative.replace('\\', "/")))
                .ok_or_else(|| format!("missing listed scene {relative}"))?;
            let bytes = std::fs::read(&scene)?;
            for looped in [false, true] {
                let layout = SceneScriptLayout::read(&bytes, looped)
                    .map_err(|e| format!("{relative} looped={looped}: {e:?}"))?;
                let mut unknown = BTreeMap::<String, usize>::new();
                let mut actions = BTreeMap::<String, usize>::new();
                let mut conditions = BTreeMap::<String, usize>::new();
                let mut bosses = Vec::new();
                for c in &layout.conditions {
                    *conditions.entry(variant(c)).or_default() += 1;
                    if let ConditionDefinition::Unsupported(n) = c {
                        *unknown.entry(format!("condition:{}", n.name)).or_default() += 1;
                    }
                }
                for a in &layout.actions {
                    *actions.entry(variant(a)).or_default() += 1;
                    if let ActionDefinition::Unsupported(n) = a {
                        *unknown.entry(format!("action:{}", n.name)).or_default() += 1;
                    }
                    if let ActionDefinition::Boss(a) = a {
                        match &a.command {
                            BossCommand::Spawn { boss, prop, generic, setup, .. } => bosses.push(json!({
                                "name": boss, "kind": if *prop {"prop"} else if *generic {"generic"} else {"named"},
                                "delayed": setup.delayed,
                            })),
                            BossCommand::SpawnGeneric { spawn, setup } => bosses.push(json!({
                                "name": spawn.type_id, "kind": "generic", "delayed": setup.delayed,
                            })),
                            _ => {}
                        }
                    }
                }
                scenes.push(json!({"scene": relative, "looped": looped, "triggers": layout.triggers.len(),
                    "actions": actions, "conditions": conditions, "unsupported": unknown, "boss_spawns": bosses}));
            }
        }
        report.push(json!({"level": path.file_name().unwrap().to_string_lossy(), "start": start.scene, "scenes": scenes}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"scope": "root campaign .lvl files", "levels": report})
        )?
    );
    Ok(())
}
