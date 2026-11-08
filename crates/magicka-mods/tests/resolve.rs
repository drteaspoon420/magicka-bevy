//! Plan suite `mods/resolve`: overrides, dependency order and cycles, version
//! ranges, conflicts, safe paths and deterministic hashes.
use magicka_mods::{
    kv,
    manifest::{parse_load_order, ModError},
    vfs::{LogicalPath, PathError},
    ModSet,
};
use std::path::{Path, PathBuf};

struct Tree(PathBuf);
impl Tree {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "magicka-mods-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn file(&self, path: &str, text: &str) -> &Self {
        let p = self.0.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
        self
    }
    fn manifest(&self, dir: &str, id: &str, version: &str, extra: &str) -> &Self {
        self.file(
            &format!("mods/{dir}/mod.kv"),
            &format!(
                "\"Mod\"\n{{\n  \"Id\" \"{id}\"\n  \"Version\" \"{version}\"\n  \"ApiVersion\" \"1\"\n{extra}\n}}\n"
            ),
        )
    }
    fn mods(&self) -> PathBuf {
        self.0.join("mods")
    }
    fn base(&self) -> PathBuf {
        self.0.join("base")
    }
}
impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}
fn path(p: &str) -> LogicalPath {
    LogicalPath::new(p).unwrap()
}

#[test]
fn kv_parses_the_documented_dialect_with_positions() {
    let doc = kv::parse(
        "t.kv",
        "#base \"common.kv\"\n// comment\n\"Items\"\n{\n  example:sword { \"Damage\" \"12.5\" /* c */ Tags \"a\\\"b\" }\n}\n",
    )
    .unwrap();
    assert_eq!(doc.bases[0].0, "common.kv");
    let items = &doc.nodes[0];
    let sword = items.get("example:sword").unwrap();
    assert_eq!(sword.span.line, 5);
    assert_eq!(sword.get_str("damage"), Some("12.5"));
    assert_eq!(sword.get_str("Tags"), Some("a\"b"));
    // Canonical writing round-trips.
    let again = kv::parse("t.kv", &kv::write(&doc.nodes)).unwrap();
    assert_eq!(kv::write(&again.nodes), kv::write(&doc.nodes));
    for (bad, why) in [
        ("\"a\" {", "not closed"),
        ("}", "unmatched"),
        ("\"a\"", "no value"),
        ("\"a\" \"b", "unterminated"),
        ("\"a\" \"b\" [$WIN32]", "platform"),
        ("#include \"x\"", "unsupported directive"),
        ("\"a\" { #base \"x\" }", "top level"),
    ] {
        let e = kv::parse("bad.kv", bad).unwrap_err();
        assert!(e.to_string().contains(why), "{bad}: {e}");
        assert!(e.to_string().starts_with("bad.kv:1:"));
    }
}

#[test]
fn logical_paths_normalize_and_reject_escapes() {
    assert_eq!(
        path("Scripts\\NPC//Items.KV").as_str(),
        "scripts/npc/items.kv"
    );
    assert_eq!(path("./a/b").as_str(), "a/b");
    assert!(matches!(
        LogicalPath::new("/etc/passwd"),
        Err(PathError::Absolute(_))
    ));
    assert!(matches!(
        LogicalPath::new("C:/x"),
        Err(PathError::Absolute(_))
    ));
    assert!(matches!(
        LogicalPath::new("a/../../b"),
        Err(PathError::Escape(_))
    ));
    assert!(LogicalPath::new("a/b:stream").is_err());
    assert!(LogicalPath::new("").is_err());
    let base = path("scripts/vscripts/magicks/storm.lua");
    assert_eq!(
        base.sibling("../util/math.lua").unwrap().as_str(),
        "scripts/vscripts/util/math.lua"
    );
    assert!(base.sibling("../../../../x").is_err());
}

#[test]
fn later_mods_win_and_hashes_are_deterministic() {
    let t = Tree::new("override");
    t.file("base/scripts/npc/items.kv", "\"Items\" {}")
        .file("base/models/wand.txt", "base")
        .manifest("a", "example.a", "1.0.0", "")
        .file("mods/a/scripts/npc/items.kv", "\"Items\" { a {} }")
        .file("mods/a/Models/Wand.txt", "a")
        .manifest(
            "b",
            "example.b",
            "2.1.0",
            "\"Dependencies\" { \"example.a\" \">=1.0\" }",
        )
        .file("mods/b/scripts/npc/items.kv", "\"Items\" { b {} }");
    let load =
        |order: &[&str]| ModSet::load(Some(&t.base()), &t.mods(), Some(&ids(order))).unwrap();
    let set = load(&["example.a", "example.b"]);
    let r = set.vfs.resolve(&path("scripts/npc/items.kv")).unwrap();
    assert_eq!(r.layer, "example.b");
    assert_eq!(set.vfs.overlaps(&path("scripts/npc/items.kv")).len(), 3);
    // Case-insensitive override of a presentation file.
    assert_eq!(
        set.vfs.resolve(&path("models/wand.txt")).unwrap().layer,
        "example.a"
    );
    let explained = set.explain("SCRIPTS/npc/items.kv").unwrap();
    assert!(
        explained.contains("winner: example.b (authoritative)"),
        "{explained}"
    );
    // Identical ordered mods → identical hashes; the user order cannot
    // move a dependent before its dependency.
    let h1 = set.content_manifest().unwrap();
    let h2 = load(&["example.b", "example.a"])
        .content_manifest()
        .unwrap();
    assert_eq!(h1, h2);
    assert_eq!(h1.mods, ["example.a@1.0.0", "example.b@2.1.0"]);
    assert!(h1.rules_hash.is_some());
    assert!(h1.presentation.contains_key("models/wand.txt"));
    // Authoritative content changes the rules hash; unmodded has none.
    t.file("mods/b/scripts/npc/items.kv", "\"Items\" { b { x 1 } }");
    assert_ne!(
        load(&["example.a", "example.b"])
            .content_manifest()
            .unwrap()
            .rules_hash,
        h1.rules_hash
    );
    assert_eq!(
        ModSet::base_only(Some(&t.base()))
            .unwrap()
            .content_manifest()
            .unwrap()
            .rules_hash,
        None
    );
}

