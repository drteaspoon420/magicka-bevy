//! Readers for Magicka's XNA 3.1 content files.
//!
//! - [`lzx`]: LZX (XMemCompress) decompressor.
//! - [`xnb`]: `.xnb` container, type-reader manifest, Texture2D structure.
//!
//! Format notes and evidence: `output/research/xnb-format.md`.

pub mod lzx;
pub mod xnb;
