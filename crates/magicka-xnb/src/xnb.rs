//! XNA 3.x `.xnb` container: file header, LZX-compressed payload, and the
//! type-reader manifest that names the `ContentTypeReader` for each object.
//!
//! Layout (XNA Game Studio 3.1, format version 4):
//!
//! ```text
//! "XNB" platform:u8 version:u8 flags:u8 file_size:u32le
//! [if flags & 0x80: decompressed_size:u32le, then LZX blocks:
//!      hi:u8 lo:u8 -> block_size = hi<<8|lo, frame_size = 0x8000
//!      or, if hi == 0xFF: frame_size:u16be block_size:u16be]
//! payload: reader_count:7bit { name:string version:i32 }* shared_count:7bit
//!          primary object: type_id:7bit (0 = null, else reader index + 1) ...
//! ```

use crate::lzx::{LzxDecoder, LzxError};
use std::fmt;

pub const FLAG_COMPRESSED: u8 = 0x80;
const HEADER_LEN: usize = 10;
const LZX_WINDOW_BITS: u32 = 16;
const DEFAULT_FRAME: usize = 0x8000;

#[derive(Debug)]
pub enum XnbError {
    BadMagic,
    Truncated(&'static str),
    FileSizeMismatch { header: u32, actual: usize },
    Lzx(LzxError),
    DecompressedSizeMismatch { expected: usize, actual: usize },
    BadString,
}

impl fmt::Display for XnbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for XnbError {}

impl From<LzxError> for XnbError {
    fn from(e: LzxError) -> Self {
        XnbError::Lzx(e)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XnbHeader {
    /// 'w' = Windows, 'x' = Xbox 360, 'm' = Zune.
    pub platform: u8,
    /// 4 for XNA 3.1 (3 = XNA 3.0, 5 = XNA 4.0).
    pub version: u8,
    pub flags: u8,
    pub file_size: u32,
}

impl XnbHeader {
    pub fn compressed(&self) -> bool {
        self.flags & FLAG_COMPRESSED != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeReader {
    /// Assembly-qualified .NET type name, e.g.
    /// `Microsoft.Xna.Framework.Content.Texture2DReader`.
    pub name: String,
    pub version: i32,
}

impl TypeReader {
    /// The type name without generic arguments' assembly qualification or
    /// the trailing `, Assembly, Version=...` part.
    pub fn short_name(&self) -> &str {
        // Generic names contain `[[...]]`; cut at the first ", " outside brackets.
        let mut depth = 0i32;
        for (i, c) in self.name.char_indices() {
            match c {
                '[' => depth += 1,
                ']' => depth -= 1,
                ',' if depth == 0 => return &self.name[..i],
                _ => {}
            }
        }
        &self.name
    }
}

#[derive(Debug, Clone)]
pub struct Xnb {
    pub header: XnbHeader,
    /// Decompressed payload (everything after the 10/14-byte header).
    pub payload: Vec<u8>,
    pub readers: Vec<TypeReader>,
    pub shared_resource_count: u32,
    /// 0 = null object, otherwise index into `readers` + 1.
    pub primary_type_id: u32,
    /// Offset in `payload` where the primary object's data begins.
    pub object_offset: usize,
}

impl Xnb {
    pub fn primary_reader(&self) -> Option<&TypeReader> {
        (self.primary_type_id as usize)
            .checked_sub(1)
            .and_then(|i| self.readers.get(i))
    }

    pub fn object_data(&self) -> &[u8] {
        &self.payload[self.object_offset..]
    }
}

/// Little-endian cursor implementing the .NET `BinaryReader` primitives
/// used by XNA's `ContentReader`.
pub struct Cursor<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], XnbError> {
        let s = self
            .data
            .get(self.pos..self.pos + n)
            .ok_or(XnbError::Truncated("bytes"))?;
        self.pos += n;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8, XnbError> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u32(&mut self) -> Result<u32, XnbError> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> Result<i32, XnbError> {
        Ok(self.u32()? as i32)
    }

    pub fn f32(&mut self) -> Result<f32, XnbError> {
        Ok(f32::from_bits(self.u32()?))
    }

    /// .NET `Read7BitEncodedInt`.
    pub fn var_u32(&mut self) -> Result<u32, XnbError> {
        let mut result = 0u32;
        for shift in (0..35).step_by(7) {
            let b = self.u8()?;
            result |= ((b & 0x7F) as u32) << shift;
            if b & 0x80 == 0 {
                return Ok(result);
            }
        }
        Err(XnbError::Truncated("7-bit int too long"))
    }

    /// .NET `BinaryReader.ReadString`: 7-bit length prefix, UTF-8 bytes.
    pub fn string(&mut self) -> Result<String, XnbError> {
        let n = self.var_u32()? as usize;
        let b = self.bytes(n)?;
        String::from_utf8(b.to_vec()).map_err(|_| XnbError::BadString)
    }
}

pub fn parse_header(bytes: &[u8]) -> Result<XnbHeader, XnbError> {
    if bytes.len() < HEADER_LEN {
        return Err(XnbError::Truncated("header"));
    }
    if &bytes[..3] != b"XNB" {
        return Err(XnbError::BadMagic);
    }
    Ok(XnbHeader {
        platform: bytes[3],
        version: bytes[4],
        flags: bytes[5],
        file_size: u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]),
    })
}

/// Return the decompressed payload (the bytes after the XNB header).
pub fn decompress_payload(bytes: &[u8]) -> Result<(XnbHeader, Vec<u8>), XnbError> {
    let header = parse_header(bytes)?;
    if header.file_size as usize != bytes.len() {
        return Err(XnbError::FileSizeMismatch {
            header: header.file_size,
            actual: bytes.len(),
        });
    }
    if !header.compressed() {
        return Ok((header, bytes[HEADER_LEN..].to_vec()));
    }
    let mut c = Cursor::new(bytes);
    c.pos = HEADER_LEN;
    let decompressed_size = c.u32()? as usize;
    let mut out = vec![0u8; decompressed_size];
    let mut out_pos = 0;
    let mut dec = LzxDecoder::new(LZX_WINDOW_BITS)?;
    while c.pos < bytes.len() && out_pos < decompressed_size {
        let hi = c.u8()?;
        let lo = c.u8()?;
        let (frame, block) = if hi == 0xFF {
            let f = ((lo as usize) << 8) | c.u8()? as usize;
            let b = ((c.u8()? as usize) << 8) | c.u8()? as usize;
            (f, b)
        } else {
            (DEFAULT_FRAME, ((hi as usize) << 8) | lo as usize)
        };
        if frame == 0 || block == 0 {
            break;
        }
        if out_pos + frame > decompressed_size {
            return Err(XnbError::DecompressedSizeMismatch {
                expected: decompressed_size,
                actual: out_pos + frame,
            });
        }
        let input = c.bytes(block)?;
        dec.decompress_frame(input, &mut out[out_pos..out_pos + frame])?;
        out_pos += frame;
    }
    if out_pos != decompressed_size {
        return Err(XnbError::DecompressedSizeMismatch {
            expected: decompressed_size,
            actual: out_pos,
        });
    }
    Ok((header, out))
}

pub fn parse(bytes: &[u8]) -> Result<Xnb, XnbError> {
    let (header, payload) = decompress_payload(bytes)?;
    let mut c = Cursor::new(&payload);
    let count = c.var_u32()? as usize;
    if count > 4096 {
        return Err(XnbError::Truncated("implausible type reader count"));
    }
    let mut readers = Vec::with_capacity(count);
    for _ in 0..count {
        let name = c.string()?;
        let version = c.i32()?;
        readers.push(TypeReader { name, version });
    }
    let shared_resource_count = c.var_u32()?;
    let primary_type_id = c.var_u32()?;
    let object_offset = c.pos;
    Ok(Xnb {
        header,
        payload,
        readers,
        shared_resource_count,
        primary_type_id,
        object_offset,
    })
}

/// Summary of an XNA 3.1 `Texture2DReader` object, used as a structural check
/// of decompression: the declared mip sizes must consume the payload exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture2DInfo {
    pub surface_format: i32,
    pub width: u32,
    pub height: u32,
    pub mip_sizes: Vec<u32>,
    /// Bytes left in the payload after the last mip (0 for a well-formed file
    /// without shared resources).
    pub trailing: usize,
}

pub fn texture2d_info(x: &Xnb) -> Result<Texture2DInfo, XnbError> {
    let mut c = Cursor::new(x.object_data());
    let surface_format = c.i32()?;
    let width = c.u32()?;
    let height = c.u32()?;
    let mips = c.u32()?;
    if mips > 32 {
        return Err(XnbError::Truncated("implausible mip count"));
    }
    let mut mip_sizes = Vec::new();
    for _ in 0..mips {
        let n = c.u32()?;
        c.bytes(n as usize)?;
        mip_sizes.push(n);
    }
    Ok(Texture2DInfo {
        surface_format,
        width,
        height,
        mip_sizes,
        trailing: c.remaining(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xnb_uncompressed(payload: &[u8]) -> Vec<u8> {
        let mut v = b"XNBw\x04\x00".to_vec();
        v.extend_from_slice(&((HEADER_LEN + payload.len()) as u32).to_le_bytes());
        v.extend_from_slice(payload);
        v
    }

    fn put_string(v: &mut Vec<u8>, s: &str) {
        v.push(s.len() as u8);
        v.extend_from_slice(s.as_bytes());
    }

    #[test]
    fn parses_uncompressed_manifest_and_texture() {
        let mut p = vec![1u8];
        put_string(
            &mut p,
            "Microsoft.Xna.Framework.Content.Texture2DReader, Microsoft.Xna.Framework.Graphics",
        );
        p.extend_from_slice(&0i32.to_le_bytes());
        p.push(0); // shared resources
        p.push(1); // primary type id
        for v in [1i32, 2, 1, 1] {
            p.extend_from_slice(&v.to_le_bytes()); // format, w, h, mips
        }
        p.extend_from_slice(&8u32.to_le_bytes());
        p.extend_from_slice(&[0xAA; 8]);
        let x = parse(&xnb_uncompressed(&p)).unwrap();
        assert_eq!(x.readers.len(), 1);
        assert_eq!(
            x.primary_reader().unwrap().short_name(),
            "Microsoft.Xna.Framework.Content.Texture2DReader"
        );
        let t = texture2d_info(&x).unwrap();
        assert_eq!(
            (t.width, t.height, t.mip_sizes.clone(), t.trailing),
            (2, 1, vec![8], 0)
        );
    }

    #[test]
    fn short_name_handles_generics() {
        let r = TypeReader {
            name: "Microsoft.Xna.Framework.Content.ListReader`1[[System.Int32, mscorlib, Version=2.0.0.0]], Microsoft.Xna.Framework".into(),
            version: 0,
        };
        assert_eq!(r.short_name(), "Microsoft.Xna.Framework.Content.ListReader`1[[System.Int32, mscorlib, Version=2.0.0.0]]");
    }

    #[test]
    fn file_size_mismatch_is_rejected() {
        let mut v = xnb_uncompressed(&[0, 0, 0]);
        v.push(0);
        assert!(matches!(parse(&v), Err(XnbError::FileSizeMismatch { .. })));
    }
}
