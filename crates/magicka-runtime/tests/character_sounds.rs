//! Character sound events: template attached sounds, the Footstep action and
//! the InternalDamage gore cue (research/audio-xact.md, 2026-10-30).
use magicka_content::character::{ActionKind, AnimationAction, AnimationClipAction};
use magicka_runtime::{
    animation_controller::{animation_value, AnimationSet},
    audio::{banks, AudioLibrary, AudioRequest, SoundSource},
    character::RuntimeCharacter,
    character_motion::{CharacterFacts, CharacterHost, CharacterMotion, GripFacts},
    character_sounds::{
        attached_sounds, footstep, AttachedSound, CharacterAudio, CharacterSounds,
        FootstepVariables, FOOTSTEP_CUES, SOUNDS_GORE,
    },
    gibs::SOUND_GIB,
    play_session::PlaySession,
    scene_loading::custom_hash,
    CharacterDefinition,
};
use magicka_sim::{
    character_physics::CharacterBody,
    damage::{attack, CastType, Damage},
    damage_delivery::DamagePayload,
    resistance::Resistance,
    spells::{Elements as E, SpellType},
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    root.is_dir().then_some(root)
}

fn definition(blood: i32, attached: Vec<AttachedSound>) -> CharacterDefinition {
    CharacterDefinition {
        id: "sound-fixture".into(),
        event_conditions: Arc::new(Vec::new()),
        faction: 4,
        ethereal: false,
        looks_ethereal: false,
        can_have_status: true,
        resistances: [Resistance::IDENTITY; 11],
        max_hit_points: 1000.0,
        health_bars: 1,
        hit_tolerance: 0.0,
        knockdown_tolerance: 1.0,
        max_panic: 0.0,
        fearless: false,
        uncharmable: false,
        nonslippery: false,
        length: 1.0,
        radius: 0.5,
        mass: 70.0,
        speed: 1.0,
        turn_speed: 1.0,
        regeneration: 10,
        bleed_rate: 0.0,
        stun_time: 0.0,
        gib_count: 0,
        gibs: Vec::new(),
        blood,
        attached_sounds: attached,
        zap_modifier: 1.0,
        break_free_strength: 1.0,
        undying: false,
        undie_time: 0.0,
        undie_hit_points: 0.0,
        model_tints: Vec::new(),
        score_value: 0,
        display_id: String::new(),
    }
}

fn attached(name: &str, bank: u16) -> AttachedSound {
    AttachedSound {
        bank,
        cue: custom_hash(name),
        name: name.to_string(),
    }
}

#[test]
fn template_attached_sounds_start_at_initialize_stop_once_on_death_and_restart() {
    let sounds = vec![
        attached("amb_loop_a", banks::CHARACTERS),
        attached("", banks::CHARACTERS),
        attached("amb_loop_b", banks::AMBIENCE),
    ];
    let mut character = RuntimeCharacter::new(definition(5, sounds.clone()));
    assert_eq!(character.sounds.attached, sounds);
    assert_eq!(
        std::mem::take(&mut character.sounds.pending),
        vec![
            CharacterAudio::AttachedStart {
                slot: 0,
                bank: banks::CHARACTERS,
                cue: custom_hash("amb_loop_a"),
            },
            CharacterAudio::AttachedStart {
                slot: 2,
                bank: banks::AMBIENCE,
                cue: custom_hash("amb_loop_b"),
            },
        ],
        "Initialize plays every named attached sound once, in slot order"
    );
    // Alive frames do not restart live cues.
    for _ in 0..3 {
        character.sounds.update(false);
    }
    assert!(character.sounds.pending.is_empty());
    // Die() stops them exactly once.
    character.sounds.update(true);
    assert_eq!(
        std::mem::take(&mut character.sounds.pending),
        vec![
            CharacterAudio::AttachedStop { slot: 0 },
            CharacterAudio::AttachedStop { slot: 2 },
        ]
    );
    character.sounds.update(true);
    assert!(character.sounds.pending.is_empty());
    // A re-initialized (revived) character starts them again.
    character.sounds.update(false);
    assert_eq!(character.sounds.pending.len(), 2);
    assert_eq!(
        character.sounds.attached_cue_names().collect::<Vec<_>>(),
        vec![
            (banks::CHARACTERS, "amb_loop_a"),
            (banks::CHARACTERS, ""),
            (banks::AMBIENCE, "amb_loop_b"),
        ]
    );
    // Templates without attached sounds request nothing.
    let silent = CharacterSounds::initialize(Vec::new());
    assert!(silent.pending.is_empty() && silent.attached_live.is_empty());
}

