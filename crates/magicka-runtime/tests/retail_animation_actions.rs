//! Read-only action attribute coverage. Trigger conditions/scheduling are not run.
use magicka_runtime::animation_action::AnimationAction;
use quick_xml::{events::Event, Reader};
use std::path::{Path, PathBuf};
fn walk(path: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else if path
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("xml"))
        {
            out.push(path);
        }
    }
}
#[test]
fn retail_animation_action_attributes_parse() {
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
    let mut counts = [0usize; 3];
    let mut errors = vec![];
    for path in &paths {
        let bytes = std::fs::read(path).unwrap();
        let mut reader = Reader::from_reader(bytes.as_slice());
        loop {
            match reader.read_event() {
                Ok(Event::Start(node) | Event::Empty(node)) => {
                    let tag = String::from_utf8_lossy(node.name().as_ref()).into_owned();
                    let index = match tag.to_ascii_lowercase().as_str() {
                        "playanimation" => 0,
                        "stopanimation" => 1,
                        "resumeanimation" => 2,
                        _ => continue,
                    };
                    counts[index] += 1;
                    let attributes = node
                        .attributes()
                        .map(|a| {
                            let a = a.unwrap();
                            (
                                String::from_utf8_lossy(a.key.as_ref()).into_owned(),
                                a.decoded_and_normalized_value(
                                    quick_xml::XmlVersion::Implicit1_0,
                                    reader.decoder(),
                                )
                                .unwrap()
                                .into_owned(),
                            )
                        })
                        .collect::<Vec<_>>();
                    if let Err(error) = AnimationAction::read(
                        &tag,
                        attributes.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                    ) {
                        errors.push(format!("{} {tag}: {error:?}", path.display()));
                    }
                }
                Ok(Event::Eof) => break,
                Err(error) => {
                    errors.push(format!("{} XML: {error}", path.display()));
                    break;
                }
                _ => {}
            }
        }
    }
    eprintln!(
        "animation attributes: {} XML files; play {}, stop {}, resume {}; {} failures",
        paths.len(),
        counts[0],
        counts[1],
        counts[2],
        errors.len()
    );
    assert!(counts.iter().sum::<usize>() > 0);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
