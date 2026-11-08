//! MainMenu tile layout, controller map and pointer ordering. Static provenance:
//! research/main-menu.md. Coordinates refer to a 1024-square right page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum TileId {
    Store,
    Adventure,
    Dlc,
    Challenge,
    Versus,
    Online,
    Exit,
    Facebook,
    Twitter,
    Leaderboards,
    Options,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Atlas {
    Store,
    Pages,
    PagesNew,
}
#[derive(Clone, Copy, Debug)]
pub struct Tile {
    pub id: TileId,
    pub atlas: Atlas,
    pub source: [f32; 4],
    pub position: [f32; 2],
    pub label: Option<&'static str>,
    pub label_offset: [f32; 2],
}
impl TileId {
    /// The widget links from MainMenu.CreateControllerMap. Missing edges hold
    /// focus; they do not wrap. This graph is independent of enum adjacency.
    pub fn neighbor(self, direction: Direction) -> Self {
        use Direction::*;
        use TileId::*;
        match (self, direction) {
            (Store, Down) => Adventure,
            (Adventure, Up) | (Dlc, Up) => Store,
            (Adventure, Right) => Dlc,
            (Adventure, Down) => Challenge,
            (Dlc, Left) => Adventure,
            (Dlc, Down) => Online,
            (Challenge, Up) | (Versus, Up) => Adventure,
            (Challenge, Right) => Versus,
            (Challenge, Down) => Exit,
            (Versus, Left) => Challenge,
            (Versus, Right) => Online,
            (Versus, Down) => Facebook,
            (Online, Left) => Versus,
            (Online, Up) => Dlc,
            (Online, Down) => Leaderboards,
            (Exit, Up) => Challenge,
            (Exit, Right) => Facebook,
            (Facebook, Left) => Exit,
            (Facebook, Up) | (Twitter, Up) => Versus,
            (Facebook, Right) => Twitter,
            (Twitter, Left) => Facebook,
            (Twitter, Right) => Leaderboards,
            (Leaderboards, Left) => Twitter,
            (Leaderboards, Up) | (Options, Up) => Online,
            (Leaderboards, Right) => Options,
            (Options, Left) => Leaderboards,
            _ => self,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Store => "Store",
            Self::Adventure => "Adventure",
            Self::Dlc => "DLC",
            Self::Challenge => "Challenge",
            Self::Versus => "Versus",
            Self::Online => "Online",
            Self::Exit => "Exit",
            Self::Facebook => "Facebook",
            Self::Twitter => "Twitter",
            Self::Leaderboards => "Leaderboards",
            Self::Options => "Options",
        }
    }
}

/// Creation order is draw order, with the back/exit tile created last. The
/// store takes its size from the original texture rather than an assumed size.
pub fn tiles(store_size: [f32; 2]) -> Vec<Tile> {
    use Atlas::{Pages, PagesNew};
    use TileId::*;
    let definitions = [
        (
            Store,
            Atlas::Store,
            [0.0, 0.0, store_size[0], store_size[1]],
            [166.0, 54.0],
            None,
        ),
        (
            Adventure,
            PagesNew,
            [1.0, 1.0, 388.0, 226.0],
            [166.0, 254.0],
            Some("#menu_main_01"),
        ),
        (
            Dlc,
            PagesNew,
            [1.0, 229.0, 360.0, 226.0],
            [577.0, 254.0],
            Some("#store_dlc"),
        ),
        (
            Challenge,
            Pages,
            [1040.0, 416.0, 336.0, 400.0],
            [163.0, 500.0],
            Some("#menu_main_02"),
        ),
        (
            Versus,
            Pages,
            [1376.0, 416.0, 336.0, 400.0],
            [387.0, 500.0],
            Some("#menu_main_03"),
        ),
        (
            Online,
            Pages,
            [1712.0, 416.0, 336.0, 400.0],
            [611.0, 500.0],
            Some("#menu_main_05"),
        ),
        (
            Facebook,
            Pages,
            [1070.0, 830.0, 104.0, 66.0],
            [453.0, 912.0],
            None,
        ),
        (
            Twitter,
            Pages,
            [1174.0, 830.0, 104.0, 66.0],
            [593.0, 912.0],
            None,
        ),
        (
            Leaderboards,
            PagesNew,
            [391.0, 38.0, 71.0, 71.0],
            [768.0, 912.0],
            None,
        ),
        (
            Options,
            PagesNew,
            [391.0, 111.0, 71.0, 71.0],
            [851.0, 912.0],
            None,
        ),
        (
            Exit,
            Pages,
            [1696.0, 48.0, 320.0, 96.0],
            [96.0, 912.0],
            None,
        ),
    ];
    definitions
        .into_iter()
        .map(|(id, atlas, source, position, label)| Tile {
            id,
            atlas,
            source,
            position,
            label,
            label_offset: if id == Online {
                [20.0, -40.0]
            } else {
                [0.0, -40.0]
            },
        })
        .collect()
}

/// InputWidget visits children in reverse creation order. Rectangles overlap
/// on the third row, so a nearest-center approximation changes pointer input.
pub fn hit_test(tiles: &[Tile], point: [f32; 2]) -> Option<TileId> {
    tiles
        .iter()
        .rev()
        .find(|tile| {
            (0..2).all(|axis| {
                point[axis] >= tile.position[axis]
                    && point[axis] < tile.position[axis] + tile.source[axis + 2]
            })
        })
        .map(|t| t.id)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MenuCursor {
    pub attached: bool,
    pub selected: Option<TileId>,
}
impl MenuCursor {
    fn attach(&mut self) -> bool {
        if self.attached {
            return false;
        }
        self.attached = true;
        self.selected = Some(TileId::Store);
        true
    }
    pub fn navigate(&mut self, direction: Direction) {
        if !self.attach() {
            self.selected = self.selected.map(|id| id.neighbor(direction));
        }
    }
    /// First Select attaches the cursor without activating the store.
    pub fn select(&mut self) -> Option<TileId> {
        if self.attach() {
            None
        } else {
            self.selected
        }
    }
    /// First Back attaches; a subsequent Back opens the quit question.
    pub fn back(&mut self) -> bool {
        !self.attach()
    }
    pub fn pointer(&mut self, hovered: Option<TileId>) {
        self.attached = false;
        self.selected = hovered;
    }
}
