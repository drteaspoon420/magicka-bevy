//! Retail staves (skips without user-supplied content): item auras give
//! their buffs to the holder and nearby characters, passives act on the
//! holder, and special abilities execute. See research/staff-items.md.
use magicka_runtime::{
    avatar::PlayerInput,
    generic_aura::BuffKind,
    play_content::load_start_scene,
    play_session::{PlaySession, SessionEvent},
    scene_script::{ActionDefinition, SceneScriptLayout},
    spawn_action::SpawnState,
};
use magicka_sim::spells::Elements as E;
use std::path::{Path, PathBuf};

const DT: f32 = 1.0 / 60.0;
const LEVEL: &str = "Havindr.lvl";

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

fn frame(s: &mut PlaySession, input: PlayerInput) -> Vec<SessionEvent> {
    s.step(DT);
    s.handle_input(input, 16.0 / 9.0);
    std::mem::take(&mut s.events)
}

fn session(root: &Path) -> PlaySession {
    let mut s = PlaySession::load(root, LEVEL, "Wizard").unwrap();
    s.disable_scripts();
    for _ in 0..60 {
        frame(&mut s, PlayerInput::default());
    }
    s
}

fn equip_staff(s: &mut PlaySession, root: &Path, name: &str) {
    let bytes = std::fs::read(root.join(format!("Data/items/Wizard/{name}.xnb"))).unwrap();
    let xnb = magicka_xnb::xnb::parse(&bytes).unwrap();
    let magicka_content::Content::Item(item) = magicka_content::parse_primary(&xnb)
        .unwrap()
        .unwrap()
        .content
    else {
        panic!("not an item");
    };
    let avatar = s.world.players[0].as_mut().unwrap();
    avatar.items.retain(|(slot, _)| *slot != 1);
    avatar
        .items
        .push((1, magicka_runtime::avatar::AvatarItem::new(*item)));
}

fn spawn_in_front(s: &mut PlaySession, root: &Path, template: &str, distance: f32) -> u16 {
    let area = load_start_scene(root, LEVEL).unwrap().start.area.unwrap();
    let xml = format!(
        r#"<Scene><trigger id="t"><then><spawn type="{template}" area="{area}1" order="idle"/></then></trigger></Scene>"#
    );
    let ActionDefinition::Spawn(action) = SceneScriptLayout::read(xml.as_bytes(), false)
        .unwrap()
        .actions
        .remove(0)
    else {
        panic!("not a spawn action");
    };
    let mut state = SpawnState::default();
    action.initialize(&mut state, &mut s.world).unwrap();
    action.execute(&mut state, &mut s.world).unwrap();
    let handle = state.spawned[0].handle.unwrap();
    let (position, forward) = (s.body().position, s.body().direction());
    let npc = &s.world.npcs[&handle];
    let body = &mut s.world.physics.bodies[npc.body];
    let target = [
        position[0] + forward[0] * distance,
        body.position[1].max(position[1]),
        position[2] + forward[2] * distance,
    ];
    let orientation = body.orientation;
    body.move_to(target, orientation);
    handle
}

fn has_buff(kinds: &[BuffKind], f: impl Fn(&BuffKind) -> bool) -> bool {
    kinds.iter().any(f)
}

fn player_buffs(s: &PlaySession) -> Vec<BuffKind> {
    s.avatar()
        .character
        .buffs
        .general
        .list
        .iter()
        .map(|b| b.kind)
        .collect()
}

#[test]
fn own_item_auras_buff_only_the_holder() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let goblin = spawn_in_front(&mut s, &root, "Goblin_miner", 1.0);
    equip_staff(&mut s, &root, "staff_of_fire");
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    let buffs = player_buffs(&s);
    assert!(has_buff(&buffs, |b| matches!(b,
        BuffKind::Resistance(r) if r.against == E::FIRE && r.multiplier == 0.25)));
    assert!(s
        .avatar()
        .character
        .buffs
        .resistances()
        .iter()
        .any(|r| r.against == E::FIRE && r.multiplier == 0.25));
    assert!(s.world.npcs[&goblin]
        .character
        .buffs
        .general
        .list
        .is_empty());
    // Removing the staff lets the 1 s buff expire.
    s.world.players[0]
        .as_mut()
        .unwrap()
        .items
        .retain(|(slot, _)| *slot != 1);
    for _ in 0..70 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(player_buffs(&s).is_empty());
}

