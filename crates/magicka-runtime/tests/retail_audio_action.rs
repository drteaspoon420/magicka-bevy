//! Read-only retail audio coverage: every PlaySound/PlayMusic cue hash is
//! checked against AudioManager's SoundList dictionary (lowercased custom hash).
use magicka_content::{assets::Asset, parse_primary, xna::XnaObject, Content};
use magicka_runtime::{
    audio_action::{sound_dictionary, AudioActionKind},
    scene_script::{ActionDefinition, SceneScriptLayout},
};
use std::collections::{BTreeMap, BTreeSet};
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
fn retail_audio_cues_exist_in_sound_list() {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    if !root.is_dir() {
        eprintln!("SKIP: retail content absent");
        return;
    }
    let bytes = std::fs::read(root.join("Audio/SoundList.xnb")).unwrap();
    let xnb = magicka_xnb::xnb::parse(&bytes).unwrap();
    let Content::Xna(object) = parse_primary(&xnb).unwrap().unwrap().content else {
        panic!("SoundList is not an XNA object")
    };
    let XnaObject::Asset(asset) = *object else {
        panic!()
    };
    let Asset::StringList(names) = *asset else {
        panic!()
    };
    let dict = sound_dictionary(names.iter().flatten().map(String::as_str)).unwrap();
    let mut paths = vec![];
    walk(&root.join("Levels"), &mut paths);
    paths.sort();
    let mut counts = BTreeMap::<&str, usize>::new();
    let mut missing = BTreeSet::new();
    let mut errors = vec![];
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
        let layout = match SceneScriptLayout::read(&bytes, false) {
            Ok(l) => l,
            Err(e) => {
                errors.push(format!("{}: {e:?}", path.display()));
                continue;
            }
        };
        for action in &layout.actions {
            let ActionDefinition::Audio(a) = action else {
                continue;
            };
            let (label, cue) = match a.kind {
                AudioActionKind::PlaySound(s) => ("playsound", s.cue),
                AudioActionKind::PlayMusic { cue, .. } => ("playmusic", cue),
                AudioActionKind::StopSound { .. } => {
                    *counts.entry("stopsound").or_default() += 1;
                    continue;
                }
                AudioActionKind::StopMusic => {
                    *counts.entry("stopmusic").or_default() += 1;
                    continue;
                }
            };
            *counts.entry(label).or_default() += 1;
            if dict.contains_key(&cue) {
                *counts.entry("cue found").or_default() += 1;
            } else {
                missing.insert(format!(
                    "{label} {}:{cue}",
                    path.file_name().unwrap().to_string_lossy()
                ));
            }
        }
    }
    eprintln!("sound list entries: {}", dict.len());
    eprintln!("retail audio counts: {counts:?}");
    eprintln!(
        "cues missing from SoundList ({}): {missing:?}",
        missing.len()
    );
    eprintln!("{} failures", errors.len());
    assert!(counts.get("playsound").copied().unwrap_or(0) > 0);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
