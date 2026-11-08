//! Headless retail scene runs: initialize every scene's actions against its own
//! level and content, then step scripts for ten simulated seconds. Reports the
//! first failure per scene; no players, AI, physics or dialog are simulated.
use magicka_content::{assets::Asset, parse_primary, xna::XnaObject, Content};
use magicka_runtime::{
    audio_action::sound_dictionary,
    level_variables::LevelVariables,
    scene_loading::custom_hash,
    scene_script::SceneScriptLayout,
    scene_world::{NpcTemplate, SceneContent, SceneWorld},
    script_runtime::{ScriptRuntime, ScriptRuntimeError},
};
use std::collections::{BTreeMap, HashMap, HashSet};
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
/// Windows-style case-insensitive resolution of a relative path (".." allowed).
fn resolve_ci(base: &Path, relative: &str) -> Option<PathBuf> {
    let mut current = base.to_path_buf();
    for part in relative.split(['/', '\\']) {
        if part == ".." {
            current.pop();
            continue;
        }
        let entry = std::fs::read_dir(&current)
            .ok()?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))?;
        current = entry.path();
    }
    Some(current)
}
fn parse(path: &Path) -> Option<Content> {
    let bytes = std::fs::read(path).ok()?;
    let xnb = magicka_xnb::xnb::parse(&bytes).ok()?;
    Some(parse_primary(&xnb)?.ok()?.content)
}

struct FsContent {
    items: HashMap<String, PathBuf>,
    characters: HashMap<String, PathBuf>,
    physics: HashMap<String, PathBuf>,
    character_cache: HashMap<String, Option<NpcTemplate>>,
    physics_cache: HashMap<String, Option<i32>>,
    effects: HashSet<i32>,
    cues: HashSet<i32>,
}
impl SceneContent for FsContent {
    fn item(&mut self, name: &str) -> Option<magicka_content::item::Item> {
        match parse(self.items.get(&name.to_ascii_lowercase())?).as_ref()? {
            Content::Item(item) => Some((**item).clone()),
            _ => None,
        }
    }

    fn character(&mut self, name: &str) -> Option<NpcTemplate> {
        let path = self.characters.get(name).cloned();
        self.character_cache
            .entry(name.into())
            .or_insert_with(|| match parse(&path?)? {
                Content::Character(t) => Some(NpcTemplate {
                    type_id: custom_hash(&t.id.to_ascii_lowercase()),
                    faction: t.faction,
                    max_hit_points: t.max_hitpoints,
                    radius: t.radius,
                    length: t.length,
                    undying: t.undying,
                }),
                _ => None,
            })
            .clone()
    }
    fn physics_max_hit_points(&mut self, name: &str) -> Option<i32> {
        let path = self.physics.get(name).cloned();
        *self
            .physics_cache
            .entry(name.into())
            .or_insert_with(|| match parse(&path?)? {
                Content::PhysicsEntity(t) => Some(t.max_hitpoints),
                _ => None,
            })
    }
    fn effect_exists(&mut self, effect: i32) -> bool {
        self.effects.contains(&effect)
    }
    fn cue_exists(&mut self, cue: i32) -> bool {
        self.cues.contains(&cue)
    }
}

fn stems(root: &Path, ext: &str) -> HashMap<String, PathBuf> {
    let mut files = vec![];
    walk(root, ext, &mut files);
    files
        .into_iter()
        .map(|p| {
            (
                p.file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .to_ascii_lowercase(),
                p,
            )
        })
        .collect()
}

/// Collapse an error to a stable category for the census.
fn category(e: &ScriptRuntimeError<String>) -> String {
    let text = format!("{e:?}");
    let text = text.split(['(', '{']).next().unwrap_or("").to_string();
    match e {
        ScriptRuntimeError::World(w) => format!("World: {}", w.split(':').next().unwrap_or(w)),
        ScriptRuntimeError::Spawn(_, inner) => {
            format!("Spawn: {:?}", inner).chars().take(80).collect()
        }
        _ => text,
    }
}

