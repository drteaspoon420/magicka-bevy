use magicka_runtime::npc_spellcasting::{AbilityContext, NpcSpellcasting, SpellAbility};
use magicka_sim::{
    damage::CastType,
    spells::{Elements as E, Spell},
};

fn ability(cast_type: CastType) -> SpellAbility {
    SpellAbility {
        min_range: 0.,
        max_range: 10.,
        arc: 1.,
        chant_speed: 0.25,
        power: 2.,
        cast_type,
        elements: vec![E::FIRE, E::FIRE],
        animations: vec![200, 201],
    }
}

#[test]
fn force_chants_at_strict_cooldown_and_combines_at_attack_start() {
    let ability = ability(CastType::Force);
    let mut state = NpcSpellcasting::default();
    let mut events = Vec::new();
    let context = AbilityContext {
        distance_squared: 156.25,
        ..Default::default()
    };
    state.tick(1.);
    assert!(state
        .update_ability(&ability, context, &mut events)
        .is_empty());
    state.tick(0.01);
    assert_eq!(
        state.update_ability(&ability, context, &mut events),
        [(E::FIRE, true)]
    );
    assert_eq!(
        state
            .execute(&ability, context, |_| panic!("not ready"))
            .unwrap(),
        None
    );
    state.tick(0.26);
    assert_eq!(
        state.update_ability(&ability, context, &mut events),
        [(E::FIRE, true)]
    );
    assert_eq!(state.execute(&ability, context, |_| 1).unwrap(), Some(201));
    assert!(state.queue.is_empty());
    assert_eq!(state.power, 2.);
    assert_eq!(state.prepared, Spell::default_for_list(&[E::FIRE, E::FIRE]));
    assert!(state.animation_cast(false, false).is_none());
    assert!(state.animation_cast(true, true).is_none());
    let cast = state.animation_cast(true, false).unwrap();
    assert_eq!(cast.spell, Spell::default_for_list(&[E::FIRE, E::FIRE]));
    assert_eq!(cast.cast_type, CastType::Force);
    assert_eq!(state.prepared, Spell::default());
}

#[test]
fn weapon_chants_every_authored_element_and_combines_at_animation_action() {
    let ability = ability(CastType::Weapon);
    let mut state = NpcSpellcasting::default();
    state.tick(2.);
    let context = AbilityContext::default();
    let conjures = state.update_ability(&ability, context, &mut Vec::new());
    assert_eq!(conjures, [(E::FIRE, true), (E::FIRE, true)]);
    assert_eq!(state.execute(&ability, context, |_| 0).unwrap(), Some(200));
    assert_eq!(state.queue.len(), 2);
    assert_eq!(state.prepared, Spell::default());
    let cast = state.animation_cast(true, false).unwrap();
    assert_eq!(cast.spell, Spell::default_for_list(&[E::FIRE, E::FIRE]));
    assert!(state.queue.is_empty());
}

#[test]
fn busy_pointer_attacking_range_and_nan_each_block_chanting() {
    let ability = ability(CastType::Force);
    for context in [
        AbilityContext {
            busy_is_next: true,
            ..Default::default()
        },
        AbilityContext {
            current_spell: true,
            ..Default::default()
        },
        AbilityContext {
            attacking: true,
            ..Default::default()
        },
        AbilityContext {
            distance_squared: 156.26,
            ..Default::default()
        },
        AbilityContext {
            distance_squared: f32::NAN,
            ..Default::default()
        },
    ] {
        let mut state = NpcSpellcasting::default();
        state.tick(2.);
        assert!(state
            .update_ability(&ability, context, &mut Vec::new())
            .is_empty());
        assert_eq!(state.chant_cooldown, -1.);
        assert!(state.queue.is_empty());
    }
}

