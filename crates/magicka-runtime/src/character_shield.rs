//! Character.OnCollision and Shield.OnCollision for spell shields.
//! See research/character-shield.md and collision-ignore.md; grips remain open.
use crate::character::RuntimeCharacter;
use magicka_sim::{
    character_health::SelfShieldType, character_physics::CharacterBody, impact_motion::Vector,
    shield_entity::ShieldEntity,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShieldNullification {
    pub position: Vector,
    pub emitter_position: Vector,
}

fn remove_pure(character: &mut RuntimeCharacter, body: &mut CharacterBody) {
    let mut events = Vec::new();
    character
        .combat
        .health
        .shield
        .remove(character.definition.mass, false, &mut events);
    for event in events {
        use magicka_sim::self_shield::ShieldEvent;
        match event {
            ShieldEvent::SetMass(mass) => body.mass = mass,
            ShieldEvent::ClearAura => character.auras.clear(),
            _ => character.shield_events.push(event),
        }
    }
}

/// One character's pure-Shield branch; the caller performs earlier death/grip
/// rejection. Calling again from the other skin observes removed shield types.
pub fn nullify_pair(
    characters: [&mut RuntimeCharacter; 2],
    bodies: [&mut CharacterBody; 2],
) -> Option<ShieldNullification> {
    let [a, b] = characters;
    let [body_a, body_b] = bodies;
    if a.combat.health.shield.kind != SelfShieldType::Shield
        || b.combat.health.shield.kind != SelfShieldType::Shield
    {
        return None;
    }
    let source = body_a.position;
    let target = body_b.position;
    remove_pure(a, body_a);
    remove_pure(b, body_b);
    let offset: Vector = std::array::from_fn(|i| target[i] - source[i]);
    let inverse = 1.0 / (offset.iter().map(|v| v * v).sum::<f32>() as f64).sqrt() as f32;
    Some(ShieldNullification {
        position: std::array::from_fn(|i| source[i] + offset[i] * inverse * body_a.radius),
        emitter_position: source,
    })
}

/// Character/character pure-Shield nullification, with skin 0 callback first
/// and skin 1 callback still executed if skin 0 rejects. The live actor
/// adapter handles Ice damage; grip relationships remain open.
pub fn pair(
    characters: [&mut RuntimeCharacter; 2],
    bodies: [&mut CharacterBody; 2],
    dead: [bool; 2],
) -> (bool, Option<ShieldNullification>) {
    let [a, b] = characters;
    let [body_a, body_b] = bodies;
    let mut effect = None;
    // The first live callback seeing both types Shield removes both. The
    // second callback observes the changed types and emits no duplicate.
    if a.combat.health.shield.kind == SelfShieldType::Shield
        && b.combat.health.shield.kind == SelfShieldType::Shield
        && (!dead[0] || !dead[1])
    {
        let (source, target, radius) = if !dead[0] {
            (body_a.position, body_b.position, body_a.radius)
        } else {
            (body_b.position, body_a.position, body_b.radius)
        };
        remove_pure(a, body_a);
        remove_pure(b, body_b);
        let offset: Vector = std::array::from_fn(|i| target[i] - source[i]);
        let inverse = 1.0 / (offset.iter().map(|v| v * v).sum::<f32>() as f64).sqrt() as f32;
        effect = Some(ShieldNullification {
            position: std::array::from_fn(|i| source[i] + offset[i] * inverse * radius),
            emitter_position: source,
        });
    }
    (
        !dead[0] && !dead[1] && a.accepts_owned_contact() && b.accepts_owned_contact(),
        effect,
    )
}

/// BasicCollisionFunctor executes both callbacks (non-short-circuit AND).
/// The supplied body holds the physics prediction during detection.
pub fn contact(
    character: &mut RuntimeCharacter,
    body: &mut CharacterBody,
    dead: bool,
    shield: &mut ShieldEntity,
) -> (bool, Option<ShieldNullification>) {
    // Round detector: Shield is skin 0. Wall detector: Character is skin 0.
    // The wall pure-shield branch rejects contact itself and also causes the
    // later Shield callback to reject via the kill timer. For the other
    // branches character processing does not change shield acceptance.
    let shield_accepts = !(shield.intersect_kill_timer > 0.0 || shield.hit_points <= 0.0);
    if dead {
        return (false, None);
    }
    if character.combat.health.shield.kind == SelfShieldType::Shield {
        remove_pure(character, body);
        shield.intersect_kill_timer = 0.25;
        let position = if shield.kind == magicka_sim::shield_entity::ShieldKind::Wall {
            let start: Vector =
                std::array::from_fn(|i| shield.position[i] - shield.forward[i] * shield.radius);
            let offset: Vector = std::array::from_fn(|i| {
                if i == 1 {
                    0.0
                } else {
                    body.position[i] - start[i]
                }
            });
            let span: Vector = std::array::from_fn(|i| {
                if i == 1 {
                    0.0
                } else {
                    shield.forward[i] * (2.0 * shield.radius)
                }
            });
            let dot = offset.iter().zip(span).map(|(a, b)| a * b).sum::<f32>();
            let length = (span.iter().map(|v| v * v).sum::<f32>() as f64).sqrt() as f32;
            let along = dot / length;
            std::array::from_fn(|i| {
                if i == 1 {
                    body.position[i]
                } else {
                    start[i] + shield.forward[i] * along
                }
            })
        } else {
            let offset: Vector = std::array::from_fn(|i| body.position[i] - shield.position[i]);
            let inverse_length =
                1.0 / (offset.iter().map(|v| v * v).sum::<f32>() as f64).sqrt() as f32;
            std::array::from_fn(|i| shield.position[i] + offset[i] * inverse_length * shield.radius)
        };
        return (
            false,
            Some(ShieldNullification {
                position,
                emitter_position: body.position,
            }),
        );
    }
    (shield_accepts && character.accepts_owned_contact(), None)
}
