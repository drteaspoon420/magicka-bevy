//! LanguageManager.SetLanguage sequencing over explicit file/font/event hosts.
use crate::localization::{LocalizationError, LocalizationTable};

/// Native enum storage allows unnamed values, including initial -1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Language(pub i32);
const CODES: &[(&str, i32)] = &[
    ("ara", 1),
    ("bul", 2),
    ("cat", 3),
    ("zho", 4),
    ("ces", 5),
    ("dan", 6),
    ("deu", 7),
    ("ell", 8),
    ("eng", 9),
    ("spa", 10),
    ("fin", 11),
    ("fra", 12),
    ("heb", 13),
    ("hun", 14),
    ("isl", 15),
    ("ita", 16),
    ("jpn", 17),
    ("kor", 18),
    ("nld", 19),
    ("nor", 20),
    ("pol", 21),
    ("por", 22),
    ("ron", 24),
    ("rus", 25),
    ("hrv", 26),
    ("slk", 27),
    ("sqi", 28),
    ("swe", 29),
    ("tha", 30),
    ("tur", 31),
    ("urd", 32),
    ("ind", 33),
    ("ukr", 34),
    ("bel", 35),
    ("slv", 36),
    ("est", 37),
    ("lav", 38),
    ("lit", 39),
    ("fas", 41),
    ("vie", 42),
    ("hye", 43),
    ("aze", 44),
    ("eus", 45),
    ("mkd", 47),
    ("afr", 54),
    ("kat", 55),
    ("fao", 56),
    ("hin", 57),
    ("msa", 62),
    ("kaz", 63),
    ("kir", 64),
    ("swa", 65),
    ("uzb", 67),
    ("tat", 68),
    ("pan", 70),
    ("guj", 71),
    ("tam", 73),
    ("tel", 74),
    ("kan", 75),
    ("mar", 78),
    ("san", 79),
    ("mon", 80),
    ("glg", 86),
    ("kok", 87),
    ("syr", 90),
    ("div", 101),
    ("srp", 31770),
];
impl Language {
    pub const ENGLISH: Self = Self(9);
    pub const UNINITIALIZED: Self = Self(-1);
    /// Named enum members only. Full CLR Enum.Parse syntax is a host concern.
    pub fn from_code(code: &str) -> Option<Self> {
        CODES
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(code))
            .map(|&(_, value)| Self(value))
    }
    /// Enum.ToString uses decimal for unnamed single values.
    pub fn directory_name(self) -> String {
        CODES
            .iter()
            .find(|&&(_, value)| value == self.0)
            .map_or_else(|| self.0.to_string(), |&(name, _)| name.into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LanguageListener(pub u64);
#[derive(Debug, PartialEq, Eq)]
pub enum LanguageChangeError<E> {
    Host(E),
    Localization(LocalizationError),
}
pub trait LanguageHost {
    type Error;
    type Table;
    /// Immediate *.loctable.xml files, in explicit host enumeration order.
    fn enumerate_tables(&mut self, language: Language) -> Result<Vec<Self::Table>, Self::Error>;
    fn read_table(&mut self, table: &Self::Table) -> Result<Vec<u8>, Self::Error>;
    /// Unload/reload only cached fonts. Called even when no fonts are cached.
    fn reload_fonts(&mut self, language: Language) -> Result<(), Self::Error>;
    fn language_changed(
        &mut self,
        listener: LanguageListener,
        state: &mut LanguageState,
    ) -> Result<(), Self::Error>;
}

#[derive(Debug)]
pub struct LanguageState {
    pub strings: LocalizationTable,
    pub available: Vec<Language>,
    current: Language,
    listeners: Vec<LanguageListener>,
}
impl LanguageState {
    /// Host supplies successfully parsed directory enum values, without sorting
    /// or deduplicating them on behalf of the original constructor.
    pub fn new(available: Vec<Language>) -> Self {
        Self {
            strings: LocalizationTable::default(),
            available,
            current: Language::UNINITIALIZED,
            listeners: vec![],
        }
    }
    pub fn current(&self) -> Language {
        self.current
    }
    pub fn listeners(&self) -> &[LanguageListener] {
        &self.listeners
    }
    pub fn subscribe(&mut self, listener: LanguageListener) {
        self.listeners.push(listener);
    }
    /// Delegate.Remove for a single host-owned callable removes its last match.
    pub fn unsubscribe(&mut self, listener: LanguageListener) {
        if let Some(index) = self.listeners.iter().rposition(|&v| v == listener) {
            self.listeners.remove(index);
        }
    }
    /// Returns false for the native no-op path. Errors preserve mutation prefixes.
    pub fn set_language<H: LanguageHost>(
        &mut self,
        requested: Language,
        host: &mut H,
    ) -> Result<bool, LanguageChangeError<H::Error>> {
        let language = if self.available.contains(&requested) {
            requested
        } else {
            Language::ENGLISH
        };
        if language == self.current {
            return Ok(false);
        }
        self.strings.strings.clear();
        self.current = language;
        let tables = host
            .enumerate_tables(language)
            .map_err(LanguageChangeError::Host)?;
        for table in tables {
            let bytes = host.read_table(&table).map_err(LanguageChangeError::Host)?;
            self.strings
                .add_xml(&bytes)
                .map_err(LanguageChangeError::Localization)?;
        }
        host.reload_fonts(language)
            .map_err(LanguageChangeError::Host)?;
        // A delegate invocation retains its invocation list despite subscriptions
        // changing inside a callback. Later callbacks see any reentrant state.
        for listener in self.listeners.clone() {
            host.language_changed(listener, self)
                .map_err(LanguageChangeError::Host)?;
        }
        Ok(true)
    }
}
