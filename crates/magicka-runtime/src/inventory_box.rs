//! PlayState.InventoryBox (Magicka.GameLogic.UI.InventoryBox): the
//! Inventory control (keyboard Tab) toggles a centred Dialog_Say box with
//! the UI/Menu/Magicks equipment picture and the weapon and staff names and
//! descriptions. See research/inventory-box.md.
use crate::bitmap_font::{FontError, FontMetrics};
use crate::localization::LocalizationTable;
use crate::scene_loading::custom_hash;

/// TextBoxEffect.BorderSize of the box.
pub const BORDER: f32 = 32.0;
/// UI/Menu/Magicks texel rectangle of the picture: (x, y, width, height).
pub const IMAGE: [f32; 4] = [880.0, 1260.0, 250.0, 230.0];
/// Text column: the picture's width; descriptions wrap at 350.
pub const TEXT_OFFSET: f32 = 250.0;
const WRAP_WIDTH: i32 = 350;
const WIDTH: f32 = 600.0;
const MIN_HEIGHT: f32 = 230.0;

/// Item.DisplayName / Item.Description string keys of one equipped item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemText {
    pub name: i32,
    pub description: i32,
}

impl ItemText {
    /// Item.Read hashes the raw strings (GetHashCodeCustom).
    pub fn new(display_name: &str, description: &str) -> Self {
        Self {
            name: custom_hash(display_name),
            description: custom_hash(description),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InventoryBox {
    pub show: bool,
    /// Open/close animation, 0..1 at 4 per second.
    pub scale: f32,
    pub owner: Option<u16>,
    weapon: i32,
    staff: i32,
    /// Maiandra14 markup (dialogue default colour).
    pub text: String,
    /// mBox: 600 wide, at least 230 high.
    pub size: [f32; 2],
}

impl InventoryBox {
    /// Active: showing or still animating closed.
    pub fn active(&self) -> bool {
        self.show | (self.scale > 0.0)
    }

    /// ShowInventory. The text is rebuilt only when either display name
    /// changed; the empty key hash stands in for native DisplayName 0.
    pub fn show_inventory(
        &mut self,
        weapon: ItemText,
        staff: ItemText,
        owner: u16,
        strings: &LocalizationTable,
        font: &FontMetrics,
    ) -> Result<(), FontError> {
        self.owner = Some(owner);
        self.show = true;
        if self.weapon == weapon.name && self.staff == staff.name && !self.text.is_empty() {
            return Ok(());
        }
        self.weapon = weapon.name;
        self.staff = staff.name;
        let empty = custom_hash("");
        let name = |id: i32| {
            if id == empty || id == 0 {
                String::new()
            } else {
                strings.get_string(id).unwrap_or_default()
            }
        };
        let description = |id: i32| -> Result<String, FontError> {
            if id == empty || id == 0 {
                return Ok(String::new());
            }
            let text = strings.get_string(id).unwrap_or_default();
            // LanguageManager.ParseReferences; an unparsable value is kept.
            let text = strings.parse_references(&text, usize::MAX).unwrap_or(text);
            Ok(font.wrap(&text, WRAP_WIDTH, true)?.text)
        };
        self.text = format!(
            "[c=1,1,1]{}[/c]\n{}\n\n[c=1,1,1]{}[/c]\n{}",
            name(weapon.name),
            description(weapon.description)?,
            name(staff.name),
            description(staff.description)?,
        );
        let height = font.measure(&self.text, true)?[1];
        self.size = [WIDTH, height.max(MIN_HEIGHT).trunc()];
        Ok(())
    }

    /// Close: only the owner (or anyone, with `None`) closes the box.
    pub fn close(&mut self, owner: Option<u16>) {
        if owner.is_none() || owner == self.owner {
            self.show = false;
            self.owner = None;
        }
    }

    /// Update: scale towards 1 while shown, towards 0 otherwise.
    pub fn update(&mut self, dt: f32) {
        if self.show {
            self.scale = (self.scale + dt * 4.0).min(1.0);
        } else if self.scale > 0.0 {
            self.scale = (self.scale - dt * 4.0).max(0.0);
        }
    }
}
