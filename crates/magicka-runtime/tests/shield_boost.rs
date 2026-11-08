use magicka_runtime::{
    avatar::{AvatarState, PlayerInput},
    play_session::PlaySession,
};
use magicka_sim::{
    character_health::{SelfShield, SelfShieldType},
    shield_entity::{ShieldKind, SHIELD_COLOR},
};
use std::path::{Path, PathBuf};
const DT: f32 = 1.0 / 60.0;

fn session() -> Option<PlaySession> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let index = s.avatar().body;
    s.world.physics.bodies[index].position = [0.0, 100.0, 0.0];
    s.world.physics.bodies[index].apply_gravity = false;
    for _ in 0..20 {
        s.step(DT);
    }
    Some(s)
}
fn spawn(s: &mut PlaySession, hp: f32) {
    let d = s.body().direction();
    let p = s.body().position;
    s.world.spawn_shield(
        s.avatar().handle,
        std::array::from_fn(|i| p[i] + d[i] * 8.0),
        2.0,
        d,
        ShieldKind::Sphere,
        hp,
        SHIELD_COLOR,
    );
}
fn press(s: &mut PlaySession, down: bool) {
    s.handle_input(
        PlayerInput {
            boost_down: down,
            ..Default::default()
        },
        16.0 / 9.0,
    );
}
fn hp(s: &PlaySession) -> f32 {
    s.world.shields[0].shield.hit_points
}

#[test]
fn live_space_press_heals_once_plays_boost_and_represses_during_cooldown() {
    let (Some(mut s), Some(mut control)) = (session(), session()) else {
        return;
    };
    spawn(&mut s, 1000.0);
    spawn(&mut control, 1000.0);
    press(&mut s, true);
    assert_eq!(s.avatar().boost.count, 1);
    for _ in 0..2 {
        s.step(DT);
        control.step(DT);
    }
    assert_eq!(s.avatar().motion.state, AvatarState::Boost);
    assert_eq!(s.avatar().motion.controller.current_animation, 36);
    assert_eq!(hp(&s) - hp(&control), 100.0);
    for _ in 0..5 {
        press(&mut s, true);
        s.step(DT);
        control.step(DT);
    }
    assert_eq!(hp(&s) - hp(&control), 100.0, "held Space is not repeated");
    press(&mut s, false);
    press(&mut s, true);
    s.step(DT);
    control.step(DT);
    assert_eq!(hp(&s) - hp(&control), 200.0);
    for _ in 0..20 {
        s.step(DT);
        control.step(DT);
    }
    assert_eq!(s.avatar().motion.state, AvatarState::Idle);
}

#[test]
fn live_boost_prioritizes_self_shield_and_permanent_shields_receive_half_healing() {
    for time_sensitive in [true, false] {
        let (Some(mut s), Some(mut control)) = (session(), session()) else {
            return;
        };
        for game in [&mut s, &mut control] {
            spawn(game, 1000.0);
            game.world.players[0]
                .as_mut()
                .unwrap()
                .character
                .combat
                .health
                .shield = SelfShield {
                kind: SelfShieldType::Shield,
                health: 100.0,
                max_health: 500.0,
                time_sensitive,
                time_since_damage: 9.0,
                ..Default::default()
            };
        }
        press(&mut s, true);
        for _ in 0..2 {
            s.step(DT);
            control.step(DT);
        }
        assert_eq!(
            hp(&s),
            hp(&control),
            "self shield takes priority over facing shield"
        );
        let a = &s.avatar().character.combat.health.shield;
        let b = &control.avatar().character.combat.health.shield;
        assert!((a.health - b.health - if time_sensitive { 50.0 } else { 25.0 }).abs() < 0.0001);
        assert!(a.time_since_damage < 2.0 * DT);
    }
}

#[test]
fn input_lock_charge_and_missing_target_reject_boost_without_deferred_held_press() {
    let Some(mut s) = session() else { return };
    press(&mut s, true);
    assert_eq!(s.avatar().boost.count, 0);
    spawn(&mut s, 1000.0);
    press(&mut s, false);
    s.world.flags.input_limited = true;
    press(&mut s, true);
    assert_eq!(s.avatar().boost.count, 0);
    s.world.flags.input_limited = false;
    press(&mut s, true);
    assert_eq!(s.avatar().boost.count, 0);
    press(&mut s, false);
    s.world.players[0].as_mut().unwrap().motion.state = AvatarState::Charge;
    press(&mut s, true);
    assert_eq!(s.avatar().boost.count, 0);
}

