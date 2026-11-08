//! Original Healthbars geometry and IconRenderer's settled queue layout.
//! See research/play-hud.md. Positions use top-left screen pixels.
use magicka_sim::spells::{Elements, Spell};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudQuad {
    pub bounds: [f32; 4],
    /// Atlas texels, before normalization by the texture dimensions.
    pub atlas: [f32; 4],
    pub color: [f32; 4],
}

/// Healthbars.AddHealthBar and the Healthbars vertex shader (shader 6).
#[derive(Clone, Copy, Debug)]
pub struct HealthBar {
    pub normalized: f32,
    pub width: f32,
    pub height: f32,
    pub damage_age: f32,
    pub fade: bool,
    pub color: [f32; 4],
    pub offset: [f32; 2],
}
impl HealthBar {
    pub fn quads(self, screen_position: [f32; 2]) -> Vec<HudQuad> {
        let alpha = if self.fade {
            (3.0 - self.damage_age).clamp(0.0, 1.0)
        } else {
            1.0
        };
        if alpha <= 0.0 {
            return vec![];
        }
        let flash = (0.3 - self.damage_age).clamp(0.0, 0.3) * 4.0;
        let fill = self.normalized.clamp(0.0, 1.0);
        let width = self.width.min(1.5);
        // The shader adds .5 in X and 16.5 in Y after projection.
        let x = screen_position[0] + 0.5 - 32.0 * width + self.offset[0];
        let y = screen_position[1] + 16.5 + self.offset[1];
        let frame = [1.0, 1.0, 1.0, self.color[3] * alpha];
        let color = [
            self.color[0] + flash,
            self.color[1] + flash,
            self.color[2] + flash,
            self.color[3] * alpha,
        ];
        [
            ([-4., 0., 0., 8.], [6., 73., 8., 79.], frame),
            ([64., 0., 68., 8.], [32., 73., 34., 79.], frame),
            ([0., 0., 64., 8.], [16., 73., 24., 79.], frame),
            ([0., 0., 64. * fill, 8.], [2., 81., 6., 87.], color),
        ]
        .into_iter()
        .map(|(b, atlas, color)| HudQuad {
            bounds: [
                x + b[0] * width,
                y + b[1] * self.height,
                x + b[2] * width,
                y + b[3] * self.height,
            ],
            atlas,
            color,
        })
        .collect()
    }
}

/// Shield.Update's world anchor and permanent, tint-colored health bar.
pub fn shield_bar(shield: &magicka_sim::shield_entity::ShieldEntity) -> ([f32; 3], HealthBar) {
    use magicka_sim::shield_entity::ShieldKind;
    let mut position = shield.position;
    let offset = match shield.kind {
        ShieldKind::Sphere => {
            position[2] += shield.radius;
            12.0
        }
        ShieldKind::Disc => {
            position[0] += shield.forward[0] * shield.radius;
            position[2] += shield.forward[2] * shield.radius
                + shield.forward[0].abs().powi(2) * shield.radius * 0.666
                + 0.5;
            16.0
        }
        ShieldKind::Wall => {
            position[0] += shield.forward[0] * 6.0 * shield.forward[2];
            position[2] += shield.forward[2] * 6.0 * shield.forward[2];
            24.0
        }
    };
    (
        position,
        HealthBar {
            normalized: shield.hit_points / shield.max_hit_points,
            width: shield.radius,
            height: 1.0,
            damage_age: 1.0,
            fade: false,
            color: shield.tint,
            offset: [0.0, offset],
        },
    )
}

/// Character.SelfShield.Update: stacked below character bars at capsule feet.
pub fn self_shield_bar(
    shield: &magicka_sim::character_health::SelfShield,
    radius: f32,
    character_bars: i32,
) -> Option<HealthBar> {
    use magicka_sim::character_health::SelfShieldType;
    if shield.kind == SelfShieldType::None {
        return None;
    }
    let [r, g, b] = magicka_sim::lightning::spell_color(&shield.spell);
    Some(HealthBar {
        normalized: shield.health / shield.max_health,
        width: radius,
        height: 1.0,
        damage_age: shield.time_since_damage,
        fade: false,
        color: [r, g, b, 1.0],
        offset: [0.0, 6.0 * character_bars as f32],
    })
}

