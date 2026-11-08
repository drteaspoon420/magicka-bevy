//! Supported attributes only; conditions/actions are not evaluated or executed.
use magicka_runtime::{
    script_attributes::DecodedVariableAction, script_condition::ScriptCondition,
};
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
fn supported_retail_condition_and_variable_attributes_parse() {
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
    let (mut conditions, mut actions) = (0, 0);
    let mut errors = vec![];
    for path in &paths {
        let bytes = std::fs::read(path).unwrap();
        let mut reader = Reader::from_reader(bytes.as_slice());
        let mut stack = Vec::<String>::new();
        loop {
            let event = reader.read_event().unwrap();
            match event {
                Event::Start(ref node) | Event::Empty(ref node) => {
                    let tag = String::from_utf8_lossy(node.name().as_ref()).to_ascii_lowercase();
                    let condition = stack.last().is_some_and(|p| p == "if")
                        && matches!(
                            tag.as_str(),
                            "counter" | "timer" | "triggered" | "nocutsceneactive" | "playercount"
                        );
                    let action = stack.iter().any(|p| p == "then")
                        && matches!(tag.as_str(), "counter" | "timer" | "addtimer");
                    if condition || action {
                        let attrs = node
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
                        let attrs = attrs.iter().map(|(k, v)| (k.as_str(), v.as_str()));
                        let result = if condition {
                            conditions += 1;
                            ScriptCondition::read(&tag, attrs).map(|_| ())
                        } else {
                            actions += 1;
                            DecodedVariableAction::read(&tag, attrs).map(|_| ())
                        };
                        if let Err(e) = result {
                            errors.push(format!("{} {tag}: {e:?}", path.display()));
                        }
                    }
                    if matches!(event, Event::Start(_)) {
                        stack.push(tag);
                    }
                }
                Event::End(_) => {
                    stack.pop();
                }
                Event::Eof => break,
                _ => {}
            }
        }
    }
    eprintln!("script attributes: {} XML files; {conditions} supported conditions, {actions} variable actions; {} failures",paths.len(),errors.len());
    assert!(conditions > 0 && actions > 0);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
