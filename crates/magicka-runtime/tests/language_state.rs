use magicka_runtime::{
    language_state::{
        Language, LanguageChangeError, LanguageHost, LanguageListener, LanguageState,
    },
    scene_loading::custom_hash,
};

fn table(value: &str) -> Vec<u8> {
    format!("<Workbook><Worksheet><Table><Row><Cell>#title</Cell><Cell>{value}</Cell></Row></Table></Worksheet></Workbook>").into_bytes()
}
#[derive(Default)]
struct Host {
    trace: Vec<String>,
    fail: Option<&'static str>,
    mutate_listeners: bool,
    reenter: bool,
}
impl LanguageHost for Host {
    type Error = &'static str;
    type Table = usize;
    fn enumerate_tables(&mut self, lang: Language) -> Result<Vec<usize>, Self::Error> {
        self.trace.push(format!("enumerate:{}", lang.0));
        if self.fail == Some("enumerate") {
            return Err("enumerate");
        }
        Ok(vec![0, 1])
    }
    fn read_table(&mut self, id: &usize) -> Result<Vec<u8>, Self::Error> {
        self.trace.push(format!("read:{id}"));
        if *id == 1 && self.fail == Some("read") {
            return Err("read");
        }
        if *id == 1 && self.fail == Some("parse") {
            return Ok(b"<Other/>".to_vec());
        }
        Ok(table(if *id == 0 { "first" } else { "second" }))
    }
    fn reload_fonts(&mut self, lang: Language) -> Result<(), Self::Error> {
        self.trace.push(format!("fonts:{}", lang.0));
        if self.fail == Some("fonts") {
            return Err("fonts");
        }
        Ok(())
    }
    fn language_changed(
        &mut self,
        listener: LanguageListener,
        state: &mut LanguageState,
    ) -> Result<(), Self::Error> {
        self.trace
            .push(format!("event:{}:{}", listener.0, state.current().0));
        assert_eq!(
            state.strings.get_string(custom_hash("#title")).as_deref(),
            Some("second")
        );
        if listener.0 == 1 && self.mutate_listeners {
            state.unsubscribe(LanguageListener(2));
            state.subscribe(LanguageListener(3));
        }
        if listener.0 == 1 && self.reenter && state.current() == Language(25) {
            state.set_language(Language::ENGLISH, self).unwrap();
        }
        if listener.0 == 1 && self.fail == Some("event") {
            return Err("event");
        }
        Ok(())
    }
}
fn state() -> LanguageState {
    let mut state = LanguageState::new(vec![Language::ENGLISH, Language(25)]);
    state.subscribe(LanguageListener(1));
    state.subscribe(LanguageListener(2));
    state
}
#[test]
fn switches_in_file_font_event_order_and_preserves_player_cache() {
    let mut state = state();
    assert_eq!(state.current(), Language::UNINITIALIZED);
    state
        .strings
        .strings
        .insert(custom_hash("#old"), "old".into());
    state.strings.set_player_names(&["Ada", "Bo"]);
    let mut host = Host::default();
    assert_eq!(state.set_language(Language(25), &mut host), Ok(true));
    assert_eq!(
        host.trace,
        [
            "enumerate:25",
            "read:0",
            "read:1",
            "fonts:25",
            "event:1:25",
            "event:2:25"
        ]
    );
    assert!(!state.strings.strings.contains_key(&custom_hash("#old")));
    assert_eq!(
        state.strings.get_string(custom_hash("#players")).as_deref(),
        Some("Ada & Bo")
    );
    assert_eq!(state.set_language(Language(25), &mut host), Ok(false));
    assert_eq!(host.trace.len(), 6);
}
#[test]
fn fallback_precedes_same_language_check_even_when_english_is_absent() {
    let mut state = LanguageState::new(vec![Language(25)]);
    let mut host = Host::default();
    assert_eq!(state.set_language(Language(999), &mut host), Ok(true));
    assert_eq!(state.current(), Language::ENGLISH);
    assert_eq!(state.set_language(Language(999), &mut host), Ok(false));
    assert_eq!(host.trace, ["enumerate:9", "read:0", "read:1", "fonts:9"]);
}
#[test]
fn failures_keep_the_completed_prefix_and_same_language_retry_does_nothing() {
    for failure in ["enumerate", "read", "parse", "fonts", "event"] {
        let mut state = state();
        state.strings.strings.insert(123, "obsolete".into());
        let mut host = Host {
            fail: Some(failure),
            ..Default::default()
        };
        let result = state.set_language(Language(25), &mut host);
        assert!(result.is_err(), "{failure}");
        assert_eq!(state.current(), Language(25));
        assert!(!state.strings.strings.contains_key(&123));
        let expected = match failure {
            "enumerate" => None,
            "read" | "parse" => Some("first"),
            _ => Some("second"),
        };
        assert_eq!(
            state
                .strings
                .strings
                .get(&custom_hash("#title"))
                .map(String::as_str),
            expected
        );
        if failure == "parse" {
            assert!(matches!(result, Err(LanguageChangeError::Localization(_))));
        } else {
            assert_eq!(result, Err(LanguageChangeError::Host(failure)));
        }
        assert!(!host.trace.iter().any(|v| v.starts_with("event:2")));
        let count = host.trace.len();
        host.fail = None;
        assert_eq!(state.set_language(Language(25), &mut host), Ok(false));
        assert_eq!(host.trace.len(), count);
    }
}
#[test]
fn listener_snapshot_survives_mutation_and_last_duplicate_is_removed() {
    let mut state = state();
    state.subscribe(LanguageListener(1));
    state.unsubscribe(LanguageListener(1));
    assert_eq!(
        state.listeners(),
        &[LanguageListener(1), LanguageListener(2)]
    );
    let mut host = Host {
        mutate_listeners: true,
        ..Default::default()
    };
    state.set_language(Language(25), &mut host).unwrap();
    assert_eq!(host.trace[4..], ["event:1:25", "event:2:25"]);
    assert_eq!(
        state.listeners(),
        &[LanguageListener(1), LanguageListener(3)]
    );
    host.trace.clear();
    state.set_language(Language::ENGLISH, &mut host).unwrap();
    assert_eq!(host.trace[4..], ["event:1:9", "event:3:9"]);
}
#[test]
fn reentrant_switch_finishes_nested_events_before_outer_list_continues() {
    let mut state = state();
    let mut host = Host {
        reenter: true,
        ..Default::default()
    };
    state.set_language(Language(25), &mut host).unwrap();
    assert_eq!(state.current(), Language::ENGLISH);
    assert_eq!(
        host.trace,
        [
            "enumerate:25",
            "read:0",
            "read:1",
            "fonts:25",
            "event:1:25",
            "enumerate:9",
            "read:0",
            "read:1",
            "fonts:9",
            "event:1:9",
            "event:2:9",
            "event:2:9"
        ]
    );
}
#[test]
fn enum_names_use_native_culture_ids_and_unnamed_values_stay_signed() {
    assert_eq!(Language::from_code("ENG"), Some(Language::ENGLISH));
    assert_eq!(Language::from_code("srp"), Some(Language(31770)));
    assert_eq!(Language::from_code("unknown"), None);
    assert_eq!(Language(25).directory_name(), "rus");
    assert_eq!(Language(-1).directory_name(), "-1");
}
