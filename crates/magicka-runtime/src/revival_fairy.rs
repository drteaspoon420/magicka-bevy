//! Fairy (Magicka.GameLogic.Entities.Fairy, MethodDefs 0x0600171a–0x06001730):
//! the single-player revival fairy. See research/revival-fairy.md.
//!
//! Each Avatar owns one inactive Fairy (Avatar.Initialize → MakeFairy).
//! Initialize activates it (TriggerCheckpoint with SpawnFairy and a single
//! connected non-Versus player, PlayState.SpawnFairies at scene entry, or
//! State.ApplyState at a checkpoint restart). While active it orbits its
//! owner with the `special_fairy` effect; when the owner dies it flies to the
//! owner's LastPositionOnGround (navmesh-snapped) for 2.1 s, plays
//! `fairy_death` 0.7 s in, then crashes (`special_fairy_crash`), runs the
//! Revive position overload (full health), deals a 3 m Earth knockback and
//! clears PlayState.mSpawnFairies for its owner (one use). It waits for the
//! replacement avatar, then deactivates it with a 5 s ethereal grace.
//!
//! This module is the deterministic simulation; `scene_world::fairy_live`
//! applies its events to the world and the presentation reads the exposed
//! positions.

use crate::xna_math::V3;

/// Fairy.FairyState.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FairyState {
    #[default]
    Orbiting,
    FlyToRevivePosition,
    Reviving,
    WaitingForAvatar,
}

/// FlyTowardsTarget's flight duration (seconds).
pub const FLIGHT_DURATION: f32 = 2.1;
/// `fairy_death` plays once `FLIGHT_DURATION - anim < 1.4`.
pub const DEATH_CUE_LEAD: f32 = 1.4;
/// OrbitAround's radius (mDefaultOrbitRadius is reassigned to 0.8 there).
pub const ORBIT_RADIUS: f32 = 0.8;
/// OrbitAround's horizontal revolutions per second.
pub const ORBIT_FREQUENCY: f32 = 0.25;
/// OrbitAround's vertical bob amplitude, frequency and base height.
pub const BOB_AMPLITUDE: f32 = 0.25;
pub const BOB_FREQUENCY: f32 = 0.35;
pub const BOB_HEIGHT: f32 = 1.0;
/// ReviveAvatar's CircleDamage range and Damage (Knockback, Earth, 100, 2).
pub const CRASH_RANGE: f32 = 3.0;
pub const CRASH_DAMAGE_AMOUNT: f32 = 100.0;
pub const CRASH_DAMAGE_MAGNITUDE: f32 = 2.0;
/// Camera.Release_NoMagReset time after the crash.
pub const CAMERA_RELEASE_TIME: f32 = 2.0;
/// WaitingForAvatar's TimedEthereal(5, false) grace.
pub const ETHEREAL_GRACE: f32 = 5.0;
/// Revive.Execute(position, playState) → health fraction 1.
pub const REVIVE_HEALTH: f32 = 1.0;

pub const EFFECT_ORBIT: &str = "special_fairy";
pub const EFFECT_CRASH: &str = "special_fairy_crash";
pub const CUE_DEATH: &str = "fairy_death";

/// The owner as the fairy reads it each update.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OwnerFacts {
    /// Character.Dead of the owner (Orbiting), or of the owner's Player's
    /// current avatar (WaitingForAvatar).
    pub dead: bool,
    pub position: V3,
    pub direction: V3,
    /// CharacterBody.LastPositionOnGround.
    pub last_position_on_ground: V3,
}