/// NonPlayerCharacter.Update's stacked bars. Only the final bar tracks HP.
pub fn npc_bars(
    normalized: f32,
    count: i32,
    radius: f32,
    damage_age: f32,
    poisoned: bool,
) -> Vec<HealthBar> {
    (0..count)
        .take(512)
        .map(|i| {
            let last = i == count - 1;
            let color = if poisoned && last {
                [0., 1., 0., 1.]
            } else if i == 0 {
                [1., 0., 0., 1.]
            } else if last {
                [2., 1.5, 1., 1.]
            } else {
                [0.6, 0.6, 0.6, 1.]
            };
            HealthBar {
                normalized: if last { normalized } else { 1.0 },
                width: radius,
                height: 1.,
                damage_age: if last {
                    damage_age
                } else {
                    damage_age.max(0.3)
                },
                fade: true,
                color,
                offset: [0., i as f32 * 6.],
            }
        })
        .collect()
}

/// IconRenderer constructor/Draw: settled slots and mirrored queue brackets.
/// Grow/glow/combine/removal animations remain pending; the tome magick hint
/// is `tome_magick::hint_quads`.
pub fn queue_quads(queue: &[Spell], position: [f32; 2]) -> Vec<HudQuad> {
    if queue.is_empty() {
        return vec![];
    }
    let mut out = bracket_quads(position);
    out.extend(queue_icon_quads(queue, position));
    out
}

/// RenderData.Draw's mirrored brackets at X -/+62.5 (floored), Y +44.
pub fn bracket_quads(position: [f32; 2]) -> Vec<HudQuad> {
    vec![
        HudQuad {
            bounds: [
                (position[0] - 62.5).floor() - 4.,
                position[1] + 36.,
                (position[0] - 62.5).floor() + 4.,
                position[1] + 52.,
            ],
            atlas: [248., 8., 256., 24.],
            color: [1.; 4],
        },
        HudQuad {
            bounds: [
                (position[0] + 62.5).floor() - 4.,
                position[1] + 36.,
                (position[0] + 62.5).floor() + 4.,
                position[1] + 52.,
            ],
            atlas: [256., 8., 248., 24.],
            color: [1.; 4],
        },
    ]
}

/// Settled queue icons in the five slots (no brackets).
pub fn queue_icon_quads(queue: &[Spell], position: [f32; 2]) -> Vec<HudQuad> {
    let mut out = vec![];
    for (slot, spell) in queue.iter().take(5).enumerate() {
        let index = if spell.element == Elements::ALL {
            11
        } else {
            (f64::from(spell.element.0).ln() / 2_f64.ln() + 0.5) as usize
        };
        let x = position[0] - 50. + 25. * slot as f32;
        let y = position[1] + 44.;
        let u = (index % 5) as f32 * 50.;
        let v = 156. + (index / 5) as f32 * 49.;
        out.push(HudQuad {
            bounds: [x - 12.5, y - 12.5, x + 12.5, y + 12.5],
            atlas: [u, v, u + 50., v + 49.],
            color: [1.; 4],
        });
    }
    out
}

/// Character.Update's displayed NormalizedHitPoints (initialized to zero).
pub fn smooth_health(previous: f32, hp: f32, max_hp: f32, dt: f32) -> f32 {
    let target = hp / max_hp;
    (target + (previous - target) * 0.002_f32.powf(dt)).clamp(0., 1.)
}

/// Spell.SHIELDCOLOR (Spell::.cctor 0x06002b6e): the aura TTL bar fill.
pub const SHIELD_COLOR: [f32; 3] = [2.0, 1.5, 1.0];

/// Character.Update's aura TTL bar (0x0600082c, IL_01e1..IL_026f): every
/// self-casted aura still alive after its Execute submits a permanent bar at
/// Position + HeightOffset (capsule feet), fill TTL / StartTTL, width mRadius,
/// height 1, damage age float.MaxValue (no flash), no fade, SHIELDCOLOR, and
/// the same (0, 6 * mNumberOfHealtBars) offset as the self-shield bar.
pub fn aura_bar(
    ttl: f32,
    start_ttl: f32,
    self_casted: bool,
    radius: f32,
    character_bars: i32,
) -> Option<HealthBar> {
    // Expired auras are removed in the same loop before any bar.
    (self_casted && ttl > 0.0).then(|| HealthBar {
        normalized: ttl / start_ttl,
        width: radius,
        height: 1.0,
        damage_age: f32::MAX,
        fade: false,
        color: [SHIELD_COLOR[0], SHIELD_COLOR[1], SHIELD_COLOR[2], 1.0],
        offset: [0.0, 6.0 * character_bars as f32],
    })
}

