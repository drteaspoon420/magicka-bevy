//! User-selected element input. The retail gesture scheme remains the default.
use bevy::prelude::*;
use magicka_runtime::saves::SaveStore;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ElementStyle {
    #[default]
    RightStick,
    BumperDpad,
}
impl ElementStyle {
    pub fn label(self) -> &'static str {
        match self {
            Self::RightStick => "Right-stick gestures",
            Self::BumperDpad => "LB + D-pad",
        }
    }
    pub fn instructions(self) -> &'static str {
        match self {
            Self::RightStick => "Right stick / right trackpad: element gestures\nD-pad Up / Down: choose magick\nRight stick click: spell wheel",
            Self::BumperDpad => "D-pad Up / Down / Left / Right: Water / Life / Shield / Cold\nHold LB / L1: Lightning / Arcane / Earth / Fire\nRB / R1 + D-pad Left / Right: choose magick",
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Self::RightStick => "right-stick",
            Self::BumperDpad => "bumper-dpad",
        }
    }
    pub fn parse(value: Option<&str>) -> Self {
        if value == Some("bumper-dpad") {
            Self::BumperDpad
        } else {
            Self::RightStick
        }
    }
    pub fn other(self) -> Self {
        match self {
            Self::RightStick => Self::BumperDpad,
            Self::BumperDpad => Self::RightStick,
        }
    }
}
#[derive(Resource, Default)]
pub(super) struct Preferences {
    pub style: ElementStyle,
    store: Option<SaveStore>,
}
impl Preferences {
    pub fn for_args(args: &[String]) -> Self {
        let root = args
            .windows(2)
            .find(|a| a[0] == "--root")
            .map(|a| a[1].clone());
        let store = root
            .or_else(|| {
                args.iter()
                    .any(|a| a == "--play")
                    .then(|| "output/.cache".into())
            })
            .map(|root| SaveStore::new(std::path::PathBuf::from(root).join("saves")));
        let style = store
            .as_ref()
            .and_then(|s| s.settings().ok())
            .map_or(ElementStyle::default(), |v| {
                ElementStyle::parse(v["controller_element_style"].as_str())
            });
        Self { style, store }
    }
    pub fn save(&mut self, style: ElementStyle) {
        self.style = style;
        if let Some(store) = &self.store {
            let result = (|| {
                let mut settings = store.settings()?;
                settings["controller_element_style"] = style.key().into();
                store.save_settings(&settings)
            })();
            if let Err(error) = result {
                warn!("controller preference: {error}");
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn user_choice_survives_reload_without_changing_other_settings() {
        let root =
            std::env::temp_dir().join(format!("magicka-controller-style-{}", std::process::id()));
        let args = vec!["--root".into(), root.to_string_lossy().into_owned()];
        let mut preferences = Preferences::for_args(&args);
        preferences.save(ElementStyle::BumperDpad);
        assert_eq!(Preferences::for_args(&args).style, ElementStyle::BumperDpad);
        preferences.save(ElementStyle::RightStick);
        assert_eq!(Preferences::for_args(&args).style, ElementStyle::RightStick);
        std::fs::remove_dir_all(root).unwrap();
    }
}
