//! Every retail ability desirability expression parses with the
//! reconstructed Expression.Read (skips without content).
use magicka_content::{parse_primary, Content};
use magicka_sim::ai_expression::Expr;
use std::path::PathBuf;

fn content() -> Option<PathBuf> {
    let root = std::env::var_os("MAGICKA_CONTENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| magicka_content::retail_content_dir());
    root.is_dir().then_some(root)
}

#[test]
fn retail_ability_expressions_parse() {
    let Some(root) = content() else {
        eprintln!("retail content absent; skipping");
        return;
    };
    let (mut total, mut distinct) = (0, std::collections::BTreeSet::new());
    let mut failures = Vec::new();
    for entry in std::fs::read_dir(root.join("Data/Characters"))
        .unwrap()
        .flatten()
    {
        let path = entry.path();
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("xnb"))
        {
            continue;
        }
        let bytes = std::fs::read(&path).unwrap();
        let xnb = magicka_xnb::xnb::parse(&bytes).unwrap();
        let Some(Ok(parsed)) = parse_primary(&xnb) else {
            continue;
        };
        let Content::Character(template) = parsed.content else {
            continue;
        };
        for ability in &template.abilities {
            if let Some(text) = &ability.fuzzy_expression {
                total += 1;
                distinct.insert(text.clone());
                if let Err(e) = Expr::read(text) {
                    failures.push(format!("{}: {text:?}: {e:?}", path.display()));
                }
            }
        }
    }
    eprintln!("{total} expressions ({} distinct)", distinct.len());
    assert!(total > 500, "{total}");
    assert!(failures.is_empty(), "{failures:#?}");
}
