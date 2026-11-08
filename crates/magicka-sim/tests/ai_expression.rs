//! Magicka.AI.Arithmetics parsing and evaluation (research/npc-combat.md).
use magicka_sim::ai_expression::*;

struct Host {
    hp: (f32, f32),
    status: u32,
    friends: u32,
    resistance: f32,
}
impl ExpressionHost for Host {
    fn has_status(&mut self, _: Subject, s: u32) -> Option<bool> {
        Some(self.status & s == s)
    }
    fn gripped(&mut self, _: Subject) -> Option<bool> {
        Some(false)
    }
    fn gripping(&mut self, _: Subject) -> Option<bool> {
        Some(false)
    }
    fn entangled(&mut self, _: Subject) -> Option<bool> {
        Some(false)
    }
    fn speed(&mut self, _: Subject) -> Option<f32> {
        Some(0.0)
    }
    fn health(&mut self, _: Subject) -> (f32, f32) {
        self.hp
    }
    fn density_count(&mut self, _: Subject, friendly: bool, _: f32) -> u32 {
        if friendly {
            self.friends
        } else {
            0
        }
    }
    fn danger(&mut self, _: u32) -> f32 {
        0.25
    }
    fn threat(&mut self, _: u32) -> Option<f32> {
        Some(0.5)
    }
    fn line_of_sight(&mut self, _: Vec3, _: Vec3, _: f32) -> bool {
        true
    }
    fn scene_hit(&mut self, _: Vec3, _: Vec3) -> bool {
        false
    }
    fn is_character(&mut self) -> bool {
        true
    }
    fn ethereal(&mut self, _: Subject) -> Option<bool> {
        Some(false)
    }
    fn resistance_against(&mut self, _: Subject, _: u32) -> Option<f32> {
        Some(self.resistance)
    }
    fn self_shielded(&mut self, _: Subject) -> Option<bool> {
        Some(false)
    }
    fn is_shield(&mut self) -> bool {
        false
    }
    fn is_barrier(&mut self) -> bool {
        false
    }
    fn grip_damage(&mut self) -> Option<(f32, f32)> {
        None
    }
    fn on_ground(&mut self, _: Subject) -> Option<bool> {
        Some(true)
    }
}

fn eval(text: &str, host: &mut Host, distance: f32) -> f32 {
    let args = ExpressionArgs::new(
        [0.0; 3],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, -distance],
        [0.0, 0.0, 1.0],
    );
    Expr::read(text).unwrap().value(&args, host).unwrap()
}

fn host() -> Host {
    Host {
        hp: (30.0, 120.0),
        status: 0x2 | 0x8,
        friends: 2,
        resistance: 0.25,
    }
}

#[test]
fn precedence_parentheses_and_not() {
    let h = &mut host();
    assert_eq!(eval("1 + 2 * 3", h, 1.0), 7.0);
    assert_eq!(eval("(1 + 2) * 3", h, 1.0), 9.0);
    assert_eq!(eval("2 ^ 3 * 2", h, 1.0), 16.0);
    // `!` binds tighter than `-`: (!0.25) - 0.5.
    assert_eq!(eval("(!s.health-0.5)*10", h, 1.0), 2.5);
    assert_eq!(eval("!t.health", h, 1.0), 0.75);
    assert_eq!(eval("10", h, 1.0), 10.0);
}

#[test]
fn equal_priority_order_follows_the_clr_quicksort() {
    let h = &mut host();
    // Two operators: the quicksort swaps equal keys, so the later one is
    // the root and the expression is left-associative.
    assert_eq!(eval("8 - 4 - 2", h, 1.0), 2.0);
    assert_eq!(eval("8 / 4 / 2", h, 1.0), 1.0);
    // Three equal keys [A, B, C]: no median swaps, the partition swaps A
    // and C, leaving [C, B, A]; the last operator is the root and the
    // next one splits its left side: ((8 - 4) - 2) - 1.
    assert_eq!(eval("8 - 4 - 2 - 1", h, 1.0), 1.0);
    // Mixed + and − share a priority, so the same order applies.
    assert_eq!(eval("8 - 4 + 2", h, 1.0), 6.0);
}

#[test]
fn variables_scopes_aliases_and_function_arguments() {
    let h = &mut host();
    assert_eq!(eval("s.status(wet) + s.status(cold)", h, 1.0), 2.0);
    assert_eq!(
        eval("(s.status(wet)+s.status(cold)+s.status(frozen))*2", h, 1.0),
        4.0
    );
    assert_eq!(eval("s.HasStatus(Wet)", h, 1.0), 1.0);
    // Danger takes a `|`-separated element list; the `/` in function
    // arguments is not an operator.
    assert_eq!(eval("t.danger(offensive|defensive) * 2", h, 1.0), 0.5);
    assert_eq!(
        eval("!s.resistance(fire)+FDensity(5)", h, 1.0),
        1.25 - 1.0 + 0.75
    );
    // DistExp(max) is symmetric about zero: exactly 0.1 at the max.
    assert!((eval("distExp(6)", h, 6.0) - 0.1).abs() < 1e-6);
    assert!((eval("distExp(2,10)", h, 6.0) - 1.0).abs() < 1e-6);
    assert!((eval("DistLin(5)", h, 0.0) - 1.0).abs() < 1e-6);
    assert!(Expr::read("bogus(3)").is_err());
    assert!(Expr::read("distExp(ln(3))").is_err());
}
