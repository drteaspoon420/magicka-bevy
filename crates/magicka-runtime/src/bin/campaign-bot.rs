//! Plays retail levels by machine, through the real input path, and reports how far
//! each one gets (`research/campaign-bot.md`). Not a test of combat: health is locked,
//! nearby enemies are removed and cutscenes are skipped, so a stall means a level's
//! progression broke, or the bot was not clever enough to see what it wanted.
//!
//!     campaign-bot <content-dir> [--out report.json] [--minutes-per-scene 8] <level.lvl>...
//!
//! With no levels, every campaign level runs. Exits 1 on a script error, a panic, or a
//! level getting less far than `BASELINE` (only checked with a budget of 5 or more).
//! Completed baseline chapters also fail if they require a forced prop kill.
//! `BOT_TRACE=1` logs goals; `BOT_WATCH=1` lets cutscenes play instead of skipping.
use magicka_runtime::entity_command::EntityCommandHost;
use magicka_runtime::{
    avatar::PlayerInput,
    camera::FACTION_FRIENDLY,
    fixed_tick::TICK_DT,
    physics_damage::PhysicsDamage,
    play_session::{PlaySession, SessionEvent},
    presence::PresenceKind,
    scene_flow::SceneFlowKind,
    scene_loading::custom_hash,
    scene_script::{ActionDefinition, ConditionDefinition, SceneScriptLayout},
    scene_world::{boss_live::BossRef, EntityKind},
    script_condition::ConditionKind,
    toggle_action::ToggleKind,
    trigger_control::TriggerControlKind,
    world_condition::WorldConditionKind,
};
use magicka_sim::{
    damage::{attack, Damage},
    navmesh::movement,
    spells::Elements,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

const ASPECT: f32 = 16.0 / 9.0;
const AREA_CENTRE_REACHED: f32 = 0.5;
/// Further down than any level's floor. Deaths past `MAX_DEATHS` end the scene.
const FELL_OUT: f32 = -200.0;
const MAX_DEATHS: usize = 10;
const LAST_STRETCH: f32 = 6.0;
const WAYPOINT_REACHED: f32 = 0.6;
/// Navmesh vertices sit on obstacle edges; the wizard's capsule (radius 0.8)
/// cannot close on one nearer than its radius. Battlefield's exit path stalled
/// 0.7 m from such a vertex (research/campaign-bot.md): after a second of
/// pushing without moving, a waypoint this near counts as reached.
const WAYPOINT_REACHED_WHEN_STILL: f32 = 1.5;
/// No movement for this long means this goal is blocked for now.
const STUCK_SECONDS: f32 = 3.0;
/// Enemies this close are removed: combat is not under test.
const ENEMY_RANGE: f32 = 14.0;

/// How far each level got on 2026-11-04: level, scene changes, completed. Getting
/// less far is a failure; getting further means this table should be raised.
/// Measured with a 5-minute budget per scene (`research/campaign-bot.md`).
/// Completed chapters must also pass without forced prop kills (2026-11-05).
const BASELINE: &[(&str, usize, bool)] = &[
    ("Wizard_Castle.lvl", 4, false),
    ("Mountaindale.lvl", 5, true),
    ("Highlands.lvl", 1, false),
    ("Havindr.lvl", 7, true),
    ("Battlefield.lvl", 3, true),
    ("EndOfWorld.lvl", 1, false),
    ("Wizard_Castle2.lvl", 0, false),
    ("Mines.lvl", 0, false),
    ("Swamp.lvl", 9, true),
    ("Niflheim.lvl", 2, true),
    ("Ruins.lvl", 5, false),
    ("EndOfWorld2.lvl", 0, false),
    ("Tsar/Tsar_WizardCastle.lvl", 2, true),
    ("Tsar/Tsar_Mountaindale.lvl", 4, true),
    ("Tsar/Tsar_Rlyeh.lvl", 1, false),
];

#[derive(Debug, Default)]
struct SceneReport {
    name: String,
    seconds: f32,
    /// "scene change", "level complete", "budget", "error" or "died".
    ended: String,
    goals_reached: usize,
    goals_total: usize,
    removed_enemies: usize,
    boss_hits: usize,
    deaths: usize,
    props_broken: usize,
    props_forced: usize,
    characters_healed: usize,
    healing_abandoned: usize,
    fell_alive: usize,
    skipped_tutorial: bool,
    /// The exit areas still unreached when the budget ran out.
    unreached_exits: Vec<String>,
    /// Why the wizard was not walking, in seconds.
    input_limited: f32,
    cutscene: f32,
    tutorial_modal: f32,
    /// Distance actually walked, in metres.
    walked: f32,
    /// Where it stood at the end, and what it was walking to.
    final_position: [f32; 3],
    final_goal: String,
}

#[derive(Debug, Default)]
struct LevelReport {
    level: String,
    scenes: Vec<SceneReport>,
    completed: bool,
    script_error: Option<String>,
    skipped_actions: BTreeSet<String>,
    panic: Option<String>,
    deaths: usize,
    deadly_goals: BTreeSet<String>,
}

/// The areas to walk to in this scene, exits first.
struct Goals {
    exits: Vec<(String, [f32; 3])>,
    /// Levers and doors: the locators of `<OnInteract>`.
    interactables: Vec<(String, [f32; 3])>,
    others: Vec<(String, [f32; 3])>,
    /// Alternative condition groups with just the goal's presence test removed.
    exit_requirements: BTreeMap<String, Vec<ExitRequirement>>,
}

struct ExitRequirement {
    trigger: i32,
    conditions: Vec<usize>,
}

fn goals(s: &PlaySession) -> Goals {
    let layout = s.runtime.layout();
    // Triggers that end the scene, directly or through the triggers they execute.
    let mut exit_triggers: BTreeSet<i32> = BTreeSet::new();
    let mut executes: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
    for trigger in &layout.triggers {
        for &action in trigger.actions.iter().flatten() {
            match &layout.actions[action] {
                ActionDefinition::Flow(flow)
                    if matches!(
                        flow.kind,
                        SceneFlowKind::ChangeScene { .. }
                            | SceneFlowKind::EnterScene { .. }
                            | SceneFlowKind::EndGame { .. }
                    ) =>
                {
                    exit_triggers.insert(trigger.id);
                }
                ActionDefinition::Control(control)
                    if control.kind == TriggerControlKind::Execute =>
                {
                    executes.entry(control.target).or_default().push(trigger.id);
                }
                _ => {}
            }
        }
    }
    let mut frontier: Vec<i32> = exit_triggers.iter().copied().collect();
    while let Some(id) = frontier.pop() {
        for &caller in executes.get(&id).into_iter().flatten() {
            if exit_triggers.insert(caller) {
                frontier.push(caller);
            }
        }
    }
    let names: BTreeMap<i32, String> = s
        .world
        .area_names()
        .into_iter()
        .map(|n| (custom_hash(&n.to_ascii_lowercase()), n))
        .collect();
    let area_of = |condition: usize| match &layout.conditions[condition] {
        ConditionDefinition::Presence(p) if !p.invert => names.get(&p.area),
        _ => None,
    };
    let (mut exits, mut others) = (Vec::new(), Vec::new());
    let mut exit_requirements: BTreeMap<String, Vec<ExitRequirement>> = BTreeMap::new();
    let mut seen = HashSet::new();
    let mut ordered: Vec<_> = layout.triggers.iter().collect();
    ordered.sort_by_key(|t| !exit_triggers.contains(&t.id));
    for trigger in ordered {
        for &condition in trigger.conditions.iter().flatten() {
            let Some(name) = area_of(condition) else {
                continue;
            };
            if exit_triggers.contains(&trigger.id) {
                for group in &trigger.conditions {
                    if group.contains(&condition) {
                        exit_requirements
                            .entry(name.clone())
                            .or_default()
                            .push(ExitRequirement {
                                trigger: trigger.id,
                                conditions: group
                                    .iter()
                                    .copied()
                                    .filter(|&c| c != condition)
                                    .collect(),
                            });
                    }
                }
            }
            if !seen.insert(name.clone()) {
                continue;
            }
            let Some(center) = s.world.area_center(name) else {
                continue;
            };
            if exit_triggers.contains(&trigger.id) {
                exits.push((name.clone(), center));
            } else {
                others.push((name.clone(), center));
            }
        }
    }
    let interactables = layout
        .interactables
        .iter()
        .filter_map(|d| {
            let at = s.world.locator_position(&d.locator)?;
            Some((d.locator.clone(), at))
        })
        .collect();
    Goals {
        exits,
        interactables,
        others,
        exit_requirements,
    }
}

/// Read-only planning: the bot still has to walk into the area and let the
/// runtime execute the complete trigger. Unknown prerequisites stay unready.
fn ready_exits(
    s: &PlaySession,
    requirements: &BTreeMap<String, Vec<ExitRequirement>>,
) -> BTreeSet<String> {
    requirements
        .iter()
        .filter(|(_, alternatives)| {
            alternatives.iter().any(|r| {
                s.runtime
                    .trigger(r.trigger)
                    .is_some_and(|t| !t.has_triggered)
                    && r.conditions
                        .iter()
                        .all(|&c| s.runtime.condition_met(c, None, &s.world).unwrap_or(false))
            })
        })
        .map(|(name, _)| name.clone())
        .collect()
}

/// `BOT_TRACE=1` logs every goal reached or given up.
fn trace() -> bool {
    std::env::var_os("BOT_TRACE").is_some()
}

fn flat_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}

