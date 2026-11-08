//! SubMenuDLC: The Stars Are Left (the Mythos campaign, through the save
//! slots), Dungeons & Daemons and Dungeons & Gargoyles (story challenges
//! with their own maps), The Other Side of the Coin and Vietnam. Each tile
//! is its banner picture with a shadowed MenuOption caption; a padlock
//! covers a tile whose DLC is not owned.
use super::{
    book::{Atlas, Font, Measure, PageDraw, Widget},
    pages::{back_button, back_item, loc, Item, List, PageInput},
    Frontend, GameMode,
};
use crate::frontend_tome::{MenuId, TomeView};
use magicka_runtime::end_game::{DUNGEONS_CAMPAIGN, MYTHOS_CAMPAIGN};
use magicka_runtime::text_geometry::TextAlign;

/// SteamAPI DLC licences. This build assumes every DLC is owned; the flags
/// stay so ownership can be wired in later.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DlcLicences {
    pub the_stars_are_left: bool,
    pub dungeons_daemons: bool,
    pub dungeons_gargoyles: bool,
    pub other_side_of_the_coin: bool,
    pub vietnam: bool,
}

impl Default for DlcLicences {
    fn default() -> Self {
        Self {
            the_stars_are_left: true,
            dungeons_daemons: true,
            dungeons_gargoyles: true,
            other_side_of_the_coin: true,
            vietnam: true,
        }
    }
}

impl DlcLicences {
    /// HackHelper.CheckLicense for a PackMan pack: every DLC app of its
    /// contents subscribed. App 0 is the base game and 42918 Vietnam; the
    /// item-pack DLCs (Holiday Spirit, Horror Props, Gadgets, Heirlooms,
    /// Sporting Goods, Meteor Shower) have no tile and count as owned, like
    /// every other DLC in this build.
    pub(crate) fn licensed(&self, app: u32) -> bool {
        match app {
            42918 => self.vietnam,
            _ => true,
        }
    }
}

struct Tile {
    texture: &'static str,
    label: &'static str,
    position: [f32; 2],
    size: [f32; 2],
}

const LARGE: [f32; 2] = [768.0, 256.0];
const SMALL: [f32; 2] = [388.0, 226.0];
const LEFT: f32 = 166.0;
const TOP: f32 = 54.0;
const SECOND_ROW: f32 = TOP + LARGE[1] + 23.0;
const THIRD_ROW: f32 = SECOND_ROW + SMALL[1] + 28.0;

const TILES: [Tile; 5] = [
    Tile {
        texture: "UI/Menu/DLC/MainMenuDLC_TheStarsAreLeft",
        label: "#tsar_menu_mythos",
        position: [LEFT, TOP],
        size: LARGE,
    },
    Tile {
        texture: "UI/Menu/DLC/MainMenuDLC_DungeonsAndDaemons",
        label: "#challenge_dungeons_chapter1",
        position: [LEFT, SECOND_ROW],
        size: SMALL,
    },
    Tile {
        texture: "UI/Menu/DLC/MainMenuDLC_DungeonsAndGargoyles",
        label: "#challenge_dungeons_chapter2",
        position: [LEFT + SMALL[0] + 23.0, SECOND_ROW],
        size: SMALL,
    },
    Tile {
        texture: "UI/Menu/DLC/MainMenuDLC_OtherSideOfTheCoin",
        label: "#challenge_osotc",
        position: [LEFT, THIRD_ROW],
        size: SMALL,
    },
    Tile {
        texture: "UI/Menu/DLC/MainMenuDLC_Vietnam",
        label: "#challenge_vietnam",
        position: [LEFT + SMALL[0] + 23.0, THIRD_ROW],
        size: SMALL,
    },
];

/// 1 - Defines.MESSAGEBOX_COLOR_DEFAULT: the tiles' unselected colour.
const TILE_COLOR: [f32; 4] = [0.8963, 0.8424, 0.9798, 1.0];
/// SubMenuDLC.LOCK_OFFSET / LOCK_SIZE in the pages atlas.
const LOCK: [f32; 4] = [944.0, 816.0, 112.0, 160.0];