/// Side effects requested by one update, in native order.
#[derive(Clone, Debug, PartialEq)]
pub enum FairyEvent {
    /// ShowGreetingDialog (fairy_greet0/1 through the DialogManager).
    Greeting,
    /// EffectManager.StartEffect(special_fairy) at the owner's position.
    StartOrbitEffect { position: V3 },
    /// EffectManager.UpdatePositionDirection(mEffectRef, ...).
    MoveOrbitEffect { position: V3, direction: V3 },
    /// FindRevivePosition: Camera.Follow(this) if the camera follows players.
    FollowCamera { position: V3 },
    /// The flight moved the fairy body (Camera.Follow target).
    BodyMoved { position: V3 },
    /// AudioManager.PlayCue(Additional, fairy_death, AudioEmitter).
    DeathCue { position: V3 },
    /// EffectManager.Stop(mEffectRef).
    StopOrbitEffect,
    /// EffectManager.StartEffect(special_fairy_crash) at the revive target.
    StartCrashEffect { position: V3 },
    /// Camera.Release_NoMagReset(2) when the camera still follows the fairy.
    ReleaseCamera { time: f32 },
    /// Revive.GetInstance().Execute(target, playState) (full health).
    Revive { position: V3, health: f32 },
    /// Helper.CircleDamage(this, PlayTime, this, origin, 3, Knockback Earth 100/2).
    CircleDamage { origin: V3, range: f32 },
    /// PlayState.RemoveFairyFrom(owner): mSpawnFairies[owner] = false.
    RemoveFairyFrom,
    /// WaitingForAvatar saw the living avatar: Active = false and
    /// TimedEthereal(0, true) + TimedEthereal(5, false) on it.
    Deactivated { ethereal_grace: f32 },
    /// Kill: Stop and Active = false.
    Killed,
}

/// One Avatar.RevivalFairy.
#[derive(Clone, Debug, PartialEq)]
pub struct RevivalFairy {
    /// Fairy.Active (Removable while false).
    pub active: bool,
    pub state: FairyState,
    /// mBody.Position: the owner while orbiting, the flight path afterwards.
    pub position: V3,
    /// The special_fairy effect's position and direction (presentation).
    pub effect_position: V3,
    pub effect_direction: V3,
    /// mDeathPos: the body position when the flight began.
    pub death_position: V3,
    /// mRevivalTarget (navmesh-snapped LastPositionOnGround).
    pub revival_target: V3,
    /// mAnimPosition (flight time).
    pub anim_position: f32,
    /// mTime (orbit phase).
    pub time: f32,
    /// mOrbitRadius (0.5 until the first OrbitAround).
    pub orbit_radius: f32,
    /// WaitBeforeRevive (0 unless a host sets it).
    pub wait_before_revive: f32,
    pub death_sound_played: bool,
    /// EffectManager.IsActive(mEffectRef).
    pub effect_active: bool,
    /// Camera.Follow(this) was issued and not released.
    pub camera_following: bool,
    /// Greeting dialogs requested (presentation pending).
    pub greetings: u32,
    /// Revivals performed by this fairy (diagnostics).
    pub revivals: u32,
}

impl Default for RevivalFairy {
    fn default() -> Self {
        Self {
            active: false,
            state: FairyState::Orbiting,
            position: [0.0; 3],
            effect_position: [0.0; 3],
            effect_direction: [0.0, 0.0, -1.0],
            death_position: [0.0; 3],
            revival_target: [0.0; 3],
            anim_position: 0.0,
            time: 0.0,
            orbit_radius: 0.5,
            wait_before_revive: 0.0,
            death_sound_played: false,
            effect_active: false,
            camera_following: false,
            greetings: 0,
            revivals: 0,
        }
    }
}