/// The navmesh path to a goal, start first. Followed point by point: recomputing it
/// every tick oscillates around a first point that can lie just behind the wizard.
fn path_to(s: &PlaySession, from: [f32; 3], goal: [f32; 3]) -> Vec<[f32; 3]> {
    let path = s.world.nav.as_ref().and_then(|nav| {
        nav.find_shortest_path(from, goal, movement::DEFAULT, &|point| ice_below(s, point))
            .ok()
    });
    match path {
        // The navmesh can end short of an area: finish with a straight walk, as a
        // player would, but only that last stretch.
        // A partial route (not found) stays on walkable ground, so it is safe too.
        Some((_, nodes)) if !nodes.is_empty() => {
            let mut points: Vec<[f32; 3]> = nodes.iter().map(|n| n.position).collect();
            if flat_distance(points[points.len() - 1], goal) < LAST_STRETCH {
                points.push(goal);
            }
            points
        }
        // Nothing at all: walking straight at it could mean walking off a cliff.
        _ => Vec::new(),
    }
}

/// NavMesh's water test supplies a point one metre above the triangle centre.
/// Query the live liquid surface, so a player's frozen crossing is walkable and
/// thawed water is excluded again on the next route calculation.
fn ice_below(s: &PlaySession, point: [f32; 3]) -> bool {
    s.world
        .collision
        .collision()
        .liquid_segment_intersect(
            magicka_sim::spray_pulse::Segment {
                origin: point,
                delta: [0.0, -4.0, 0.0],
            },
            false,
            true,
            false,
        )
        .is_some()
}

fn remove_enemies(s: &mut PlaySession, position: [f32; 3], range: f32) -> usize {
    let hostile: Vec<u16> = s
        .world
        .entities
        .iter()
        .filter(|e| {
            e.added
                && !e.removed
                && !e.dead
                && matches!(e.kind, EntityKind::Npc { .. })
                && e.faction & FACTION_FRIENDLY != FACTION_FRIENDLY
                && flat_distance(e.position, position) < range
        })
        .map(|e| e.handle)
        .collect();
    for &handle in &hostile {
        s.world.terminate_character(handle);
    }
    hostile.len()
}

/// Boss fights are not under test either: every 5 s of a fight, a huge hit on each
/// boss's damage zones and on Jormungandr. Character bosses fall to `remove_enemies`.
fn hurt_bosses(s: &mut PlaySession) -> bool {
    let hit = Damage {
        attack: attack::DAMAGE,
        element: Elements::ARCANE.0,
        amount: 1_000_000.0,
        magnitude: 1.0,
    };
    let mut any = false;
    let mut triggers = Vec::new();
    for zone in s.world.physics_damage.values_mut() {
        // Death's zone is a boss zone too, and its delivery comes first.
        if zone.death_boss.is_some() {
            // Death only loses health to Life.
            let life = Damage {
                element: Elements::LIFE.0,
                ..hit
            };
            // At 1 it runs its own defeat; more hits could interrupt that.
            let hp = zone
                .death_boss
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .combat
                .hit_points();
            // `BOT_KEEP_HITTING=1` reproduces a player who never stops.
            if hp > 1.0 || std::env::var_os("BOT_KEEP_HITTING").is_some() {
                zone.deliver(&[life], 15, &mut triggers);
                any = true;
            }
        } else if zone.boss_zone.is_some() {
            zone.boss_inbox.push((hit, 15));
            any = true;
        }
    }
    for boss in s.world.jormungandr.values_mut() {
        boss.damage(&[hit], true);
        any = true;
    }
    any
}

fn usable(all: &[(String, [f32; 3])], deadly: &BTreeSet<String>) -> Vec<(String, [f32; 3])> {
    all.iter()
        .filter(|g| !deadly.contains(&g.0))
        .cloned()
        .collect()
}

