use magicka_network::{control::*, transport::*};
use magicka_runtime::play_session::PlaySession;
use std::path::PathBuf;
struct Harness {
    host: LoopbackHost,
    clients: [LoopbackClient; 2],
    authority: InputAuthority,
    session: PlaySession,
}
impl Harness {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let content = magicka_content::retail_content_dir();
        let level = root
            .join("../../scenarios/feature-lab.lvl")
            .canonicalize()
            .unwrap();
        let mut session = PlaySession::load(&content, level.to_str().unwrap(), "Wizard").unwrap();
        session.step(1. / 60.);
        let mut host = LoopbackHost::bind("127.0.0.1:0".parse().unwrap()).unwrap();
        let mut clients = [
            LoopbackClient::connect(host.address()).unwrap(),
            LoopbackClient::connect(host.address()).unwrap(),
        ];
        for _ in 0..400 {
            for c in &mut clients {
                c.update();
            }
            host.update();
            if clients.iter().all(|c| c.player_slot().is_some()) {
                break;
            }
        }
        for client in &clients {
            session
                .join_player(client.player_slot().unwrap(), "Wizard", "start")
                .unwrap();
        }
        Self {
            host,
            clients,
            authority: InputAuthority::new(1, 1).unwrap(),
            session,
        }
    }
    fn requests(&mut self, requests: [PickupRequest; 2]) -> [PickupReceipt; 2] {
        for (client, request) in self.clients.iter_mut().zip(&requests) {
            client.send_pickup(request.clone()).unwrap();
        }
        let mut receipts = [None, None];
        for _ in 0..400 {
            for (i, client) in self.clients.iter_mut().enumerate() {
                client.update();
                for receipt in client.take_pickup_receipts() {
                    assert_eq!(
                        (receipt.sequence, receipt.target),
                        (requests[i].sequence, requests[i].target)
                    );
                    assert!(receipts[i].is_none(), "unexpected duplicate receipt");
                    receipts[i] = Some(receipt);
                }
            }
            self.host.update();
            for owned in self.host.take_pickups() {
                let outcome = match self.authority.accept_pickup(owned.slot, &owned.request) {
                    Ok(target) => self.session.pickup_remote(owned.slot, target),
                    Err(error) => PickupOutcome::Rejected(PickupRejection::Authority(error)),
                };
                self.host
                    .send_pickup_receipt(owned.slot, owned.request.receipt(outcome))
                    .unwrap();
            }
            if receipts.iter().all(Option::is_some) {
                return receipts.map(Option::unwrap);
            }
        }
        panic!("pickup receipts did not reach their owners");
    }
    fn pair(target: PickupTarget, epoch: u64, sequence: u64) -> [PickupRequest; 2] {
        std::array::from_fn(|_| PickupRequest {
            session: 1,
            epoch,
            sequence,
            target,
        })
    }
    fn near(&mut self, position: [f32; 3]) {
        for (i, c) in self.clients.iter().enumerate() {
            let body = self.session.world.players[c.player_slot().unwrap()]
                .as_ref()
                .unwrap()
                .body;
            self.session.world.physics.bodies[body].position = [
                position[0] + if i == 0 { -1. } else { 1. },
                position[1],
                position[2],
            ];
        }
    }
}
#[test]
fn remote_contested_books_and_weapons_have_one_winner_and_no_stale_exchange() {
    let mut h = Harness::new();
    let book = *h.session.world.book_pickups.keys().next().unwrap();
    let target = PickupTarget {
        handle: book,
        generation: h.session.world.book_pickups[&book].generation,
    };
    // Both remote players begin outside range; the host rejects without unlocking.
    for c in &h.clients {
        let body = h.session.world.players[c.player_slot().unwrap()]
            .as_ref()
            .unwrap()
            .body;
        h.session.world.physics.bodies[body].position = [-15., 1., -10.];
    }
    let denied = h.requests(Harness::pair(target, 1, 1));
    assert!(denied
        .iter()
        .all(|r| r.outcome == PickupOutcome::Rejected(PickupRejection::OutOfRange)));
    assert_eq!(h.session.unlocked_magicks.to_array::<4>(), [0; 4]);
    let transform = h.session.world.book_pickups[&book].transform;
    h.near([transform[12], transform[13], transform[14]]);
    let receipts = h.requests(Harness::pair(target, 1, 2));
    assert_eq!(
        receipts
            .iter()
            .filter(|r| matches!(r.outcome, PickupOutcome::Book { .. }))
            .count(),
        1
    );
    assert_eq!(
        receipts
            .iter()
            .filter(|r| r.outcome == PickupOutcome::Rejected(PickupRejection::Unavailable))
            .count(),
        1
    );
    assert!(h.session.unlocked_magicks.iter().all(|bits| *bits != 0));
    assert!(h
        .session
        .physics_frame(1, 1)
        .unwrap()
        .bodies
        .iter()
        .all(|b| b.handle != book));
    let replay = h.requests(Harness::pair(target, 1, 2));
    assert!(replay.iter().all(
        |r| r.outcome == PickupOutcome::Rejected(PickupRejection::Authority(InputError::Stale))
    ));
    let local_items: Vec<_> = h
        .session
        .avatar()
        .items
        .iter()
        .map(|(slot, item)| (*slot, item.definition.name.clone()))
        .collect();
    let weapon = h
        .session
        .spawn_item_pickup("weapon_kris", transform)
        .unwrap();
    let target = PickupTarget {
        handle: weapon,
        generation: h.session.world.item_pickups[&weapon].generation,
    };
    let receipts = h.requests(Harness::pair(target, 1, 3));
    assert_eq!(
        receipts
            .iter()
            .filter(|r| r.outcome == PickupOutcome::Swapped)
            .count(),
        1
    );
    assert_eq!(
        receipts
            .iter()
            .filter(|r| r.outcome == PickupOutcome::Rejected(PickupRejection::Lifetime))
            .count(),
        1
    );
    let winners = h
        .clients
        .iter()
        .filter(|c| {
            h.session.world.players[c.player_slot().unwrap()]
                .as_ref()
                .unwrap()
                .items
                .iter()
                .any(|(_, item)| item.definition.name == "weapon_kris")
        })
        .count();
    assert_eq!(winners, 1);
    assert_eq!(
        h.session
            .avatar()
            .items
            .iter()
            .map(|(slot, item)| (*slot, item.definition.name.clone()))
            .collect::<Vec<_>>(),
        local_items
    );
    let generation = h.session.world.item_pickups[&weapon].generation;
    let delayed = h.requests(Harness::pair(target, 1, 4));
    assert!(delayed
        .iter()
        .all(|r| r.outcome == PickupOutcome::Rejected(PickupRejection::Lifetime)));
    assert_eq!(h.session.world.item_pickups[&weapon].generation, generation);
    h.session.change_scene_named("corridor", "start").unwrap();
    h.authority.advance_epoch(2).unwrap();
    let old_scene = h.requests(Harness::pair(target, 1, 5));
    assert!(old_scene.iter().all(
        |r| r.outcome == PickupOutcome::Rejected(PickupRejection::Authority(InputError::Epoch))
    ));
    println!("remote pickup receipts: range denied, one book collector, shared unlock, one weapon exchange, replay/lifetime/old-scene requests rejected");
}
