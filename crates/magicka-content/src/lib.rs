//! Parsers for Magicka's own XNB content types, layered on `magicka-xnb`.
//!
//! Specs and evidence: `output/research/content-formats.md`.

pub mod ani;
pub mod assets;
pub mod character;
pub mod character_scene;
pub mod common;
pub mod export_manifest;
pub mod gltf;
pub mod item;
pub mod level;
pub mod mesh;
pub mod physics;
pub mod scene;
pub mod skinned;
pub mod texture;
pub mod xact;
pub mod xna;

pub use common::{ContentError, Reader, Result};
pub use magicka_xnb::retail_content_dir;

use magicka_xnb::xnb::Xnb;

/// Primary-object kinds this crate understands.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    Character(Box<character::CharacterTemplate>),
    Item(Box<item::Item>),
    PhysicsEntity(Box<physics::PhysicsEntityTemplate>),
    Level(Box<level::LevelModel>),
    /// Any standard XNA / XNAnimation / PolygonHead primary object.
    Xna(Box<xna::XnaObject>),
}

/// A fully parsed file: primary object, shared resources, and the number of
/// unconsumed payload bytes (0 for a correct parse).
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub content: Content,
    pub shared: Vec<Option<xna::XnaObject>>,
    pub trailing: usize,
}

/// Parse an XNB's primary object followed by its shared resources, exactly
/// as `ContentReader.ReadAsset` does. Returns `None` only for a null primary
/// object.
pub fn parse_primary(x: &Xnb) -> Option<Result<Parsed>> {
    let reader = x.primary_reader()?.short_name();
    let types = x.readers.iter().map(|t| t.name.clone()).collect();
    let mut r = Reader::with_types(x.object_data(), types);
    let parsed = match reader {
        "Magicka.ContentReaders.CharacterTemplateReader" => {
            character::CharacterTemplate::read(&mut r).map(|c| Content::Character(Box::new(c)))
        }
        "Magicka.ContentReaders.ItemReader" => {
            item::Item::read(&mut r).map(|i| Content::Item(Box::new(i)))
        }
        "Magicka.ContentReaders.PhysicsEntityTemplateReader" => {
            physics::PhysicsEntityTemplate::read(&mut r)
                .map(|p| Content::PhysicsEntity(Box::new(p)))
        }
        "Magicka.ContentReaders.LevelModelReader" => {
            level::LevelModel::read(&mut r).map(|l| Content::Level(Box::new(l)))
        }
        _ => xna::read_body(&mut r, &x.readers[x.primary_type_id as usize - 1].name)
            .map(|o| Content::Xna(Box::new(o))),
    };
    Some(parsed.and_then(|content| {
        let shared = (0..x.shared_resource_count)
            .map(|_| xna::read_object(&mut r))
            .collect::<Result<_>>()?;
        Ok(Parsed {
            content,
            shared,
            trailing: r.remaining(),
        })
    }))
}
