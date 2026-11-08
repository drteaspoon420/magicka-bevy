//! Read-only private corpus validation. Never executes target binaries.
use magicka_content::{assets::Asset, xna::XnaObject, Content};
use magicka_runtime::{
    bitmap_font::FontMetrics,
    error_message_box::{ok_title_id, ErrorMessageBoxState},
    language_state::{Language, LanguageHost, LanguageListener, LanguageState},
    menu_text_item::MenuTextItemState,
    message_box_collection::{MessageBoxHandle, MessageBoxes},
    scene_loading::custom_hash,
    text_geometry::TextAlign,
    waiting_message_box::{default_message_id, WaitingMessageBoxState},
};
use std::path::PathBuf;

struct Host {
    root: PathBuf,
    metrics: Option<FontMetrics>,
    font_language: Language,
    boxes: MessageBoxes,
    handles: Vec<MessageBoxHandle>,
    menu: Option<MenuTextItemState>,
    events: Vec<(u64, i32)>,
    files: usize,
}
impl Host {
    fn load_font(&self, language: Language) -> (Language, FontMetrics) {
        let selected = if self
            .root
            .join("Languages")
            .join(language.directory_name())
            .join("Font")
            .is_dir()
        {
            language
        } else {
            Language::ENGLISH
        };
        let path = self
            .root
            .join("Languages")
            .join(selected.directory_name())
            .join("Font/MenuDefault.xnb");
        let bytes = std::fs::read(path).unwrap();
        let xnb = magicka_xnb::xnb::parse(&bytes).unwrap();
        let parsed = magicka_content::parse_primary(&xnb).unwrap().unwrap();
        assert_eq!(parsed.trailing, 0);
        let Content::Xna(object) = parsed.content else {
            panic!("not XNA")
        };
        let XnaObject::Asset(asset) = *object else {
            panic!("not an asset")
        };
        let Asset::BitmapFont(font) = *asset else {
            panic!("not a font")
        };
        let mut metrics = FontMetrics::from_content(&font).unwrap();
        metrics.spacing = 1;
        (selected, metrics)
    }
}
impl LanguageHost for Host {
    type Error = String;
    type Table = PathBuf;
    fn enumerate_tables(&mut self, language: Language) -> Result<Vec<PathBuf>, String> {
        let dir = self.root.join("Languages").join(language.directory_name());
        let mut files = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .ends_with(".loctable.xml")
            })
            .collect::<Vec<_>>();
        files.sort(); // Test choice, not recovered native filesystem order.
        Ok(files)
    }
    fn read_table(&mut self, file: &PathBuf) -> Result<Vec<u8>, String> {
        self.files += 1;
        std::fs::read(file).map_err(|e| e.to_string())
    }
    fn reload_fonts(&mut self, language: Language) -> Result<(), String> {
        if self.metrics.is_none() {
            return Ok(());
        }
        let (selected, fresh) = self.load_font(language);
        if let Some(metrics) = self.metrics.as_mut() {
            metrics.read_from(&fresh);
            self.font_language = selected;
        }
        Ok(())
    }
    fn language_changed(
        &mut self,
        listener: LanguageListener,
        state: &mut LanguageState,
    ) -> Result<(), String> {
        self.events.push((listener.0, state.current().0));
        let font = self.metrics.as_ref().unwrap();
        match listener.0 {
            1 | 2 => self
                .boxes
                .language_changed(self.handles[listener.0 as usize - 1], &state.strings, font)
                .map_err(|e| format!("{e:?}")),
            // Represents the containing SubMenu forwarding to its single item.
            3 => self
                .menu
                .as_mut()
                .unwrap()
                .language_changed(&state.strings, font)
                .map_err(|e| format!("{e:?}")),
            _ => panic!("unknown listener"),
        }
    }
}
#[test]
#[ignore = "requires private original localization tables and menu fonts"]
fn private_original_language_switch_reloads_fonts_before_owned_callbacks() {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    let available = std::fs::read_dir(root.join("Languages"))
        .unwrap()
        .map(|e| e.unwrap())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| Language::from_code(&e.file_name().to_string_lossy()))
        .collect::<Vec<_>>();
    let mut state = LanguageState::new(available);
    let mut host = Host {
        root,
        metrics: None,
        font_language: Language::UNINITIALIZED,
        boxes: MessageBoxes::default(),
        handles: vec![],
        menu: None,
        events: vec![],
        files: 0,
    };
    state.strings.set_player_names(&["Ada", "Bo"]);
    state.set_language(Language::ENGLISH, &mut host).unwrap();
    let (selected, font) = host.load_font(state.current());
    host.font_language = selected;
    host.metrics = Some(font);
    let font = host.metrics.as_ref().unwrap();
    assert_eq!(font.spacing, 1);
    assert!(state.strings.try_get_string(default_message_id()).found);
    assert!(state.strings.try_get_string(ok_title_id()).found);
    let mut waiting = WaitingMessageBoxState::new([800., 600.]);
    waiting
        .show(default_message_id(), &state.strings, font, |_| {
            Ok::<_, ()>(())
        })
        .unwrap();
    let original_waiting = waiting.message.clone();
    waiting.kill(); // Still subscribed after Kill, unlike Dispose.
    host.handles.push(host.boxes.insert_waiting(waiting));
    host.handles.push(
        host.boxes.insert(
            ErrorMessageBoxState::new_localized([800., 600.], &state.strings, font).unwrap(),
        ),
    );
    let original_ok = host
        .boxes
        .get(host.handles[1])
        .unwrap()
        .ok_button
        .as_ref()
        .unwrap()
        .characters
        .clone();
    let menu_id = custom_hash("#menu_main_02");
    host.menu = Some(
        MenuTextItemState::new_localized(
            menu_id,
            [0.; 2],
            TextAlign::Left,
            40,
            &state.strings,
            font,
        )
        .unwrap(),
    );
    let original_name = host.menu.as_ref().unwrap().text.clone();
    for listener in 1..=3 {
        state.subscribe(LanguageListener(listener));
    }
    for (language, expected_font) in [
        (Language(25), Language(25)),
        (Language(7), Language::ENGLISH),
    ] {
        assert!(state.available.contains(&language));
        state.set_language(language, &mut host).unwrap();
        assert_eq!(host.metrics.as_ref().unwrap().spacing, 0);
        assert_eq!(host.font_language, expected_font);
        let waiting = host.boxes.get_waiting(host.handles[0]).unwrap();
        assert_eq!(
            waiting.message,
            state.strings.get_string(default_message_id())
        );
        assert!(waiting.base.killed);
        if language == Language(25) {
            assert_ne!(waiting.message, original_waiting);
        }
        let menu = host.menu.as_ref().unwrap();
        let displayed: Vec<_> = menu
            .characters
            .iter()
            .copied()
            .take_while(|&u| u != 0)
            .collect();
        assert_eq!(
            displayed,
            state
                .strings
                .get_string(menu_id)
                .unwrap()
                .encode_utf16()
                .collect::<Vec<_>>()
        );
        assert_eq!(menu.text, original_name);
        assert_eq!(
            host.boxes
                .get(host.handles[1])
                .unwrap()
                .ok_button
                .as_ref()
                .unwrap()
                .characters,
            original_ok
        );
        assert_eq!(
            state.strings.get_string(custom_hash("#players")).as_deref(),
            Some("Ada & Bo")
        );
        println!("language={} font_language={} spacing={} dictionary_entries={} files_read={} waiting_units={} menu_bounds={:?}..{:?}",
            state.current().0, host.font_language.0, host.metrics.as_ref().unwrap().spacing, state.strings.strings.len(), host.files,
            waiting.message.as_deref().unwrap().encode_utf16().count(), menu.top_left, menu.bottom_right);
    }
    assert_eq!(
        host.events,
        [(1, 25), (2, 25), (3, 25), (1, 7), (2, 7), (3, 7)]
    );
}