fn physical_hit(character: &mut RuntimeCharacter, amount: f32, element: E) -> u16 {
    magicka_runtime::avatar::deliver_to_character(
        character,
        &[Resistance::IDENTITY; 11],
        false,
        7,
        [0.0, 0.0, 0.0],
        [0.0, 0.0, -1.0],
        DamagePayload::Single(Damage {
            attack: attack::DAMAGE,
            element: element.0,
            amount,
            magnitude: 2.0,
        }),
        None,
        [0.0, 0.0, 2.0],
        0.0,
    )
}

fn one_shots(sounds: &mut CharacterSounds) -> Vec<(u16, i32, Option<f32>)> {
    std::mem::take(&mut sounds.pending)
        .into_iter()
        .filter_map(|a| match a {
            CharacterAudio::OneShot {
                bank,
                cue,
                magnitude,
                ..
            } => Some((bank, cue, magnitude)),
            _ => None,
        })
        .collect()
}

#[test]
fn physical_damage_on_a_bleeding_character_plays_the_gore_cue_with_the_damage_amount() {
    magicka_runtime::global_settings::set_blood_and_gore(true);
    let gore = (banks::MISC, custom_hash("misc_hit_blood"));
    for blood in 0..5 {
        let mut character = RuntimeCharacter::new(definition(blood, Vec::new()));
        physical_hit(&mut character, 25.0, E::EARTH);
        assert_eq!(
            one_shots(&mut character.sounds),
            vec![(gore.0, gore.1, Some(25.0))],
            "blood type {blood}: Magnitude is the pre-multiplied damage amount"
        );
        assert_eq!(character.blood_decals.len(), 1);
    }
    // BloodType.none hashes an empty cue name: nothing is requested.
    let mut character = RuntimeCharacter::new(definition(5, Vec::new()));
    physical_hit(&mut character, 25.0, E::ARCANE);
    assert!(one_shots(&mut character.sounds).is_empty());
    assert!(character.blood_decals.is_empty());
    // Non-physical, non-arcane elements take the other InternalDamage branch.
    let mut character = RuntimeCharacter::new(definition(0, Vec::new()));
    physical_hit(&mut character, 25.0, E::FIRE);
    assert!(one_shots(&mut character.sounds).is_empty());
    // Zero damage does not bleed.
    let mut character = RuntimeCharacter::new(definition(0, Vec::new()));
    physical_hit(&mut character, 0.0, E::EARTH);
    assert!(one_shots(&mut character.sounds).is_empty());
    assert_eq!(SOUNDS_GORE[5], "");
}

fn make_body(material: u8, grounded: bool, levitating: bool, depth: f32) -> CharacterBody {
    let mut body = CharacterBody::new(0.5, 1.0, 70.0, 5.0);
    body.enable();
    body.ground_material = material;
    body.owner.levitating = levitating;
    body.owner.water_depth = depth;
    if !grounded {
        // sync_external_motion with grounded=false sets a 0.3 s air time.
        body.sync_external_motion(body.position, body.orientation, [0.0; 3], false);
    }
    body
}

