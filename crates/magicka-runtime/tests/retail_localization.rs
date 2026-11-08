use magicka_content::{assets::Asset, parse_primary, xna::XnaObject, Content};
use magicka_runtime::bitmap_font::{FontMetrics, MessageTextLayout};
use magicka_runtime::dialog_definition::DialogDefinition;
use magicka_runtime::localization::LocalizationTable;
use magicka_runtime::text_box_initialize::{
    TextBoxFonts, TextBoxInit, TextBoxInitHost, TextBoxLocation, TextBoxWidget,
};
use magicka_runtime::text_geometry::{GeometryFont, TextGeometry, TextStyle};
use std::path::{Path, PathBuf};
fn walk(dir: &Path, paths: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, paths);
        } else if path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".loctable.xml")
        {
            paths.push(path);
        }
    }
}
fn dialog_paths(dir: &Path, paths: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            dialog_paths(&path, paths);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("xml"))
        {
            paths.push(path);
        }
    }
}
fn read_font(path: &Path) -> magicka_content::assets::BitmapFont {
    let bytes = std::fs::read(path).unwrap();
    let xnb = magicka_xnb::xnb::parse(&bytes).unwrap();
    let parsed = parse_primary(&xnb).unwrap().unwrap();
    assert_eq!(parsed.trailing, 0);
    let Content::Xna(object) = parsed.content else {
        panic!("expected font object")
    };
    let XnaObject::Asset(asset) = *object else {
        panic!("expected font asset")
    };
    let Asset::BitmapFont(content) = *asset else {
        panic!("expected bitmap font")
    };
    content
}
struct BackgroundHost {
    calls: usize,
}
impl TextBoxInitHost for BackgroundHost {
    type Error = ();
    fn load_background(&mut self, path: &str) -> Result<(), ()> {
        assert_eq!(path, "UI/HUD/Dialog_Say");
        self.calls += 1;
        Ok(())
    }
}
#[test]
fn retail_localization_tables_decode_without_executing_content() {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    if !root.is_dir() {
        eprintln!("SKIP: retail content absent");
        return;
    }
    let mut paths = vec![];
    walk(&root.join("Languages"), &mut paths);
    paths.sort();
    let mut rows = 0;
    let mut errors = vec![];
    let mut english = LocalizationTable::default();
    english.set_player_names(&["FixtureWizard"]);
    for path in &paths {
        let mut table = LocalizationTable::default();
        match table.add_xml(&std::fs::read(path).unwrap()) {
            Ok(count) => rows += count,
            Err(e) => errors.push(format!("{}: {e:?}", path.display())),
        }
        if path.parent().unwrap().file_name().unwrap() == "eng" {
            english.add_xml(&std::fs::read(path).unwrap()).unwrap();
        }
    }
    eprintln!(
        "localization corpus: {} files, {rows} accepted rows; {} failures",
        paths.len(),
        errors.len()
    );
    assert!(!paths.is_empty());
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    let content = read_font(&root.join("Languages/eng/Font/Maiandra16.xnb"));
    let mut font = FontMetrics::from_content(&content).unwrap();
    // FontManager custom spacing for Maiandra16, from its static initializer.
    font.spacing = 1;
    // TextBox's body is a fixed Maiandra14 TypingText, despite Message.Initialize
    // using Maiandra16 for wrapping/measurement. Native Maiandra14 spacing is zero.
    let geometry_font =
        GeometryFont::from_content(&read_font(&root.join("Languages/eng/Font/Maiandra14.xnb")))
            .unwrap();
    let mut name_geometry_font = GeometryFont::from_content(&content).unwrap();
    name_geometry_font.metrics.spacing = 1;
    let background_bytes = std::fs::read(root.join("UI/HUD/Dialog_Say.xnb")).unwrap();
    let background_xnb = magicka_xnb::xnb::parse(&background_bytes).unwrap();
    let background = parse_primary(&background_xnb).unwrap().unwrap();
    assert_eq!(background.trailing, 0);
    let Content::Xna(object) = background.content else {
        panic!("expected background")
    };
    let XnaObject::Asset(asset) = *object else {
        panic!("expected background asset")
    };
    let Asset::Texture2D(texture) = *asset else {
        panic!("expected background texture")
    };
    assert!(texture.width > 0 && texture.height > 0);
    let mut background_host = BackgroundHost { calls: 0 };
    let mut paths = vec![];
    dialog_paths(&root.join("Levels"), &mut paths);
    paths.sort();
    let (mut texts, mut missing_initial, mut rendered_glyphs, mut named_boxes) = (0, 0, 0, 0);
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
        for dialog in DialogDefinition::read_collection(&[&bytes]).unwrap() {
            for interaction in dialog.interactions {
                for message in interaction.messages {
                    let mut layout = MessageTextLayout::default();
                    layout
                        .initialize(&message, &english, &font, 10_000)
                        .unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
                    assert_eq!(layout.texts.len(), message.texts.len());
                    assert!(layout.texts.iter().all(Option::is_some));
                    let mut widget = TextBoxWidget::default();
                    for (i, wrapped) in layout.texts.iter().enumerate() {
                        widget
                            .initialize(
                                TextBoxInit {
                                    scene: Some(1),
                                    location: TextBoxLocation::Screen {
                                        position: [0.; 2],
                                        name: message.owner,
                                    },
                                    text: wrapped.as_deref(),
                                    minimum_size: layout.minimum_box_size,
                                    force_on_screen: message.force_on_screen,
                                    ttl: message.texts[i].ttl,
                                },
                                &english,
                                TextBoxFonts {
                                    body: &geometry_font.metrics,
                                    owner_name: &font,
                                },
                                &mut background_host,
                                10_000,
                            )
                            .unwrap_or_else(|e| panic!("{}: textbox: {e:?}", path.display()));
                        named_boxes += usize::from(widget.show_name);
                        if widget.show_name {
                            TextGeometry::build(
                                &name_geometry_font,
                                &widget.name_characters,
                                TextStyle::dialogue(),
                            )
                            .unwrap();
                        }
                        assert!(widget.state.size.iter().all(|&v| v >= 0));
                        let typing = &mut widget.state.typing;
                        let geometry = TextGeometry::rebuild_typing(
                            &geometry_font,
                            typing,
                            TextStyle::dialogue(),
                        )
                        .unwrap_or_else(|e| panic!("{}: geometry: {e:?}", path.display()));
                        rendered_glyphs += geometry.quads.len();
                        assert!(geometry
                            .quads
                            .iter()
                            .flat_map(|q| q.vertices.iter())
                            .all(|v| v
                                .position
                                .iter()
                                .chain(v.uv.iter())
                                .chain(v.color.iter())
                                .all(|n| n.is_finite())));
                        // Large one-shot dt is a harness completion guard only.
                        typing
                            .update(10_000.0)
                            .unwrap_or_else(|e| panic!("{}: typing: {e:?}", path.display()));
                        assert!(typing.finished());
                    }
                    assert!(layout.minimum_box_size.iter().all(|v| v.is_finite()));
                    for text in message.texts {
                        texts += 1;
                        missing_initial +=
                            usize::from(!english.strings.contains_key(&text.localization));
                        // Budget is a harness guard, not a recovered engine constant.
                        if let Err(e) = english.expand_message(&text, 10_000) {
                            errors.push(format!(
                                "{}: localization {}: {e:?}",
                                path.display(),
                                text.localization
                            ));
                        }
                    }
                }
            }
        }
    }
    eprintln!("English dialog text phase: {texts} texts, {missing_initial} missing initial keys; {} expansion failures", errors.len());
    eprintln!("English dialog wrapping/measurement: {texts} text entries initialized with original Maiandra16 metrics and custom spacing");
    eprintln!(
        "English dialog geometry/typing: {texts} texts, {rendered_glyphs} Maiandra14 glyph quads, all finish using actual CPU geometry counts"
    );
    eprintln!("English TextBox Initialize screen path: {} texture requests, {named_boxes} named boxes; original Dialog_Say texture {}x{} decoded",background_host.calls,texture.width,texture.height);
    assert!(texts > 0);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}