/// Microsoft.Xna.Framework.Vector3.SmoothStep.
pub fn smooth_step(a: V3, b: V3, amount: f32) -> V3 {
    let t = amount.clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// OrbitAround's offset from the owner at orbit phase `time`.
pub fn orbit_offset(time: f32) -> V3 {
    let angle = std::f32::consts::PI * 2.0 * ORBIT_FREQUENCY * time;
    let dx = ORBIT_RADIUS * angle.cos();
    let dz = ORBIT_RADIUS * angle.sin();
    let bob = std::f32::consts::PI * 2.0 * BOB_FREQUENCY * time;
    let dy = BOB_AMPLITUDE * bob.sin() + BOB_HEIGHT;
    [dx, dy, dz]
}

/// FlyTowardsTarget's effect position at flight time `anim` (< 2.1 s): a
/// spiral of radius 1.5 that grows for the first second and shrinks for the
/// second, four turns per second, rising 2 m over the second second and
/// dropping in the last 0.1 s; the first second blends from `death`.
pub fn flight_effect_position(anim: f32, death: V3, target: V3) -> V3 {
    let radius = if anim < 1.0 {
        anim / 1.0
    } else {
        1.0 - (anim - 1.0) / 1.0
    } * 1.5;
    let angle = (anim - 1.0) * (std::f32::consts::PI * 4.0);
    let height = if anim > 2.0 {
        1.0 - (anim - 1.0 - 1.0) / 0.1
    } else if anim > 1.0 {
        (anim - 1.0) / 1.0
    } else {
        0.0
    } * 2.0;
    let spiral = [
        target[0] + angle.cos() * radius,
        target[1] + height,
        target[2] + angle.sin() * radius,
    ];
    if anim < 1.0 {
        smooth_step(death, spiral, anim / 1.0)
    } else {
        spiral
    }
}

/// FlyTowardsTarget's body position: a smooth step from `death` to `target`
/// over the first second, then the target.
pub fn flight_body_position(anim: f32, death: V3, target: V3) -> V3 {
    if anim < 1.0 {
        smooth_step(death, target, anim / 1.0)
    } else {
        target
    }
}

impl RevivalFairy {
    /// Fairy.Initialize(playState, showGreeting) + Play. The greeting only
    /// shows when the fairy was inactive. Returns the requested effects.
    pub fn initialize(&mut self, show_greeting: bool, owner_position: V3) -> Vec<FairyEvent> {
        let mut events = Vec::new();
        if !self.active && show_greeting {
            self.greetings += 1;
            events.push(FairyEvent::Greeting);
        }
        self.active = true;
        // Play.
        self.state = FairyState::Orbiting;
        if !self.effect_active {
            self.effect_active = true;
            self.effect_position = owner_position;
            events.push(FairyEvent::StartOrbitEffect {
                position: owner_position,
            });
        }
        self.position = owner_position;
        self.death_sound_played = false;
        events
    }

    /// Fairy.Kill: Stop the effect and deactivate (the state is kept; the
    /// next Initialize's Play resets it to Orbiting).
    pub fn kill(&mut self) -> Vec<FairyEvent> {
        let mut events = Vec::new();
        if self.effect_active {
            self.effect_active = false;
            events.push(FairyEvent::StopOrbitEffect);
        }
        self.active = false;
        self.camera_following = false;
        events.push(FairyEvent::Killed);
        events
    }

    /// Fairy.Update. `owner` is None when the owner no longer exists;
    /// `snap` is NavMesh.GetNearestPosition with MovementProperties.Default.
    pub fn update(
        &mut self,
        dt: f32,
        owner: Option<&OwnerFacts>,
        snap: &dyn Fn(V3) -> V3,
    ) -> Vec<FairyEvent> {
        let mut events = Vec::new();
        if !self.active {
            return events;
        }
        match self.state {
            FairyState::Orbiting => {
                if let Some(owner) = owner.filter(|o| o.dead) {
                    if self.wait_before_revive > 0.0 {
                        self.wait_before_revive -= dt;
                    }
                    if self.wait_before_revive <= 0.0 {
                        self.wait_before_revive = 0.0;
                        self.find_revive_position(owner, snap, &mut events);
                    } else {
                        self.orbit_around(dt, owner, &mut events);
                    }
                } else if let Some(owner) = owner {
                    self.orbit_around(dt, owner, &mut events);
                }
            }
            FairyState::FlyToRevivePosition => self.fly_towards_target(dt, &mut events),
            FairyState::Reviving => self.revive_avatar(&mut events),
            FairyState::WaitingForAvatar => {
                if owner.is_some_and(|o| !o.dead) {
                    self.active = false;
                    events.push(FairyEvent::Deactivated {
                        ethereal_grace: ETHEREAL_GRACE,
                    });
                }
            }
        }
        events
    }

    fn find_revive_position(
        &mut self,
        owner: &OwnerFacts,
        snap: &dyn Fn(V3) -> V3,
        events: &mut Vec<FairyEvent>,
    ) {
        events.push(FairyEvent::FollowCamera {
            position: self.position,
        });
        self.anim_position = 0.0;
        self.death_position = self.position;
        self.revival_target = snap(owner.last_position_on_ground);
        self.state = FairyState::FlyToRevivePosition;
        self.orbit_radius = ORBIT_RADIUS;
    }

    fn orbit_around(&mut self, dt: f32, owner: &OwnerFacts, events: &mut Vec<FairyEvent>) {
        self.orbit_radius = ORBIT_RADIUS;
        let offset = orbit_offset(self.time);
        let position = [
            owner.position[0] + offset[0],
            owner.position[1] + offset[1],
            owner.position[2] + offset[2],
        ];
        self.time += dt;
        self.position = owner.position;
        self.effect_position = position;
        self.effect_direction = owner.direction;
        events.push(FairyEvent::MoveOrbitEffect {
            position,
            direction: owner.direction,
        });
    }

    fn fly_towards_target(&mut self, dt: f32, events: &mut Vec<FairyEvent>) {
        self.anim_position += dt;
        if FLIGHT_DURATION - self.anim_position < DEATH_CUE_LEAD && !self.death_sound_played {
            self.death_sound_played = true;
            events.push(FairyEvent::DeathCue {
                position: self.position,
            });
        }
        if self.anim_position >= FLIGHT_DURATION {
            self.state = FairyState::Reviving;
            return;
        }
        let effect =
            flight_effect_position(self.anim_position, self.death_position, self.revival_target);
        self.effect_position = effect;
        self.effect_direction = [0.0, 0.0, -1.0];
        events.push(FairyEvent::MoveOrbitEffect {
            position: effect,
            direction: [0.0, 0.0, -1.0],
        });
        self.position =
            flight_body_position(self.anim_position, self.death_position, self.revival_target);
        events.push(FairyEvent::BodyMoved {
            position: self.position,
        });
    }

    fn revive_avatar(&mut self, events: &mut Vec<FairyEvent>) {
        self.effect_active = false;
        events.push(FairyEvent::StopOrbitEffect);
        events.push(FairyEvent::StartCrashEffect {
            position: self.revival_target,
        });
        if self.camera_following {
            self.camera_following = false;
            events.push(FairyEvent::ReleaseCamera {
                time: CAMERA_RELEASE_TIME,
            });
        }
        events.push(FairyEvent::Revive {
            position: self.revival_target,
            health: REVIVE_HEALTH,
        });
        self.revivals += 1;
        self.state = FairyState::WaitingForAvatar;
        events.push(FairyEvent::CircleDamage {
            origin: self.position,
            range: CRASH_RANGE,
        });
        events.push(FairyEvent::RemoveFairyFrom);
        self.death_sound_played = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(dead: bool) -> OwnerFacts {
        OwnerFacts {
            dead,
            position: [10.0, 0.0, 5.0],
            direction: [0.0, 0.0, 1.0],
            last_position_on_ground: [12.0, 0.5, 7.0],
        }
    }

    #[test]
    fn orbit_math_matches_orbit_around() {
        assert_eq!(orbit_offset(0.0), [0.8, 1.0, 0.0]);
        // A quarter revolution at t = 1 s: cos(π/2) ≈ 0, sin = 1.
        let o = orbit_offset(1.0);
        assert!(o[0].abs() < 1e-6 && (o[2] - 0.8).abs() < 1e-6);
        assert!((o[1] - (1.0 + 0.25 * (std::f32::consts::PI * 0.7).sin())).abs() < 1e-6);
    }

    #[test]
    fn flight_positions_start_at_death_and_end_at_target() {
        let death = [0.0, 0.0, 0.0];
        let target = [4.0, 1.0, 4.0];
        assert_eq!(flight_body_position(0.0, death, target), death);
        assert_eq!(flight_body_position(1.0, death, target), target);
        assert_eq!(flight_body_position(2.0, death, target), target);
        assert_eq!(flight_effect_position(0.0, death, target), death);
        // At 1 s: radius 1.5 at angle 0, no height.
        assert_eq!(flight_effect_position(1.0, death, target), [5.5, 1.0, 4.0]);
        // At 2 s: radius 0, height 2.
        let p = flight_effect_position(2.0, death, target);
        assert!((p[0] - 4.0).abs() < 1e-5 && (p[1] - 3.0).abs() < 1e-5);
        // At 2.05 s the height has halved.
        let p = flight_effect_position(2.05, death, target);
        assert!((p[1] - 2.0).abs() < 1e-3);
    }

    #[test]
    fn one_use_revival_sequence() {
        let snap = |p: V3| [p[0], 0.0, p[2]];
        let mut f = RevivalFairy::default();
        assert_eq!(
            f.initialize(true, [10.0, 0.0, 5.0]),
            vec![
                FairyEvent::Greeting,
                FairyEvent::StartOrbitEffect {
                    position: [10.0, 0.0, 5.0]
                }
            ]
        );
        // A second Initialize while active neither greets nor restarts.
        assert!(f.initialize(true, [10.0, 0.0, 5.0]).is_empty());
        let alive = owner(false);
        let e = f.update(0.5, Some(&alive), &snap);
        assert!(matches!(e[0], FairyEvent::MoveOrbitEffect { .. }));
        assert_eq!(f.time, 0.5);
        assert_eq!(f.position, alive.position);
        let dead = owner(true);
        let e = f.update(0.1, Some(&dead), &snap);
        assert_eq!(
            e,
            vec![FairyEvent::FollowCamera {
                position: [10.0, 0.0, 5.0]
            }]
        );
        assert_eq!(f.state, FairyState::FlyToRevivePosition);
        assert_eq!(f.revival_target, [12.0, 0.0, 7.0]);
        f.camera_following = true;
        let mut cue_at: Option<f32> = None;
        let mut t = 0.0f32;
        while f.state == FairyState::FlyToRevivePosition {
            t += 0.1;
            for e in f.update(0.1, Some(&dead), &snap) {
                if let FairyEvent::DeathCue { .. } = e {
                    assert!(cue_at.is_none());
                    cue_at = Some(t);
                }
            }
        }
        // 2.1 - anim < 1.4 first holds around anim = 0.7 (float steps).
        let cue_at = cue_at.unwrap();
        assert!((0.65..0.85).contains(&cue_at), "cue at {cue_at}");
        assert!((t - 2.1).abs() < 0.15, "flight ended at {t}");
        assert_eq!(f.state, FairyState::Reviving);
        assert_eq!(f.position, f.revival_target);
        let e = f.update(0.1, Some(&dead), &snap);
        assert_eq!(
            e,
            vec![
                FairyEvent::StopOrbitEffect,
                FairyEvent::StartCrashEffect {
                    position: [12.0, 0.0, 7.0]
                },
                FairyEvent::ReleaseCamera { time: 2.0 },
                FairyEvent::Revive {
                    position: [12.0, 0.0, 7.0],
                    health: 1.0
                },
                FairyEvent::CircleDamage {
                    origin: [12.0, 0.0, 7.0],
                    range: 3.0
                },
                FairyEvent::RemoveFairyFrom,
            ]
        );
        assert_eq!(f.state, FairyState::WaitingForAvatar);
        assert!(f.active);
        assert!(f.update(0.1, Some(&dead), &snap).is_empty());
        assert_eq!(
            f.update(0.1, Some(&alive), &snap),
            vec![FairyEvent::Deactivated {
                ethereal_grace: 5.0
            }]
        );
        assert!(!f.active);
        assert!(f.update(0.1, Some(&dead), &snap).is_empty());
    }

    #[test]
    fn wait_before_revive_delays_the_flight() {
        let snap = |p: V3| p;
        let mut f = RevivalFairy::default();
        f.initialize(false, [0.0; 3]);
        f.wait_before_revive = 0.25;
        let dead = owner(true);
        let e = f.update(0.1, Some(&dead), &snap);
        assert!(matches!(e[0], FairyEvent::MoveOrbitEffect { .. }));
        f.update(0.1, Some(&dead), &snap);
        assert_eq!(f.state, FairyState::Orbiting);
        f.update(0.1, Some(&dead), &snap);
        assert_eq!(f.state, FairyState::FlyToRevivePosition);
        assert_eq!(f.wait_before_revive, 0.0);
    }

    #[test]
    fn kill_stops_the_effect_and_deactivates() {
        let mut f = RevivalFairy::default();
        f.initialize(false, [1.0; 3]);
        assert_eq!(
            f.kill(),
            vec![FairyEvent::StopOrbitEffect, FairyEvent::Killed]
        );
        assert!(!f.active && !f.effect_active);
        assert_eq!(f.kill(), vec![FairyEvent::Killed]);
    }
}