#[derive(Default)]
pub(crate) struct DlcPage {
    list: List,
}

impl DlcPage {
    pub(crate) fn enter(&mut self) {
        self.list.selected = Some(0);
    }
    fn owned(s: &Frontend, i: usize) -> bool {
        let d = s.dlc;
        [
            d.the_stars_are_left,
            d.dungeons_daemons,
            d.dungeons_gargoyles,
            d.other_side_of_the_coin,
            d.vietnam,
        ][i]
    }
    fn layout(&self, s: &Frontend, measure: &mut Measure) -> (Vec<Widget>, Vec<Item>) {
        let mut out = Vec::new();
        let mut items = Vec::new();
        let half_line = measure.line_height(Font::MenuOption) * 0.5;
        for (i, tile) in TILES.iter().enumerate() {
            let on = self.list.selected == Some(i);
            let color = if on { [1.0; 4] } else { TILE_COLOR };
            out.push(
                Widget::image(
                    Atlas::Content(tile.texture.into()),
                    [0.0, 0.0, tile.size[0], tile.size[1]],
                    tile.position,
                    tile.size,
                )
                .color(color),
            );
            // UIUtils.GetSingleTileTextPosition, with a two-pixel shadow.
            let text = [
                tile.position[0] + tile.size[0] * 0.5 - 2.0,
                tile.position[1] + tile.size[1] - half_line + 5.0,
            ];
            let label = loc(s, tile.label);
            out.push(
                Widget::text(
                    Font::MenuOption,
                    label.clone(),
                    [text[0] + 2.0, text[1] + 2.0],
                    TextAlign::Center,
                )
                .color([0.0, 0.0, 0.0, 1.0]),
            );
            out.push(Widget::text(Font::MenuOption, label, text, TextAlign::Center).color(color));
            if !Self::owned(s, i) {
                out.push(Widget::image(
                    Atlas::Pages,
                    LOCK,
                    [
                        tile.position[0] + (tile.size[0] - LOCK[2]) * 0.5,
                        tile.position[1] + (tile.size[1] - LOCK[3]) * 0.5,
                    ],
                    [LOCK[2], LOCK[3]],
                ));
            }
            items.push(Item::new(
                tile.position,
                [
                    tile.position[0] + tile.size[0],
                    tile.position[1] + tile.size[1],
                ],
                true,
            ));
        }
        out.push(back_button(self.list.selected == Some(TILES.len())));
        items.push(back_item());
        (out, items)
    }
    pub(crate) fn update(
        &mut self,
        s: &mut Frontend,
        tome: &mut TomeView,
        input: &PageInput,
        measure: &mut Measure,
    ) {
        if input.back {
            tome.pop();
            return;
        }
        let (_, items) = self.layout(s, measure);
        let Some(choice) = self.list.update(&items, input) else {
            return;
        };
        if choice < TILES.len() && !Self::owned(s, choice) {
            // The store page is not part of this build.
            return;
        }
        match choice {
            0 => {
                s.mode = GameMode::Mythos;
                s.campaign = MYTHOS_CAMPAIGN[0].into();
                tome.push(MenuId::SaveSlot, 3);
            }
            1..=4 => {
                // SetSettings(GameType.StoryChallange, level): straight to
                // character select, no save slot.
                s.mode = GameMode::StoryChallenge;
                s.fresh = true;
                s.campaign = match choice {
                    1 => DUNGEONS_CAMPAIGN[0],
                    2 => DUNGEONS_CAMPAIGN[1],
                    3 => "Challenges/ch_osotc.lvl",
                    _ => "Challenges/ch_vietnam.lvl",
                }
                .into();
                tome.push(MenuId::Character, 1);
            }
            _ => tome.pop(),
        }
    }
    pub(crate) fn draw(&self, s: &Frontend, draw: &mut PageDraw, measure: &mut Measure) {
        let (out, _) = self.layout(s, measure);
        let mut page = draw.right(MenuId::Dlc);
        for widget in out {
            page.push(widget);
        }
    }
}
