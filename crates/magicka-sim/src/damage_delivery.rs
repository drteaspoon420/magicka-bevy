//! Helper.Damage extension methods and Character.GetDamageModifier.
//! Pure request generation; no network transport. See research/damage-delivery.md.
use crate::{
    character_impact::{ImpactCharacter, ImpactContext, ImpactEvent},
    character_status::StatusError,
    damage::Damage,
    impact_motion::Vector,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkRole {
    Offline,
    Client,
    Server,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackerKind {
    Other,
    Character,
    LocalAvatar,
    RemoteAvatar,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DamagePayload {
    Single(Damage),
    Collection([Damage; 5]),
}
impl DamagePayload {
    /// Apply the attacking character's BoostDamage list before InternalDamage.
    /// Call once per public delivery, before NPC aggro accounting as well.
    pub fn with_boosts(mut self, boosts: &[Damage]) -> Self {
        self.modify(boosts);
        self
    }

    fn modify(&mut self, boosts: &[Damage]) {
        let entries = match self {
            Self::Single(d) => std::slice::from_mut(d),
            Self::Collection(d) => d.as_mut_slice(),
        };
        for entry in entries {
            let mut multiplier = 1.0;
            let mut bias = 0.0;
            for boost in boosts {
                if boost.element & entry.element != 0 {
                    bias += boost.amount;
                    multiplier *= boost.magnitude;
                }
            }
            entry.amount *= multiplier;
            entry.amount += bias;
        }
    }
    pub fn request_slots(self) -> [Damage; 5] {
        match self {
            Self::Collection(d) => d,
            Self::Single(d) => {
                let mut slots = [Damage::default(); 5];
                slots[0] = d;
                slots
            }
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct DeliveryContext {
    pub network: NetworkRole,
    pub attacker_kind: AttackerKind,
    pub target_handle: u64,
    pub attacker_handle: Option<u64>,
    pub time_stamp: f64,
    pub target_position: Vector,
    pub attack_position: Vector,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageRequest {
    pub target_handle: u64,
    pub attacker_handle: Option<u64>,
    pub time_stamp: f64,
    pub relative_attack_position: Vector,
    pub damages: [Damage; 5],
}
#[derive(Clone, Debug, PartialEq)]
pub enum DeliveryEvent {
    Request(DamageRequest),
    Impact(ImpactEvent),
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreparedDelivery {
    pub payload: DamagePayload,
    pub features: u8,
    pub request: Option<DamageRequest>,
}
/// `boosts` contains only BoostDamage buffs in original list order.
pub fn prepare(
    mut payload: DamagePayload,
    ctx: DeliveryContext,
    boosts: &[Damage],
) -> PreparedDelivery {
    if ctx.attacker_kind != AttackerKind::Other {
        payload.modify(boosts);
    }
    let (features, send) = match (ctx.network, ctx.attacker_kind) {
        (NetworkRole::Client, AttackerKind::LocalAvatar) => (14, true),
        (NetworkRole::Client, AttackerKind::RemoteAvatar)
        | (NetworkRole::Server, AttackerKind::RemoteAvatar) => (8, false),
        (NetworkRole::Client, _) => (14, false),
        (NetworkRole::Server, AttackerKind::LocalAvatar) => (15, true),
        _ => (15, false),
    };
    let request = send.then(|| DamageRequest {
        target_handle: ctx.target_handle,
        attacker_handle: ctx.attacker_handle,
        time_stamp: ctx.time_stamp,
        relative_attack_position: std::array::from_fn(|i| {
            ctx.attack_position[i] - ctx.target_position[i]
        }),
        damages: payload.request_slots(),
    });
    PreparedDelivery {
        payload,
        features,
        request,
    }
}
impl ImpactCharacter {
    /// Send-request event precedes local InternalDamage events. This mutates the
    /// impact context's network/attacker facts to match the delivery route.
    pub fn deliver(
        &mut self,
        payload: DamagePayload,
        delivery: DeliveryContext,
        boosts: &[Damage],
        impact: &mut ImpactContext<'_>,
        events: &mut Vec<DeliveryEvent>,
    ) -> Result<u16, StatusError> {
        let prepared = prepare(payload, delivery, boosts);
        if let Some(request) = prepared.request {
            events.push(DeliveryEvent::Request(request));
        }
        impact.features = prepared.features;
        impact.network_client = delivery.network == NetworkRole::Client;
        impact.attacker = delivery.attacker_handle;
        impact.attacker_avatar = matches!(
            delivery.attacker_kind,
            AttackerKind::LocalAvatar | AttackerKind::RemoteAvatar
        );
        impact.attacker_local_avatar = delivery.attacker_kind == AttackerKind::LocalAvatar;
        impact.attacker_is_self = delivery.attacker_handle == Some(delivery.target_handle);
        impact.attack_position = delivery.attack_position;
        impact.motion.position = delivery.target_position;
        let mut effects = vec![];
        let result = match prepared.payload {
            DamagePayload::Single(d) => self.impact(d, impact, &mut effects),
            DamagePayload::Collection(d) => self.impact_collection(d, impact, &mut effects),
        };
        for event in &effects {
            if let crate::character_impact::ImpactEvent::Status(event) = event {
                impact.status.cues.observe(*event);
            }
        }
        events.extend(effects.into_iter().map(DeliveryEvent::Impact));
        result
    }
}
