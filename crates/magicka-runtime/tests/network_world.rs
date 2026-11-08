//! Script-driven world state reaches clients, which run no scene scripts.
use magicka_network::world::{PickupKind, WorldPresentation};
use magicka_runtime::{
    animated_hierarchy::PlayCommand,
    network_world::{world_presentation, RemoteWorld},
    play_session::PlaySession,
    tutorial::DialogHint,
};
use std::path::PathBuf;

fn content() -> Option<PathBuf> {
    let root = magicka_content::retail_content_dir();
    root.is_dir().then_some(root)
}

/// The wire form: serde round trip plus validation.
fn wire(state: WorldPresentation) -> WorldPresentation {
    assert!(state.valid());
    serde_json::from_slice(&serde_json::to_vec(&state).unwrap()).unwrap()
}

fn books(session: &PlaySession) -> Vec<(u16, u8)> {
    session
        .world
        .entities
        .iter()
        .filter(|e| e.added && !e.removed && !e.dead)
        .filter_map(|e| Some((e.handle, session.world.book_pickups.get(&e.handle)?.magick)))
        .collect()
}

#[test]
fn clients_mirror_scripted_pickups_tutorial_parts_and_gating() {
    let Some(content) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut host = PlaySession::load(&content, "Wizard_Castle.lvl", "Wizard").unwrap();
    host.disable_scripts();
    let mut client = PlaySession::load(&content, "Wizard_Castle.lvl", "Wizard").unwrap();
    client.disable_scripts();
    // A client mirror player shifts the client's handle allocation.
    client.join_player_at_start(1, "Wizard").unwrap();
    let haste = magicka_runtime::magick_books::magick_id("haste").unwrap();
    let mut at = magicka_content::skinned::IDENTITY;
    at[12..15].copy_from_slice(&host.body().position);
    // What a SpawnMagick trigger leaves behind.
    let book = host
        .world
        .mirror_pickup(&PickupKind::Book { magick: haste }, at)
        .unwrap();
    host.world.tutorial.set_dialog(DialogHint {
        text: "Hello".into(),
        element: 0,
        magick: 0,
        trigger: 42,
        scale: None,
        size: None,
    });
    host.world.flags.tutorial_elements_disabled = 0b1010;
    host.world.flags.tutorial_push_disabled = true;
    let part = host.world.collision.parts().first().map(|p| p.name.clone());
    if let Some(part) = &part {
        host.world
            .collision
            .play(
                std::slice::from_ref(part),
                true,
                PlayCommand {
                    start: 0.0,
                    end: f32::MAX,
                    speed: 1.0,
                    looping: false,
                    resume: false,
                },
            )
            .unwrap();
        host.world
            .collision
            .update(0.5, &magicka_content::skinned::IDENTITY)
            .unwrap();
    }

    let mut remote = RemoteWorld::default();
    remote
        .apply(
            &wire(world_presentation(&host)),
            &mut client,
            1. / 60.,
            true,
        )
        .unwrap();
    let local = remote.local_pickup(book).expect("host book mapped");
    assert!(
        books(&client).contains(&(local, haste)),
        "client shows the book"
    );
    assert_eq!(remote.host_pickup(local), Some(book));
    assert!(
        !client.world.tutorial.dialog_done(),
        "client shows the dialog"
    );
    assert!(
        (0..8).all(|i| client.keyboard_hud.icons[i].enabled == ((0b1010 >> i) & 1 == 0)),
        "the client HUD follows the host's disabled elements"
    );
    // Clients without a wizard (spectators, invaders) get no modal dialog.
    let mut observer = PlaySession::load(&content, "Wizard_Castle.lvl", "Wizard").unwrap();
    observer.disable_scripts();
    RemoteWorld::default()
        .apply(
            &wire(world_presentation(&host)),
            &mut observer,
            1. / 60.,
            false,
        )
        .unwrap();
    assert!(observer.world.tutorial.dialog_done());
    assert_eq!(client.world.flags.tutorial_elements_disabled, 0b1010);
    assert!(client.world.flags.tutorial_push_disabled);
    assert_eq!(client.magicks_allowed(), host.magicks_allowed());
    if part.is_some() {
        assert_eq!(
            client.world.collision.playback_states(),
            host.world.collision.playback_states(),
            "client level parts follow the host"
        );
    }

    // Dismissing here (after the holdoff) hides only this machine's copy;
    // the host's dialog stays and does not reappear on the client.
    for _ in 0..90 {
        remote
            .apply(
                &wire(world_presentation(&host)),
                &mut client,
                1. / 60.,
                true,
            )
            .unwrap();
    }
    assert!(remote.dismiss_dialog(&mut client));
    remote
        .apply(
            &wire(world_presentation(&host)),
            &mut client,
            1. / 60.,
            true,
        )
        .unwrap();
    assert!(client.world.tutorial.dialog_done());
    assert!(!host.world.tutorial.dialog_done());

    // Picked up on the host: gone on the client.
    let entity = host
        .world
        .entities
        .iter_mut()
        .find(|e| e.handle == book)
        .unwrap();
    entity.removed = true;
    entity.dead = true;
    remote
        .apply(
            &wire(world_presentation(&host)),
            &mut client,
            1. / 60.,
            true,
        )
        .unwrap();
    assert!(!books(&client).iter().any(|(_, m)| *m == haste));
}