#[test]
fn footstep_selection_follows_ground_material_contact_and_levitation() {
    let expect = |material: u8| FOOTSTEP_CUES[material as usize];
    for material in 0..10u8 {
        let step = footstep(&make_body(material, true, false, 0.25));
        assert_eq!(
            step.map(|(cue, _)| cue),
            expect(material),
            "material {material}"
        );
        if let Some((_, variables)) = step {
            assert_eq!(
                variables,
                FootstepVariables {
                    weight: 70.0,
                    depth: 0.25
                }
            );
        }
    }
    assert_eq!(
        footstep(&make_body(0, true, false, 0.0)).unwrap().0,
        "footstep_generic"
    );
    assert_eq!(
        footstep(&make_body(7, true, false, 0.0)).unwrap().0,
        "footstep_generic"
    );
    assert_eq!(
        footstep(&make_body(8, true, false, 0.0)).unwrap().0,
        "footstep_water"
    );
    assert!(
        footstep(&make_body(9, true, false, 0.0)).is_none(),
        "lava is silent"
    );
    assert!(
        footstep(&make_body(0, false, false, 0.0)).is_none(),
        "airborne"
    );
    assert!(
        footstep(&make_body(0, true, true, 0.0)).is_none(),
        "levitating"
    );
}

struct RecordingHost {
    sounds: Vec<(u16, i32)>,
    footsteps: Vec<(String, FootstepVariables)>,
    route_default: bool,
}
impl CharacterHost for RecordingHost {
    fn cast_spell(&mut self, _: bool, _: Option<&str>) {}
    fn spell_animation_end(&mut self) {}
    fn current_spell(&self) -> Option<bool> {
        None
    }
    fn end_cast(&mut self) {}
    fn entered_dead_state(&mut self) {}
    fn die(&mut self) {}
    fn play_animation_sound(&mut self, bank: u16, sound: &str) {
        self.sounds.push((bank, custom_hash(sound)));
    }
    fn play_footstep(&mut self, cue: &str, variables: FootstepVariables) {
        self.footsteps.push((cue.to_string(), variables));
        if self.route_default {
            self.play_animation_sound(banks::FOOTSTEPS, cue);
        }
    }
}

fn facts() -> CharacterFacts {
    CharacterFacts {
        hit_points: 100.0,
        undie: false,
        cannot_die_without_explicit_kill: false,
        dead: false,
        overkilled: false,
        avatar: false,
        cast_type: CastType::Blast,
        spell_type: SpellType::Push,
        magick_animation: None,
        charge_unlocked: false,
        in_cutscene: false,
        frozen: false,
        random: 0.0,
        weapon_spell: None,
        fear: None,
        panicking: false,
        stumbling: false,
        max_panic: 0.0,
        stunned: false,
        knocked_down: false,
        on_ground: true,
        grip: GripFacts::default(),
    }
}

fn walking_set() -> Arc<AnimationSet> {
    let clip = AnimationClipAction {
        animation: "move_walk".into(),
        clip: "walk".into(),
        speed: 1.0,
        blend_time: 0.0,
        looping: true,
        actions: vec![
            AnimationAction {
                start_time: 0.0,
                end_time: 0.0,
                kind: ActionKind::Footstep,
            },
            AnimationAction {
                start_time: 0.5,
                end_time: 0.5,
                kind: ActionKind::Footstep,
            },
        ],
    };
    let idle = AnimationClipAction {
        animation: "idle".into(),
        clip: "stand".into(),
        speed: 1.0,
        blend_time: 0.0,
        looping: true,
        actions: Vec::new(),
    };
    let durations = [("walk".to_string(), 1.0), ("stand".to_string(), 1.0)]
        .into_iter()
        .collect();
    Arc::new(AnimationSet::new(&[idle, clip], durations))
}

