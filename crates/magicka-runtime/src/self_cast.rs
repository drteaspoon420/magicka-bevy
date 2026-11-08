//! Runtime owner adapter for SpraySpell.CastSelf; no rendering/network transport.
use crate::character::RuntimeCharacter;
use magicka_sim::{
    character_impact::{ImpactCharacter, ImpactContext},
    character_status::StatusError,
    damage::{Damage, DamageError},
    damage_delivery::{DamagePayload, DeliveryContext, DeliveryEvent},
    self_spray::{SelfSpray, SelfSprayHost},
    spells::{Spell, SpellType},
};
#[derive(Clone, Debug, PartialEq)]
pub enum SelfCastEvent {
    StartEffect {
        element_index: usize,
        name: &'static str,
    },
    Delivery(DeliveryEvent),
}
#[derive(Debug, PartialEq)]
pub enum SelfCastError {
    NotSpray,
    Damage(DamageError),
    Status(StatusError),
}
struct Host<'a, 'b> {
    combat: &'a mut ImpactCharacter,
    impact: ImpactContext<'b>,
    delivery: DeliveryContext,
    boosts: &'a [Damage],
    events: &'a mut Vec<SelfCastEvent>,
    error: Option<StatusError>,
}
impl SelfSprayHost for Host<'_, '_> {
    fn electrocute_self(&mut self, _: f32) {} // Original Character.Electrocute is empty.
    fn start_effect(&mut self, element_index: usize, name: &'static str) {
        self.events.push(SelfCastEvent::StartEffect {
            element_index,
            name,
        });
    }
    fn damage_self(&mut self, damages: [Damage; 5], timestamp: f64) {
        self.delivery.time_stamp = timestamp;
        let mut events = vec![];
        if let Err(error) = self.combat.deliver(
            DamagePayload::Collection(damages),
            self.delivery,
            self.boosts,
            &mut self.impact,
            &mut events,
        ) {
            self.error = Some(error);
        }
        self.events
            .extend(events.into_iter().map(SelfCastEvent::Delivery));
    }
}
impl RuntimeCharacter {
    /// Uses loaded character resistances/tuning and enforces self attacker and
    /// attack position. Caller supplies live equipment, cue, network and geometry
    /// facts. The returned effect must be retained and updated by the owner.
    pub fn cast_self_spray<'a>(
        &'a mut self,
        spell: Spell,
        from_staff: bool,
        mut delivery: DeliveryContext,
        boosts: &[Damage],
        mut impact: ImpactContext<'a>,
        events: &mut Vec<SelfCastEvent>,
    ) -> Result<SelfSpray, SelfCastError> {
        if spell.spell_type() != SpellType::Spray {
            return Err(SelfCastError::NotSpray);
        }
        impact.status.base = &self.definition.resistances;
        impact.status.ethereal = self.ethereal;
        impact.status.can_have_status = self.definition.can_have_status;
        impact.status.current_spell = true;
        impact.hit_tolerance = self.definition.hit_tolerance;
        impact.stun_time = self.definition.stun_time;
        impact.bleed_rate = self.definition.bleed_rate;
        impact.motion.mass = self.definition.mass;
        delivery.attacker_handle = Some(delivery.target_handle);
        delivery.attack_position = delivery.target_position;
        let timestamp = delivery.time_stamp;
        let first_event = events.len();
        let mut host = Host {
            combat: &mut self.combat,
            impact,
            delivery,
            boosts,
            events,
            error: None,
        };
        let effect = SelfSpray::cast(spell, from_staff, timestamp, &mut host);
        let error = host.error;
        self.damage_numbers
            .extend(events[first_event..].iter().filter_map(|e| match e {
                SelfCastEvent::Delivery(DeliveryEvent::Impact(
                    magicka_sim::character_impact::ImpactEvent::Notify { amount, .. },
                )) => Some(crate::floating_text::NumberRequest::Direct { amount: *amount }),
                _ => None,
            }));
        let effect = effect.map_err(SelfCastError::Damage)?;
        if let Some(error) = error {
            return Err(SelfCastError::Status(error));
        }
        Ok(effect)
    }
}