#[test]
fn all_target_aura_reaches_characters_within_its_radius() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let near = spawn_in_front(&mut s, &root, "Goblin_miner", 3.0);
    let far = spawn_in_front(&mut s, &root, "Goblin_miner", 12.0);
    equip_staff(&mut s, &root, "staff_frost");
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    let cold = |s: &PlaySession, h: u16| {
        s.world.npcs[&h]
            .character
            .buffs
            .resistances()
            .iter()
            .any(|r| r.against == E::COLD && r.multiplier == 0.0)
    };
    assert!(cold(&s, near));
    assert!(!cold(&s, far));
    assert!(has_buff(&player_buffs(&s), |b| matches!(
        b,
        BuffKind::Resistance(_)
    )));
    // Visual category Defensive: a six-decal ring around the holder and a
    // buff decal under each recipient.
    let staff = &s
        .avatar()
        .items
        .iter()
        .find(|(slot, _)| *slot == 1)
        .unwrap()
        .1;
    let ring = staff.auras[0].ring.as_ref().unwrap();
    assert_eq!(ring.decal_cell, 30);
    assert!(ring
        .ring
        .iter()
        .all(|r| r.is_some_and(|r| s.world.decals.get(r).is_some())));
    let decal = s.world.npcs[&near].character.buffs.general.list[0].decal;
    assert!(decal.is_some_and(|r| s.world.decals.get(r).is_some()));
}

#[test]
fn passives_and_modifier_buffs_reach_their_consumers() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    equip_staff(&mut s, &root, "staff_heirloom_grandfather_clock");
    frame(&mut s, PlayerInput::default());
    assert_eq!(s.body().speed_multiplier(), 2.0);
    equip_staff(&mut s, &root, "staff_cold");
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    let general = &s.avatar().character.buffs.general;
    assert_eq!(general.spell_ttl_modifier(), (2.0, 0.0));
    equip_staff(&mut s, &root, "staff_umis_trident");
    frame(&mut s, PlayerInput::default());
    assert!(s.avatar().character.wet_lightning_passive);
    equip_staff(&mut s, &root, "staff_lovestaff");
    for _ in 0..3 {
        frame(&mut s, PlayerInput::default());
    }
    assert!((s.avatar().character.buffs.general.agro_multiplier() + 0.3).abs() < 1e-6);
}

#[test]
fn staff_of_life_heals_everyone_nearby_once_a_second() {
    let Some(root) = content() else {
        return;
    };
    let mut s = session(&root);
    let goblin = spawn_in_front(&mut s, &root, "Goblin_miner", 3.0);
    s.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 100.0;
    let npc = s.world.npcs.get_mut(&goblin).unwrap();
    let goblin_max = npc.character.combat.health.max_hit_points;
    npc.character.combat.health.hit_points = 10.0;
    equip_staff(&mut s, &root, "staff_of_life");
    for _ in 0..40 {
        frame(&mut s, PlayerInput::default());
    }
    assert_eq!(s.avatar().character.combat.health.hit_points, 100.0);
    for _ in 0..40 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.avatar().character.combat.health.hit_points > 100.0);
    let healed = s.world.npcs[&goblin].character.combat.health.hit_points;
    assert!(healed > 10.0 && healed <= goblin_max, "{healed}");
}