#[test]
fn footstep_actions_execute_once_per_window_on_the_ground_only() {
    let animations = walking_set();
    let walk = animation_value("move_walk").unwrap();
    let mut motion = CharacterMotion::new(animations.clone());
    let mut body = make_body(2, true, false, 0.0);
    let mut host = RecordingHost {
        sounds: Vec::new(),
        footsteps: Vec::new(),
        route_default: true,
    };
    motion.force_animation(walk, &mut host);
    for _ in 0..20 {
        motion.update(&mut body, &facts(), 1.0 / 60.0, &mut host);
    }
    assert_eq!(
        host.footsteps,
        vec![(
            "footstep_grass".to_string(),
            FootstepVariables {
                weight: 70.0,
                depth: 0.0
            }
        )],
        "the first window runs once; the 0.5 window has not been reached"
    );
    assert_eq!(
        host.sounds,
        vec![(banks::FOOTSTEPS, custom_hash("footstep_grass"))],
        "the default host route plays the cue from the Footsteps bank"
    );
    // Airborne at the second window: nothing.
    body.sync_external_motion(body.position, body.orientation, [0.0; 3], false);
    for _ in 0..15 {
        motion.update(&mut body, &facts(), 1.0 / 60.0, &mut host);
    }
    assert_eq!(host.footsteps.len(), 1, "airborne footsteps are silent");

    // Lava has no footstep cue even when grounded.
    let mut motion = CharacterMotion::new(animations);
    let mut body = make_body(9, true, false, 0.0);
    let mut host = RecordingHost {
        sounds: Vec::new(),
        footsteps: Vec::new(),
        route_default: false,
    };
    motion.force_animation(walk, &mut host);
    for _ in 0..30 {
        motion.update(&mut body, &facts(), 1.0 / 60.0, &mut host);
    }
    assert!(host.footsteps.is_empty() && host.sounds.is_empty());
}

