//! SubMenuCharacterSelect's "Allowed drops" (research/rulesets.md, "Lobby
//! pack selection"): the overview row of enabled PackMan packs under the
//! level box of a Challenge or Versus lobby, and the ChangingPacks list in
//! which the host toggles the item and magick packs that Survival and
//! Versus luggage draw from. Layout, hit testing and navigation are pure
//! functions over the catalogue and the enabled flags; the character page
//! owns the input frame and the persisted settings.
//!
//! Native geometry (page pixels, GUIBasicEffect centred unit quads of 64):
//! the label at (128, 368); the overview thumbs from x 128 at y 408 with
//! scroll arrows at x 64 and 448 once more than five packs are enabled; the
//! list beside its MenuScrollBar at (480, 320) of height 448, seven rows of
//! 64 from y 96 with thumbs from x 128 and headings at x 96, and the OK
//! button at (512, 576). Not reproduced: the pack tooltips, the Steam store
//! overlay for unowned packs and the "new pack" star beside the label.
use super::book::{Atlas, Font, Measure, Widget};
use super::controls_pages::ScrollBar;
use super::pages::{menu_text_item, text_button, Item};
use magicka_runtime::ruleset::{PackCatalogue, PackKind};
use magicka_runtime::text_geometry::TextAlign;

/// PackMan.Thumbnails.
pub(crate) const THUMBS: &str = "UI/Menu/PackThumbs";
const CELL: f32 = 64.0;
/// mPacksText (DrawPacksOverview).
pub(crate) const LABEL: [f32; 2] = [128.0, 368.0];
/// The label's hit box reaches the thumb row (HitPackOverview).
const LABEL_HEIGHT: f32 = 40.0;
const OVERVIEW_TOP: f32 = 408.0;
const OVERVIEW_LEFT: f32 = 128.0;
const OVERVIEW_SLOTS: usize = 5;
const ARROW_LEFT: f32 = 64.0;
const ARROW_RIGHT: f32 = 448.0;
/// mSpecialScrollBar while changing packs: x 480, height 448 from y 96.
const LIST_BAR: [f32; 2] = [480.0, 320.0];
const LIST_HEIGHT: f32 = 448.0;
const LIST_TOP: f32 = 96.0;
const LIST_LEFT: f32 = 128.0;
const LIST_HEADING_X: f32 = 96.0;
const LIST_ROWS: usize = 7;
const PER_ROW: usize = 5;
/// mPackCloseButton: below the bar, right-aligned text of width 200.
const OK: [f32; 2] = [512.0, LIST_BAR[1] + LIST_HEIGHT * 0.5 + 32.0];
const OK_WIDTH: f32 = 200.0;
/// The "new pack" overlay: PackThumbs' bottom-right cell (the atlas is
/// 1024 × 128, TextureOffset ((W − 64) / W, (H − 64) / H)).
const NEW_CELL: [u32; 2] = [15, 1];
/// The pages-atlas arrow (MenuScrollBar's), turned for left and right.
const ARROW: [f32; 4] = [1280.0, 96.0, 64.0, 64.0];
/// Selected thumbs draw at 1.5 × colour.
const HIGHLIGHT: [f32; 4] = [1.5, 1.5, 1.5, 1.0];