/// When walking fails: go to a scripted prop and cast every element at it in turn, with
/// the real cast input. Many gates are props whose death or damage runs a trigger.
#[derive(Debug)]
struct Siege {
    target: u16,
    element: usize,
    timer: f32,
    /// Seconds spent getting there; a prop out of reach is given up.
    walking: f32,
    /// Health before the current element's attempt; keep a damaging spell.
    health_before: f32,
}

/// `ELEMENT_KEYS` indices: Arcane, Fire, Life, Lightning, Cold, Water, Earth.
const SIEGE_ELEMENTS: [usize; 7] = [5, 7, 1, 4, 3, 0, 6];
const SIEGE_RANGE: f32 = 8.0;
const SIEGE_SEARCH: f32 = 400.0;
const SIEGE_WALK_LIMIT: f32 = 40.0;
const CAST_START: f32 = 0.3;
const CAST_END: f32 = 2.0;
const ELEMENT_TIME: f32 = 2.2;

/// Do not besiege non-solid props just to switch their lights off.
/// Keep it if a predicate observes its presence or a callback's execution;
/// unsupported predicates also prevent this exclusion.
fn lighting_only_prop(layout: &SceneScriptLayout, unique_id: i32, zone: &PhysicsDamage) -> bool {
    if zone.solid || (zone.on_death == 0 && zone.on_damage == 0) {
        return false;
    }
    let callbacks = [zone.on_death, zone.on_damage];
    if layout.conditions.iter().any(|c| match c {
        ConditionDefinition::Known(c) => matches!(c.kind,
            ConditionKind::Triggered { id } if id != 0 && callbacks.contains(&id)),
        ConditionDefinition::Presence(p) => match p.kind {
            PresenceKind::Id { id } => id == unique_id,
            PresenceKind::Physics { .. } => true,
            _ => false,
        },
        ConditionDefinition::Unsupported(_) => true,
        _ => false,
    }) {
        return false;
    }
    callbacks.into_iter().filter(|&id| id != 0).all(|id| {
        layout
            .triggers
            .iter()
            .find(|t| t.id == id)
            .is_some_and(|t| {
                t.actions
                    .iter()
                    .flatten()
                    .all(|&i| match &layout.actions[i] {
                        ActionDefinition::Toggle(a) => matches!(a.kind, ToggleKind::Light { .. }),
                        _ => false,
                    })
            })
    })
}

