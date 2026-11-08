//! NPC Agent reconstruction (research/npc-agents.md): AIManager scheduling,
//! GetAvoidance's sight term, idle-order event playback with animation
//! triggers, and proximity reactions.
use magicka_content::character::AnimationClipAction;
use magicka_runtime::{
    ai_event::{AiEvent, AnimationEvent},
    animation_controller::{animation_value, AnimationSet},
    character_motion::ActorFrame,
    npc::{
        avoidance, scheduled, AgentWeights, Neighbour, NpcActorTemplate, NpcCharacter,
        REACT_PROXIMITY,
    },
    CharacterDefinition,
};
use magicka_sim::{character_physics::CharacterBody, resistance::Resistance};
use std::{collections::HashMap, sync::Arc};

fn clip(animation: &str, clip: &str, looping: bool) -> AnimationClipAction {
    AnimationClipAction {
        animation: animation.into(),
        clip: clip.into(),
        speed: 1.0,
        blend_time: 0.0,
        looping,
        actions: Vec::new(),
    }
}

fn template() -> NpcActorTemplate {
    let durations = HashMap::from([("idle".to_string(), 2.0), ("eat".to_string(), 0.5)]);
    let animations = Arc::new(AnimationSet::new(
        &[
            clip("idle", "idle", true),
            clip("cutscene_eat", "eat", false),
        ],
        durations,
    ));
    NpcActorTemplate {
        summon_element_cue: None,
        definition: CharacterDefinition {
            id: "test".into(),
            faction: 1,
            ethereal: false,
            looks_ethereal: false,
            can_have_status: true,
            resistances: [Resistance::IDENTITY; 11],
            max_hit_points: 100.0,
            health_bars: 1,
            event_conditions: Arc::new(Vec::new()),
            hit_tolerance: 0.0,
            knockdown_tolerance: 0.0,
            max_panic: 0.0,
            fearless: false,
            uncharmable: false,
            nonslippery: false,
            length: 1.0,
            radius: 0.4,
            mass: 80.0,
            speed: 4.0,
            turn_speed: 10.0,
            regeneration: 0,
            bleed_rate: 0.0,
            stun_time: 0.0,
            gib_count: 0,
            gibs: Vec::new(),
            blood: 5,
            attached_sounds: Vec::new(),
            zap_modifier: 1.0,
            break_free_strength: 1.0,
            undying: false,
            undie_time: 0.0,
            undie_hit_points: 0.0,
            model_tints: Vec::new(),
            score_value: 0,
            display_id: String::new(),
        },
        animations,
        move_abilities: 0,
        weights: AgentWeights {
            alert_radius: 10.0,
            friendly_avoidance: 1.0,
            enemy_avoidance: 1.0,
            sight_avoidance: 2.0,
            danger_avoidance: 1.0,
            group: Default::default(),
        },
        abilities: Arc::new(Vec::new()),
        target_weights: Default::default(),
        rig: None,
        weapons: Vec::new(),
        held_effects: Vec::new(),
    }
}

#[test]
fn ai_manager_runs_twenty_agents_from_an_advancing_index() {
    let mut index = 0;
    assert_eq!(scheduled(&mut index, 3), vec![2, 0, 1]);
    assert_eq!(scheduled(&mut index, 3), vec![1, 2, 0]);
    let mut index = 0;
    let run = scheduled(&mut index, 25);
    assert_eq!(run.len(), 20);
    assert_eq!(run[0], 20);
    assert_eq!(run[5], 0);
    assert!(scheduled(&mut index, 0).is_empty());
}