/// The localized texts the panel draws.
pub(crate) struct Labels {
    /// `#menu_enabledpacks`.
    pub allowed: String,
    /// `#menu_tome_03` (LOC_ITEMPACKS).
    pub items: String,
    /// `#menu_tome_02` (LOC_MAGICKPACKS).
    pub magicks: String,
    /// `#add_menu_ok`.
    pub ok: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OverviewHit {
    ScrollLeft,
    ScrollRight,
    /// The label or a thumb: open the list.
    Open,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Row {
    Heading(PackKind),
    /// Up to five AllPacks indices.
    Packs(Vec<usize>),
}

pub(crate) struct PackPanel {
    pub catalogue: PackCatalogue,
    /// IPack.Enabled per AllPacks index.
    pub enabled: Vec<bool>,
    /// IPack.License == Yes per AllPacks index.
    pub licensed: Vec<bool>,
    /// mPackScrollValue: the first enabled pack the overview shows.
    pub scroll: usize,
    /// State.ChangingPacks.
    pub open: bool,
    /// mSelectedPack while open: a pack, or None for the OK button.
    pub selected: Option<usize>,
    /// mSpecialScrollBar.Value while open.
    pub list_scroll: i32,
    /// DLC_StatusHelper: the list was closed with OK once, so no pack is
    /// "new" any more.
    pub seen: bool,
}

impl PackPanel {
    /// `License != Yes` clears Enabled and keeps it clear.
    pub(crate) fn new(
        catalogue: PackCatalogue,
        enabled: Vec<bool>,
        licensed: Vec<bool>,
        seen: bool,
    ) -> Self {
        let n = catalogue.packs.len();
        let licensed: Vec<bool> = (0..n)
            .map(|i| licensed.get(i).copied().unwrap_or(true))
            .collect();
        let enabled = (0..n)
            .map(|i| enabled.get(i).copied().unwrap_or(false) && licensed[i])
            .collect();
        Self {
            catalogue,
            enabled,
            licensed,
            scroll: 0,
            open: false,
            selected: None,
            list_scroll: 0,
            seen,
        }
    }

    fn len(&self) -> usize {
        self.catalogue.packs.len()
    }
    fn item_count(&self) -> usize {
        self.catalogue.item_count()
    }
    fn magick_count(&self) -> usize {
        self.catalogue.magick_count()
    }
    pub(crate) fn enabled_count(&self) -> usize {
        self.enabled.iter().filter(|e| **e).count()
    }
    /// The `--packs` launch spec of the enabled packs.
    pub(crate) fn spec(&self) -> String {
        self.catalogue.spec(&self.enabled)
    }

    // ----- the overview row (State.Normal) -----

    /// More than five enabled packs: the overview scrolls.
    pub(crate) fn has_arrows(&self) -> bool {
        self.enabled_count() > OVERVIEW_SLOTS
    }
    /// DrawPacksOverview: up to five enabled packs from the scroll value
    /// (reset to the first pack when they all fit), wrapping.
    pub(crate) fn overview_packs(&self) -> Vec<usize> {
        let count = self.enabled_count();
        if count == 0 {
            return Vec::new();
        }
        let mut i = if self.has_arrows() { self.scroll } else { 0 };
        let mut out = Vec::new();
        for _ in 0..count.min(OVERVIEW_SLOTS) {
            while !self.enabled[i] {
                i = (i + 1) % self.len();
            }
            out.push(i);
            i = (i + 1) % self.len();
        }
        out
    }
    /// ScrollPack: the next enabled pack in `dir`.
    pub(crate) fn scroll_pack(&mut self, dir: i32) {
        let n = self.len() as i32;
        if n == 0 {
            return;
        }
        let mut i = self.scroll as i32;
        loop {
            i = (i + dir).rem_euclid(n);
            if self.enabled[i as usize] || i as usize == self.scroll {
                break;
            }
        }
        self.scroll = i as usize;
    }
    /// HitPackOverview: the arrows, a shown thumb or the label.
    pub(crate) fn overview_hit(&self, p: [f32; 2], label_width: f32) -> Option<OverviewHit> {
        if (OVERVIEW_TOP..=OVERVIEW_TOP + CELL).contains(&p[1]) {
            if self.has_arrows() {
                if (ARROW_LEFT..=ARROW_LEFT + CELL).contains(&p[0]) {
                    return Some(OverviewHit::ScrollLeft);
                }
                if (ARROW_RIGHT..=ARROW_RIGHT + CELL).contains(&p[0]) {
                    return Some(OverviewHit::ScrollRight);
                }
            }
            let shown = self.overview_packs().len();
            return (0..shown)
                .any(|j| {
                    let x = OVERVIEW_LEFT + CELL * j as f32;
                    (x..=x + CELL).contains(&p[0])
                })
                .then_some(OverviewHit::Open);
        }
        let label = Item::new(
            LABEL,
            [LABEL[0] + label_width, LABEL[1] + LABEL_HEIGHT],
            true,
        );
        label.inside(p).then_some(OverviewHit::Open)
    }
    /// One thumbnail: the pack's cell when licensed (else the first cell),
    /// desaturated when disabled.
    fn thumb(&self, pack: usize, position: [f32; 2], highlighted: bool) -> Widget {
        let cell = if self.licensed[pack] {
            self.catalogue.packs[pack].thumb
        } else {
            [0, 0]
        };
        Widget::image(
            Atlas::Content(THUMBS.into()),
            [cell[0] as f32 * CELL, cell[1] as f32 * CELL, CELL, CELL],
            position,
            [CELL, CELL],
        )
        .color(if highlighted { HIGHLIGHT } else { [1.0; 4] })
        .saturation(if self.enabled[pack] { 1.0 } else { 0.0 })
    }
    /// The label (returned as the keyboard item), the arrows and the shown
    /// thumbs; `selected` is the lobby's position 5, which brightens the
    /// first shown pack (mSelectedPack = mPackScrollValue).
    pub(crate) fn overview_widgets(
        &self,
        labels: &Labels,
        selected: bool,
        out: &mut Vec<Widget>,
        measure: &mut Measure,
    ) -> Item {
        let item = menu_text_item(
            out,
            measure,
            Font::MenuOption,
            labels.allowed.clone(),
            LABEL,
            TextAlign::Left,
            selected,
            true,
        );
        if self.has_arrows() {
            let centre_y = OVERVIEW_TOP + CELL * 0.5;
            out.push(Widget::sprite(
                Atlas::Pages,
                ARROW,
                [32.0, 32.0],
                [0.0, -1.0, 1.0, 0.0],
                [ARROW_LEFT + CELL * 0.5, centre_y],
            ));
            out.push(Widget::sprite(
                Atlas::Pages,
                ARROW,
                [32.0, 32.0],
                [0.0, 1.0, -1.0, 0.0],
                [ARROW_RIGHT + CELL * 0.5, centre_y],
            ));
        }
        for (j, &pack) in self.overview_packs().iter().enumerate() {
            out.push(self.thumb(
                pack,
                [OVERVIEW_LEFT + CELL * j as f32, OVERVIEW_TOP],
                selected && j == 0,
            ));
        }
        item
    }

    // ----- the list (State.ChangingPacks) -----

    /// Enter ChangingPacks: the first pack, the list at its top.
    pub(crate) fn open(&mut self) {
        self.open = true;
        self.selected = Some(0);
        self.list_scroll = 0;
    }
    /// Leave the list; OK (click or confirm) marks every pack used.
    pub(crate) fn close(&mut self, ok: bool) {
        self.open = false;
        if ok {
            self.seen = true;
        }
    }
    fn rows(&self) -> Vec<Row> {
        let items = self.item_count();
        let mut rows = vec![Row::Heading(PackKind::Item)];
        rows.extend(
            (0..items)
                .collect::<Vec<_>>()
                .chunks(PER_ROW)
                .map(|c| Row::Packs(c.to_vec())),
        );
        rows.push(Row::Heading(PackKind::Magick));
        rows.extend(
            (items..self.len())
                .collect::<Vec<_>>()
                .chunks(PER_ROW)
                .map(|c| Row::Packs(c.to_vec())),
        );
        rows
    }
    /// 2 + ⌈items / 5⌉ + ⌈magicks / 5⌉.
    pub(crate) fn row_count(&self) -> usize {
        2 + self.item_count().div_ceil(PER_ROW) + self.magick_count().div_ceil(PER_ROW)
    }
    /// SetMaxValue(rows − 7).
    pub(crate) fn max_scroll(&self) -> i32 {
        (self.row_count() as i32 - LIST_ROWS as i32).max(0)
    }
    pub(crate) fn scroll_bar(&self) -> ScrollBar {
        ScrollBar {
            position: LIST_BAR,
            height: LIST_HEIGHT,
            max: self.max_scroll(),
        }
    }
    fn row_of(&self, pack: usize) -> usize {
        let items = self.item_count();
        if pack < items {
            1 + pack / PER_ROW
        } else {
            2 + items.div_ceil(PER_ROW) + (pack - items) / PER_ROW
        }
    }
    /// Scroll so the selected pack's row is visible.
    fn reveal(&mut self, pack: usize) {
        let row = self.row_of(pack) as i32;
        if row < self.list_scroll {
            self.list_scroll = row;
        } else if row >= self.list_scroll + LIST_ROWS as i32 {
            self.list_scroll = row - LIST_ROWS as i32 + 1;
        }
        self.list_scroll = self.list_scroll.clamp(0, self.max_scroll());
    }
    pub(crate) fn set_list_scroll(&mut self, value: i32) {
        self.list_scroll = value.clamp(0, self.max_scroll());
    }
    /// Toggle a pack (a pack without a licence stays disabled; natively the
    /// click opens its store page). Returns whether anything changed.
    pub(crate) fn toggle(&mut self, pack: usize) -> bool {
        if pack >= self.len() || !self.licensed[pack] {
            return false;
        }
        self.enabled[pack] = !self.enabled[pack];
        true
    }
    /// ControllerRight: the next pack of the same row, wrapping within it.
    pub(crate) fn move_right(&mut self) {
        let Some(sel) = self.selected else { return };
        let items = self.item_count();
        let (base, local, count) = if sel < items {
            (0, sel, items)
        } else {
            (items, sel - items, self.magick_count())
        };
        let mut n = local / PER_ROW * PER_ROW + (local + 1) % PER_ROW;
        if n >= count {
            n = local / PER_ROW * PER_ROW;
        }
        self.selected = Some(base + n);
    }
    /// ControllerLeft: the previous pack of the same row, wrapping within it.
    pub(crate) fn move_left(&mut self) {
        let Some(sel) = self.selected else { return };
        let items = self.item_count();
        let (base, local, count) = if sel < items {
            (0, sel, items)
        } else {
            (items, sel - items, self.magick_count())
        };
        let mut n = local / PER_ROW * PER_ROW + (local + PER_ROW - 1) % PER_ROW;
        if n >= count {
            n = count - 1;
        }
        self.selected = Some(base + n);
    }
    /// ControllerDown: the row below (items → magicks at the same column,
    /// the last magick row's end, or the OK button); OK wraps to the first
    /// pack.
    pub(crate) fn move_down(&mut self) {
        let items = self.item_count();
        let magicks = self.magick_count();
        let Some(sel) = self.selected else {
            self.selected = Some(0);
            self.list_scroll = 0;
            return;
        };
        let mut n = sel + PER_ROW;
        if n >= items + magicks {
            if (n - items) / PER_ROW * PER_ROW < magicks {
                self.list_scroll = self.max_scroll();
                n = items + magicks - 1;
            } else {
                self.selected = None;
                return;
            }
        } else if sel < items && n >= items {
            n = if n / PER_ROW * PER_ROW >= items {
                (items + n % PER_ROW).min(items + magicks - 1)
            } else {
                items - 1
            };
        }
        self.selected = Some(n);
        self.reveal(n);
    }
    /// ControllerUp: the row above (magicks → the last item row at the same
    /// column, or the OK button); OK goes to the last magick row's start.
    pub(crate) fn move_up(&mut self) {
        let items = self.item_count();
        let magicks = self.magick_count();
        let Some(sel) = self.selected else {
            let rows = magicks / PER_ROW - usize::from(magicks % PER_ROW == 0 && magicks > 0);
            let n = (items + rows * PER_ROW).min(items + magicks.saturating_sub(1));
            self.selected = Some(n);
            self.list_scroll = self.max_scroll();
            return;
        };
        if sel < PER_ROW {
            self.selected = None;
            return;
        }
        let mut n = sel - PER_ROW;
        if sel >= items && n < items {
            let rows = items / PER_ROW - usize::from(items % PER_ROW == 0 && items > 0);
            n = rows * PER_ROW + (sel - items) % PER_ROW;
            if n >= items {
                n = items - 1;
            }
        }
        self.selected = Some(n);
        self.reveal(n);
    }
    /// HitPackList: the thumb cell under `p`, if any.
    pub(crate) fn list_hit(&self, p: [f32; 2]) -> Option<usize> {
        if p[0] < LIST_LEFT || p[0] > LIST_LEFT + CELL * PER_ROW as f32 {
            return None;
        }
        let rows = self.rows();
        for (r, row) in rows
            .iter()
            .enumerate()
            .skip(self.list_scroll.max(0) as usize)
            .take(LIST_ROWS)
        {
            let top = LIST_TOP + CELL * (r as i32 - self.list_scroll) as f32;
            if !(top..=top + CELL).contains(&p[1]) {
                continue;
            }
            let Row::Packs(packs) = row else { return None };
            let col = ((p[0] - LIST_LEFT) / CELL) as usize;
            return packs.get(col).copied();
        }
        None
    }
    /// The scroll bar, the visible rows and the OK button (returned as its
    /// item).
    pub(crate) fn list_widgets(
        &self,
        labels: &Labels,
        out: &mut Vec<Widget>,
        measure: &mut Measure,
    ) -> Item {
        self.scroll_bar().draw(out, self.list_scroll);
        let rows = self.rows();
        for (r, row) in rows
            .iter()
            .enumerate()
            .skip(self.list_scroll.max(0) as usize)
            .take(LIST_ROWS)
        {
            let top = LIST_TOP + CELL * (r as i32 - self.list_scroll) as f32;
            match row {
                Row::Heading(kind) => {
                    let text = match kind {
                        PackKind::Item => labels.items.clone(),
                        PackKind::Magick => labels.magicks.clone(),
                    };
                    // Text.Draw at the row centre − 14.
                    out.push(Widget::text(
                        Font::MenuOption,
                        text,
                        [LIST_HEADING_X, top + CELL * 0.5 - 14.0],
                        TextAlign::Left,
                    ));
                }
                Row::Packs(packs) => {
                    for (col, &pack) in packs.iter().enumerate() {
                        let position = [LIST_LEFT + CELL * col as f32, top];
                        out.push(self.thumb(pack, position, self.selected == Some(pack)));
                        if !self.seen && self.licensed[pack] && self.enabled[pack] {
                            out.push(Widget::image(
                                Atlas::Content(THUMBS.into()),
                                [
                                    NEW_CELL[0] as f32 * CELL,
                                    NEW_CELL[1] as f32 * CELL,
                                    CELL,
                                    CELL,
                                ],
                                position,
                                [CELL, CELL],
                            ));
                        }
                    }
                }
            }
        }
        let size = measure.text(Font::MenuOption, &labels.ok);
        text_button(
            out,
            &labels.ok,
            size,
            OK,
            TextAlign::Right,
            OK_WIDTH,
            self.selected.is_none(),
            true,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use magicka_runtime::ruleset::Pack;

    /// PackMan's layout: thirteen item packs then six magick packs.
    fn catalogue(items: usize, magicks: usize) -> PackCatalogue {
        let pack = |kind, i: usize| Pack {
            kind,
            name: format!(
                "#{}_{i}",
                if kind == PackKind::Item {
                    "pack"
                } else {
                    "mpack"
                }
            ),
            app: 0,
            thumb: [i as u32 % 16, 0],
            items: if kind == PackKind::Item {
                vec![format!("item{i}")]
            } else {
                Vec::new()
            },
            magicks: if kind == PackKind::Magick {
                vec![i as u8]
            } else {
                Vec::new()
            },
            default_enabled: true,
        };
        PackCatalogue {
            packs: (0..items)
                .map(|i| pack(PackKind::Item, i))
                .chain((0..magicks).map(|i| pack(PackKind::Magick, i)))
                .collect(),
        }
    }
    fn panel(items: usize, magicks: usize) -> PackPanel {
        let n = items + magicks;
        PackPanel::new(
            catalogue(items, magicks),
            vec![true; n],
            vec![true; n],
            true,
        )
    }

    #[test]
    fn native_pack_counts_fill_seven_rows_without_scrolling() {
        let p = panel(13, 6);
        assert_eq!(p.row_count(), 7);
        assert_eq!(p.max_scroll(), 0);
        let rows = p.rows();
        assert_eq!(rows[0], Row::Heading(PackKind::Item));
        assert_eq!(rows[1], Row::Packs((0..5).collect()));
        assert_eq!(rows[3], Row::Packs((10..13).collect()));
        assert_eq!(rows[4], Row::Heading(PackKind::Magick));
        assert_eq!(rows[5], Row::Packs((13..18).collect()));
        assert_eq!(rows[6], Row::Packs(vec![18]));
        assert_eq!(panel(13, 11).max_scroll(), 1, "a third magick row scrolls");
    }

    #[test]
    fn overview_shows_five_enabled_packs_from_the_scroll_and_wraps() {
        let mut p = panel(13, 6);
        assert!(p.has_arrows());
        assert_eq!(p.overview_packs(), [0, 1, 2, 3, 4]);
        p.scroll_pack(1);
        assert_eq!(p.scroll, 1);
        assert_eq!(p.overview_packs(), [1, 2, 3, 4, 5]);
        p.enabled[2] = false;
        assert_eq!(
            p.overview_packs(),
            [1, 3, 4, 5, 6],
            "disabled packs are skipped"
        );
        p.scroll_pack(1);
        assert_eq!(p.scroll, 3, "ScrollPack lands on an enabled pack");
        p.scroll_pack(-1);
        p.scroll_pack(-1);
        assert_eq!(p.scroll, 0);
        p.scroll_pack(-1);
        assert_eq!(p.scroll, 18, "wraps to the last pack");
        p.enabled = vec![false; 19];
        p.enabled[7] = true;
        p.enabled[15] = true;
        assert!(!p.has_arrows());
        assert_eq!(p.overview_packs(), [7, 15], "all fit: shown from the first");
        p.enabled = vec![false; 19];
        assert_eq!(p.overview_packs(), Vec::<usize>::new());
    }

    #[test]
    fn overview_hits_arrows_thumbs_and_the_label() {
        let mut p = panel(13, 6);
        assert_eq!(
            p.overview_hit([70.0, 440.0], 200.0),
            Some(OverviewHit::ScrollLeft)
        );
        assert_eq!(
            p.overview_hit([470.0, 440.0], 200.0),
            Some(OverviewHit::ScrollRight)
        );
        assert_eq!(
            p.overview_hit([130.0, 440.0], 200.0),
            Some(OverviewHit::Open)
        );
        assert_eq!(
            p.overview_hit([440.0, 440.0], 200.0),
            Some(OverviewHit::Open)
        );
        assert_eq!(
            p.overview_hit([300.0, 380.0], 200.0),
            Some(OverviewHit::Open),
            "label"
        );
        assert_eq!(
            p.overview_hit([400.0, 380.0], 200.0),
            None,
            "past the label"
        );
        assert_eq!(p.overview_hit([300.0, 500.0], 200.0), None);
        p.enabled = vec![false; 19];
        p.enabled[0] = true;
        p.enabled[1] = true;
        p.enabled[2] = true;
        assert_eq!(
            p.overview_hit([70.0, 440.0], 200.0),
            None,
            "no arrows when all fit"
        );
        assert_eq!(
            p.overview_hit([330.0, 440.0], 200.0),
            None,
            "empty fourth slot"
        );
        assert_eq!(
            p.overview_hit([250.0, 440.0], 200.0),
            Some(OverviewHit::Open)
        );
    }

    #[test]
    fn list_navigation_follows_the_native_row_rules() {
        let mut p = panel(13, 6);
        p.open();
        assert_eq!(p.selected, Some(0));
        for _ in 0..4 {
            p.move_right();
        }
        assert_eq!(p.selected, Some(4));
        p.move_right();
        assert_eq!(p.selected, Some(0), "wraps within the row");
        p.selected = Some(12);
        p.move_right();
        assert_eq!(p.selected, Some(10), "a short row wraps to its start");
        p.move_left();
        assert_eq!(p.selected, Some(12), "and back to its last pack");
        p.selected = Some(0);
        p.move_down();
        p.move_down();
        assert_eq!(p.selected, Some(10));
        p.move_down();
        assert_eq!(p.selected, Some(13), "into the magicks at the same column");
        p.selected = Some(12);
        p.move_down();
        assert_eq!(p.selected, Some(15));
        p.selected = Some(13);
        p.move_down();
        assert_eq!(p.selected, Some(18), "the second magick row");
        p.move_down();
        assert_eq!(p.selected, None, "then the OK button");
        p.move_down();
        assert_eq!(p.selected, Some(0), "OK wraps to the first pack");
        p.selected = Some(14);
        p.move_down();
        assert_eq!(
            p.selected,
            Some(18),
            "a column past the last row ends on the last pack"
        );
        p.selected = None;
        p.move_up();
        assert_eq!(
            p.selected,
            Some(18),
            "OK goes to the last magick row's start"
        );
        p.move_up();
        assert_eq!(p.selected, Some(13));
        p.move_up();
        assert_eq!(
            p.selected,
            Some(10),
            "into the last item row at the same column"
        );
        p.selected = Some(17);
        p.move_up();
        assert_eq!(p.selected, Some(12), "clamped to the last item pack");
        p.selected = Some(3);
        p.move_up();
        assert_eq!(p.selected, None, "above the first row is the OK button");
    }

    #[test]
    fn list_hits_map_cells_to_packs() {
        let p = panel(13, 6);
        assert_eq!(p.list_hit([130.0, 100.0]), None, "the Items heading");
        assert_eq!(p.list_hit([130.0, 170.0]), Some(0));
        assert_eq!(p.list_hit([400.0, 170.0]), Some(4));
        assert_eq!(p.list_hit([200.0, 300.0]), Some(11));
        assert_eq!(
            p.list_hit([400.0, 300.0]),
            None,
            "the short row has no fifth pack"
        );
        assert_eq!(p.list_hit([130.0, 380.0]), None, "the Magicks heading");
        assert_eq!(p.list_hit([130.0, 430.0]), Some(13));
        assert_eq!(p.list_hit([130.0, 500.0]), Some(18));
        assert_eq!(p.list_hit([460.0, 170.0]), None, "right of the cells");
        assert_eq!(p.list_hit([130.0, 560.0]), None, "below the rows");
    }

    #[test]
    fn toggles_respect_licences_and_build_the_launch_spec() {
        let mut licensed = vec![true; 19];
        licensed[6] = false;
        let mut p = PackPanel::new(catalogue(13, 6), vec![true; 19], licensed, false);
        assert!(!p.enabled[6], "an unlicensed pack is forced off");
        assert!(!p.toggle(6), "and cannot be enabled");
        assert!(p.toggle(0));
        assert!(!p.enabled[0]);
        assert!(p.toggle(0));
        assert!(p.toggle(18));
        assert_eq!(p.enabled_count(), 17);
        let spec = p.spec();
        assert!(spec.starts_with("pack_0,pack_1,"));
        assert!(!spec.contains("pack_6,"));
        assert!(!spec.contains("mpack_5"));
        assert_eq!(p.catalogue.selection(&spec), p.enabled);
        p.open();
        p.close(false);
        assert!(!p.seen, "Back leaves the new-pack marks");
        p.close(true);
        assert!(p.seen, "OK marks every pack used");
    }
}
