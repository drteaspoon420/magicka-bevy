//! Asset-backed Jormungandr boss. Native contracts and remaining host gaps:
//! research/boss-jormungandr.md. No synthetic animation durations.
use crate::{
    animation_controller::AnimationController,
    boss_health::BossHealth,
    character_pose::{CharacterSkeleton, Pose},
};
use magicka_content::skinned::{self, Mat, Resolved};
use magicka_sim::{
    damage::{attack, Damage},
    spells::Elements as E,
    spray_pulse::Segment,
    status::Status as S,
};
use std::collections::{BTreeMap, HashMap};
pub const MODEL: &str = "Models/Bosses/Jormungandr/Jormungandr";
pub const CLIPS: [&str; 12] = [
    "intro",
    "dive_attack",
    "dive_recoil",
    "emerge",
    "submerge",
    "idle",
    "spit",
    "venom",
    "attack",
    "hit",
    "die",
    "die_above",
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Sleep,
    Intro,
    Underground,
    Leap,
    Risen,
    Spit,
    Bite,
    HitRisen,
    HitLeap,
    Dead,
}
#[derive(Clone, Debug)]
pub struct Spine {
    pub axis: Segment,
    pub radius: f32,
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub parent: Mat,
    pub local_poses: Vec<Pose>,
    pub bones: Vec<Mat>,
    pub head: [f32; 3],
    pub spine: Vec<Spine>,
    pub visible: bool,
    /// Capsule origins remain available while the mesh waits underground.
    pub body_points: Vec<[f32; 3]>,
}
#[derive(Default, Debug)]
pub struct Events {
    pub sounds: Vec<&'static str>,
    pub spit: Vec<([f32; 3], [f32; 3])>,
    pub clear_barriers: Option<([f32; 3], f32)>,
    pub tail_whip: Option<[f32; 3]>,
}
pub struct Jormungandr {
    pub health: BossHealth,
    pub state: State,
    pub last_state: State,
    pub dead: bool,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub orientation: Mat,
    pub controller: AnimationController,
    pub skeleton: CharacterSkeleton,
    pub frame: Frame,
    pub ground_effects: [Option<u64>; 3],
    target_locked: bool,
    pub flash: f32,
    pub visible: bool,
    pub update_animation: bool,
    pub idle_timer: f32,
    pub attack_count: i32,
    pub hit: bool,
    pub hits: BTreeMap<u16, f32>,
    pub spine_hits: BTreeMap<u16, f32>,
    durations: HashMap<String, f32>,
    spine_lengths: [f32; 25],
    played: bool,
    cleared: bool,
    growled: bool,
    whipped: bool,
    whip_damaged: bool,
    mid_leap: bool,
    spit_timer: f32,
    spit_count: u8,
    rng: u64,
    /// Set by the world each step (`boss_scale.rs`).
    pub damage_divisor: f32,
}
pub fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>().sqrt()
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn normalized(a: [f32; 3]) -> [f32; 3] {
    let len = distance(a, [0.; 3]);
    a.map(|v| v / len)
}
fn translation(m: &Mat) -> [f32; 3] {
    [m[12], m[13], m[14]]
}
fn transform(p: [f32; 3], d: [f32; 3]) -> Mat {
    let right = normalized([-d[2], 0., d[0]]);
    let up = [
        -right[2] * d[1],
        right[2] * d[0] - right[0] * d[2],
        right[0] * d[1],
    ];
    let mut m = skinned::IDENTITY;
    m[..3].copy_from_slice(&right);
    m[4..7].copy_from_slice(&up);
    m[8..11].copy_from_slice(&d.map(|v| -v));
    m[12..15].copy_from_slice(&p);
    m
}
fn spine_name(i: usize) -> String {
    match i {
        0..=10 => format!("back{:02}", 11 - i),
        11 => "center".into(),
        12..=23 => format!("front{:02}", i - 11),
        24 => "NeckBase".into(),
        _ => "NeckMid".into(),
    }
}
impl Jormungandr {
    pub fn new(asset: Resolved, orientation: Mat, seed: u64) -> Result<Self, String> {
        let skeleton = CharacterSkeleton::new(asset.skeleton, asset.clips);
        let durations = skeleton.clip_durations();
        for clip in CLIPS {
            if !durations.contains_key(clip) {
                return Err(format!("Jormungandr missing clip {clip}"));
            }
        }
        for name in (0..26)
            .map(spine_name)
            .chain(["HeadBase".into(), "NeckEnd".into()])
        {
            if skeleton.bone(&name).is_none() {
                return Err(format!("Jormungandr missing bone {name}"));
            }
        }
        let mut controller = AnimationController::default();
        controller.start_named_clip("intro", durations["intro"], false);
        controller.stop();
        // Native capsule lengths come from bind matrices, before any clip is
        // sampled. An animated intro pose must not redefine their lengths.
        let bones = vec![skinned::IDENTITY; skeleton.skeleton.bones.len()];
        let points: Vec<_> = (0..26)
            .map(|i| {
                translation(
                    &crate::animation_effects::bone_orientation(
                        &skeleton.skeleton,
                        &bones,
                        &spine_name(i),
                    )
                    .unwrap(),
                )
            })
            .collect();
        let spine_lengths = std::array::from_fn(|i| distance(points[i], points[i + 1]));
        let mut health = BossHealth::new(10000.);
        health.resistances[0].multiplier = 0.75;
        health.resistances[8].multiplier = 0.75;
        let mut actor = Self {
            health,
            state: State::Sleep,
            last_state: State::Sleep,
            dead: false,
            position: translation(&orientation),
            direction: [-orientation[8], -orientation[9], -orientation[10]],
            orientation,
            controller,
            skeleton,
            frame: Frame {
                parent: orientation,
                local_poses: Vec::new(),
                bones: Vec::new(),
                head: [0., -100., 0.],
                spine: Vec::new(),
                visible: true,
                body_points: points.iter().copied().skip(1).collect(),
            },
            ground_effects: [None; 3],
            target_locked: false,
            flash: 0.,
            visible: true,
            update_animation: false,
            idle_timer: 0.,
            attack_count: 0,
            hit: false,
            hits: BTreeMap::new(),
            spine_hits: BTreeMap::new(),
            durations,
            spine_lengths,
            played: false,
            cleared: false,
            growled: false,
            whipped: false,
            whip_damaged: false,
            mid_leap: false,
            spit_timer: 0.,
            spit_count: 0,
            rng: seed.max(1),
            damage_divisor: 1.0,
        };
        actor.sample();
        Ok(actor)
    }
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn warning_time(&self) -> f32 {
        0.75 + 1.75 * self.health.hp / 10000.
    }
    fn clip(&mut self, name: &str, fade: f32, looped: bool) {
        if fade == 0. {
            self.controller
                .start_named_clip(name, self.durations[name], looped);
        } else {
            self.controller
                .crossfade_named_clip(name, self.durations[name], fade, looped);
        }
    }
    fn change(&mut self, state: State, out: &mut Events) {
        if state == self.state {
            return;
        }
        if matches!(
            self.state,
            State::Leap | State::Bite | State::HitRisen | State::HitLeap
        ) {
            self.hit = false;
        }
        self.last_state = self.state;
        self.state = state;
        self.played = false;
        self.cleared = false;
        match state {
            State::Intro => {
                self.clip("intro", 0., false);
                self.visible = true;
                self.update_animation = true;
                out.sounds.push("boss_jormungandr_wakeup");
            }
            State::Underground => {
                self.visible = false;
                self.update_animation = false;
                self.idle_timer = 0.;
                self.attack_count = 0;
                self.hit = false;
                self.hits.clear();
                self.spine_hits.clear();
                self.health.statuses = [Default::default(); 9];
                self.health.mask = 0;
            }
            State::Risen => {
                self.idle_timer = 0.;
                self.spine_hits.clear();
                if self.controller.clip_name() == Some("emerge") {
                    out.sounds.push("boss_jormungandr_unburrow");
                }
            }
            State::Leap => {
                self.clip("dive_attack", 0., false);
                self.visible = false;
                self.update_animation = false;
                self.idle_timer = 0.;
                self.hits.clear();
                self.growled = false;
                self.whipped = false;
                self.whip_damaged = false;
                self.mid_leap = false;
                self.target_locked = false;
                out.sounds.push("boss_jormungandr_unburrow");
            }
            State::Bite => {
                self.attack_count -= 1;
                self.clip("attack", 0.15, false);
                self.hits.clear();
                out.sounds.push("boss_jormungandr_prebite");
            }
            State::Spit => {
                self.clip("venom", 0.2, false);
                self.spit_count = 0;
                self.spit_timer = 0.;
                out.sounds.push("boss_jormungandr_prespit");
            }
            State::HitRisen => {
                self.clip("hit", 0.5, false);
                if self.controller.time > 1.1 {
                    self.controller.stop();
                }
                out.sounds.push("boss_jormungandr_pain");
            }
            State::HitLeap => {
                self.clip("dive_recoil", 0.2, false);
                out.sounds.push("boss_jormungandr_pain");
            }
            State::Dead => {
                self.visible = true;
                self.update_animation = true;
                self.clip(
                    if self.last_state == State::Underground {
                        "die"
                    } else {
                        "die_above"
                    },
                    0.,
                    false,
                );
                self.idle_timer = 0.;
                out.sounds.push("boss_jormungandr_death");
            }
            State::Sleep => {}
        }
    }
    fn turn(&mut self, target: [f32; 3], speed: f32, dt: f32, flat_distance: bool) {
        if self.health.has(S::Frozen) {
            return;
        }
        let mut d = sub(
            target,
            if self.state == State::Spit || self.controller.clip_name() == Some("idle") {
                self.frame.head
            } else {
                self.position
            },
        );
        let len = if flat_distance {
            d[1] = 0.;
            distance(d, [0.; 3])
        } else {
            distance(d, [0.; 3])
        };
        d[1] = 0.;
        d = if len > f32::from_bits(1) {
            d.map(|v| v / len)
        } else {
            [0., 0., 1.]
        };
        let q = crate::summon_death_actor::turn_orientation(
            crate::summon_death_actor::rotation_quaternion(&self.orientation),
            d,
            if self.health.has(S::Cold) {
                speed * 0.5
            } else {
                speed
            },
            dt,
        );
        self.orientation = skinned::pose_matrix(translation(&self.orientation), q, [1.; 3]);
        self.direction = [
            -self.orientation[8],
            -self.orientation[9],
            -self.orientation[10],
        ];
    }
    /// Placement host runs the six-way navmesh/liquid checks and the native
    /// final-direction ground ray; None leaves the boss underground.
    pub fn update(
        &mut self,
        dt: f32,
        started: bool,
        client: bool,
        target: Option<[f32; 3]>,
        blocked: bool,
        mut leap: impl FnMut([f32; 3], f32) -> Option<[f32; 3]>,
    ) -> Events {
        let mut out = Events::default();
        let warning = self.warning_time();
        if self.state == State::Sleep && started {
            self.change(State::Intro, &mut out);
        }
        if self.hit && self.state != State::Dead {
            let state = match self.state {
                State::Leap => Some(State::HitLeap),
                State::Risen | State::Bite => Some(State::HitRisen),
                _ => None,
            };
            if let Some(s) = state {
                if !client {
                    self.change(s, &mut out);
                }
            } else {
                self.hit = false;
            }
        }
        let finished = self.controller.has_finished();
        let fading = self.controller.crossfade_enabled();
        let fraction = self.controller.time / self.controller.clip_duration();
        let clip = self.controller.clip_name().unwrap_or("").to_string();
        match self.state {
            State::Intro => {
                if !fading && finished && !client {
                    self.change(State::Underground, &mut out);
                } else if !fading && self.controller.time > 2.2 && !self.played {
                    self.played = true;
                    out.sounds.push("boss_jormungandr_dive");
                }
            }
            State::Underground if !client => {
                if self.health.hp <= 0. {
                    self.change(State::Dead, &mut out);
                } else {
                    self.idle_timer += dt;
                    if self.idle_timer > warning {
                        if let Some(target) = target {
                            let choice = self.random();
                            self.attack_count = (self.random() * 2.) as i32 + 2;
                            if choice <= 0.35 + 0.4 * self.health.hp / 10000. {
                                self.position = [target[0], target[1] - 1., target[2]];
                                self.orientation = skinned::pose_matrix(
                                    self.position,
                                    [0., (1.25f32).sin(), 0., (1.25f32).cos()],
                                    [1.; 3],
                                );
                                self.clip("emerge", 0., false);
                                self.change(State::Risen, &mut out);
                            } else {
                                let angle = self.random() * std::f32::consts::TAU;
                                if let Some(p) = leap(target, angle) {
                                    self.position = p;
                                    self.change(State::Leap, &mut out);
                                }
                            }
                        }
                    }
                }
            }
            State::Leap => {
                self.idle_timer += dt;
                // Native LeapState locks the target once, before the warning.
                if !self.target_locked {
                    if let Some(target) = target {
                        self.direction = normalized(sub(target, self.position));
                        self.orientation = transform(self.position, self.direction);
                        self.rotate_x(125.);
                        for i in [0, 2] {
                            self.orientation[12 + i] += self.direction[i] * 8.;
                        }
                        self.orientation[13] -= 9.5;
                        self.target_locked = true;
                    }
                }
                if !fading && fraction > 55. / 62. && !self.whip_damaged {
                    out.tail_whip = self.frame.spine.first().map(|s| s.axis.origin);
                    self.whip_damaged = true;
                }
                if self.idle_timer > warning {
                    self.visible = true;
                    self.update_animation = true;
                    if self.idle_timer > warning + 0.9 && !self.mid_leap {
                        self.mid_leap = true;
                    } else if self.idle_timer > warning + 0.6 && !self.growled {
                        self.growled = true;
                        out.sounds.push("boss_jormungandr_bite");
                    } else if self.idle_timer > warning + 1.9 && !self.whipped {
                        self.whipped = true;
                        out.sounds.push("boss_jormungandr_whip");
                    }
                    self.rotate_x(-275. / self.controller.clip_duration() * dt);
                    self.position = self.frame.head;
                    if finished && !client {
                        self.change(State::Underground, &mut out);
                    }
                }
            }
            State::Risen => {
                self.idle_timer += dt;
                if clip == "submerge" {
                    if !self.cleared {
                        out.clear_barriers = Some((self.position, 3.));
                        self.cleared = true;
                        out.sounds.push("boss_jormungandr_dive");
                    }
                    if finished && !fading && !client {
                        self.change(State::Underground, &mut out);
                    }
                } else if clip == "emerge" {
                    if fraction >= 0.125 && !self.played {
                        self.played = true;
                        out.sounds.push("boss_jormungandr_rise");
                    }
                    if finished {
                        if !client {
                            self.clip("idle", 0.5, true);
                        }
                        self.idle_timer = 0.;
                    }
                    if self.idle_timer > warning {
                        self.visible = true;
                        self.update_animation = true;
                        if !self.cleared {
                            out.clear_barriers = Some((self.position, 2.));
                            self.cleared = true;
                        }
                    }
                } else if self.health.hp <= 0. && !client {
                    self.change(State::Dead, &mut out);
                } else if !fading && self.attack_count <= 0 {
                    if !client {
                        self.clip("submerge", 0.5, false);
                    }
                } else if let Some(target) = target {
                    self.turn(target, 2., dt, true);
                    if self.idle_timer > warning + 0.8 && !client {
                        if (distance(target, self.position) > 15. || blocked)
                            && self.last_state != State::Spit
                        {
                            self.change(State::Spit, &mut out);
                        } else {
                            self.change(State::Bite, &mut out);
                        }
                    }
                }
            }
            State::Spit => {
                if let Some(target) = target {
                    self.turn(target, 1., dt, true);
                    if !fading {
                        self.spit_timer -= dt;
                        if fraction >= 0.375 && !self.played {
                            self.played = true;
                            out.sounds.push("boss_jormungandr_spit");
                        }
                        if fraction >= 0.38 && self.spit_timer <= 0. && self.spit_count < 4 {
                            self.spit_count += 1;
                            self.spit_timer += 0.5;
                            if !client {
                                let mut delta = sub(target, self.frame.head);
                                delta[1] = 0.;
                                let d = normalized(delta);
                                let origin =
                                    std::array::from_fn(|i| self.frame.head[i] + d[i] * 0.5);
                                let g = magicka_sim::character_physics::GRAVITY[1];
                                let time = ((origin[1] - target[1]) / (-0.5 * g)).sqrt();
                                let mut velocity = sub(target, origin).map(|v| v / time);
                                velocity[1] -= g * time * 0.5;
                                out.spit.push((origin, velocity));
                            }
                        }
                    }
                }
                if finished && !fading && !client {
                    self.clip("idle", 0.5, true);
                    self.change(State::Risen, &mut out);
                }
            }
            State::Bite => {
                if let Some(target) = target {
                    self.turn(target, 1.5, dt, false);
                }
                if !fading && fraction >= 0.4 && !self.played {
                    self.played = true;
                    let _speed = 1. + self.random() * 0.6 * (1. - self.health.hp / 10000.);
                    out.sounds.push("boss_jormungandr_bite");
                }
                if finished && !fading && !client {
                    self.clip("idle", 0.5, true);
                    self.change(State::Risen, &mut out);
                }
            }
            State::HitRisen => {
                if finished && !fading && !client {
                    self.clip("submerge", 0.6, false);
                    self.change(State::Risen, &mut out);
                }
            }
            State::HitLeap => {
                if !fading && fraction > 0.666 {
                    self.idle_timer -= dt;
                    self.rotate_x(275. / self.controller.clip_duration() * dt);
                    if self.idle_timer <= warning && !client {
                        self.change(State::Underground, &mut out);
                    }
                }
            }
            State::Dead => {
                self.orientation = transform(self.position, self.direction);
                if finished && !fading {
                    self.dead = true;
                }
            }
            _ => {}
        }
        for list in [&mut self.hits, &mut self.spine_hits] {
            list.retain(|_, ttl| {
                *ttl -= dt;
                *ttl > 0.
            });
        }
        self.controller.speed = if self.health.has(S::Frozen) {
            0.
        } else if self.health.has(S::Cold) {
            0.5
        } else {
            1.
        };
        if self.update_animation {
            self.controller.update(dt);
        }
        self.sample();
        self.controller.cues.clear();
        self.controller.signals.clear();
        self.health.update(dt, self.dead);
        self.flash = (self.flash - dt).max(0.);
        out
    }
    fn rotate_x(&mut self, degrees: f32) {
        let half = degrees.to_radians() * 0.5;
        let m = skinned::pose_matrix([0.; 3], [half.sin(), 0., 0., half.cos()], [1.; 3]);
        self.orientation = skinned::mul(&m, &self.orientation);
    }
    fn sample(&mut self) {
        let bones = self
            .skeleton
            .skinned_for(&self.controller, &self.orientation);
        let poses = self.skeleton.local_poses(self.controller.pose_sample());
        let head = if self.update_animation {
            let m = crate::animation_effects::bone_orientation(
                &self.skeleton.skeleton,
                &bones,
                "HeadBase",
            )
            .unwrap();
            std::array::from_fn(|i| m[12 + i] - self.orientation[8 + i] * 0.6)
        } else {
            [0., -100., 0.]
        };
        let points: Vec<_> = (0..26)
            .map(|i| {
                translation(
                    &crate::animation_effects::bone_orientation(
                        &self.skeleton.skeleton,
                        &bones,
                        &spine_name(i),
                    )
                    .unwrap(),
                )
            })
            .collect();
        let spine = if self.update_animation {
            (0..25)
                .map(|i| {
                    let d = normalized(sub(points[i], points[i + 1]));
                    Spine {
                        axis: Segment {
                            origin: points[i + 1],
                            // JigLibX Capsule.GetEnd uses Matrix.Backward;
                            // CreateVertebraeOrientation assigned Forward=d.
                            delta: d.map(|v| -v * self.spine_lengths[i]),
                        },
                        radius: (0.333 + i as f32 * 0.2).min(1.),
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        self.frame = Frame {
            // UpdateBoss rebuilds capsule-local transforms only when it calls
            // AnimationController.Update. Hidden warning frames retain them.
            body_points: if self.update_animation {
                points.into_iter().skip(1).collect()
            } else {
                self.frame.body_points.clone()
            },
            parent: self.orientation,
            local_poses: poses,
            bones,
            head,
            spine,
            visible: self.visible,
        };
    }
    pub fn vulnerable(&self) -> bool {
        self.visible && !self.dead && self.controller.clip_name() != Some("submerge")
    }
    /// OnCollision records the contact before filtering the damageable's
    /// type. Solid barriers and shields always take the OR branch in retail
    /// (`clip != submerge || clip != emerge`), costing the boss 750 HP.
    pub fn hit_barrier(&mut self, handle: u16, max_hp: f32, solid: bool) -> Option<Damage> {
        if !self.update_animation
            || self.collision_damage().is_empty()
            || self.hits.contains_key(&handle)
        {
            return None;
        }
        self.hits.insert(handle, 0.25);
        if !solid {
            return None;
        }
        self.health.hp -= 750.;
        self.hit = true;
        Some(Damage {
            attack: attack::DAMAGE,
            element: E::EARTH.0,
            amount: (max_hp * 0.75) as i32 as f32,
            magnitude: 1.,
        })
    }
    pub fn damage(&mut self, damages: &[Damage], authoritative: bool) -> u16 {
        if !authoritative || !self.vulnerable() {
            return 0;
        }
        let mut result = 0;
        for &d in damages {
            if d.amount == 0. && d.magnitude == 0. {
                continue;
            }
            let before = self.health.hp;
            let r = self
                .health
                .damage(crate::boss_scale::scaled(d, self.damage_divisor));
            result |= r;
            if r & (1 | 2 | 0x200) != 0 && r & 0x80 == 0 {
                self.flash = 0.1;
                if before - self.health.hp >= 250. {
                    self.hit = true;
                }
            }
        }
        result
    }
    pub fn collision_damage(&self) -> Vec<Damage> {
        let d = |attack, element, amount, magnitude| Damage {
            attack,
            element,
            amount,
            magnitude,
        };
        match self.state {
            State::Leap => vec![
                d(attack::DAMAGE, E::EARTH.0, 150., 1.),
                d(attack::STATUS, 0, 30., 1.),
                d(attack::PUSHED, E::EARTH.0, 70., 2.),
            ],
            State::Bite => vec![
                d(attack::DAMAGE, E::EARTH.0, 325., 1.),
                d(attack::STATUS, E::POISON.0, 30., 2.),
                d(attack::KNOCKBACK, E::EARTH.0, 70., 2.),
            ],
            State::Risen
                if self.controller.clip_name() == Some("emerge") && self.update_animation =>
            {
                vec![
                    d(attack::KNOCKBACK, E::EARTH.0, 150., 2.),
                    d(attack::DAMAGE, E::EARTH.0, 300., 2.),
                ]
            }
            _ => Vec::new(),
        }
    }
    pub fn segment_fraction(&self, seg: Segment) -> Option<f32> {
        if !self.update_animation || self.dead {
            return None;
        }
        magicka_sim::capsule_intersection::segment_fraction(
            seg,
            self.frame.head,
            [0., 1., 0.],
            0.,
            1.1,
        )
    }
    /// BossDamageZone's inside-radius and co-centred arc cases differ from Entity.
    pub fn arc(
        position: [f32; 3],
        radius: f32,
        query: magicka_sim::arc_intersection::ArcQuery,
    ) -> Option<[f32; 3]> {
        if (position[1] - query.origin[1]).abs() > query.height_difference {
            return None;
        }
        let mut origin = query.origin;
        origin[1] = 0.;
        let squared = (position[0] - origin[0]).powi(2) + (position[2] - origin[2]).powi(2);
        if squared <= 1e-6 {
            return None;
        }
        if squared.sqrt() < radius {
            return Some(origin);
        }
        magicka_sim::arc_intersection::entity_arc(position, radius, query)
    }
    pub fn spray_entity(&self, handle: u16) -> Option<crate::spray_world::WorldEntity> {
        self.update_animation
            .then_some(crate::spray_world::WorldEntity {
                handle,
                position: self.frame.head,
                radius: 1.1,
                dead: self.dead,
                damageable: true,
                frozen: self.health.has(S::Frozen),
                geometry: crate::spray_world::ArcGeometry::Boss,
            })
    }
}
pub fn spit_conditions() -> magicka_sim::missile_events::MissileConditions {
    use magicka_sim::missile_events::{condition as C, MissileConditions, MissileEvent as V};
    let mut c = MissileConditions::default();
    let sound = V::NamedSound {
        bank: crate::audio::banks::CHARACTERS,
        name: "boss_jormungandr_spit",
        magnitude: 1.,
        stop_on_remove: false,
    };
    let splash = V::NamedEffect {
        name: "jormungandr_spit_splash",
        follow: false,
        world_align: false,
    };
    let poison = Damage {
        attack: attack::STATUS,
        element: E::POISON.0,
        amount: 75.,
        magnitude: 4.,
    };
    c.slots[0].condition.flags = C::DEFAULT;
    c.slots[0].condition.repeat = true;
    c.slots[0].events = vec![
        V::NamedEffect {
            name: "jormungandr_spit_projectile",
            follow: true,
            world_align: false,
        },
        sound.clone(),
    ];
    c.slots[1].condition.flags = C::HIT;
    c.slots[1].events = vec![
        splash.clone(),
        sound.clone(),
        V::Damage {
            damage: poison,
            velocity_based: false,
        },
        V::Remove { bounces: 0 },
    ];
    c.slots[2].condition.flags = C::COLLISION;
    c.slots[2].events = vec![
        sound,
        splash,
        V::Splash {
            radius: 2.,
            damage: poison,
        },
        V::Splash {
            radius: 2.,
            damage: Damage {
                attack: attack::PUSHED,
                element: E::EARTH.0,
                amount: 70.,
                magnitude: 1.,
            },
        },
        V::Remove { bounces: 0 },
    ];
    c
}

#[cfg(test)]
mod transform_tests {
    use super::*;
    #[test]
    fn native_transform_keeps_up_and_mesh_winding() {
        for d in [[0., 0., 1.], [1., 0., 0.], normalized([2., 1., -3.])] {
            let m = transform([4., 5., 6.], d);
            assert!(m[5] > 0., "up must point above ground");
            let determinant = m[0] * (m[5] * m[10] - m[6] * m[9])
                - m[4] * (m[1] * m[10] - m[2] * m[9])
                + m[8] * (m[1] * m[6] - m[2] * m[5]);
            assert!(
                (determinant - 1.).abs() < 1e-5,
                "mirrored boss: {determinant}"
            );
            assert_eq!(&m[12..15], &[4., 5., 6.]);
        }
    }
}
