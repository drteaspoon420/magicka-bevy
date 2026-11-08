//! Grimnir's first encounter: six dreams, not the later Grimnir2 caster.
//! Native state/timing provenance: research/boss-grimnir.md.
pub const MODEL: &str = "Models/Bosses/grimnir/grimnir_mesh";
pub const ANIMATION: &str = "Models/Bosses/grimnir/grimnir_animation";
pub const VENUES: [char; 6] = ['a', 'c', 'f', 'd', 'e', 'b'];
pub const MINIONS: [&str; 6] = [
    "goblin_shaman",
    "druid",
    "warlock",
    "goblin_warlock",
    "necromancer",
    "dwarf_mage",
];
pub const COUNTS: [usize; 6] = [4, 2, 2, 2, 2, 1];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    IntroFade,
    Action,
    FadeTransition,
    OutroFade,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Transition {
    Wait,
    Out,
    In,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Spawn(usize),
    FadeOut,
    Teleport(usize),
    FadeIn,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Dreams {
    pub state: State,
    pub venue: usize,
    pub health: f32,
    next: usize,
    timer: f32,
    spawn_delay: f32,
    spawned: bool,
    transition: Transition,
}
impl Default for Dreams {
    fn default() -> Self {
        Self {
            state: State::IntroFade,
            venue: 0,
            health: 6.,
            next: 0,
            timer: 0.25,
            spawn_delay: 0.,
            spawned: false,
            transition: Transition::Wait,
        }
    }
}
impl Dreams {
    fn action(&mut self) {
        self.venue = self.next;
        self.state = State::Action;
        self.spawn_delay = 1.;
        self.spawned = false;
    }
    pub fn corporealize(&mut self) {
        self.health = 0.;
        self.state = State::OutroFade;
    }
    /// Pass the retained list length before this update's Dead pruning.
    /// Native does not consume excess dt across state/transition boundaries.
    pub fn update(&mut self, dt: f32, minions: usize) -> Vec<Command> {
        let mut out = Vec::new();
        match self.state {
            State::IntroFade => {
                self.timer -= dt;
                if self.timer - 0.2 <= 0. {
                    self.action();
                }
            }
            State::Action => {
                self.spawn_delay -= dt;
                if !self.spawned && self.spawn_delay <= 0. {
                    out.push(Command::Spawn(self.venue));
                    self.spawned = true;
                } else if self.spawned && minions == 0 {
                    self.health -= 1.;
                    if self.health <= 0. || self.venue == 5 {
                        self.state = State::OutroFade;
                    } else {
                        self.state = State::FadeTransition;
                        self.transition = Transition::Wait;
                        self.timer = 1.5;
                        self.next += 1;
                    }
                }
            }
            State::FadeTransition => match self.transition {
                Transition::Wait => {
                    self.timer -= dt;
                    if self.timer <= 0. {
                        out.push(Command::FadeOut);
                        self.transition = Transition::Out;
                        self.timer = 0.25;
                    }
                }
                Transition::Out => {
                    self.timer -= dt;
                    if self.timer - 0.2 <= 0. {
                        out.extend([Command::Teleport(self.next), Command::FadeIn]);
                        self.transition = Transition::In;
                        self.timer = 0.25;
                    }
                }
                Transition::In => {
                    self.timer -= dt;
                    if self.timer - 0.2 <= 0. {
                        self.action();
                    }
                }
            },
            State::OutroFade => {}
        }
        out
    }
}

/// Spawn visits the selected locator then walks backwards around four slots.
pub fn spawn_slots(venue: usize, start: usize) -> impl Iterator<Item = usize> {
    (0..COUNTS[venue]).map(move |i| (start + 4 - i) % 4)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_steps_preserve_spawn_and_transition_frame_boundaries() {
        let mut d = Dreams::default();
        assert!(d.update(10., 0).is_empty());
        assert_eq!(d.state, State::Action);
        assert_eq!(d.update(10., 0), [Command::Spawn(0)]);
        assert_eq!(d.health, 6.);
        assert!(d.update(10., 4).is_empty());
        assert_eq!(d.health, 6.);
        assert!(d.update(0., 0).is_empty());
        assert_eq!(d.health, 5.);
        assert_eq!(d.update(10., 0), [Command::FadeOut]);
        assert_eq!(d.update(10., 0), [Command::Teleport(1), Command::FadeIn]);
        assert!(d.update(10., 0).is_empty());
        assert_eq!(d.venue, 1);
        assert_eq!(d.state, State::Action);
        assert_eq!(d.health, 5.);
    }
    #[test]
    fn every_venue_requires_a_wave_and_final_outro_stays_inert() {
        let mut d = Dreams::default();
        d.update(0.1, 0);
        for venue in 0..6 {
            assert_eq!(d.update(1., 0), [Command::Spawn(venue)]);
            for _ in 0..30 {
                d.update(1., COUNTS[venue]);
            }
            assert_eq!(d.health, (6 - venue) as f32);
            d.update(0., 0);
            assert_eq!(d.health, (5 - venue) as f32);
            if venue < 5 {
                assert_eq!(d.update(1.5, 0), [Command::FadeOut]);
                assert_eq!(
                    d.update(0.1, 0),
                    [Command::Teleport(venue + 1), Command::FadeIn]
                );
                d.update(0.1, 0);
            }
        }
        assert_eq!(d.state, State::OutroFade);
        assert!(d.update(100., 0).is_empty());
        assert_eq!(d.health, 0.);
        assert_eq!(spawn_slots(0, 1).collect::<Vec<_>>(), [1, 0, 3, 2]);
        assert_eq!(spawn_slots(5, 3).collect::<Vec<_>>(), [3]);
    }
}
