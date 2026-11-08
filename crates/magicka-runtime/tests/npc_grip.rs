//! Hold grips: an orc's dash grabs the player, grip-attacks it and is
//! broken free by Space. See research/npc-grip.md.
use magicka_runtime::{
    animation_controller::animation_value, avatar::PlayerInput, character_motion::CharacterState,
    npc_combat::AbilityBody, play_session::PlaySession,
};
use std::path::Path;

fn session() -> Option<PlaySession> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    s.world.flags.cutscene = false;
    s.world.flags.input_limited = false;
    for _ in 0..60 {
        s.step(1. / 60.);
        s.handle_input(PlayerInput::default(), 16. / 9.);
    }
    Some(s)
}

/// An Orc_regular whose only usable abilities are its Dash and DamageGrip.
fn grappling_orc(s: &mut PlaySession) -> u16 {
    let h = s.spawn_npc("Orc_regular", "attack", 4.).unwrap();
    let n = s.world.npcs.get_mut(&h).unwrap();
    for i in 0..n.combat.abilities.len() {
        if !matches!(
            n.combat.abilities[i].body,
            AbilityBody::Dash { .. } | AbilityBody::DamageGrip
        ) {
            n.combat.cooldowns[i] = 1000.;
        }
    }
    h
}

/// Step until the orc holds the player; returns the frame.
fn wait_for_grip(s: &mut PlaySession, h: u16) -> usize {
    for frame in 0..900 {
        s.step(1. / 60.);
        s.handle_input(PlayerInput::default(), 16. / 9.);
        if s.world.npcs[&h].character.grip.gripped.is_some() {
            return frame;
        }
    }
    panic!(
        "orc never gripped the player (state {:?}, log {:?})",
        s.world.npcs[&h].motion.state,
        s.world.log.iter().rev().take(8).collect::<Vec<_>>()
    );
}

#[test]
fn orc_dash_grips_player_who_is_held_entangled_and_damaged() {
    let Some(mut s) = session() else {
        return;
    };
    let h = grappling_orc(&mut s);
    let start = wait_for_grip(&mut s, h);
    let player = s.avatar().handle;
    assert_eq!(s.world.npcs[&h].character.grip.gripped, Some(player));
    assert_eq!(s.avatar().character.grip.gripper, Some(h));
    let hp = s.avatar().character.combat.health.hit_points;
    // The grab frame still carries the dash collision's shove; measure from
    // the first held update.
    s.step(1. / 60.);
    s.handle_input(PlayerInput::default(), 16. / 9.);
    let held_at = s.body().position;
    let grapple = animation_value("spec_grapple").unwrap();
    let (mut entangled, mut gripping, mut grapple_seen) = (
        false,
        s.world.npcs[&h].motion.state == CharacterState::Gripping,
        false,
    );
    for _ in 0..240 {
        s.step(1. / 60.);
        // Trying to walk away does nothing while held.
        s.handle_input(
            PlayerInput {
                cursor_ndc: Some([0.9, 0.0]),
                move_down: true,
                ..Default::default()
            },
            16. / 9.,
        );
        entangled |= s.avatar().motion.state == CharacterState::Entangled;
        let orc = &s.world.npcs[&h];
        gripping |= orc.motion.state == CharacterState::Gripping;
        grapple_seen |= orc.motion.state == CharacterState::GripAttack
            && orc.motion.controller.current_animation == grapple;
    }
    let p = s.body().position;
    let moved = (p[0] - held_at[0]).hypot(p[2] - held_at[2]);
    eprintln!(
        "gripped at frame {start}; player moved {moved}, hp {hp} -> {}",
        s.avatar().character.combat.health.hit_points
    );
    assert!(entangled, "player never entered EntangledState");
    assert!(gripping, "orc never entered GrippingState");
    assert!(grapple_seen, "orc never grip-attacked with spec_grapple");
    assert!(moved < 0.1, "held player walked {moved}");
    assert!(
        s.avatar().character.combat.health.hit_points < hp,
        "grip attack did no damage"
    );
    // The orc stands in front of its victim at the sum of the radii.
    let orc = s.world.physics.bodies[s.world.npcs[&h].body].position;
    let gap = (orc[0] - p[0]).hypot(orc[2] - p[2]);
    let radii = s.world.npcs[&h].definition.radius + s.avatar().character.definition.radius;
    assert!((gap - radii).abs() < 1e-3, "gap {gap} radii {radii}");
}

#[test]
fn mashing_space_breaks_free_and_knocks_the_orc_down() {
    let Some(mut s) = session() else {
        return;
    };
    let h = grappling_orc(&mut s);
    wait_for_grip(&mut s, h);
    let tolerance = s.world.npcs[&h].character.grip.tolerance;
    let strength = s.avatar().character.definition.break_free_strength;
    let mut presses = 0;
    let mut released = None;
    for frame in 0..600 {
        let press = frame % 2 == 0;
        presses += usize::from(press && s.avatar().character.grip.gripper.is_some());
        s.handle_input(
            PlayerInput {
                boost_down: press,
                ..Default::default()
            },
            16. / 9.,
        );
        s.step(1. / 60.);
        if s.world.npcs[&h].character.grip.gripped.is_none() {
            released = Some(frame);
            break;
        }
    }
    let frame = released.expect("break-free never released the grip");
    eprintln!(
        "released after {presses} presses (tolerance {tolerance}, strength {strength}) at {frame}"
    );
    assert!(s.avatar().character.grip.gripper.is_none());
    assert_eq!(s.avatar().character.grip.break_free_counter, 0);
    assert!(presses as f32 * strength >= tolerance);
    let mut knocked = false;
    let mut free = false;
    for _ in 0..30 {
        s.step(1. / 60.);
        s.handle_input(PlayerInput::default(), 16. / 9.);
        knocked |= s.world.npcs[&h].motion.state == CharacterState::KnockDown;
        free |= matches!(
            s.avatar().motion.state,
            CharacterState::Idle | CharacterState::Move
        );
    }
    assert!(knocked, "released orc was not knocked down");
    assert!(free, "player stayed entangled");
    assert!(s.physics_body_allows_move());
}

trait AvatarBody {
    fn physics_body_allows_move(&self) -> bool;
}
impl AvatarBody for PlaySession {
    fn physics_body_allows_move(&self) -> bool {
        self.body().allow_move
    }
}