/// Every staff special fires through input and is reconstructed (no
/// "not reconstructed" log or event), without panicking.
#[test]
fn every_staff_special_executes() {
    let Some(root) = content() else {
        return;
    };
    let dir = root.join("Data/items/Wizard");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            let stem = name.strip_suffix(".xnb")?;
            stem.starts_with("staff_").then(|| stem.to_owned())
        })
        .collect();
    names.sort();
    let mut missing = Vec::new();
    let mut s = session(&root);
    let _goblin = spawn_in_front(&mut s, &root, "Goblin_miner", 4.0);
    for name in names {
        equip_staff(&mut s, &root, &name);
        let has_special = s
            .avatar()
            .items
            .iter()
            .any(|(slot, item)| *slot == 1 && item.definition.special_ability.is_some());
        if !has_special {
            continue;
        }
        s.world.log.clear();
        let mut unsupported = Vec::new();
        for i in 0..90 {
            let events = frame(
                &mut s,
                PlayerInput {
                    cast_self_down: i < 2,
                    ..Default::default()
                },
            );
            for e in events {
                if let SessionEvent::Avatar(
                    magicka_runtime::avatar::AvatarEvent::SpellNotReconstructed(r),
                ) = e
                {
                    unsupported.push(r);
                }
            }
        }
        unsupported.extend(
            s.world
                .log
                .iter()
                .filter(|l| l.contains("not reconstructed"))
                .cloned(),
        );
        if !unsupported.is_empty() {
            missing.push(format!("{name}: {unsupported:?}"));
        }
        // Keep the avatar alive and in place for the next staff.
        let p = s.world.players[0].as_mut().unwrap();
        p.character.combat.health.hit_points = p.character.combat.health.max_hit_points;
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

fn fire_special(s: &mut PlaySession, frames: usize) -> Vec<SessionEvent> {
    let mut all = Vec::new();
    for i in 0..frames {
        all.extend(frame(
            s,
            PlayerInput {
                cast_self_down: i < 2,
                ..Default::default()
            },
        ));
    }
    all
}

fn fresh(root: &Path, staff: &str, distance: f32) -> (PlaySession, u16) {
    let mut s = session(root);
    let goblin = spawn_in_front(&mut s, root, "Goblin_miner", distance);
    equip_staff(&mut s, root, staff);
    (s, goblin)
}

#[test]
fn staff_specials_act_on_the_world() {
    let Some(root) = content() else {
        return;
    };
    // DrainLife: the goblin loses what the caster misses, returned after 1 s.
    let (mut s, goblin) = fresh(&root, "staff_vlads_gauntlet", 3.0);
    s.world.players[0]
        .as_mut()
        .unwrap()
        .character
        .combat
        .health
        .hit_points = 300.0;
    let before = s.world.npcs[&goblin].character.combat.health.hit_points;
    fire_special(&mut s, 30);
    assert!(s.world.npcs[&goblin].character.combat.health.hit_points < before);
    fire_special(&mut s, 60);
    assert!(s.avatar().character.combat.health.hit_points > 300.0);

    // CharmMetal charms the faced enemy for 15 s.
    let (mut s, goblin) = fresh(&root, "staff_musician_last_mic_stand", 4.0);
    fire_special(&mut s, 20);
    assert!(s.world.npcs[&goblin].character.is_charmed());

    // ConfuseWho charms the arc target and remembers it as a victim.
    let (mut s, goblin) = fresh(&root, "staff_who", 2.0);
    fire_special(&mut s, 20);
    assert!(s.world.npcs[&goblin].character.is_charmed());
    assert_eq!(s.world.staff_effects.len(), 1);

    // StaffShield raises a self shield.
    let (mut s, _) = fresh(&root, "staff_shield", 4.0);
    fire_special(&mut s, 20);
    assert!(s.avatar().character.combat.health.shield.active());

    // IceCacoon: Wet and Cold on the caster.
    let (mut s, _) = fresh(&root, "staff_cold", 4.0);
    fire_special(&mut s, 3);
    let statuses = &s.avatar().character.combat.statuses;
    assert!(
        statuses
            .effect(magicka_sim::status::Status::Wet)
            .is_some_and(|e| e.magnitude > 0.0)
            || statuses.has(magicka_sim::status::Status::Frozen)
    );

    // TimeWarpStaff starts the global time warp.
    let (mut s, _) = fresh(&root, "staff_timewarp", 4.0);
    fire_special(&mut s, 20);
    assert!(s.world.ability_effects.time_warp_state().is_some());

    // Grenade: a model missile that explodes after 3 s.
    let (mut s, _) = fresh(&root, "staff_patriot", 4.0);
    fire_special(&mut s, 30);
    let grenade = s
        .world
        .missiles
        .iter()
        .any(|m| m.body.item_model.as_deref() == Some("Models/Missiles/liberty_grenade"));
    assert!(grenade);
    fire_special(&mut s, 200);
    assert!(s
        .world
        .missiles
        .iter()
        .all(|m| m.body.dead || m.body.item_model.is_none()));

    // TractorPush shoves the goblin away from the caster.
    let (mut s, goblin) = fresh(&root, "staff_musician_bongo_drum", 3.0);
    let distance = |s: &PlaySession| {
        let a = s.body().position;
        let b = s.world.physics.bodies[s.world.npcs[&goblin].body].position;
        ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    let start = distance(&s);
    fire_special(&mut s, 60);
    assert!(distance(&s) > start + 1.0, "{} -> {}", start, distance(&s));

    // RandomMine casts a Shield Force spell from the staff.
    let (mut s, _) = fresh(&root, "staff_shared_randomness", 4.0);
    let events = fire_special(&mut s, 30);
    assert!(events.iter().any(|e| matches!(
        e,
        SessionEvent::Avatar(magicka_runtime::avatar::AvatarEvent::SpellCast { spell, cast_type, .. })
            if *cast_type == magicka_sim::damage::CastType::Force && spell.element.intersects(E::SHIELD)
    )));
}

#[test]
fn staff_of_the_dead_lets_dying_allies_get_back_up() {
    let Some(root) = content() else {
        return;
    };
    let (mut s, ally) = fresh(&root, "staff_of_the_dead", 2.0);
    let foe = spawn_in_front(&mut s, &root, "Goblin_miner", -2.5);
    s.world.npcs.get_mut(&ally).unwrap().faction_override =
        Some(magicka_runtime::camera::FACTION_FRIENDLY);
    for _ in 0..10 {
        frame(&mut s, PlayerInput::default());
    }
    assert!(s.world.npcs[&ally].character.undying);
    assert!(!s.world.npcs[&foe].character.undying);
    for h in [ally, foe] {
        s.world
            .npcs
            .get_mut(&h)
            .unwrap()
            .character
            .combat
            .health
            .hit_points = 0.0;
    }
    let mut died = false;
    for _ in 0..600 {
        frame(&mut s, PlayerInput::default());
        died |= s.world.npcs[&ally].character.combat.dead;
    }
    assert!(died, "the ally never finished dying");
    let a = &s.world.npcs[&ally].character;
    assert!(!a.combat.dead && a.combat.health.hit_points > 0.0);
    assert!(s
        .world
        .npcs
        .get(&foe)
        .is_none_or(|n| n.character.combat.dead));
}
