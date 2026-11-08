use super::*;
use crate::{buffs::NonvisualKind, play_session::PlaySession};
use magicka_sim::{
    damage::Damage, damage_delivery::DamagePayload, resistance::Resistance, spells::Elements as E,
};

pub(super) fn session() -> Option<(PlaySession, u16)> {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content");
    if !root.is_dir() {
        return None;
    }
    let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    s.disable_scripts();
    let area = crate::play_content::load_start_scene(&root, "Havindr.lvl")
        .unwrap()
        .start
        .area
        .unwrap();
    let xml = format!(
        r#"<Scene><trigger id="t"><then><spawn type="Goblin_miner" area="{area}1" order="idle"/></then></trigger></Scene>"#
    );
    let crate::scene_script::ActionDefinition::Spawn(action) =
        crate::scene_script::SceneScriptLayout::read(xml.as_bytes(), false)
            .unwrap()
            .actions
            .remove(0)
    else {
        panic!()
    };
    let mut state = crate::spawn_action::SpawnState::default();
    action.initialize(&mut state, &mut s.world).unwrap();
    action.execute(&mut state, &mut s.world).unwrap();
    let npc = state.spawned[0].handle.unwrap();
    for c in [
        &mut s.world.npcs.get_mut(&npc).unwrap().character,
        &mut s.world.players[0].as_mut().unwrap().character,
    ] {
        c.definition.resistances = [Resistance::IDENTITY; 11];
        c.definition.max_hit_points = 1000.0;
        c.combat.health.max_hit_points = 1000.0;
        c.combat.health.hit_points = 1000.0;
        c.combat.health.shield = Default::default();
    }
    Some((s, npc))
}

fn boost(c: &mut crate::character::RuntimeCharacter, multiplier: f32) {
    c.add_nonvisual_buff(
        42,
        NonvisualKind::BoostDamage(Damage {
            attack: 1,
            element: E::EARTH.0,
            amount: 0.0,
            magnitude: multiplier,
        }),
        0.5,
        false,
    );
}

fn hit(element: E) -> Damage {
    Damage {
        attack: 1,
        element: element.0,
        amount: 10.0,
        magnitude: 2.0,
    }
}

#[test]
fn scene_hits_read_attacker_buffs_once_for_players_and_npcs() {
    for npc_attacker in [false, true] {
        for multiplier in [3.0, 0.5] {
            let Some((mut s, npc)) = session() else {
                return;
            };
            let player = s.avatar().handle;
            let (attacker, target) = if npc_attacker {
                boost(
                    &mut s.world.npcs.get_mut(&npc).unwrap().character,
                    multiplier,
                );
                (npc, player)
            } else {
                boost(
                    &mut s.world.players[0].as_mut().unwrap().character,
                    multiplier,
                );
                (player, npc)
            };
            // A target's buff must not modify the incoming attack.
            if npc_attacker {
                boost(&mut s.world.players[0].as_mut().unwrap().character, 7.0);
            } else {
                boost(&mut s.world.npcs.get_mut(&npc).unwrap().character, 7.0);
            }
            s.world
                .hit_character(target, hit(E::EARTH), Some(attacker), [0.0; 3]);
            s.world
                .hit_character(target, hit(E::FIRE), Some(attacker), [0.0; 3]);
            let c = if npc_attacker {
                &s.avatar().character
            } else {
                &s.world.npcs[&npc].character
            };
            assert_eq!(
                c.combat.health.hit_points,
                1000.0 - 20.0 * multiplier - 20.0
            );
            // Avatar alerts include the victim again (research/npc-combat.md).
            if !npc_attacker {
                assert_eq!(
                    s.world.npcs[&npc].combat.attacked_by,
                    vec![(player, 2.0 * (20.0 * multiplier + 20.0))]
                );
            }
        }
    }
}

#[test]
fn beam_collections_modify_each_slot_once() {
    let Some((mut s, npc)) = session() else {
        return;
    };
    let player = s.avatar().handle;
    boost(&mut s.world.players[0].as_mut().unwrap().character, 3.0);
    let damage = [
        hit(E::EARTH),
        hit(E::FIRE),
        hit(E::EARTH | E::FIRE),
        Damage::default(),
        Damage::default(),
    ];
    s.world
        .with_rail_world(|w| {
            let slots = w
                .hit
                .boosted_payload(Some(player), DamagePayload::Collection(damage))
                .request_slots();
            assert_eq!(slots[0].amount, 30.0);
            assert_eq!(slots[0].magnitude, 2.0);
            assert_eq!(slots[1], damage[1]);
            assert_eq!(slots[3], Damage::default());
            w.deliver_collection(npc, Some(player), damage, 0.0, [0.0; 3]);
        })
        .unwrap();
    assert_eq!(s.world.npcs[&npc].character.combat.health.hit_points, 860.0);
    assert_eq!(s.world.npcs[&npc].combat.attacked_by, vec![(player, 280.0)]);
}

#[test]
fn spray_reads_buffs_while_the_caster_is_out_of_the_player_array() {
    let Some((mut s, npc)) = session() else {
        return;
    };
    let combatants = s.world.combatants();
    let mut p = s.world.players[0].take().unwrap();
    boost(&mut p.character, 3.0);
    let (before, after) = s.world.physics.bodies.split_at_mut(p.body);
    let (_, after) = after.split_first_mut().unwrap();
    let mut w = SceneSprayEffects {
        physics_damage: &mut s.world.physics_damage,
        dispensers: &mut s.world.dispensers,
        entities: &mut s.world.entities,
        caster_boosts: p.character.buffs.damage_boosts(),
        combatants,
        npcs: &mut s.world.npcs,
        players: &mut s.world.players,
        bodies_before: before,
        bodies_after: after,
        caster_index: p.body,
        triggers: &mut s.world.pending_triggers,
        shields: &mut s.world.shields,
        mines: &mut s.world.mines,
        grease: &mut s.world.grease,
        barriers: &mut s.world.barriers,
        barrier_events: Vec::new(),
        death: &mut s.world.death,
        jormungandr: &mut s.world.jormungandr,
        elemental_eggs: &mut s.world.elemental_eggs,
        missiles: &mut s.world.missiles,
        death_damage_features: true,
    };
    let mut damages = [Damage::default(); 5];
    damages[0] = hit(E::EARTH);
    damages[1] = hit(E::FIRE);
    w.deliver(
        crate::spray_world::SprayDamage {
            target: npc,
            attacker: p.handle,
            damages,
            position: [0.0; 3],
            timestamp: 0.0,
        },
        DamagePayload::Collection(damages),
    );
    assert_eq!(s.world.npcs[&npc].character.combat.health.hit_points, 920.0);
    assert_eq!(
        s.world.npcs[&npc].combat.attacked_by,
        vec![(p.handle, 160.0)]
    );
}