/// Full-health predicates are authored healing goals, rather than a reason to
/// change an NPC's health directly. Only living friendly NPCs are candidates.
fn healing_goals(layout: &SceneScriptLayout) -> BTreeMap<i32, usize> {
    layout
        .conditions
        .iter()
        .enumerate()
        .filter_map(|(index, c)| match c {
            ConditionDefinition::World(c) if !c.invert => match c.kind {
                WorldConditionKind::CharacterHealth {
                    id,
                    compare: 0,
                    health: 1.0,
                } if id != 0 => Some((id, index)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn healing_target(
    s: &PlaySession,
    goals: &BTreeMap<i32, usize>,
    skipped: &BTreeSet<u16>,
) -> Option<u16> {
    s.world
        .entities
        .iter()
        .filter(|e| {
            e.added
                && !e.removed
                && !e.dead
                && e.faction & FACTION_FRIENDLY == FACTION_FRIENDLY
                && !skipped.contains(&e.handle)
        })
        .filter_map(|e| {
            let &condition = goals.get(&e.unique_id)?;
            let npc = s.world.npcs.get(&e.handle)?;
            let health = &npc.character.combat.health;
            (health.hit_points > 0.0
                && health.hit_points < health.max_hit_points
                && !s
                    .runtime
                    .condition_met(condition, None, &s.world)
                    .unwrap_or(false))
            .then_some((e.handle, flat_distance(e.position, s.body().position)))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(handle, _)| handle)
}

struct Healing {
    target: u16,
    timer: f32,
    elapsed: f32,
}

/// A live prop a script waits on, or a door, nearest first.
fn siege_target(s: &PlaySession, from: [f32; 3], done: &BTreeSet<u16>) -> Option<u16> {
    s.world
        .physics_damage
        .iter()
        .filter(|(handle, zone)| {
            !done.contains(handle)
                && zone.boss_zone.is_none()
                && zone.death_boss.is_none()
                && zone.health > 0.0
        })
        .filter_map(|(&handle, zone)| {
            let entity = s.world.entities.iter().find(|e| e.handle == handle)?;
            if !entity.added || entity.removed || entity.dead {
                return None;
            }
            let door = entity
                .template_name
                .as_deref()
                .is_some_and(|n| n.to_ascii_lowercase().contains("door"));
            if zone.on_death == 0 && zone.on_damage == 0 && !door {
                return None;
            }
            if lighting_only_prop(s.runtime.layout(), entity.unique_id, zone) {
                return None;
            }
            let distance = flat_distance(entity.position, from);
            (distance < SIEGE_SEARCH).then_some((handle, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(handle, _)| handle)
}

/// Kills a prop through the scripted entity command. The report counts these apart
/// from props broken by casting: this is not evidence that the gate can be passed.
fn force_kill(s: &mut PlaySession, handle: u16, report: &mut SceneReport) {
    if EntityCommandHost::kill(&mut s.world, handle).is_ok() {
        report.props_forced += 1;
    }
}

/// One scene, until it ends. `budget` is in seconds of game time.
fn play_scene(s: &mut PlaySession, budget: f32, level: &mut LevelReport) -> SceneReport {
    let start_scene = s.scene_name().to_string();
    let mut report = SceneReport {
        name: start_scene.clone(),
        ended: "budget".into(),
        ..Default::default()
    };
    // The tutorial's lessons need casting; the pause menu's skip fires this trigger.
    let tutorial = custom_hash("skip_tutorial");
    if s.runtime.layout().triggers.iter().any(|t| t.id == tutorial) {
        report.skipped_tutorial = s.run_trigger("skip_tutorial", true).unwrap_or(false);
    }
    let goals = goals(s);
    report.goals_total = goals.exits.len() + goals.interactables.len() + goals.others.len();
    let interactables: BTreeSet<String> = goals.interactables.iter().map(|g| g.0.clone()).collect();
    let exit_names: BTreeSet<String> = goals.exits.iter().map(|g| g.0.clone()).collect();
    let all: Vec<(String, [f32; 3])> = goals
        .exits
        .into_iter()
        .chain(goals.interactables)
        .chain(goals.others)
        .collect();
    let requirements = goals.exit_requirements;
    let mut previously_ready = ready_exits(s, &requirements);
    let mut reached: BTreeSet<String> = BTreeSet::new();
    let mut queue = usable(&all, &level.deadly_goals);
    let (mut anchor, mut still) = (s.body().position, 0.0_f32);
    let (mut pushing_anchor, mut pushing) = (s.body().position, 0.0_f32);
    let mut frame = 0_u64;
    let (mut path, mut path_goal): (Vec<[f32; 3]>, String) = (Vec::new(), String::new());
    let mut siege: Option<Siege> = None;
    let heal_goals = healing_goals(s.runtime.layout());
    let life_key = magicka_runtime::avatar::ELEMENT_KEYS
        .iter()
        .position(|&element| element == Elements::LIFE)
        .expect("Life input key");
    let mut healing: Option<Healing> = None;
    let mut healing_skipped = BTreeSet::new();
    let mut done: BTreeSet<u16> = BTreeSet::new();
    while report.seconds < budget {
        frame += 1;
        report.seconds += TICK_DT;
        let position = s.body().position;
        let free_to_heal =
            !s.input_limited() && !s.world.flags.cutscene && !s.avatar().ai.controls_avatar();
        if healing.is_none() && siege.is_none() && free_to_heal {
            healing = healing_target(s, &heal_goals, &healing_skipped).map(|target| {
                if trace() {
                    eprintln!("  [{:6.1}] healing character {target}", report.seconds);
                }
                Healing {
                    target,
                    timer: 0.0,
                    elapsed: 0.0,
                }
            });
        }
        if frame.is_multiple_of(30) && siege.is_none() && healing.is_none() {
            let ready = ready_exits(s, &requirements);
            let newly_ready: Vec<_> = all
                .iter()
                .filter(|(name, _)| {
                    ready.contains(name)
                        && !previously_ready.contains(name)
                        && !level.deadly_goals.contains(name)
                })
                .cloned()
                .collect();
            for goal in newly_ready.into_iter().rev() {
                if trace() {
                    eprintln!(
                        "  [{:6.1}] exit prerequisites met: {}",
                        report.seconds, goal.0
                    );
                }
                queue.retain(|g| g.0 != goal.0);
                queue.insert(0, goal);
                path.clear();
                path_goal.clear();
                still = 0.0;
            }
            previously_ready = ready;
        }
        let dialog_click = frame.is_multiple_of(45);
        // Goals rotate: an exit that cannot be reached yet goes to the back of the
        // queue, so walking elsewhere can open it.
        let mut no_route = false;
        let movement_input =
            queue
                .first()
                .filter(|_| healing.is_none())
                .and_then(|(name, center)| {
                    if path_goal != *name || path.is_empty() {
                        path = path_to(s, position, *center);
                        path_goal = name.clone();
                    }
                    if path.is_empty() {
                        no_route = true;
                        return None;
                    }
                    let reach = if pushing > 1.0 {
                        WAYPOINT_REACHED_WHEN_STILL
                    } else {
                        WAYPOINT_REACHED
                    };
                    while path.len() > 1 && flat_distance(path[0], position) < reach {
                        path.remove(0);
                    }
                    let (dx, dz) = (path[0][0] - position[0], path[0][2] - position[2]);
                    let length = dx.hypot(dz).max(1e-3);
                    Some([dx / length, dz / length])
                });
        if no_route {
            let goal = queue.remove(0);
            queue.push(goal);
            path_goal.clear();
        }
        // The siege, if any, overrides walking.
        let (mut elements, mut cast_down) = ([false; 8], false);
        let mut movement_input = movement_input;
        if let Some(current) = siege.as_mut() {
            let target = s
                .world
                .entities
                .iter()
                .find(|e| e.handle == current.target && e.added && !e.removed && !e.dead)
                .map(|e| e.position)
                .filter(|_| {
                    s.world
                        .physics_damage
                        .get(&current.target)
                        .is_some_and(|z| z.health > 0.0)
                });
            match target {
                None => {
                    report.props_broken += 1;
                    siege = None;
                    path.clear();
                }
                Some(at) => {
                    let (dx, dz) = (at[0] - position[0], at[2] - position[2]);
                    let distance = dx.hypot(dz).max(1e-3);
                    if distance > SIEGE_RANGE && current.walking < SIEGE_WALK_LIMIT {
                        current.walking += TICK_DT;
                        if path_goal != "#siege" || path.is_empty() {
                            path = path_to(s, position, at);
                            path_goal = "#siege".into();
                        }
                        while path.len() > 1 && flat_distance(path[0], position) < WAYPOINT_REACHED
                        {
                            path.remove(0);
                        }
                        let step = path.first().copied().unwrap_or(at);
                        let (wx, wz) = (step[0] - position[0], step[2] - position[2]);
                        let length = wx.hypot(wz).max(1e-3);
                        movement_input = Some([wx / length, wz / length]);
                    } else if distance > SIEGE_RANGE {
                        // Out of reach: run its script anyway, and count that.
                        done.insert(current.target);
                        force_kill(s, current.target, &mut report);
                        siege = None;
                        path.clear();
                    } else {
                        // A faint push only turns the wizard to face it.
                        movement_input = Some([dx / distance * 0.05, dz / distance * 0.05]);
                        if current.timer < TICK_DT {
                            elements[SIEGE_ELEMENTS[current.element]] = true;
                        }
                        cast_down = (CAST_START..CAST_END).contains(&current.timer);
                        if trace()
                            && ((current.timer / TICK_DT).round() as usize).is_multiple_of(30)
                        {
                            eprintln!(
                                "      element {} t {:.1} queue {} beams {} state {:?} facing {:?} to {:?} distance {:.1} limited {} hp {:?}",
                                SIEGE_ELEMENTS[current.element],
                                current.timer,
                                s.avatar().conjure.queue.len(),
                                s.beams.len(),
                                s.avatar().motion.state,
                                s.body().direction().map(|v| (v * 10.0).round() / 10.0),
                                [(dx / distance * 10.0).round() / 10.0, (dz / distance * 10.0).round() / 10.0],
                                distance,
                                s.input_limited(),
                                s.world.physics_damage.get(&current.target).map(|z| z.health),
                            );
                        }
                        current.timer += TICK_DT;
                        if current.timer > ELEMENT_TIME {
                            current.timer = 0.0;
                            let health = s.world.physics_damage[&current.target].health;
                            let damage = current.health_before - health;
                            // Some scripted detectors have effectively unbreakable
                            // health. Keep a useful spell only if its measured rate
                            // can finish within this scene's remaining budget.
                            let can_finish = damage > 0.0
                                && health / damage * ELEMENT_TIME <= budget - report.seconds;
                            if !can_finish {
                                current.element += 1;
                            }
                            current.health_before = health;
                            if current.element == SIEGE_ELEMENTS.len() {
                                // No element broke it, or it could not be hit.
                                done.insert(current.target);
                                force_kill(s, current.target, &mut report);
                                siege = None;
                                path.clear();
                            }
                        }
                    }
                }
            }
        }
        if let Some(current) = healing.as_mut() {
            // Yield to authored cutscenes and AvatarMove. Life is queued and
            // cast through the same pad controls as the prop siege.
            movement_input = None;
            if free_to_heal {
                let target = s
                    .world
                    .entities
                    .iter()
                    .find(|e| e.handle == current.target && e.added && !e.removed && !e.dead)
                    .and_then(|e| {
                        let npc = s.world.npcs.get(&e.handle)?;
                        (npc.character.combat.health.hit_points > 0.0).then(|| {
                            let full = heal_goals.get(&e.unique_id).is_some_and(|&c| {
                                s.runtime.condition_met(c, None, &s.world).unwrap_or(false)
                            });
                            (e.position, full)
                        })
                    });
                current.elapsed += TICK_DT;
                let target =
                    target.filter(|(_, full)| *full || current.elapsed <= SIEGE_WALK_LIMIT);
                match target {
                    Some((_, true)) => {
                        report.characters_healed += 1;
                        if trace() {
                            eprintln!(
                                "  [{:6.1}] healed character {}",
                                report.seconds, current.target
                            );
                        }
                        healing = None;
                        path.clear();
                    }
                    // Reuse the bot's approach budget; an unreachable target is
                    // reported and skipped, never healed by editing its HP.
                    None => {
                        healing_skipped.insert(current.target);
                        report.healing_abandoned += 1;
                        if trace() {
                            eprintln!(
                                "  [{:6.1}] abandoned healing character {}",
                                report.seconds, current.target
                            );
                        }
                        healing = None;
                        path.clear();
                    }
                    Some((at, false)) => {
                        let (dx, dz) = (at[0] - position[0], at[2] - position[2]);
                        let distance = dx.hypot(dz).max(1e-3);
                        if distance > SIEGE_RANGE {
                            if path_goal != "#heal" || path.is_empty() {
                                path = path_to(s, position, at);
                                path_goal = "#heal".into();
                            }
                            while path.len() > 1
                                && flat_distance(path[0], position) < WAYPOINT_REACHED
                            {
                                path.remove(0);
                            }
                            if let Some(step) = path.first() {
                                let (wx, wz) = (step[0] - position[0], step[2] - position[2]);
                                let length = wx.hypot(wz).max(1e-3);
                                movement_input = Some([wx / length, wz / length]);
                            }
                        } else {
                            movement_input = Some([dx / distance * 0.05, dz / distance * 0.05]);
                            elements[life_key] = current.timer < TICK_DT;
                            cast_down = (CAST_START..CAST_END).contains(&current.timer);
                            current.timer += TICK_DT;
                            if current.timer > ELEMENT_TIME {
                                current.timer = 0.0;
                            }
                        }
                    }
                }
            }
        }
        s.handle_input_with_dialog_select(
            PlayerInput {
                elements,
                cast_down,
                pad_movement: Some(movement_input.unwrap_or([0.0; 2])),
                // Interacts with whatever stands in reach, and advances dialogs.
                move_down: frame.is_multiple_of(30) && healing.is_none(),
                // When blocked, swing once at whatever is in the way. Repeated swings
                // would root the wizard in the attack state.
                pad_attack: healing.is_none() && still > 1.0 && still < 1.1,
                ..Default::default()
            },
            ASPECT,
            dialog_click,
        );
        // `BOT_WATCH=1` lets cutscenes play instead of skipping them.
        if frame.is_multiple_of(30) && std::env::var_os("BOT_WATCH").is_none() {
            s.skip_cutscene();
        }
        s.step(TICK_DT);
        // Health is locked: this measures progression only.
        if let Some(avatar) = s.world.players[0].as_mut() {
            let health = &mut avatar.character.combat.health;
            health.hit_points = health.max_hit_points;
        }
        if trace() && frame.is_multiple_of(120) {
            let a = s.avatar();
            eprintln!(
                "  [{:6.1}] walk: goal {:?} at {:?} velocity {:?} movement {:?} state {:?} next {:?} ({} left) ai {} limited {} still {still:.1}",
                report.seconds,
                queue.first().map(|(n, _)| n.as_str()),
                s.body().position.map(|v| (v * 10.0).round() / 10.0),
                s.body().velocity.map(|v| (v * 100.0).round() / 100.0),
                s.body().movement().map(|v| (v * 100.0).round() / 100.0),
                a.motion.state,
                path.first().map(|p| p.map(|v| (v * 10.0).round() / 10.0)),
                path.len(),
                a.ai.controls_avatar(),
                s.input_limited(),
            );
        }
        if trace() && frame.is_multiple_of(300) {
            let fight = &s.world.boss_fight;
            let (dead, running, count) = (fight.dead, fight.running, fight.bosses.len());
            let death_hp: Vec<(f32, String, bool, bool)> = fight
                .bosses
                .iter()
                .filter_map(|b| match b {
                    BossRef::Death(d) => {
                        let a = d.actor.lock().unwrap();
                        Some((
                            a.combat.hit_points(),
                            format!("{:?}", a.state),
                            a.drawn,
                            a.ethereal,
                        ))
                    }
                    _ => None,
                })
                .collect();
            eprintln!(
                "  [{:6.1}] boss fight: {count} bosses, running {running}, dead {dead}, Death hp {death_hp:?}",
                report.seconds
            );
        }
        // Only in a running fight, as a player's input is off before that.
        if frame.is_multiple_of(300) && s.world.boss_fight.running && hurt_bosses(s) {
            report.boss_hits += 1;
        }
        // `BOT_KEEP_ENEMIES=1` tells a script hang from a removed NPC the script waits on.
        // Never in a cutscene or with input off: a script can be waiting on that NPC's
        // animation, as tmd_s5BF's boss entrance is.
        if frame.is_multiple_of(15)
            && !s.input_limited()
            && !s.world.flags.cutscene
            && std::env::var_os("BOT_KEEP_ENEMIES").is_none()
        {
            report.removed_enemies += remove_enemies(s, position, ENEMY_RANGE);
        }
        for event in s.events.drain(..) {
            if let SessionEvent::ScriptSkipped(name) = event {
                level.skipped_actions.insert(name);
            }
        }
        // Health is locked, so this is a fall or a script's kill: a player would
        // restart from the checkpoint. Locked health would otherwise fall forever.
        if s.dead() || s.body().position[1] < FELL_OUT {
            report.deaths += 1;
            level.deaths += 1;
            // Whatever it was walking to killed it: not that one again.
            if let Some((name, _)) = queue.first() {
                level.deadly_goals.insert(name.clone());
                let name = name.clone();
                queue.retain(|g| g.0 != name);
            }
            // Past the kill plane and not dead would be a game bug, not a bot limit.
            if !s.dead() {
                report.fell_alive += 1;
            }
            if level.deaths > MAX_DEATHS {
                report.ended = "died".into();
                return report;
            }
            match s.restart_at_checkpoint() {
                Ok(true) => {}
                Ok(false) => {
                    if let Err(e) = s.restart_level() {
                        level.script_error = Some(format!("restart: {e:?}"));
                        report.ended = "error".into();
                        return report;
                    }
                }
                Err(e) => {
                    level.script_error = Some(format!("checkpoint: {e:?}"));
                    report.ended = "error".into();
                    return report;
                }
            }
            path.clear();
        }
        if let Some(error) = &s.script_error {
            level.script_error = Some(format!("{}: {error:?}", s.scene_name()));
            report.ended = "error".into();
            return report;
        }
        if s.end_game.level_complete {
            report.ended = "level complete".into();
            return report;
        }
        if s.scene_name() != start_scene {
            report.ended = "scene change".into();
            return report;
        }
        // Arrival and blockage.
        let now = s.body().position;
        report.walked += flat_distance(now, position);
        report.final_position = now;
        report.input_limited += TICK_DT * f32::from(u8::from(s.input_limited()));
        report.cutscene += TICK_DT * f32::from(u8::from(s.world.flags.cutscene));
        report.tutorial_modal += TICK_DT * f32::from(u8::from(!s.world.tutorial.dialog_done()));
        report.final_goal = queue.first().map_or(String::new(), |g| g.0.clone());
        if flat_distance(now, anchor) > 0.25 {
            anchor = now;
            still = 0.0;
        } else if !s.input_limited() {
            still += TICK_DT;
        }
        // Pushing against something: movement asked for, none happening. Not
        // the same as standing still (Tsar castle's start idles free for a
        // moment before the first ramp node).
        let m = s.body().movement();
        if flat_distance(now, pushing_anchor) > 0.25 || m[0].hypot(m[2]) < 0.1 {
            pushing_anchor = now;
            pushing = 0.0;
        } else {
            pushing += TICK_DT;
        }
        // Any area walked through counts, not only the one being walked to.
        for (name, _) in &all {
            if !reached.contains(name) && s.world.area_contains(name, now) {
                reached.insert(name.clone());
                report.goals_reached += 1;
            }
        }
        if let Some((name, center)) = queue.first().cloned() {
            // An area counts on entering its box, as its trigger does. Walking
            // near a lever does not mean the action button actually activated it.
            let arrived = if interactables.contains(&name) {
                let id = custom_hash(&name.to_ascii_lowercase());
                s.world.flags.interactables_disabled.contains(&id)
                    || s.runtime.trigger(id).is_some_and(|t| t.has_triggered)
            } else {
                s.world.area_contains(&name, now)
                    || flat_distance(now, center) < AREA_CENTRE_REACHED
            };
            if trace() && !arrived && still > STUCK_SECONDS {
                // What stands in the way.
                let near: Vec<String> = s
                    .world
                    .entities
                    .iter()
                    .filter(|e| e.added && !e.removed && !e.dead)
                    .filter(|e| flat_distance(e.position, now) < 5.0 + e.radius)
                    .map(|e| {
                        format!(
                            "{:?}@{:?} r{:.1}",
                            std::mem::discriminant(&e.kind),
                            e.position.map(|v| v.round()),
                            e.radius
                        )
                    })
                    .take(8)
                    .collect();
                eprintln!("      near: {near:?}");
                let a = s.avatar();
                eprintln!(
                    "      ai {} path {:?} state {:?} velocity {:?} movement {:?} waypoints {} first {:?} limited {} locks {} dialog {}",
                    a.ai.controls_avatar(),
                    a.ai.path.len(),
                    a.motion.state,
                    s.body().velocity.map(|v| (v * 100.0).round() / 100.0),
                    s.body().movement().map(|v| (v * 100.0).round() / 100.0),
                    path.len(),
                    path.first().map(|p| p.map(|v| v.round())),
                    s.input_limited(),
                    s.world.player_input_locked[0],
                    s.world.dialogs.as_ref().is_some_and(|m| m.can_advance(Some(0))),
                );
            }
            if trace() && (arrived || still > STUCK_SECONDS) {
                eprintln!(
                    "  [{:6.1}] {} {name} at {:?}, goal centre {:?}",
                    report.seconds,
                    if arrived { "reached" } else { "stuck on" },
                    now.map(|v| v.round()),
                    center.map(|v| v.round()),
                );
            }
            if arrived {
                if reached.insert(name.clone()) {
                    report.goals_reached += 1;
                }
                queue.remove(0);
                still = 0.0;
            } else if siege.is_some() || healing.is_some() {
                still = 0.0;
            } else if still > STUCK_SECONDS {
                // Something may have to die first; then come back to this goal.
                still = 0.0;
                if std::env::var_os("BOT_KEEP_ENEMIES").is_none() {
                    report.removed_enemies += remove_enemies(s, now, f32::INFINITY);
                }
                let goal = queue.remove(0);
                queue.push(goal);
                path.clear();
                if siege.is_none() {
                    if let Some(target) = siege_target(s, now, &done) {
                        if trace() {
                            let e = s.world.entities.iter().find(|e| e.handle == target);
                            eprintln!(
                                "  [{:6.1}] siege {:?} at {:?}, hp {:?}",
                                report.seconds,
                                e.and_then(|e| e.template_name.clone()),
                                e.map(|e| e.position.map(|v| v.round())),
                                s.world.physics_damage.get(&target).map(|z| z.health),
                            );
                        }
                        siege = Some(Siege {
                            target,
                            element: 0,
                            timer: 0.0,
                            walking: 0.0,
                            health_before: s.world.physics_damage[&target].health,
                        });
                    }
                }
            }
        } else {
            // Every goal visited, and the scene goes on: scripts may have opened
            // something, so go round again.
            queue = usable(&all, &level.deadly_goals);
            still = 0.0;
            // Touring goals did not end the scene: a prop may have to go first.
            if siege.is_none() && healing.is_none() {
                siege = siege_target(s, now, &done).map(|target| Siege {
                    target,
                    element: 0,
                    timer: 0.0,
                    walking: 0.0,
                    health_before: s.world.physics_damage[&target].health,
                });
            }
        }
    }
    report.unreached_exits = exit_names.difference(&reached).cloned().collect();
    report
}

fn play_level(content: &Path, level: &str, budget: f32) -> LevelReport {
    let mut report = LevelReport {
        level: level.into(),
        ..Default::default()
    };
    let mut s = match PlaySession::load(content, level, "Wizard") {
        Ok(s) => s,
        Err(e) => {
            report.script_error = Some(format!("load: {e:?}"));
            return report;
        }
    };
    s.headless_dialogs = Some([1280.0, 720.0]);
    for _ in 0..64 {
        let scene = play_scene(&mut s, budget, &mut report);
        let ended = scene.ended.clone();
        report.scenes.push(scene);
        // Restarting at the level start can replay scenes forever.
        if report
            .scenes
            .iter()
            .filter(|c| c.name == report.scenes.last().unwrap().name)
            .count()
            > 3
        {
            break;
        }
        match ended.as_str() {
            "scene change" => continue,
            "level complete" => report.completed = true,
            _ => {}
        }
        break;
    }
    report
}

fn json_list(items: impl IntoIterator<Item = String>) -> String {
    let items: Vec<String> = items.into_iter().map(|i| format!("{i:?}")).collect();
    format!("[{}]", items.join(","))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut out: Option<PathBuf> = None;
    let mut minutes = 8.0_f32;
    let mut positional = Vec::new();
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--out" => out = it.next().map(PathBuf::from),
            "--minutes-per-scene" => {
                minutes = it.next().and_then(|v| v.parse().ok()).expect("minutes");
            }
            _ => positional.push(a),
        }
    }
    if positional.is_empty() {
        eprintln!("usage: campaign-bot <content-dir> [--out report.json] [--minutes-per-scene M] [level.lvl...]");
        std::process::exit(2);
    }
    let content = PathBuf::from(positional.remove(0));
    let levels = if positional.is_empty() {
        magicka_runtime::end_game::VANILLA_CAMPAIGN
            .iter()
            .chain(magicka_runtime::end_game::MYTHOS_CAMPAIGN.iter())
            .map(|l| l.to_string())
            .collect()
    } else {
        positional
    };
    std::panic::set_hook(Box::new(|_| {}));
    let mut lines = Vec::new();
    let mut failed = false;
    for level in &levels {
        let path = content.clone();
        let name = level.clone();
        let report = std::panic::catch_unwind(move || play_level(&path, &name, minutes * 60.0))
            .unwrap_or_else(|p| LevelReport {
                level: level.clone(),
                panic: Some(
                    p.downcast_ref::<String>()
                        .cloned()
                        .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                        .unwrap_or_else(|| "panic".into()),
                ),
                ..Default::default()
            });
        failed |= report.script_error.is_some() || report.panic.is_some();
        let changes = report
            .scenes
            .iter()
            .filter(|c| c.ended == "scene change")
            .count();
        if let Some((_, expected, complete)) = BASELINE.iter().find(|b| b.0 == level) {
            if minutes >= 5.0 && (changes < *expected || (*complete && !report.completed)) {
                println!("REGRESSION {level}: {changes} scene changes, baseline {expected}");
                failed = true;
            }
            if minutes >= 5.0 && *complete && report.scenes.iter().any(|s| s.props_forced > 0) {
                println!("REGRESSION {level}: completed baseline requires no forced prop kills");
                failed = true;
            }
        }
        println!(
            "{}: {} scenes, {}{}{}",
            report.level,
            report.scenes.len(),
            if report.completed {
                "completed"
            } else {
                "stalled"
            },
            report
                .script_error
                .as_ref()
                .map_or(String::new(), |e| format!(", script error {e}")),
            report
                .panic
                .as_ref()
                .map_or(String::new(), |e| format!(", panic {e}")),
        );
        for scene in &report.scenes {
            println!(
                "  {:<16} {:>6.1} s  {:<14} goals {}/{}, enemies {}, boss hits {}, props {} (forced {}), healed {} (abandoned {}), deaths {} (alive {}), walked {:.0} m, limited {:.0} s, cutscene {:.0} s, modal {:.0} s, at {:?} to {}{}",
                scene.name,
                scene.seconds,
                scene.ended,
                scene.goals_reached,
                scene.goals_total,
                scene.removed_enemies,
                scene.boss_hits,
                scene.props_broken,
                scene.props_forced,
                scene.characters_healed,
                scene.healing_abandoned,
                scene.deaths,
                scene.fell_alive,
                scene.walked,
                scene.input_limited,
                scene.cutscene,
                scene.tutorial_modal,
                scene.final_position.map(|v| v.round()),
                scene.final_goal,
                if scene.unreached_exits.is_empty() {
                    String::new()
                } else {
                    format!(", unreached exits {:?}", scene.unreached_exits)
                },
            );
        }
        let scenes: Vec<String> = report
            .scenes
            .iter()
            .map(|c| {
                format!(
                    "{{\"name\":{:?},\"seconds\":{:.1},\"ended\":{:?},\"goals_reached\":{},\"goals_total\":{},\"removed_enemies\":{},\"props_broken\":{},\"props_forced\":{},\"characters_healed\":{},\"healing_abandoned\":{},\"unreached_exits\":{}}}",
                    c.name, c.seconds, c.ended, c.goals_reached, c.goals_total, c.removed_enemies,
                    c.props_broken,
                    c.props_forced,
                    c.characters_healed,
                    c.healing_abandoned,
                    json_list(c.unreached_exits.iter().cloned())
                )
            })
            .collect();
        lines.push(format!(
            "{{\"level\":{:?},\"completed\":{},\"script_error\":{:?},\"panic\":{:?},\"skipped\":{},\"scenes\":[{}]}}",
            report.level,
            report.completed,
            report.script_error.as_deref().unwrap_or(""),
            report.panic.as_deref().unwrap_or(""),
            json_list(report.skipped_actions.iter().cloned()),
            scenes.join(",")
        ));
    }
    if let Some(out) = out {
        std::fs::write(&out, format!("[\n{}\n]\n", lines.join(",\n"))).expect("write report");
    }
    std::process::exit(i32::from(failed));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swamp_bot_heals_the_villagers_through_life_casting_and_leaves_the_scene() {
        let root = magicka_content::retail_content_dir();
        if !root.is_dir() {
            return;
        }
        let mut s = PlaySession::load(&root, "Swamp.lvl", "Wizard").unwrap();
        s.change_scene_named("sw_s5", "start").unwrap();
        s.headless_dialogs = Some([1280.0, 720.0]);
        let report = play_scene(&mut s, 200.0, &mut LevelReport::default());
        eprintln!(
            "village traversal: {report:?}; all_healed {}",
            s.runtime.variables.counter(custom_hash("all_healed"))
        );
        assert_eq!(report.ended, "scene change", "{report:?}");
        assert_eq!(s.scene_name(), "sw_s6");
        assert_eq!(s.runtime.variables.counter(custom_hash("all_healed")), 1);
        assert_eq!(report.characters_healed, 6);
        assert_eq!(report.healing_abandoned, 0);
        assert_eq!(report.props_forced, 0);
        assert_eq!(report.deaths, 0);
        assert!(s.script_error.is_none(), "{:?}", s.script_error);
    }

    #[test]
    fn havindr_bot_completes_without_forcing_the_fire_props() {
        let root = magicka_content::retail_content_dir();
        if !root.is_dir() {
            return;
        }
        let report = play_level(&root, "Havindr.lvl", 300.0);
        eprintln!("Havindr traversal: {report:?}");
        assert!(report.completed, "{report:?}");
        assert!(report.script_error.is_none(), "{:?}", report.script_error);
        assert!(report
            .scenes
            .iter()
            .all(|s| s.props_forced == 0 && s.deaths == 0));
    }

    #[test]
    fn lighting_props_stay_siege_targets_when_solid_or_observed_by_the_script() {
        use magicka_runtime::{script_condition::ScriptCondition, script_runtime::ScriptRuntime};
        let root = magicka_content::retail_content_dir();
        if !root.is_dir() {
            return;
        }
        let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        s.change_scene_named("havindr_s4", "start").unwrap();
        for _ in 0..2 {
            s.step(TICK_DT);
        }
        let fire = s
            .world
            .entities
            .iter()
            .find(|e| {
                e.added
                    && !e.removed
                    && e.template_name
                        .as_deref()
                        .is_some_and(|n| n.eq_ignore_ascii_case("havindr_fire"))
            })
            .unwrap();
        let (handle, position, unique_id) = (fire.handle, fire.position, fire.unique_id);
        let done = BTreeSet::new();
        assert_eq!(
            siege_target(&s, position, &done),
            None,
            "unobserved, non-solid light props must not start a siege"
        );
        s.world.physics_damage.get_mut(&handle).unwrap().solid = true;
        assert_eq!(
            siege_target(&s, position, &done),
            Some(handle),
            "physical obstacles remain eligible"
        );
        s.world.physics_damage.get_mut(&handle).unwrap().solid = false;
        let original = s.runtime.layout().clone();
        let mut layout = original.clone();
        let callback = s.world.physics_damage[&handle].on_death;
        layout
            .conditions
            .push(ConditionDefinition::Known(ScriptCondition {
                invert: false,
                kind: ConditionKind::Triggered { id: callback },
            }));
        s.runtime = ScriptRuntime::new(layout, s.runtime.variables.clone());
        assert_eq!(
            siege_target(&s, position, &done),
            Some(handle),
            "observed death triggers remain eligible"
        );
        let mut layout = original.clone();
        layout.conditions.push(ConditionDefinition::Presence(
            magicka_runtime::presence::PresenceCondition {
                area: custom_hash("any"),
                invert: false,
                kind: PresenceKind::Id { id: unique_id },
            },
        ));
        s.runtime = ScriptRuntime::new(layout, s.runtime.variables.clone());
        assert_eq!(
            siege_target(&s, position, &done),
            Some(handle),
            "observed entity presence remains eligible"
        );
        let mut layout = original;
        layout.conditions.push(ConditionDefinition::Presence(
            magicka_runtime::presence::PresenceCondition {
                area: custom_hash("any"),
                invert: false,
                kind: PresenceKind::Physics {
                    only_damageable: true,
                    compare: -1,
                    number: 1,
                },
            },
        ));
        s.runtime = ScriptRuntime::new(layout, s.runtime.variables.clone());
        assert_eq!(
            siege_target(&s, position, &done),
            Some(handle),
            "physics counts remain eligible"
        );
    }

    #[test]
    fn havindr_bot_returns_to_the_exit_after_finishing_the_market() {
        let root = magicka_content::retail_content_dir();
        if !root.is_dir() {
            return;
        }
        let mut s = PlaySession::load(&root, "Havindr.lvl", "Wizard").unwrap();
        s.headless_dialogs = Some([1280.0, 720.0]);
        let mut level = LevelReport::default();
        for _ in 0..2 {
            assert_eq!(play_scene(&mut s, 300.0, &mut level).ended, "scene change");
        }
        assert_eq!(s.scene_name(), "havindr_s3");
        let report = play_scene(&mut s, 300.0, &mut level);
        eprintln!("market traversal: {report:?}");
        assert_eq!(report.ended, "scene change", "{report:?}");
        assert_eq!(s.scene_name(), "havindr_s4");
        assert_eq!(report.deaths, 0);
        assert_eq!(report.props_forced, 0);
        assert!(s.script_error.is_none(), "{:?}", s.script_error);
    }

    /// Enter through the preceding scenes: a direct bf_s3 load changes when
    /// the wizard reaches the first lever relative to periodic action presses.
    #[test]
    fn battlefield_bot_activates_bridges_and_completes_the_chapter() {
        let root = magicka_content::retail_content_dir();
        if !root.is_dir() {
            return;
        }
        let report = play_level(&root, "Battlefield.lvl", 300.0);
        assert!(report.script_error.is_none(), "{:?}", report.script_error);
        assert!(report.panic.is_none(), "{:?}", report.panic);
        let bridge_scene = report
            .scenes
            .iter()
            .find(|s| s.name == "bf_s3")
            .expect("the bot must enter the bridge scene");
        assert_eq!(bridge_scene.ended, "scene change", "{bridge_scene:?}");
        assert!(report.completed, "{report:?}");
        assert!(report
            .scenes
            .iter()
            .all(|s| s.deaths == 0 && s.props_forced == 0));
    }

    #[test]
    fn battlefield_route_uses_existing_ice_and_rejects_thawed_water() {
        let root = magicka_content::retail_content_dir();
        if !root.is_dir() {
            return;
        }
        let mut s = PlaySession::load(&root, "Battlefield.lvl", "Wizard").unwrap();
        s.change_scene_named("bf_s2", "start").unwrap();
        let exit = s.world.area_center("trigger_exit").unwrap();
        let shore = [39.0, -0.3, -75.1];
        let route = |s: &PlaySession, live: bool| {
            s.world
                .nav
                .as_ref()
                .unwrap()
                .find_shortest_path(shore, exit, movement::DEFAULT, &|p| live && ice_below(s, p))
                .unwrap()
        };
        let (old_found, old_nodes) = route(&s, false);
        let (found, nodes) = route(&s, true);
        eprintln!("no ice callback: complete {old_found}, last {:?}; live ice: complete {found}, last {:?}",
            old_nodes.last().map(|n| n.position), nodes.last().map(|n| n.position));
        assert!(!old_found);
        assert!(found, "the existing frozen river must connect to the exit");
        let points = path_to(&s, shore, exit);
        assert!(flat_distance(*points.last().unwrap(), exit) < AREA_CENTRE_REACHED);
        // A thaw changes the next query; routing must not cache frozen support.
        for water in &mut s.world.collision.collision_mut().water {
            water.freeze.vertices.fill(0.0);
        }
        assert!(
            !route(&s, true).0,
            "thawed water must block the route again"
        );
    }
}
