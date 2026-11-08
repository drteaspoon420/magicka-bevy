use magicka_runtime::{
    animation_action::AnimationAction,
    level_variables::LevelVariables,
    scene_loading::custom_hash,
    scene_script::{ConditionDefinition, SceneScriptLayout},
    script_runtime::*,
    world_condition::*,
};
use magicka_sim::{
    damage_delivery::NetworkRole,
    spells::{Elements, Spell},
};

fn read(tag: &str, attrs: &[(&str, &str)]) -> WorldCondition {
    WorldCondition::read(tag, attrs.iter().copied()).unwrap()
}

#[derive(Default)]
struct Facts {
    health: Option<CharacterHealthFacts>,
    spell: Option<(i32, Spell)>,
    players: Vec<ConditionPlayer>,
    boss: Option<BossFacts>,
    died: bool,
    achievements: Vec<String>,
    looped: Option<bool>,
}
impl WorldConditionFacts for Facts {
    fn character_health(&self, id: i32) -> Option<CharacterHealthFacts> {
        (id == custom_hash("hero")).then_some(self.health?)
    }
    fn current_spell(&self, id: i32) -> Option<(i32, Spell)> {
        (id == custom_hash("hero")).then_some(self.spell?)
    }
    fn sender_unique_id(&self, sender: u16) -> Option<i32> {
        (sender == 3).then(|| custom_hash("friend"))
    }
    fn players(&self) -> Vec<ConditionPlayer> {
        self.players.clone()
    }
    fn boss_fight(&self) -> Option<BossFacts> {
        self.boss
    }
    fn died_in_level(&self) -> bool {
        self.died
    }
    fn has_achievement(&self, name: Option<&str>) -> bool {
        name.is_some_and(|n| self.achievements.iter().any(|a| a == n))
    }
    fn has_dlc(&self, dlc: i32) -> bool {
        dlc == 0
    }
    fn campaign_looped(&self) -> Option<bool> {
        self.looped
    }
}
fn player(playing: bool, template: &str, statuses: i32, queue: Vec<u32>) -> ConditionPlayer {
    ConditionPlayer {
        playing,
        avatar: Some(ConditionAvatar {
            template_id: custom_hash(template),
            statuses,
            queue,
        }),
    }
}

#[test]
fn character_and_interactor_conditions() {
    let mut f = Facts {
        health: Some(CharacterHealthFacts {
            dead: false,
            hit_points: 50.0,
            max_hit_points: 200.0,
        }),
        ..Default::default()
    };
    let met = |c: &WorldCondition, f: &Facts, s| c.is_met(f, s).unwrap();
    assert!(met(
        &read("characterHealth", &[("id", "Hero"), ("health", "0.25")]),
        &f,
        None
    ));
    assert!(met(
        &read(
            "characterhealth",
            &[("id", "hero"), ("health", "0.5"), ("compareMethod", "less")]
        ),
        &f,
        None
    ));
    assert!(met(
        &read(
            "characterhealth",
            &[
                ("id", "hero"),
                ("health", "0.1"),
                ("compareMethod", "greater")
            ]
        ),
        &f,
        None
    ));
    // Dead or missing characters count as zero health.
    f.health = Some(CharacterHealthFacts {
        dead: true,
        hit_points: 50.0,
        max_hit_points: 200.0,
    });
    assert!(met(
        &read("characterhealth", &[("id", "hero"), ("health", "0")]),
        &f,
        None
    ));
    assert!(met(
        &read("characterhealth", &[("id", "nobody"), ("health", "0")]),
        &f,
        None
    ));
    assert!(WorldCondition::read("characterhealth", [("health", "2")]).is_err());

    let interactor = read("interactorIs", &[("id", "Friend")]);
    assert!(met(&interactor, &f, Some(3)));
    assert!(!met(&interactor, &f, Some(4)));
    assert!(!met(&interactor, &f, None));

    f.spell = Some((
        3,
        Spell {
            element: Elements::LIFE,
            mag: {
                let mut m = [0.0; 11];
                m[6] = 1.0;
                m
            },
            ..Default::default()
        },
    ));
    assert!(met(
        &read(
            "characterCasting",
            &[("id", "hero"), ("castType", "self"), ("element", "life")]
        ),
        &f,
        None
    ));
    assert!(!met(
        &read(
            "charactercasting",
            &[("id", "hero"), ("castType", "area"), ("element", "life")]
        ),
        &f,
        None
    ));
    // Composite masks never match a single indexed magnitude.
    assert!(!met(
        &read(
            "charactercasting",
            &[
                ("id", "hero"),
                ("castType", "self"),
                ("element", "life,fire")
            ]
        ),
        &f,
        None
    ));
    // Unset ID defaults to -1.
    assert!(matches!(
        read("charactercasting", &[]).kind,
        WorldConditionKind::CharacterCasting { id: -1, .. }
    ));
}