#[test]
fn retail_character_cues_resolve_in_the_sound_banks() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    let library = AudioLibrary::load(&root);
    assert!(library.errors.is_empty(), "{:?}", library.errors);
    let mut paths = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xnb")) {
                out.push(p);
            }
        }
    }
    walk(&root.join("Data"), &mut paths);
    paths.sort();
    let mut cues: std::collections::BTreeMap<(u16, String), Vec<String>> = Default::default();
    // Cues this crate's character sound code emits (attached, footstep, gore,
    // gib) must resolve; authored PlaySound actions are reported, since
    // AudioManager.PlayCue silently ignores hashes outside mDictSound.
    let mut emitted: std::collections::BTreeSet<(u16, String)> = Default::default();
    let mut attached_inventory = Vec::new();
    let mut templates = 0;
    for path in &paths {
        let bytes = std::fs::read(path).unwrap();
        let xnb = magicka_xnb::xnb::parse(&bytes).unwrap();
        let Some(Ok(parsed)) = magicka_content::parse_primary(&xnb) else {
            continue;
        };
        let magicka_content::Content::Character(template) = parsed.content else {
            continue;
        };
        templates += 1;
        let id = template.id.clone();
        let attached = attached_sounds(&template);
        assert_eq!(attached.len(), template.attached_sounds.len().min(4));
        for (sound, (name, bank)) in attached.iter().zip(&template.attached_sounds) {
            assert_eq!(sound.name, name.to_lowercase());
            assert_eq!(sound.bank, *bank as u16);
            assert_eq!(sound.cue, custom_hash(&sound.name));
            attached_inventory.push(format!("{id}: bank {:#x} {}", sound.bank, sound.name));
            emitted.insert((sound.bank, sound.name.clone()));
            cues.entry((sound.bank, sound.name.clone()))
                .or_default()
                .push(format!("{id} attached"));
        }
        for (set, clips) in template.animation_sets.iter().enumerate() {
            for clip in clips {
                for action in &clip.actions {
                    if let ActionKind::PlaySound { sound, bank } = &action.kind {
                        cues.entry((*bank as u16, sound.to_lowercase()))
                            .or_default()
                            .push(format!("{id} set {set} {}", clip.animation));
                    }
                }
            }
        }
    }
    assert!(templates > 0, "no character templates under Data");
    for cue in FOOTSTEP_CUES.iter().flatten() {
        emitted.insert((banks::FOOTSTEPS, cue.to_string()));
        cues.entry((banks::FOOTSTEPS, cue.to_string()))
            .or_default()
            .push("Footstep.sCueNames".into());
    }
    for cue in SOUNDS_GORE.iter().filter(|c| !c.is_empty()) {
        emitted.insert((banks::MISC, cue.to_string()));
        cues.entry((banks::MISC, cue.to_string()))
            .or_default()
            .push("Defines.SOUNDS_GORE".into());
    }
    emitted.insert((banks::MISC, SOUND_GIB.to_string()));
    cues.entry((banks::MISC, SOUND_GIB.to_string()))
        .or_default()
        .push("DeadState.SOUND_GIB".into());
    let mut missing = Vec::new();
    let mut authored_gaps = Vec::new();
    for ((bank, name), users) in &cues {
        if library.has_cue(*bank, custom_hash(name)) {
            continue;
        }
        let line = format!(
            "bank {bank:#x} cue {name:?} used by {} e.g. {:?}",
            users.len(),
            users[0]
        );
        if emitted.contains(&(*bank, name.clone())) {
            missing.push(line);
        } else {
            authored_gaps.push(line);
        }
    }
    eprintln!(
        "character templates: {templates}; distinct cues: {}",
        cues.len()
    );
    eprintln!("attached sounds: {attached_inventory:#?}");
    for ((bank, name), users) in &cues {
        eprintln!(
            "cue bank {bank:#x} {name}: {} users, first {:?}",
            users.len(),
            users[0]
        );
    }
    eprintln!(
        "authored PlaySound cues absent from the banks (native plays nothing): {authored_gaps:#?}"
    );
    assert_eq!(attached_inventory.len(), 3, "{attached_inventory:?}");
    assert!(missing.is_empty(), "unresolved emitted cues: {missing:#?}");
    assert!(
        authored_gaps.len() < 16,
        "authored gaps grew beyond the recorded 12: {authored_gaps:#?}"
    );
    // The common enemy voice cues on hit/death/alert clips resolve.
    for name in ["chr_goblin_pain", "chr_goblin_death", "chr_goblin_angry"] {
        assert!(
            library.has_cue(banks::CHARACTERS, custom_hash(name)),
            "{name}"
        );
    }
}

#[test]
fn retail_npc_hit_requests_the_gore_cue_at_its_emitter() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    magicka_runtime::global_settings::set_blood_and_gore(true);
    let mut session = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    session.disable_scripts();
    session.world.flags.cutscene = false;
    session.world.flags.input_limited = false;
    let handle = session.spawn_npc("Goblin_warrior", "idle", 5.0).unwrap();
    // Spawn animation and spawn immortality first.
    for _ in 0..180 {
        session.step(1.0 / 60.0);
    }
    session.audio.clear();
    session.world.audio_requests.clear();
    let (position, blood) = {
        let npc = session.world.npcs.get_mut(&handle).unwrap();
        let body = &mut session.world.physics.bodies[npc.body];
        let front = std::array::from_fn(|i| body.position[i] + body.direction()[i] * 1.0);
        magicka_runtime::avatar::deliver_to_body(
            &mut npc.character,
            body,
            &[Resistance::IDENTITY; 11],
            false,
            handle,
            DamagePayload::Single(Damage {
                attack: attack::DAMAGE,
                element: E::ARCANE.0,
                amount: 5.0,
                magnitude: 1.0,
            }),
            None,
            front,
            session.world.play_time,
        );
        assert_eq!(
            npc.character.blood_decals.len(),
            1,
            "the hit reached InternalDamage's gore branch (hp {})",
            npc.character.combat.health.hit_points
        );
        assert_eq!(
            npc.character.sounds.pending,
            vec![CharacterAudio::OneShot {
                bank: banks::MISC,
                cue: custom_hash("misc_hit_blood"),
                magnitude: Some(5.0),
                variables: Vec::new(),
            }]
        );
        (body.position, npc.definition.blood)
    };
    assert_ne!(blood, 5, "Goblin_warrior bleeds");
    session.step(1.0 / 60.0);
    let gore: Vec<_> = session
        .audio
        .iter()
        .filter_map(|(request, _)| match request {
            AudioRequest::Start {
                bank,
                cue,
                source: SoundSource::Emitter(p),
                magnitude,
                ..
            } if *bank == banks::MISC && *cue == custom_hash("misc_hit_blood") => {
                Some((*p, *magnitude))
            }
            _ => None,
        })
        .collect();
    assert_eq!(gore.len(), 1, "{:?}", session.audio);
    assert_eq!(gore[0].1, Some(5.0));
    let d: f32 = (0..3).map(|i| (gore[0].0[i] - position[i]).powi(2)).sum();
    assert!(
        d.sqrt() < 1.0,
        "emitter at the NPC: {:?} vs {position:?}",
        gore[0].0
    );
}

