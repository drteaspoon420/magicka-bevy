//! Character-backed Vlad encounter rules (research/boss-vlad.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Swamp,
    Diplomat,
    Spirit,
}
impl Kind {
    pub fn from_class(class: &str) -> Option<Self> {
        match class {
            "vlad" => Some(Self::Swamp),
            "vlad2" => Some(Self::Diplomat),
            "vlad3" => Some(Self::Spirit),
            _ => None,
        }
    }
    pub fn template(self) -> &'static str {
        match self {
            Self::Swamp => "boss_vlad_swamp",
            Self::Diplomat => "boss_vlad_diplomat",
            Self::Spirit => "boss_vlad_spirit",
        }
    }
    pub fn equipment_visible(self) -> [bool; 2] {
        [self != Self::Swamp, false]
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Intro,
    Battle,
    Outro,
}
#[derive(Clone, Debug, PartialEq)]
pub struct VladBoss {
    pub handle: u16,
    pub kind: Kind,
    pub phase: Phase,
}
impl VladBoss {
    pub fn new(handle: u16, kind: Kind) -> Self {
        Self {
            handle,
            kind,
            phase: Phase::Intro,
        }
    }
    /// All three native mDead fields are initialized false and never set true.
    /// Scripts use health thresholds and explicit messages to progress.
    pub fn dead(&self) -> bool {
        false
    }
}