/// A host wizard's self-shield, aura halo and weapon trails reach the
/// client's mirror of that wizard, where the viewer's accessors expose them
/// exactly as they expose the local simulation's own.
#[test]
fn clients_mirror_remote_wizard_self_shield_halo_and_weapon_trails() {
    use magicka_network::world::{ArcaneBladeState, AvatarPresentation, TRAIL_VERTICES};
    use magicka_runtime::avatar::{IceBlade, SpellEffect, ICE_BLADE_TRAIL};
    use magicka_runtime::network_world::avatar_presentation;
    use magicka_sim::character_health::{SelfShield, SelfShieldType};
    use magicka_sim::spell_cast::{EffectKind, EffectLifecycle};
    let Some(content) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut host = PlaySession::load(&content, "Wizard_Castle.lvl", "Wizard").unwrap();
    host.disable_scripts();
    host.join_player_at_start(1, "Wizard").unwrap();
    // Host slot 1 (a remote player on the host) has everything active.
    let trail: [[f32; 3]; ICE_BLADE_TRAIL] =
        std::array::from_fn(|i| [i as f32, 1.0 + (i % 2) as f32, 0.25 * i as f32]);
    {
        let p = host.world.players[1].as_mut().unwrap();
        p.character.combat.health.shield = SelfShield {
            kind: SelfShieldType::Shield,
            health: 35.0,
            max_health: 60.0,
            magnitude: 1.5,
            noise_offsets: [[0.1, 0.2], [0.3, 0.4], [0.5, 0.6]],
            time_since_damage: 0.1,
            time_sensitive: true,
            ..Default::default()
        };
        p.character.halo_draw = Some([0.2, 0.8, 0.3, 0.666]);
        p.current_spell = Some(SpellEffect::Weapon {
            kind: EffectKind::Projectile,
            spell: Default::default(),
            timestamp: 0.0,
            lifecycle: EffectLifecycle {
                active: true,
                min_ttl: 0.0,
                max_ttl: 1.0,
                from_staff: false,
                cast_type: magicka_sim::damage::CastType::Weapon,
                time_stamp: 0.0,
            },
            ice_blade: Some(IceBlade {
                range: 5.0,
                damages: Default::default(),
                timestamp: 0.0,
                trail: Some(Box::new(trail)),
                orientation: None,
            }),
        });
        p.arcane_blades
            .push(magicka_runtime::arcane_blade::ArcaneBlade::new(
                3,
                magicka_sim::spells::Elements::ARCANE,
                4.0,
            ));
    }
    // The host's own accessors see the live simulation.
    assert_eq!(
        host.ice_blade_trails(),
        vec![(host.world.players[1].as_ref().unwrap().handle, trail)]
    );
    assert_eq!(host.arcane_blade_draws().len(), 1);
    let records = avatar_presentation(&host);
    assert_eq!(
        records.len(),
        1,
        "only wizards with something to draw: {records:?}"
    );
    assert_eq!(records[0].slot, 1);
    assert_eq!(
        records[0].ice_blade_trail.as_ref().map(Vec::len),
        Some(TRAIL_VERTICES)
    );
    assert_eq!(records[0].arcane_blades.len(), 1);

    // A client playing host slot 2 mirrors host slot 1 in its own slot 1.
    let mut client = PlaySession::load(&content, "Wizard_Castle.lvl", "Wizard").unwrap();
    client.disable_scripts();
    client.join_player_at_start(1, "Wizard").unwrap();
    let mirror = client.world.players[1].as_ref().unwrap().handle;
    let mut remote = RemoteWorld::default();
    let state = wire(world_presentation(&host));
    remote.apply(&state, &mut client, 1. / 60., true).unwrap();
    remote.apply_avatars(&state, &mut client, Some(2), 10.0);

    let shields = client.self_shields();
    let shield = shields
        .iter()
        .find(|d| d.handle == mirror)
        .expect("the mirror has the host's self-shield");
    assert_eq!(shield.shield.kind, SelfShieldType::Shield);
    assert_eq!(shield.shield.health, 35.0);
    assert_eq!(
        shield.shield.noise_offsets,
        [[0.1, 0.2], [0.3, 0.4], [0.5, 0.6]]
    );
    assert_eq!(
        client.world.players[1]
            .as_ref()
            .unwrap()
            .character
            .halo_draw,
        Some([0.2, 0.8, 0.3, 0.666])
    );
    assert_eq!(
        client
            .aura_halos()
            .iter()
            .map(|(h, _, _)| *h)
            .collect::<Vec<_>>(),
        vec![mirror],
        "the halo billboard path sees the mirror"
    );
    assert_eq!(client.ice_blade_trails(), vec![(mirror, trail)]);
    let blades = client.arcane_blade_draws();
    assert_eq!((blades[0].owner, blades[0].id), (mirror, 3));
    assert_eq!(blades[0].center, blades[0].edge.map(|c| c + 1.0));
    // The client's own wizard (local slot 0) has nothing to draw.
    assert!(!shields.iter().any(|d| d.handle == client.avatar().handle));

    // The host's shield drops and the blade dies: the mirror follows, and a
    // hand-made record is applied by the same path.
    {
        let p = host.world.players[1].as_mut().unwrap();
        p.character.combat.health.shield = SelfShield::default();
        p.character.halo_draw = None;
        p.current_spell = None;
        p.arcane_blades.clear();
    }
    let state = wire(world_presentation(&host));
    remote.apply_avatars(&state, &mut client, Some(2), 10.1);
    assert!(client.self_shields().is_empty());
    assert!(client.ice_blade_trails().is_empty());
    assert!(client.arcane_blade_draws().is_empty());
    assert!(client.remote_avatars.is_empty());
    let mut state = state;
    state.avatars.push(AvatarPresentation {
        slot: 1,
        arcane_blades: vec![ArcaneBladeState {
            id: 9,
            color: [0.1, 0.2, 0.3],
            vertices: vec![[0.0; 3]; TRAIL_VERTICES],
        }],
        ..Default::default()
    });
    assert!(state.valid());
    remote.apply_avatars(&wire(state), &mut client, Some(2), 10.2);
    assert_eq!(client.arcane_blade_draws()[0].id, 9);
}

