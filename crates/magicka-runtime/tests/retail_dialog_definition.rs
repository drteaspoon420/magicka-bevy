use magicka_runtime::dialog_definition::DialogDefinition;
use std::path::{Path, PathBuf};
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml")) {
            out.push(p);
        }
    }
}
#[test]
fn retail_dialog_roots_decode_without_executing_content() {
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
    let (mut files, mut dialogs, mut interactions, mut messages, mut texts) = (0, 0, 0, 0, 0);
    let mut errors = vec![];
    for path in paths {
        let bytes = std::fs::read(&path).unwrap();
        let mut reader = quick_xml::Reader::from_reader(bytes.as_slice());
        let is_dialog = loop {
            match reader.read_event().unwrap() {
                quick_xml::events::Event::Start(n) | quick_xml::events::Event::Empty(n) => {
                    break n.name().as_ref().eq_ignore_ascii_case(b"dialogs")
                }
                quick_xml::events::Event::Eof => break false,
                _ => {}
            }
        };
        if !is_dialog {
            continue;
        }
        files += 1;
        match DialogDefinition::read_collection(&[&bytes]) {
            Ok(defs) => {
                for dialog in defs {
                    dialogs += 1;
                    for i in dialog.interactions {
                        interactions += 1;
                        for m in i.messages {
                            messages += 1;
                            texts += m.texts.len();
                        }
                    }
                }
            }
            Err(error) => errors.push(format!("{}: {error:?}", path.display())),
        }
    }
    eprintln!("dialog corpus: {files} files, {dialogs} dialogs, {interactions} interactions, {messages} messages, {texts} texts; {} failures", errors.len());
    assert!(files > 0);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
