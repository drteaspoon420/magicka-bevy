//! Shared Character spell dispatch and effect lifetime for avatars and NPCs.
use super::*;

pub(crate) struct CharacterSpellHost<'a> {
    pub owner_is_character: bool,
    pub avatar: bool,
    pub handle: u16,
    pub equipment: &'a [Resistance; 11],
    pub character: &'a mut RuntimeCharacter,
    pub current_spell: &'a mut Option<SpellEffect>,
    pub current_spell_definition: &'a mut Option<(CastType, Spell)>,
    pub events: &'a mut Vec<AvatarEvent>,
    pub rail_kills: &'a mut Vec<magicka_sim::railgun::RailId>,
    pub source: Option<SpraySource>,
    pub play_time: f64,
}
impl CharacterSpellHost<'_> {
    pub(crate) fn cast(&mut self, spell: Spell, cast_type: CastType, from_staff: bool) {
        self.cast_inner(spell, cast_type, from_staff, None);
    }

    /// `new XSpell().CastArea(spell, owner, fromStaff)` without Spell.Cast's
    /// element dispatch (e.g. WardedExpulsion's PushSpell with a Shield spell).
    pub(crate) fn cast_effect(
        &mut self,
        kind: EffectKind,
        spell: Spell,
        cast_type: CastType,
        from_staff: bool,
    ) {
        self.cast_inner(spell, cast_type, from_staff, Some(kind));
    }

    fn cast_inner(
        &mut self,
        spell: Spell,
        cast_type: CastType,
        from_staff: bool,
        kind: Option<EffectKind>,
    ) {
        self.events.push(AvatarEvent::SpellCast {
            spell,
            cast_type,
            spell_power: self.character.spell_power,
        });
        let greased = self.character.combat.statuses.has(Status::Greased);
        let general = &self.character.buffs.general;
        let (ttl_modifier, range_modifier) = if self.owner_is_character {
            (general.spell_ttl_modifier(), general.spell_range_modifier())
        } else {
            ((1.0, 0.0), (1.0, 0.0))
        };
        let mut phase = CastPhase {
            ttl_modifier,
            range_modifier,
            current: None,
            events: self.events,
            source: self.source.unwrap_or(SpraySource {
                owner_handle: 0,
                owner_position: [0.0; 3],
                owner_direction: [0.0, 0.0, -1.0],
                cast_position: [0.0; 3],
                cast_direction: [0.0, 0.0, -1.0],
                weapon_position: [0.0; 3],
                weapon_direction: [0.0, 0.0, -1.0],
            }),
            facts: ForceCastFacts {
                from_staff,
                character: self.owner_is_character,
                panic_cast: false,
                greased,
                timestamp: self.play_time,
            },
            owner_damage: Vec::new(),
            owner_shield: None,
            power: &mut self.character.spell_power,
            additional_radius: self.character.additional_push_radius,
        };
        match kind {
            Some(kind) => SpellCastHost::cast(&mut phase, kind, spell, cast_type, from_staff),
            None => spell.cast_with(
                cast_type,
                from_staff,
                CastOwner {
                    avatar: self.avatar,
                    remote: false,
                },
                &mut phase,
            ),
        }
        let CastPhase {
            current,
            owner_damage,
            owner_shield,
            ..
        } = phase;
        *self.current_spell = current;
        *self.current_spell_definition = self.current_spell.as_ref().map(|_| (cast_type, spell));
        if let Some((spell, plan)) = owner_shield {
            apply_shield_self(self.character, &spell, &plan);
        }
        // SpraySpell.CastForce greased-owner fire (single payload, public
        // route, CastSource translation as attack position).
        for (payload, timestamp, attack_position) in owner_damage {
            let payload = payload.with_boosts(&self.character.buffs.damage_boosts());
            self.character.status_current_spell = self.current_spell.is_some();
            let (position, facing) = self.source.map_or(([0.0; 3], [0.0, 0.0, -1.0]), |s| {
                (s.owner_position, s.owner_direction)
            });
            let hit_points_before = self.character.combat.health.hit_points;
            let result = deliver_to_character(
                self.character,
                self.equipment,
                self.avatar,
                self.handle,
                position,
                facing,
                payload,
                Some(self.handle),
                attack_position,
                timestamp,
            );
            apply_status_spell_stop(
                self.character,
                self.current_spell,
                self.events,
                self.rail_kills,
            );
            if cast_type == CastType::SelfCast {
                self.events.push(AvatarEvent::SelfCastApplied {
                    result,
                    hit_points_before,
                    hit_points_after: self.character.combat.health.hit_points,
                });
            }
        }
    }
    pub(crate) fn update<
        W: SprayWorld
            + PushWorld
            + magicka_sim::lightning::LightningScene
            + magicka_sim::railgun::RailControl,
    >(
        &mut self,
        body: &mut CharacterBody,
        dt: f32,
        world: &mut W,
        cast_type: CastType,
        button_none: bool,
        animation_finished: bool,
        rng: &mut u64,
    ) -> bool {
        let turn = self.character.definition.turn_speed;
        let source = self.source.expect("live spell source");
        let owner = EffectOwner {
            avatar: self.avatar,
            dead: self.character.combat.dead
                && !self.character.combat.cannot_die_without_explicit_kill,
            button_none,
            cast_none: cast_type == CastType::Blast,
        };
        let Some(effect) = self.current_spell.as_mut() else {
            body.owner.turn_speed = turn;
            return false;
        };
        let (keep, turn_factor) = match effect {
            SpellEffect::Push(push) => {
                let (keep, hits) = push.update(dt, self.handle, body.position, world);
                self.events.extend(
                    hits.into_iter()
                        .map(|target| AvatarEvent::PushHit { target }),
                );
                (keep, 0.)
            }
            SpellEffect::ForceSpray(spray) => spray.update(
                dt,
                owner,
                false,
                source,
                &mut LiveSprayHost {
                    world,
                    events: self.events,
                },
            ),
            SpellEffect::SelfSpray(spray) => spray.update(
                dt,
                owner,
                false,
                |_, _| {},
                |l| {
                    if l.active {
                        self.events.push(AvatarEvent::SelfEffectsStopped);
                    }
                    l.active = false;
                },
            ),
            SpellEffect::SelfBeam(beam) => beam.update(
                dt,
                owner,
                &mut LiveSelfBeamHost {
                    character: self.character,
                    body,
                    equipment: self.equipment,
                    avatar: self.avatar,
                    handle: self.handle,
                    events: self.events,
                },
            ),
            SpellEffect::ShieldSelf(shield) => shield.update(dt, animation_finished),
            SpellEffect::ShieldWeapon {
                shield,
                spell,
                has_cast,
            } => {
                // DeInitialize runs AnimationEnd when it has not run yet.
                let active = shield.lifecycle.active;
                let result = shield.update(dt, animation_finished);
                if active && !shield.lifecycle.active && !*has_cast {
                    *has_cast = true;
                    self.events
                        .push(AvatarEvent::ShieldWeapon { spell: *spell });
                }
                result
            }
            SpellEffect::SelfLightning(lightning) => lightning.update(dt, owner, |l| {
                if l.active {
                    self.events.push(AvatarEvent::SelfEffectsStopped);
                }
                l.active = false;
            }),
            SpellEffect::Projectile(projectile) => {
                let launch_source = magicka_sim::projectile_launch::LaunchSource {
                    owner_position: body.position,
                    owner_direction: source.owner_direction,
                    cast_position: source.cast_position,
                    may_spawn: true,
                };
                let mut host = LaunchHost {
                    world,
                    events: self.events,
                    rng,
                };
                projectile.update(dt, owner, launch_source, &mut host)
            }
            SpellEffect::Lightning(lightning) => {
                let source = magicka_sim::lightning::LightningSource {
                    owner: self.handle,
                    owner_direction: body.direction(),
                    cast_direction: source.cast_direction,
                    cast_position: source.cast_position,
                    weapon_direction: source.weapon_direction,
                    weapon_position: source.weapon_position,
                    character: self.owner_is_character,
                    wet: self
                        .character
                        .combat
                        .statuses
                        .has(magicka_sim::status::Status::Wet)
                        && !self.character.wet_lightning_passive,
                    play_time: self.play_time,
                };
                let mut host = LightningHost {
                    world,
                    events: self.events,
                    rng,
                };
                lightning.update(dt, owner, source, &mut host)
            }
            SpellEffect::RailGun(rail) => {
                let source = magicka_sim::railgun::RailSource {
                    owner: self.handle,
                    owner_position: body.position,
                    owner_direction: body.direction(),
                    cast_position: source.cast_position,
                };
                rail.update(dt, owner, source, world)
            }
            SpellEffect::SprayArea(area) => area.update(dt, owner, |l| l.active = false),
            SpellEffect::SprayWeapon(weapon) => {
                self.events.push(AvatarEvent::SelfEffectsPosition {
                    position: source.weapon_position,
                    direction: source.owner_direction,
                });
                let active = weapon.lifecycle.active;
                let result = weapon.update(dt, owner);
                if active && !weapon.lifecycle.active {
                    self.events.push(AvatarEvent::SelfEffectsStopped);
                }
                result
            }
            SpellEffect::GroundArea(lifecycle) => {
                lifecycle.active = false;
                let (keep, _) =
                    lifecycle.update(dt, EffectKind::Projectile, owner, |l| l.active = false);
                (keep, 0.0)
            }
            SpellEffect::RailGunArea(lifecycle) => {
                // ttl 0 - dt <= 0: DeInitialize, no base CastUpdate.
                lifecycle.active = false;
                (false, 0.0325)
            }
            SpellEffect::Unreconstructed { kind, lifecycle } => {
                let kind = *kind;
                lifecycle.update(dt, kind, owner, |l| l.active = false)
            }
            SpellEffect::Weapon {
                kind,
                lifecycle,
                ice_blade,
                ..
            } => {
                let kind = *kind;
                let result = lifecycle.update(dt, kind, owner, |l| l.active = false);
                // DeInitialize kills a blade AnimationEnd did not.
                if !lifecycle.active {
                    if let Some(blade) = ice_blade.take() {
                        self.events.push(blade.strike());
                    }
                }
                result
            }
        };
        body.owner.turn_speed = turn * turn_factor;
        keep
    }
    pub(crate) fn animation_end(&mut self) {
        if let Some(SpellEffect::ShieldWeapon {
            spell, has_cast, ..
        }) = self.current_spell.as_mut()
        {
            // ShieldSpell.AnimationEnd sets mHasCast and places its entity
            // without checking mHasCast first.
            *has_cast = true;
            self.events
                .push(AvatarEvent::ShieldWeapon { spell: *spell });
            return;
        }
        if let Some(SpellEffect::Weapon {
            kind: EffectKind::Projectile,
            ice_blade,
            ..
        }) = self.current_spell.as_mut()
        {
            if let Some(blade) = ice_blade.take() {
                self.events.push(blade.strike());
            }
        }
        let Some(SpellEffect::Weapon {
            kind: EffectKind::Projectile,
            spell,
            timestamp,
            ..
        }) = self.current_spell.as_ref()
        else {
            return;
        };
        if spell.element.0 & E::EARTH.0 == 0 {
            return;
        }
        let Some(source) = self.source else { return };
        let mut earth = *spell;
        // spell.IceMagnitude = 0; Element &= ~Ice.
        if let Some(i) = E::BY_INDEX.iter().position(|x| *x == E::ICE) {
            earth.mag[i] = 0.0;
        }
        earth.element = E(earth.element.0 & !E::ICE.0);
        let Ok(list) =
            magicka_sim::damage::calculate_damage(&earth, SpellType::Projectile, CastType::Weapon)
        else {
            return;
        };
        let mut damages = [Damage::default(); 5];
        for (slot, d) in damages.iter_mut().zip(list) {
            *slot = d;
        }
        let d = source.owner_direction;
        self.events.push(AvatarEvent::UndergroundAttack {
            position: source.weapon_position,
            velocity: [d[0] * 40.0, d[2] * 40.0],
            range: 15.0,
            damages,
            timestamp: *timestamp,
        });
    }
    pub(crate) fn end(&mut self) {
        if let Some(mut effect) = self.current_spell.take() {
            stop_spell_effect(&mut effect, self.events, self.rail_kills);
        }
        *self.current_spell_definition = None;
        self.character.status_current_spell = false;
    }
    pub(crate) fn stop_for_status(&mut self) {
        apply_status_spell_stop(
            self.character,
            self.current_spell,
            self.events,
            self.rail_kills,
        );
    }
}
