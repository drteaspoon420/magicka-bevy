//! Real Battlefield boss/template with isolated absorbed-element visual controls.
use super::*;
use magicka_runtime::{
    avatar::deliver_to_body,
    boss_action::{BossAction, BossCommand, BossSetup},
    scene_loading::custom_hash,
    scene_world::boss_live::BossRef,
    script_runtime::ScriptWorld,
};
use magicka_sim::{
    damage::{attack, Damage},
    damage_delivery::DamagePayload,
    resistance::Resistance,
    spells::Elements,
};
pub(super) const SCRIPT: &[Step] = &[
    (1, [0., 0.], Act::WarlordStart, "spawn"),
    (90, [0., 0.], Act::WarlordWait, "ready"),
    (30, [0., 0.], Act::WarlordFire, "absorbed_fire"),
    (30, [0., 0.], Act::WarlordClear, "empty"),
    (30, [0., 0.], Act::WarlordArcane, "absorbed_arcane"),
];
pub(super) fn prepare(
    s: &mut PlaySession,
    act: Option<Act>,
    last: &mut Option<Act>,
) -> Option<bool> {
    let act = act?;
    if !matches!(
        act,
        Act::WarlordStart
            | Act::WarlordWait
            | Act::WarlordFire
            | Act::WarlordClear
            | Act::WarlordArcane
    ) {
        return None;
    }
    if act == Act::WarlordStart && *last != Some(act) {
        s.disable_scripts();
        s.world.flags.cutscene = false;
        s.world.flags.input_limited = false;
        s.world.players[0].as_mut().unwrap().character.immortal_time = f32::MAX;
        for _ in 0..60 {
            s.step(1. / 60.);
        }
        s.world
            .boss_action(
                &BossAction {
                    delay: 0.,
                    command: BossCommand::Spawn {
                        boss: "warlord".into(),
                        prop: false,
                        generic: false,
                        area: custom_hash("boss_spawn"),
                        unique_id: 0,
                        mesh: -1,
                        on_death: 0,
                        on_damage: 0,
                        setup: BossSetup {
                            delayed: true,
                            ..Default::default()
                        },
                    },
                },
                false,
            )
            .unwrap()
            .unwrap();
        let BossRef::Warlord(boss) = &s.world.boss_fight.bosses[0] else {
            panic!("native Warlord wrapper")
        };
        let npc = s.world.npcs.get_mut(&boss.handle).unwrap();
        npc.ai.set_order(0, None);
        npc.react_to = 0;
        npc.reaction_order = 0;
    }
    let BossRef::Warlord(boss) = &s.world.boss_fight.bosses[0] else {
        panic!("Warlord fixture")
    };
    let handle = boss.handle;
    if *last != Some(act)
        && matches!(
            act,
            Act::WarlordFire | Act::WarlordClear | Act::WarlordArcane
        )
    {
        let npc = s.world.npcs.get_mut(&handle).unwrap();
        let queue = npc.character.combat.absorb_shield.as_mut().unwrap();
        queue.retrieve(&mut Vec::new());
        if act != Act::WarlordClear {
            npc.character.block_item = 0;
            let body = &mut s.world.physics.bodies[npc.body];
            let behind = std::array::from_fn(|i| body.position[i] - body.direction()[i] * 2.);
            deliver_to_body(
                &mut npc.character,
                body,
                &[Resistance::IDENTITY; 11],
                false,
                handle,
                DamagePayload::Single(Damage {
                    attack: attack::DAMAGE,
                    element: if act == Act::WarlordFire {
                        Elements::FIRE.0
                    } else {
                        Elements::ARCANE.0
                    },
                    amount: 100.,
                    magnitude: 3.,
                }),
                None,
                behind,
                0.,
            );
        }
        // Native particle lifetimes advance; clear also waits for old particles.
        for _ in 0..60 {
            s.step(1. / 60.);
        }
        let name = if act == Act::WarlordFire {
            "swordelementeffect_fire"
        } else {
            "swordelementeffect_arcane"
        };
        let expected = usize::from(act != Act::WarlordClear);
        assert_eq!(
            s.effects.active_effects_by_hash(custom_hash(name)),
            expected
        );
        eprintln!(
            "Warlord chant control act={} queue={:?} fire={} arcane={}",
            if act == Act::WarlordFire {
                "fire"
            } else if act == Act::WarlordClear {
                "empty"
            } else {
                "arcane"
            },
            s.world.npcs[&handle]
                .character
                .combat
                .absorb_shield
                .as_ref()
                .unwrap()
                .peek()
                .element,
            s.effects
                .active_effects_by_hash(custom_hash("swordelementeffect_fire")),
            s.effects
                .active_effects_by_hash(custom_hash("swordelementeffect_arcane"))
        );
    }
    let npc = &s.world.npcs[&handle];
    let mut target = s.world.physics.bodies[npc.body].position;
    target[1] += 1.;
    s.world.camera.move_to(target, 0.);
    s.world.camera.set_position(target, false);
    s.world.camera.set_magnification(1.);
    *last = Some(act);
    Some(!matches!(act, Act::WarlordStart | Act::WarlordWait))
}
