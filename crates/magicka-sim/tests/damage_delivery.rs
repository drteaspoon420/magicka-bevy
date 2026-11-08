use magicka_sim::{damage::Damage, damage_delivery::*, spells::Elements as E};
fn context(network: NetworkRole, attacker_kind: AttackerKind) -> DeliveryContext {
    DeliveryContext {
        network,
        attacker_kind,
        target_handle: 2,
        attacker_handle: Some(1),
        time_stamp: 1.5,
        target_position: [1.0, 2.0, 3.0],
        attack_position: [5.0, 8.0, 13.0],
    }
}
fn damage(element: E, amount: f32, magnitude: f32) -> Damage {
    Damage {
        attack: 1,
        element: element.0,
        amount,
        magnitude,
    }
}
#[test]
fn all_network_actor_routes() {
    for role in [
        NetworkRole::Offline,
        NetworkRole::Client,
        NetworkRole::Server,
    ] {
        for actor in [
            AttackerKind::Other,
            AttackerKind::Character,
            AttackerKind::LocalAvatar,
            AttackerKind::RemoteAvatar,
        ] {
            let p = prepare(
                DamagePayload::Single(Damage::default()),
                context(role, actor),
                &[],
            );
            let expected = match role {
                NetworkRole::Offline => 15,
                NetworkRole::Client => {
                    if actor == AttackerKind::RemoteAvatar {
                        8
                    } else {
                        14
                    }
                }
                NetworkRole::Server => {
                    if actor == AttackerKind::RemoteAvatar {
                        8
                    } else {
                        15
                    }
                }
            };
            assert_eq!(p.features, expected);
            assert_eq!(
                p.request.is_some(),
                role != NetworkRole::Offline && actor == AttackerKind::LocalAvatar
            );
        }
    }
}
#[test]
fn matching_boosts_multiply_then_bias_and_do_not_change_magnitude() {
    let d = damage(E::FIRE | E::COLD, 10.0, 2.0);
    let boosts = [
        damage(E::FIRE, 3.0, 2.0),
        damage(E::COLD, -1.0, 4.0),
        damage(E::EARTH, 100.0, 100.0),
    ];
    let p = prepare(
        DamagePayload::Single(d),
        context(NetworkRole::Server, AttackerKind::LocalAvatar),
        &boosts,
    );
    let request = p.request.unwrap();
    assert_eq!(request.damages[0].amount, 82.0);
    assert_eq!(request.damages[0].magnitude, 2.0);
    assert_eq!(request.relative_attack_position, [4.0, 6.0, 10.0]);
    assert_eq!(request.damages[1..], [Damage::default(); 4]);
    let p = prepare(
        DamagePayload::Single(d),
        context(NetworkRole::Offline, AttackerKind::Other),
        &boosts,
    );
    assert_eq!(p.payload, DamagePayload::Single(d));
}
#[test]
fn collection_modifies_each_slot_once_and_preserves_zero_mask_entries() {
    let mut slots = [Damage::default(); 5];
    slots[0] = damage(E::FIRE, 10.0, 1.0);
    slots[4] = damage(E::LIFE, -10.0, 0.5);
    let p = prepare(
        DamagePayload::Collection(slots),
        context(NetworkRole::Client, AttackerKind::LocalAvatar),
        &[damage(E::FIRE | E::LIFE, 2.0, 3.0)],
    );
    let d = p.payload.request_slots();
    assert_eq!(d[0].amount, 32.0);
    assert_eq!(d[4].amount, -28.0);
    assert_eq!(d[1], Damage::default());
    assert_eq!(p.request.unwrap().damages, d);
}