#[test]
fn sight_avoidance_steps_sideways_out_of_an_enemy_gaze() {
    let weights = template().weights;
    // An enemy 6 m north looking straight at us (south).
    let enemy = Neighbour {
        handle: 2,
        position: [0.0, 0.0, -6.0],
        direction: [0.0, 0.0, 1.0],
        radius: 0.5,
        faction: Some(4),
        danger: 0.0,
        invisible: false,
    };
    let a = avoidance([0.0; 3], 1, &weights, &[enemy]);
    // Falloff r²(1−r)·6.75 at r = .5 is .84375; facing^.25 = 1; weight 2.
    assert!((a[0].abs() - 1.6875).abs() < 1e-5, "{a:?}");
    assert!(a[1].abs() < 1e-6 && a[2].abs() < 1e-6, "{a:?}");
    // Friends and harmless entities contribute danger only (zero here).
    let friend = Neighbour {
        faction: Some(1),
        ..enemy
    };
    assert_eq!(avoidance([0.0; 3], 1, &weights, &[friend]), [0.0; 3]);
    // Looking away removes the sight term.
    let away = Neighbour {
        direction: [0.0, 0.0, -1.0],
        ..enemy
    };
    assert_eq!(avoidance([0.0; 3], 1, &weights, &[away]), [0.0, 0.0, 0.0]);
}

fn frame<'a>(
    body: &'a mut CharacterBody,
    triggers: &'a mut Vec<(i32, Option<u16>)>,
) -> ActorFrame<'a> {
    ActorFrame {
        body,
        nav: None,
        move_abilities: 0,
        triggers,
        sender: 7,
        entity_position: &|_| None,
        locator: &|_| None,
        closest_area_point: &|_, _| None,
        avoidance: &|_| [0.0; 3],
        in_cutscene: false,
        play_time: 0.0,
    }
}

#[test]
fn idle_order_animation_event_fires_its_trigger_when_the_clip_finishes() {
    let t = template();
    let mut body = CharacterBody::new(0.4, 1.0, 80.0, 4.0);
    let mut npc = NpcCharacter::new(7, 0, &t);
    let eat = animation_value("cutscene_eat").unwrap();
    npc.set_order(
        1,
        0,
        0,
        0,
        Some(vec![AiEvent::Animation(AnimationEvent {
            animation: eat,
            trigger: 99,
            ..Default::default()
        })]),
    );
    let mut triggers = Vec::new();
    let mut fired_at = None;
    for step in 0..120 {
        let dt = 1.0 / 60.0;
        npc.agent_update(frame(&mut body, &mut triggers), 1, &[], None);
        npc.update_motion(&mut body, false, dt);
        npc.time_since_update += dt;
        if fired_at.is_none() && !triggers.is_empty() {
            fired_at = Some(step);
        }
    }
    assert_eq!(triggers, vec![(99, Some(7))]);
    // Blend time 0 (event and clip action): the crossfade completes on the
    // next update, the 0.5 s clip runs, and the trigger fires on the agent
    // update after HasFinished (one-frame agent lag included).
    let fired = fired_at.unwrap();
    assert!((30..=34).contains(&fired), "fired at frame {fired}");
    assert!(!npc.ai.controls_avatar());
}

#[test]
fn proximity_reaction_switches_order_and_fires_the_reaction_trigger() {
    let t = template();
    let mut body = CharacterBody::new(0.4, 1.0, 80.0, 4.0);
    let mut npc = NpcCharacter::new(7, 0, &t);
    npc.set_order(1, REACT_PROXIMITY, 2, 55, None);
    let friend = Neighbour {
        handle: 3,
        position: [1.0, 0.0, 0.0],
        direction: [0.0, 0.0, -1.0],
        radius: 0.5,
        faction: Some(1),
        danger: 0.0,
        invisible: false,
    };
    let mut triggers = Vec::new();
    let outcome = npc.agent_update(frame(&mut body, &mut triggers), 1, &[friend], None);
    assert!(!outcome.reacted && triggers.is_empty() && npc.ai.order == 1);
    let enemy = Neighbour {
        faction: Some(0x100),
        ..friend
    };
    let outcome = npc.agent_update(frame(&mut body, &mut triggers), 1, &[enemy], None);
    assert!(outcome.reacted);
    assert_eq!(npc.ai.order, 2);
    assert_eq!(triggers, vec![(55, Some(7))]);
}