#[test]
fn player_boss_and_global_conditions() {
    let mut f = Facts {
        players: vec![
            player(true, "wizard_red", 0, vec![0x40]),
            player(false, "wizard_blue", 2, vec![]),
            ConditionPlayer {
                playing: true,
                avatar: None,
            },
            player(true, "wizard_blue", 0, vec![8]),
        ],
        achievements: vec!["mastery".into()],
        ..Default::default()
    };
    let met = |c: &WorldCondition, f: &Facts| c.is_met(f, None);
    assert_eq!(
        met(&read("robeInUse", &[("name", "Wizard_Blue")]), &f),
        Ok(true)
    );
    assert_eq!(
        met(
            &read("robeinuse", &[("name", "wizard_blue"), ("player", "2")]),
            &f
        ),
        Ok(false)
    );
    assert_eq!(
        met(
            &read("robeinuse", &[("name", "wizard_blue"), ("player", "4")]),
            &f
        ),
        Ok(true)
    );
    assert_eq!(
        met(&read("robeinuse", &[("name", ""), ("invert", "true")]), &f),
        Ok(true)
    );
    assert_eq!(
        met(&read("robeinuse", &[("name", "x"), ("player", "5")]), &f),
        Err(WorldConditionError::PlayerIndex(5))
    );
    // The status filter is ignored: any status on a playing avatar matches.
    assert_eq!(
        met(
            &read("playersHaveStatusEffect", &[("statusEffect", "wet")]),
            &f
        ),
        Ok(false)
    );
    f.players[3].avatar.as_mut().unwrap().statuses = 1;
    assert_eq!(
        met(
            &read("playershavestatuseffect", &[("statusEffect", "wet")]),
            &f
        ),
        Ok(true)
    );
    assert_eq!(
        met(&read("playerHasElement", &[("element", "fire")]), &f),
        Ok(true)
    );
    assert_eq!(
        met(&read("playerhaselement", &[("element", "cold")]), &f),
        Ok(false)
    );

    // No boss fight: false even when inverted.
    assert_eq!(met(&read("bossDead", &[("invert", "true")]), &f), Ok(false));
    assert_eq!(
        met(
            &read("bossHealth", &[("value", "0.5"), ("invert", "true")]),
            &f
        ),
        Ok(false)
    );
    f.boss = Some(BossFacts {
        normalized_health: 0.5,
        dead: false,
    });
    assert_eq!(met(&read("bossdead", &[("invert", "true")]), &f), Ok(true));
    assert_eq!(
        met(&read("bosshealth", &[("value", "0.5000005")]), &f),
        Ok(true)
    );
    assert_eq!(
        met(
            &read(
                "bosshealth",
                &[("value", "0.4"), ("compareMethod", "greater")]
            ),
            &f
        ),
        Ok(true)
    );
    assert_eq!(
        met(&read("bosshealth", &[("compareMethod", "7")]), &f),
        Err(WorldConditionError::InvalidCompare(7))
    );

    assert_eq!(met(&read("anyPlayerHasDied", &[]), &f), Ok(false));
    f.died = true;
    assert_eq!(
        met(&read("anyplayerhasdied", &[("invert", "true")]), &f),
        Ok(false)
    );
    assert_eq!(
        met(&read("hasAchievement", &[("name", "Mastery")]), &f),
        Ok(true)
    );
    assert_eq!(met(&read("hasDLC", &[("type", "WizardHat")]), &f), Ok(true));
    assert_eq!(met(&read("hasdlc", &[("type", "vietnam")]), &f), Ok(false));
}

struct World {
    facts: Option<Facts>,
}
impl ScriptWorld for World {
    type Error = &'static str;
    fn facts(&self) -> ScriptWorldFacts {
        ScriptWorldFacts::default()
    }
    fn condition_facts(&self) -> Option<&dyn WorldConditionFacts> {
        self.facts.as_ref().map(|f| f as _)
    }
    fn choose_group(&mut self, _: usize) -> Result<usize, Self::Error> {
        Ok(0)
    }
    fn notify_trigger(
        &mut self,
        _: &mut ScriptRuntime,
        _: i32,
        _: usize,
        _: Option<u16>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
    fn animation(&mut self, _: &mut ScriptRuntime, _: &AnimationAction) -> Result<(), Self::Error> {
        Ok(())
    }
    fn action_executed(&mut self, _: &mut ScriptRuntime, _: usize) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[test]
fn runtime_evaluates_world_conditions_with_sender() {
    let layout = SceneScriptLayout::read(
        br#"<Scene><trigger id="use" autorun="false" repeat="true"><if><interactorIs id="friend"/></if>
        <then><counter name="c" value="1"/></then></trigger></Scene>"#,
        false,
    )
    .unwrap();
    assert!(matches!(
        layout.conditions[0],
        ConditionDefinition::World(_)
    ));
    let mut rt = ScriptRuntime::new(layout, LevelVariables::default());
    let mut world = World { facts: None };
    assert_eq!(
        rt.execute_trigger(
            custom_hash("use"),
            Some(3),
            false,
            NetworkRole::Server,
            &mut world
        ),
        Err(ScriptRuntimeError::WorldCondition(
            0,
            WorldConditionError::MissingWorldFacts
        ))
    );
    world.facts = Some(Facts::default());
    assert_eq!(
        rt.execute_trigger(
            custom_hash("use"),
            Some(4),
            false,
            NetworkRole::Server,
            &mut world
        ),
        Ok(false)
    );
    assert_eq!(
        rt.execute_trigger(
            custom_hash("use"),
            Some(3),
            false,
            NetworkRole::Server,
            &mut world
        ),
        Ok(true)
    );
}

#[test]
fn campaign_looped_reads_the_save_flag_and_throws_outside_campaign() {
    let c = WorldCondition::read("IsCampaignLooped", [("invert", "true")]).unwrap();
    let mut f = Facts::default();
    assert_eq!(c.is_met(&f, None), Err(WorldConditionError::NotCampaign));
    f.looped = Some(false);
    assert_eq!(c.is_met(&f, None), Ok(true));
    f.looped = Some(true);
    assert_eq!(c.is_met(&f, None), Ok(false));
}