#[test]
fn lost_target_is_rechecked_before_entering_boost_state_and_capacity_clamps() {
    let Some(mut s) = session() else { return };
    spawn(&mut s, 4995.0);
    press(&mut s, true);
    for _ in 0..2 {
        s.step(DT);
    }
    assert_eq!(hp(&s), 5000.0);
    for _ in 0..20 {
        s.step(DT);
    }
    press(&mut s, false);
    press(&mut s, true);
    s.world.shields[0].shield.hit_points = 0.0;
    s.step(DT);
    assert_eq!(s.avatar().boost.count, 0);
    assert_eq!(s.avatar().motion.state, AvatarState::Idle);
}

#[test]
fn passive_reset_and_integer_multiplier_order_match_native_boost_queue() {
    let mut b = magicka_runtime::shield_boost::ShieldBoost::default();
    b.press(false, true, [2.9, 3.1]);
    assert_eq!(
        b.count, 6,
        "first press resets passive bonus before multiplying"
    );
    b.count = 0;
    b.press(true, true, [2.9, 3.1]);
    assert_eq!(b.count, 12);
    assert_eq!(b.cooldown, 0.25);
}

#[test]
fn recognized_magick_queue_takes_priority_over_a_nearby_shield() {
    // Avatar.Boost: CombineMagick wins only for an allowed (unlocked) magick.
    let Some(mut s) = session() else { return };
    spawn(&mut s, 1000.0);
    use magicka_sim::spells::{Elements as E, MagickType, Spell};
    s.unlocked_magicks[0] |= 1 << MagickType::Haste as u8;
    s.world.players[0].as_mut().unwrap().conjure.queue = [E::LIGHTNING, E::ARCANE, E::FIRE]
        .map(Spell::default_for)
        .to_vec();
    press(&mut s, true);
    assert_eq!(s.avatar().boost.count, 0);
    // CombineMagick, then the CastType setter's CombineSpell: the queue
    // moves into the prepared magick spell and CastType becomes Magick.
    let avatar = s.avatar();
    assert!(avatar.conjure.queue.is_empty());
    assert_eq!(
        (avatar.spell.element, avatar.spell.magick),
        (E::ALL, MagickType::Haste)
    );
    assert_eq!(avatar.cast.cast_type, magicka_sim::damage::CastType::Magick);
}

#[test]
fn locked_magick_queue_falls_through_to_shield_boost() {
    let Some(mut s) = session() else { return };
    spawn(&mut s, 1000.0);
    use magicka_sim::spells::{Elements as E, Spell};
    s.world.players[0].as_mut().unwrap().conjure.queue = [E::LIGHTNING, E::ARCANE, E::FIRE]
        .map(Spell::default_for)
        .to_vec();
    press(&mut s, true);
    assert_eq!(
        s.avatar().boost.count,
        1,
        "CombineMagick fails, so Space boosts"
    );
    assert_eq!(s.avatar().conjure.queue.len(), 3);
}

#[test]
fn live_staff_passive_and_boost_aura_apply_after_the_first_press_reset() {
    let (Some(mut s), Some(mut control)) = (session(), session()) else {
        return;
    };
    spawn(&mut s, 1000.0);
    spawn(&mut control, 1000.0);
    let (_, staff) = s.world.players[0]
        .as_mut()
        .unwrap()
        .items
        .iter_mut()
        .find(|(slot, _)| *slot == 1)
        .unwrap();
    staff.definition.passive_ability = 1;
    staff
        .definition
        .auras
        .push(magicka_content::common::AuraStorage {
            target: 0,
            visual_category: 0,
            color: [1.0; 3],
            effect: String::new(),
            ttl: f32::INFINITY,
            radius: 0.0,
            target_types: String::new(),
            target_faction: 0,
            kind: magicka_content::common::Aura::Boost { magnitude: 2.9 },
        });
    press(&mut s, true);
    assert_eq!(s.avatar().boost.count, 2);
    for _ in 0..2 {
        s.step(DT);
        control.step(DT);
    }
    assert_eq!(hp(&s) - hp(&control), 200.0);
    press(&mut s, false);
    press(&mut s, true);
    assert_eq!(s.avatar().boost.count, 4);
    s.step(DT);
    control.step(DT);
    assert_eq!(hp(&s) - hp(&control), 600.0);
}
