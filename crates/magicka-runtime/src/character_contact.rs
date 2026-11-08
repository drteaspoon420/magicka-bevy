//! Synchronous character-pair shield callbacks over the live predicted bodies.
use crate::{
    avatar::PlayerAvatar, character::RuntimeCharacter, character_shield, npc::NpcCharacter,
    npc_combat::Combatant,
};
use magicka_sim::{
    character_physics::CharacterBody,
    damage::{Damage, DamageError},
    damage_delivery::DamagePayload,
    resistance::Resistance,
};

pub enum ContactActor<'a> {
    Player(&'a mut PlayerAvatar),
    Npc(&'a mut NpcCharacter),
}
impl ContactActor<'_> {
    pub fn handle(&self) -> u16 {
        match self {
            Self::Player(p) => p.handle,
            Self::Npc(n) => n.handle,
        }
    }
    pub fn character(&mut self) -> &mut RuntimeCharacter {
        match self {
            Self::Player(p) => &mut p.character,
            Self::Npc(n) => &mut n.character,
        }
    }
    pub fn dead(&self) -> bool {
        match self {
            Self::Player(p) => p.dead(),
            Self::Npc(n) => n.dead(),
        }
    }
    fn receive(
        &mut self,
        body: &mut CharacterBody,
        attacker: &Combatant,
        damages: &[Damage],
        notify: &mut impl FnMut(&mut NpcCharacter, &Combatant, f32),
        triggers: &mut Vec<(i32, Option<u16>)>,
    ) {
        let mut collection = [Damage::default(); 5];
        collection[..damages.len()].copy_from_slice(damages);
        let payload = DamagePayload::Collection(collection);
        match self {
            Self::Player(p) => {
                p.receive_payload(body, payload, Some(attacker.handle), [0.0; 3], 0.0);
            }
            Self::Npc(n) => {
                for d in damages.iter().filter(|d| d.attack != 0) {
                    notify(n, attacker, d.amount * d.magnitude);
                }
                let result = crate::avatar::deliver_to_body(
                    &mut n.character,
                    body,
                    &[Resistance::IDENTITY; 11],
                    false,
                    n.handle,
                    payload,
                    Some(attacker.handle),
                    [0.0; 3],
                    0.0,
                );
                if result & magicka_sim::character_impact::result::DAMAGED != 0 && n.on_damage != 0
                {
                    triggers.push((n.on_damage, Some(n.handle)));
                }
            }
        }
    }
}

fn callback(
    source: &mut ContactActor<'_>,
    target: &mut ContactActor<'_>,
    bodies: [&mut CharacterBody; 2],
    attacker: &Combatant,
    notify: &mut impl FnMut(&mut NpcCharacter, &Combatant, f32),
    triggers: &mut Vec<(i32, Option<u16>)>,
) -> Result<(bool, Option<character_shield::ShieldNullification>), DamageError> {
    if source.dead() {
        return Ok((false, None));
    }
    let [source_body, target_body] = bodies;
    let target_handle = target.handle();
    let character = source.character();
    character.combat.health.shield.ice_contact(
        target_handle,
        source_body.velocity,
        target_body.velocity,
        &mut character.collision_hits,
        |damages| target.receive(target_body, attacker, damages, notify, triggers),
    )?;
    let effect = character_shield::nullify_pair(
        [source.character(), target.character()],
        [source_body, target_body],
    );
    let character = source.character();
    Ok((character.accepts_owned_contact(), effect))
}

/// Both callbacks execute, including after a first rejection. The second
/// rechecks death and shield state after the first's synchronous damage.
pub fn pair(
    actors: [&mut ContactActor<'_>; 2],
    bodies: [&mut CharacterBody; 2],
    attackers: [&Combatant; 2],
    mut notify: impl FnMut(&mut NpcCharacter, &Combatant, f32),
    triggers: &mut Vec<(i32, Option<u16>)>,
) -> Result<(bool, Option<character_shield::ShieldNullification>), DamageError> {
    let [a, b] = actors;
    let [body_a, body_b] = bodies;
    let (accept_a, effect_a) =
        callback(a, b, [body_a, body_b], attackers[0], &mut notify, triggers)?;
    let (accept_b, effect_b) =
        callback(b, a, [body_b, body_a], attackers[1], &mut notify, triggers)?;
    if accept_a & accept_b {
        // CharacterBody collision handling: touching an enemy character ends
        // invisibility unless that one is a non-avatar under half our mass
        // (pushed aside instead). Native runs this on the Skin0 side only;
        // JigLibX pair order is not modelled, so both sides check (INFERRED).
        let masses = [body_a.mass, body_b.mass];
        for (me, other, actor) in [(0, 1, &mut *a), (1, 0, &mut *b)] {
            let light = !attackers[other].avatar && masses[other] < masses[me] * 0.5;
            if !light
                && attackers[me].faction & attackers[other].faction == 0
                && actor.character().is_invisible()
            {
                actor.character().set_invisible(0.0);
            }
        }
    }
    Ok((accept_a & accept_b, effect_a.or(effect_b)))
}
