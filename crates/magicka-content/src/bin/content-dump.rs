//! Dump or survey Magicka content files.
//!
//!     content-dump <file.xnb>            pretty Debug dump of one object
//!     content-dump --survey <dir>        variant coverage histogram
//!     content-dump --shared <file.xnb>   dump the object and its shared resources
//!
//! Output is derived from proprietary data; keep it out of version control.

use magicka_content::character::{ActionKind, CharacterTemplate};
use magicka_content::common::{Aura, AuraStorage, Buff, BuffStorage, ConditionCollection, Event};
use magicka_content::item::Item;
use magicka_content::{parse_primary, Content};
use magicka_xnb::xnb;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .collect();
    v.sort();
    for p in v {
        if p.is_dir() {
            walk(&p, out);
        } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xnb")) {
            out.push(p);
        }
    }
}

/// Variant name of an enum value from its Debug form (`Foo { .. }` / `Foo(..)` / `Foo`).
fn variant<T: std::fmt::Debug>(v: &T) -> String {
    let s = format!("{v:?}");
    s.split([' ', '(', '{']).next().unwrap_or("").to_string()
}

#[derive(Default)]
struct Hist(BTreeMap<String, usize>);

impl Hist {
    fn add(&mut self, k: impl Into<String>) {
        *self.0.entry(k.into()).or_default() += 1;
    }
}

fn conditions(h: &mut Hist, c: &ConditionCollection) {
    for ec in &c.0 {
        for e in &ec.events {
            h.add(format!("Event::{}", variant(e)));
            if let Event::Light(_) = e {}
        }
    }
}

fn buff(h: &mut Hist, b: &BuffStorage) {
    h.add(format!("Buff::{}", variant(&b.kind)));
    if let Buff::Unknown(t) = b.kind {
        h.add(format!("Buff::Unknown({t})"));
    }
}

fn aura(h: &mut Hist, a: &AuraStorage) {
    h.add(format!("Aura::{}", variant(&a.kind)));
    if let Aura::Buff(b) = &a.kind {
        buff(h, b);
    }
}

fn character(h: &mut Hist, c: &CharacterTemplate) {
    h.add("CharacterTemplate");
    if c.attached_sound_count > 4 {
        h.add("quirk: >4 attached sounds");
    }
    for set in &c.animation_sets {
        for clip in set {
            for a in &clip.actions {
                h.add(format!("Action::{}", variant(&a.kind)));
                if let ActionKind::SpecialAbility {
                    ability: Some(_), ..
                } = &a.kind
                {
                    h.add("Action::SpecialAbility(inline ability)");
                }
            }
        }
    }
    for a in &c.abilities {
        h.add(format!("Ability::{}", variant(&a.kind)));
        if a.fuzzy_expression.is_some() {
            h.add("Ability(fuzzy expression)");
        }
    }
    conditions(h, &c.event_conditions);
    c.buffs.iter().for_each(|b| buff(h, b));
    c.auras.iter().for_each(|a| aura(h, a));
}

fn item(h: &mut Hist, i: &Item) {
    h.add("Item");
    if i.special_ability.is_some() {
        h.add("Item(special ability)");
    }
    if i.light.is_some() {
        h.add("Item(light)");
    }
    for c in [&i.melee_conditions, &i.gun_conditions, &i.ranged_conditions] {
        conditions(h, c);
    }
    i.auras.iter().for_each(|a| aura(h, a));
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() == 2 && args[0] == "--survey" {
        let mut files = Vec::new();
        walk(Path::new(&args[1]), &mut files);
        let mut h = Hist::default();
        let mut failed = 0;
        for f in &files {
            let Ok(x) = xnb::parse(&std::fs::read(f).unwrap()) else {
                continue;
            };
            match parse_primary(&x) {
                Some(Ok(p)) if p.trailing != 0 => {
                    failed += 1;
                    eprintln!("{}: {} trailing bytes", f.display(), p.trailing);
                }
                Some(Ok(p)) => match &p.content {
                    Content::Character(c) => character(&mut h, c),
                    Content::Item(i) => item(&mut h, i),
                    Content::PhysicsEntity(pe) => {
                        h.add("PhysicsEntityTemplate");
                        if pe.animated.is_some() {
                            h.add("PhysicsEntityTemplate(animated)");
                        }
                        if pe.collision_mesh.is_some() {
                            h.add("PhysicsEntityTemplate(collision mesh)");
                        }
                        if pe.model.as_ref().is_some_and(|m| !m.meshes.is_empty()) {
                            h.add("PhysicsEntityTemplate(model with meshes)");
                        }
                        conditions(&mut h, &pe.conditions);
                    }
                    Content::Xna(o) => {
                        let kind = match o.as_ref() {
                            magicka_content::xna::XnaObject::Asset(a) => {
                                format!("Asset::{}", variant(a.as_ref()))
                            }
                            other => format!("Xna::{}", variant(other)),
                        };
                        h.add(kind);
                    }
                    Content::Level(l) => {
                        h.add("LevelModel");
                        let roots = l.model.as_ref().map_or(0, |m| m.roots.len());
                        h.0.entry("LevelModel: bitree roots (sum)".into())
                            .and_modify(|v| *v += roots)
                            .or_insert(roots);
                        for p in &l.animated_parts {
                            h.add("LevelModel: animated part (top level)");
                            if p.nav_mesh.is_some() {
                                h.add("LevelModel: animated part with nav mesh");
                            }
                        }
                        for q in &l.liquids {
                            h.add(format!("LevelModel: liquid {}", variant(&q.material)));
                        }
                        if !l.force_fields.is_empty() {
                            h.add("LevelModel: has force fields");
                        }
                        for t in &l.lights {
                            h.add(format!("LevelModel: light type {}", t.light_type));
                        }
                        if l.camera_mesh.is_some() {
                            h.add("LevelModel: camera mesh");
                        }
                    }
                },
                Some(Err(e)) => {
                    failed += 1;
                    eprintln!("{}: {e}", f.display());
                }
                None => {}
            }
        }
        for (k, n) in &h.0 {
            println!("{n:6}  {k}");
        }
        return if failed == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    let with_shared = args.len() == 2 && args[0] == "--shared";
    let args = if with_shared { &args[1..] } else { &args[..] };
    if args.len() == 1 {
        let x = match xnb::parse(&std::fs::read(&args[0]).expect("read")) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        };
        match parse_primary(&x) {
            Some(Ok(p)) => {
                println!("{:#?}", p.content);
                if with_shared {
                    for (i, shared) in p.shared.iter().enumerate() {
                        println!("shared[{}] = {shared:#?}", i + 1);
                    }
                }
                eprintln!(
                    "shared resources: {}, trailing bytes: {}",
                    p.shared.len(),
                    p.trailing
                );
                ExitCode::SUCCESS
            }
            Some(Err(e)) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
            None => {
                eprintln!("unsupported reader: {:?}", x.primary_reader());
                ExitCode::FAILURE
            }
        }
    } else {
        eprintln!("usage: content-dump <file.xnb> | --survey <dir>");
        ExitCode::from(2)
    }
}