#[test]
fn queue_cancellation_retries_and_selection_requires_compatible_prefix() {
    let mut ability = ability(CastType::Force);
    ability.elements = vec![E::FIRE, E::COLD];
    let mut state = NpcSpellcasting::default();
    assert!(state.allows(None));
    state.tick(2.);
    state.update_ability(&ability, AbilityContext::default(), &mut Vec::new());
    assert!(!state.allows(None));
    assert!(state.allows(Some(&ability)));
    let incompatible = SpellAbility {
        elements: vec![E::ARCANE],
        ..ability.clone()
    };
    assert!(!state.allows(Some(&incompatible)));
    state.tick(0.26);
    assert_eq!(
        state.update_ability(&ability, AbilityContext::default(), &mut Vec::new()),
        [(E::COLD, false)]
    );
    assert!(
        state.queue.is_empty(),
        "opposing elements cancel instead of forcing queue completion"
    );
    assert!(state
        .execute(&ability, AbilityContext::default(), |_| 0)
        .unwrap()
        .is_none());
}

#[test]
fn same_cast_type_preserves_the_native_setter_queue_behavior() {
    let mut state = NpcSpellcasting::default();
    state.queue.push(Spell::default_for(E::LIFE));
    state.set_cast_type(CastType::SelfCast);
    assert!(state.queue.is_empty());
    state.queue.push(Spell::default_for(E::ARCANE));
    state.set_cast_type(CastType::SelfCast);
    assert_eq!(state.queue.len(), 1);
    assert_eq!(state.prepared.element, E::LIFE);
    state.set_cast_type(CastType::Blast);
    assert_eq!(state.queue.len(), 1);
    state.set_cast_type(CastType::Force);
    assert!(state.queue.is_empty());
    assert_eq!(state.prepared.element, E::ARCANE);
}

#[test]
fn npc_magicks_recognize_the_recipe_without_player_unlocks() {
    let mut state = NpcSpellcasting::default();
    for element in [E::WATER, E::STEAM] {
        magicka_sim::spells::try_add_to_queue(
            &mut state.queue,
            5,
            Spell::default_for(element),
            &mut Vec::new(),
        );
    }
    state.set_cast_type(CastType::Magick);
    assert_eq!(state.prepared.element, E::ALL);
    assert_eq!(state.prepared.magick, magicka_sim::spells::MagickType::Rain);
    let invocation = state.animation_cast(true, false).unwrap();
    assert_eq!(
        invocation.spell.magick,
        magicka_sim::spells::MagickType::Rain
    );
    assert_eq!(state.cast_type, CastType::Blast);
    assert_eq!(state.prepared, Spell::default());
    assert!(state.queue.is_empty());
}

#[test]
fn caster_direction_preserves_self_facing_and_the_tiny_delta_branch() {
    let ability = ability(CastType::SelfCast);
    assert_eq!(
        ability.desired_direction([0.; 3], [0.; 3], [1., 0., 0.], true),
        [1., 0., 0.]
    );
    assert_eq!(
        ability.desired_direction([0.; 3], [0., 3., 4.], [1., 0., 0.], false),
        [0., 0.6, 0.8]
    );
    assert_eq!(
        ability.desired_direction([0.; 3], [1e-7, 0., 0.], [1., 0., 0.], false),
        [1e-7, 0., 1.]
    );
}

#[test]
fn retail_casting_abilities_decode_without_invented_recipes_or_animations() {
    use magicka_content::{parse_primary, Content};
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../../target/Magicka/Content")
        });
    if !root.is_dir() {
        return;
    }
    let mut count = 0;
    let mut families = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(root.join("Data/Characters")).unwrap() {
        let path = entry.unwrap().path();
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("xnb"))
        {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let parsed = parse_primary(&magicka_xnb::xnb::parse(&bytes).unwrap())
            .unwrap()
            .unwrap();
        let Content::Character(template) = parsed.content else {
            continue;
        };
        for ability in &template.abilities {
            if let Some(decoded) = SpellAbility::from_content(ability)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
            {
                count += 1;
                *families
                    .entry(format!("{:?}", decoded.cast_type))
                    .or_insert(0) += 1;
                assert!(!decoded.animations.is_empty(), "{}", path.display());
            }
        }
    }
    eprintln!("{count} retail casting abilities: {families:?}");
    assert!(count > 0);
}
