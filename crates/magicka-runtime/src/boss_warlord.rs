//! Warlord wrapper state; character damage behavior lives in character_impact.
// Character::.ctor creates Capsule(radius=4.5, length=1). Warlord.Initialize
// reads it before ApplyTemplate, then adds 0.1 to its ground placement.
// Evidence: research/boss-warlord.md, constructor-placement-il.txt.
pub(crate) const FRESH_SPAWN_HEIGHT: f32 = 4.5 + 1.0 * 0.5 + 0.1;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Intro,
    Battle,
    Death,
}
#[derive(Clone, Debug, PartialEq)]
pub struct WarlordBoss {
    pub handle: u16,
    pub phase: Phase,
    pub taunt_timer: f32,
}
impl WarlordBoss {
    pub fn new(handle: u16) -> Self {
        Self {
            handle,
            phase: Phase::Intro,
            taunt_timer: 0.,
        }
    }
}