/// Zap pulses carry serials: a client applies each once, arms the mirror's
/// zap timer for the flash window, and a late joiner skips history.
#[test]
fn zap_pulses_apply_once_per_serial_and_arm_the_mirror_flash() {
    use magicka_network::world::PulseKind;
    use magicka_runtime::network_world::{PulseTracker, ZAP_FLASH_SECONDS};
    let Some(content) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let mut host = PlaySession::load(&content, "Wizard_Castle.lvl", "Wizard").unwrap();
    host.disable_scripts();
    host.join_player_at_start(1, "Wizard").unwrap();
    let mut tracker = PulseTracker::default();
    tracker.observe(&host, 1.0);
    assert!(tracker.recent().is_empty());
    // Character.mZapTimer armed on slot 1: one pulse, not one per tick.
    host.world.players[1]
        .as_mut()
        .unwrap()
        .character
        .combat
        .zap_timer = ZAP_FLASH_SECONDS;
    tracker.observe(&host, 1.0);
    tracker.observe(&host, 1.0 + 1. / 60.);
    assert_eq!(tracker.recent().len(), 1);
    // Expired and re-armed: a second serial.
    host.world.players[1]
        .as_mut()
        .unwrap()
        .character
        .combat
        .zap_timer = 0.0;
    tracker.observe(&host, 1.2);
    host.world.players[1]
        .as_mut()
        .unwrap()
        .character
        .combat
        .zap_timer = ZAP_FLASH_SECONDS;
    tracker.observe(&host, 1.25);
    let pulses = tracker.recent();
    assert_eq!(
        pulses
            .iter()
            .map(|p| (p.serial, p.slot, p.kind))
            .collect::<Vec<_>>(),
        vec![(1, 1, PulseKind::Zap), (2, 1, PulseKind::Zap)]
    );

    let mut client = PlaySession::load(&content, "Wizard_Castle.lvl", "Wizard").unwrap();
    client.disable_scripts();
    client.join_player_at_start(1, "Wizard").unwrap();
    let mut state = world_presentation(&host);
    state.pulses = vec![pulses[0]];
    let mut remote = RemoteWorld::default();
    // The first snapshot primes: history is not replayed.
    assert!(remote
        .apply_avatars(&wire(state.clone()), &mut client, Some(2), 1.01)
        .is_empty());
    assert!(remote
        .apply_avatars(&wire(state.clone()), &mut client, Some(2), 1.02)
        .is_empty());
    state.pulses = pulses.clone();
    let fresh = remote.apply_avatars(&wire(state.clone()), &mut client, Some(2), 1.26);
    assert_eq!(fresh.iter().map(|p| p.serial).collect::<Vec<_>>(), vec![2]);
    let zap = |client: &PlaySession| {
        client.world.players[1]
            .as_ref()
            .unwrap()
            .character
            .combat
            .zap_timer
    };
    assert!((zap(&client) - (ZAP_FLASH_SECONDS - 0.01)).abs() < 1e-4);
    assert_eq!(
        client
            .zapped_characters()
            .iter()
            .map(|z| z.handle)
            .collect::<Vec<_>>(),
        vec![client.world.players[1].as_ref().unwrap().handle],
        "the zap flash path sees the mirror"
    );
    // The same pulses again: nothing fresh; after the flash window the
    // timer is disarmed even though clients never step characters.
    assert!(remote
        .apply_avatars(&wire(state.clone()), &mut client, Some(2), 1.27)
        .is_empty());
    remote.apply_avatars(&wire(state.clone()), &mut client, Some(2), 1.5);
    assert_eq!(zap(&client), 0.0);
    assert!(client.zapped_characters().is_empty());
    // A late joiner primes on what it first sees and draws only later pulses.
    let mut late = RemoteWorld::default();
    assert!(late
        .apply_avatars(&wire(state.clone()), &mut client, Some(2), 1.5)
        .is_empty());
    // The host forgets pulses after the window.
    tracker.observe(&host, 3.0);
    assert!(tracker.recent().is_empty());
}