#[test]
fn dependency_errors_fail_before_play() {
    let t = Tree::new("deps");
    t.manifest("a", "x.a", "1.0.0", "\"Dependencies\" { \"x.b\" \">=2\" }")
        .manifest("b", "x.b", "1.5.0", "\"Dependencies\" { \"x.c\" \"*\" }")
        .manifest("c", "x.c", "1.0.0", "\"Dependencies\" { \"x.b\" \"*\" }")
        .manifest("d", "x.d", "1.0.0", "\"Conflicts\" { \"x.c\" \"<2\" }")
        .manifest(
            "e",
            "x.e",
            "1.0.0",
            "\"Dependencies\" { \"base.game\" \">=1.0.0\" }",
        );
    let err = |list: &[&str]| ModSet::load(None, &t.mods(), Some(&ids(list))).unwrap_err();
    assert!(matches!(err(&["x.a"]), ModError::Missing { .. }));
    assert!(matches!(
        err(&["x.a", "x.b", "x.c"]),
        ModError::Version { .. }
    ));
    match err(&["x.b", "x.c"]) {
        ModError::Cycle(c) => assert!(c.len() == 3 && c[0] == c[2], "{c:?}"),
        e => panic!("{e}"),
    }
    assert!(matches!(
        err(&["x.c", "x.d", "x.b"]),
        ModError::Conflict(..)
    ));
    assert!(matches!(err(&["nope"]), ModError::UnknownMod(_)));
    assert!(ModSet::load(None, &t.mods(), Some(&ids(&["x.e"]))).is_ok());
}

#[test]
fn manifests_load_order_and_unsafe_layers_are_validated() {
    assert_eq!(
        parse_load_order(
            "# order\nenabled = [\n  \"example.my_magicks\", # first\n  \"boosted\"\n]\n"
        )
        .unwrap(),
        ids(&["example.my_magicks", "boosted"])
    );
    assert!(parse_load_order("enabled = [\"a\", \"a\"]").is_err());
    assert!(parse_load_order("enabled = [a]").is_err());
    assert!(parse_load_order("other = 1").is_err());
    let t = Tree::new("unsafe");
    t.manifest("v", "x.visual", "1.0.0", "\"ClientOnly\" \"1\"")
        .file("mods/v/scripts/npc/items.kv", "\"Items\" {}");
    assert!(ModSet::load(None, &t.mods(), Some(&ids(&["x.visual"]))).is_err());
    let t = Tree::new("case");
    t.manifest("c", "x.case", "1.0.0", "")
        .file("mods/c/Models/A.txt", "1")
        .file("mods/c/models/a.txt", "2");
    let e = ModSet::load(None, &t.mods(), Some(&ids(&["x.case"]))).unwrap_err();
    assert!(e.to_string().contains("collide"), "{e}");
    #[cfg(unix)]
    {
        let t = Tree::new("link");
        t.manifest("l", "x.link", "1.0.0", "");
        std::os::unix::fs::symlink(std::env::temp_dir(), t.mods().join("l/escape")).unwrap();
        let e = ModSet::load(None, &t.mods(), Some(&ids(&["x.link"]))).unwrap_err();
        assert!(e.to_string().contains("outside"), "{e}");
    }
    let t = Tree::new("manifest");
    t.manifest("bad", "Bad Id", "1.0.0", "");
    assert!(matches!(
        magicka_mods::manifest::discover(&t.mods()),
        Err(ModError::Manifest(..))
    ));
    let t = Tree::new("api");
    t.file(
        "mods/api/mod.kv",
        "\"Mod\" { \"Id\" \"x.api\" \"Version\" \"1.0.0\" \"ApiVersion\" \"99\" }",
    );
    let e = magicka_mods::manifest::discover(&t.mods()).unwrap_err();
    assert!(e.to_string().contains("ApiVersion 99"), "{e}");
    let _ = Path::new("");
}

/// `load_order.toml` is per-user and untracked: the first load writes the
/// default (nothing enabled) and later loads read the user's edits.
#[test]
fn a_missing_load_order_is_created_with_nothing_enabled() {
    let t = Tree::new("first-run");
    t.manifest("a", "a", "1.0.0", "");
    let order = t.mods().join("load_order.toml");
    assert!(!order.exists());
    let set = ModSet::load(None, &t.mods(), None).unwrap();
    assert!(set.mods.is_empty(), "nothing enabled on first run");
    assert_eq!(
        std::fs::read_to_string(&order).unwrap(),
        magicka_mods::DEFAULT_LOAD_ORDER
    );
    assert!(parse_load_order(magicka_mods::DEFAULT_LOAD_ORDER)
        .unwrap()
        .is_empty());
    std::fs::write(&order, "enabled = [\"a\"]\n").unwrap();
    let set = ModSet::load(None, &t.mods(), None).unwrap();
    let enabled: Vec<_> = set.mods.iter().map(|m| m.id.clone()).collect();
    assert_eq!(enabled, ids(&["a"]));
}