#[test]
fn retail_scenes_run_headless() {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    if !root.is_dir() {
        eprintln!("SKIP: retail content absent");
        return;
    }
    let Content::Xna(sound_list) = parse(&root.join("Audio/SoundList.xnb")).unwrap() else {
        panic!()
    };
    let XnaObject::Asset(asset) = *sound_list else {
        panic!()
    };
    let Asset::StringList(names) = *asset else {
        panic!()
    };
    let cues = sound_dictionary(names.iter().flatten().map(String::as_str))
        .unwrap()
        .into_keys()
        .collect();
    let mut content = FsContent {
        items: stems(
            &resolve_ci(&root, "Data/items/Wizard").expect("Wizard item directory"),
            "xnb",
        ),
        characters: stems(&root.join("Data/Characters"), "xnb"),
        physics: stems(&root.join("Data/PhysicsEntities"), "xnb"),
        character_cache: HashMap::new(),
        physics_cache: HashMap::new(),
        effects: stems(&root.join("Effects"), "xml")
            .into_keys()
            .map(|s| custom_hash(&s))
            .collect(),
        cues,
    };
    let mut paths = vec![];
    walk(&root.join("Levels"), "xml", &mut paths);
    paths.sort();
    // Level .lvl files map scenes to their dialog file (plus Dialog_Fairy).
    let mut lvls = vec![];
    walk(&root.join("Levels"), "lvl", &mut lvls);
    let mut scene_dialogs = HashMap::<String, PathBuf>::new();
    for lvl in &lvls {
        let text = std::fs::read_to_string(lvl).unwrap();
        let tag = |t: &str| {
            let open = format!("<{t}>");
            text.find(&open).map(|s| {
                let rest = &text[s + open.len()..];
                rest[..rest.find('<').unwrap()].trim().to_string()
            })
        };
        let Some(dialogs) = tag("Dialogs") else {
            continue;
        };
        let dialog_path = resolve_ci(lvl.parent().unwrap(), &dialogs)
            .unwrap_or_else(|| panic!("{} names missing {dialogs}", lvl.display()));
        for scene in text.split("<Scene>").skip(1) {
            let scene = scene[..scene.find('<').unwrap()].trim();
            let file = Path::new(scene)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase();
            scene_dialogs.insert(file, dialog_path.clone());
        }
    }
    let fairy = std::fs::read(root.join("Levels/Dialog_Fairy.xml")).unwrap();
    let mut levels =
        BTreeMap::<PathBuf, Option<std::sync::Arc<magicka_content::level::LevelModel>>>::new();
    let mut outcomes = BTreeMap::<String, usize>::new();
    let mut details = vec![];
    let (mut scenes, mut clean) = (0, 0);
    let (mut spawned, mut executed) = (0usize, 0usize);
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
        scenes += 1;
        let text = String::from_utf8_lossy(&bytes);
        let model = text
            .find("<model>")
            .and_then(|s| {
                text[s + 7..]
                    .find("</model>")
                    .map(|e| text[s + 7..s + 7 + e].trim().to_string())
            })
            .unwrap();
        let xnb = path.parent().unwrap().join(format!("{model}.xnb"));
        let level = levels
            .entry(xnb.clone())
            .or_insert_with(|| match parse(&xnb)? {
                Content::Level(l) => Some(std::sync::Arc::new(*l)),
                _ => None,
            });
        let level = level.as_ref().expect("level parses");
        let layout = SceneScriptLayout::read(&bytes, false).unwrap();
        let mut world = SceneWorld::new(level.clone(), &mut content, 0x5eed).unwrap();
        world.register_interactables(&layout);
        if let Some(dialog_path) = scene_dialogs.get(
            &path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_ascii_lowercase(),
        ) {
            let level_dialogs = std::fs::read(dialog_path).unwrap();
            let definitions =
                magicka_runtime::dialog_definition::DialogDefinition::read_collection(&[
                    &level_dialogs,
                    &fairy,
                ])
                .unwrap();
            world.dialogs =
                Some(magicka_runtime::dialog_manager::DialogManager::new(definitions).unwrap());
        }
        let names: Vec<String> = layout
            .actions
            .iter()
            .map(|a| match a {
                magicka_runtime::scene_script::ActionDefinition::Unsupported(n) => {
                    n.name.to_ascii_lowercase()
                }
                _ => String::new(),
            })
            .collect();
        let mut runtime = ScriptRuntime::new(layout, LevelVariables::default());
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let result = runtime.initialize_actions(&mut world).and_then(|_| {
            for _ in 0..600 {
                world.step(&mut runtime, 1.0 / 60.0)?;
                if world.pending_scene.is_some() {
                    break;
                }
            }
            Ok(())
        });
        spawned += world.entities.iter().filter(|e| e.added).count();
        executed += runtime.history().len();
        let outcome = match &result {
            Ok(()) => {
                clean += 1;
                if world.pending_scene.is_some() {
                    "ok: scene change requested".to_string()
                } else {
                    "ok".to_string()
                }
            }
            Err(ScriptRuntimeError::UnsupportedAction(i)) => {
                details.push(format!("{name}: unsupported {}", names[*i]));
                format!("UnsupportedAction: {}", names[*i])
            }
            Err(e) => {
                details.push(format!("{name}: {e:?}"));
                category(e)
            }
        };
        *outcomes.entry(outcome).or_default() += 1;
    }
    eprintln!("headless scenes: {scenes}, clean ten-second runs: {clean}");
    eprintln!("entities added: {spawned}, actions executed: {executed}");
    eprintln!("outcomes: {outcomes:#?}");
    for d in &details {
        eprintln!("  {}", d.chars().take(220).collect::<String>());
    }
    assert_eq!(scenes, 142);
    // This bare world has no models, dialogs or character templates, so scenes that
    // need one stop early: known harness limits, not script errors. Anything else,
    // or fewer clean scenes, is a regression. Measured 2026-11-04: 102 clean.
    const HARNESS_LIMITS: [&str; 10] = [
        "ok",
        "Dialog",
        "MissingDialogActionHost",
        "Spawn: Host",
        "SpawnPhysInitialize",
        "World: tutorial localization",
        "World: missing ",
        "World: Jormungandr model",
        "World: Vlad requires",
        "World: Warlord requires",
    ];
    let unexpected: Vec<_> = outcomes
        .iter()
        .filter(|(outcome, _)| !HARNESS_LIMITS.iter().any(|k| outcome.starts_with(k)))
        .collect();
    assert!(unexpected.is_empty(), "new scene outcomes: {unexpected:#?}");
    assert!(clean >= 102, "clean scenes fell to {clean}");
}
