//! Read-only retail validation; skips if user-owned data is absent.
use magicka_content::{parse_primary, Content};
use magicka_runtime::{CharacterDefinition, ItemDefinition};
use magicka_sim::{
    damage::{calculate_damage, CastType},
    spells::{Elements as E, Spell},
};
use std::path::{Path, PathBuf};
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
#[test]
fn retail_character_and_item_tables_feed_simulation() {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../target/Magicka/Content")
        });
    if !root.is_dir() {
        eprintln!("SKIP: retail content absent");
        return;
    }
    let mut paths = vec![];
    walk(&root.join("Data"), &mut paths);
    paths.sort();
    let (mut characters, mut items) = (0, 0);
    for path in paths {
        let bytes = std::fs::read(&path).unwrap();
        let xnb = magicka_xnb::xnb::parse(&bytes).unwrap();
        let parsed = parse_primary(&xnb)
            .unwrap_or_else(|| panic!("{}: null primary", path.display()))
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        match parsed.content {
            Content::Character(template) => {
                characters += 1;
                for players in [1, 4] {
                    let def = CharacterDefinition::from_content(&template, players)
                        .unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
                    let state = def.combat_state();
                    assert_eq!(state.health.hit_points, template.max_hitpoints);
                    assert_eq!(state.health.max_hit_points, template.max_hitpoints);
                    assert_eq!(
                        state.motion.knockdown_tolerance,
                        template.knockdown_tolerance
                    );
                    assert_eq!(
                        def.can_have_status,
                        !template.id.to_ascii_lowercase().contains("elemental")
                    );
                    assert!(def.health_bars >= 1);
                    assert!(def.length >= 0.01);
                    let spell = Spell::default_for(E::FIRE);
                    let mut damage =
                        calculate_damage(&spell, spell.spell_type(), CastType::Force).unwrap()[1];
                    damage
                        .apply_resistances(Some(&def.resistances), None, &[])
                        .unwrap();
                    assert!(
                        damage.amount.is_finite() && damage.magnitude.is_finite(),
                        "{}",
                        path.display()
                    );
                }
            }
            Content::Item(item) => {
                items += 1;
                let def = ItemDefinition::from_content(&item).unwrap();
                assert_eq!(def.block_value, item.block_value);
            }
            _ => {}
        }
    }
    eprintln!("runtime adapters validated: {characters} character templates (1 and 4 players), {items} items");
    assert!(
        characters > 0 && items > 0,
        "retail data directory had no relevant templates"
    );
}