/// Aura bars for `(ttl, start_ttl, self_casted)` auras in list order.
pub fn aura_bars(
    auras: impl IntoIterator<Item = (f32, f32, bool)>,
    radius: f32,
    character_bars: i32,
) -> Vec<HealthBar> {
    auras
        .into_iter()
        .filter_map(|(ttl, start, self_casted)| {
            aura_bar(ttl, start, self_casted, radius, character_bars)
        })
        .collect()
}

/// A live character's aura bars: native keeps one mAuras list in AddAura
/// order; the rewrite's resistance auras precede its general auras
/// (INFERRED ordering between the two lists).
pub fn character_aura_bars(c: &crate::character::RuntimeCharacter) -> Vec<HealthBar> {
    aura_bars(
        c.auras
            .iter()
            .map(|a| (a.ttl, a.start_ttl, a.self_casted))
            .chain(
                c.generic_auras
                    .iter()
                    .map(|a| (a.ttl, a.start_ttl, a.self_casted)),
            ),
        c.definition.radius,
        c.definition.health_bars,
    )
}

/// Item.Read (0x06002808) / BookOfMagick::.ctor (0x06001a6f) PickUpString:
/// the `#item_pick_up` / `#magick_pick_up` template with `#1;` replaced by
/// the white-tagged display name. An item without a display name keeps the
/// template unreplaced; books always replace (their names always exist).
pub fn pickup_text(template: &str, name: Option<&str>) -> String {
    match name {
        Some(name) => template.replace("#1;", &format!("[c=1,1,1]{name}[/c]")),
        None => template.to_owned(),
    }
}

/// Avatar.InternalPickUp (0x060016f0, RVA 0x000adaa8, IL_00b1 / IL_034a):
/// `Player.ObtainedTextBox.Initialize(scene, Maiandra14, PickUpString,
/// Vector2.Zero, forceOnScreen: true, owner: this, TTL 2)` followed by
/// `DialogManager.AddTextBox`. One box per player is reused: a repeated
/// pickup re-initializes it (keeping a still-growing scale) instead of
/// stacking a second notice. It lives in the dialog host's additional list,
/// which updates, auto-advances and drops it like the native list.
pub fn obtained_notice<W: crate::text_box_initialize::TextBoxInitHost>(
    text: &mut crate::dialog_text_host::DialogTextHost<W>,
    owner: i32,
    message: &str,
) -> Result<(), String>
where
    W::Error: std::fmt::Debug,
{
    use crate::text_box_initialize::{TextBoxFonts, TextBoxInit, TextBoxLocation, TextBoxWidget};
    let index = match text
        .additional
        .iter()
        .position(|w| w.state.owner == Some(owner))
    {
        Some(i) => i,
        None => {
            text.additional.push(TextBoxWidget::default());
            text.additional.len() - 1
        }
    };
    text.additional[index]
        .initialize(
            TextBoxInit {
                scene: text.scene,
                location: TextBoxLocation::Entity(Some(owner)),
                text: Some(message),
                minimum_size: [0.0; 2],
                force_on_screen: true,
                ttl: 2.0,
            },
            &text.strings,
            TextBoxFonts {
                body: &text.body_font,
                owner_name: &text.name_font,
            },
            &mut text.world,
            text.substitution_budget,
        )
        .map_err(|e| format!("obtained textbox: {e:?}"))
}

/// NotifierButton.ButtonIntensity per frame. KeyboardMouseController.Update
/// (0x06001230, IL_0da1/IL_0f26) and XInputController.Update (0x06002c7a,
/// IL_0a13/IL_0c14) call SetButtonIntensity(0.5) while the grip or boost
/// prompt shows and Boosts > 0; NotifierButton.Update (0x0600011b) then,
/// unless the alpha is at most epsilon, lerps toward the target 1 with weight
/// Math.Pow(0.001, dt): `Lerp(1, intensity, k)`.
pub fn notifier_intensity(previous: f32, boost_available: bool, alpha: f32, dt: f32) -> f32 {
    let intensity = if boost_available { 0.5 } else { previous };
    if alpha <= f32::EPSILON {
        return intensity;
    }
    let k = 0.001_f64.powf(f64::from(dt)) as f32;
    1.0 + (intensity - 1.0) * k
}