#[test]
fn retail_bugswarm_attached_loop_starts_at_spawn_and_stops_on_death() {
    let Some(root) = content() else {
        eprintln!("SKIP: retail content absent");
        return;
    };
    let mut session = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
    session.disable_scripts();
    session.world.flags.cutscene = false;
    session.world.flags.input_limited = false;
    session.audio.clear();
    let handle = session.spawn_npc("Bugswarm", "idle", 5.0).unwrap();
    assert_eq!(
        session.world.npcs[&handle]
            .character
            .sounds
            .attached_cue_names()
            .collect::<Vec<_>>(),
        vec![(banks::ADDITIONAL, "chr_swarm_idle")]
    );
    session.step(1.0 / 60.0);
    let position = session.world.physics.bodies[session.world.npcs[&handle].body].position;
    let cue = custom_hash("chr_swarm_idle");
    let starts: Vec<_> = session
        .audio
        .iter()
        .filter_map(|(request, _)| match request {
            AudioRequest::Start {
                key,
                bank,
                cue: c,
                source: SoundSource::Emitter(p),
                ..
            } if *bank == banks::ADDITIONAL && *c == cue => Some((*key, *p)),
            _ => None,
        })
        .collect();
    assert_eq!(starts.len(), 1, "{:?}", session.audio);
    let (key, at) = starts[0];
    let d: f32 = (0..3).map(|i| (at[i] - position[i]).powi(2)).sum();
    assert!(d.sqrt() < 1.0, "{at:?} vs {position:?}");
    // Alive frames neither restart nor move the cue (native never re-applies
    // 3D to attached cues).
    session.audio.clear();
    for _ in 0..30 {
        session.step(1.0 / 60.0);
    }
    assert!(
        !session.audio.iter().any(|(r, _)| match r {
            AudioRequest::Start { cue: c, .. } => *c == cue,
            AudioRequest::Move { key: k, .. } => *k == key,
            _ => false,
        }),
        "{:?}",
        session.audio
    );
    assert!(!session
        .audio
        .iter()
        .any(|(r, _)| matches!(r, AudioRequest::Stop { key: k, .. } if *k == key)));
    magicka_runtime::entity_command::EntityCommandHost::kill(&mut session.world, handle).unwrap();
    let mut stopped = false;
    for _ in 0..240 {
        session.step(1.0 / 60.0);
        if session
            .audio
            .iter()
            .any(|(r, _)| matches!(r, AudioRequest::Stop { key: k, .. } if *k == key))
        {
            stopped = true;
            break;
        }
    }
    assert!(
        stopped,
        "Die() stops the attached cue: {:?}",
        session.world.log
    );
}
