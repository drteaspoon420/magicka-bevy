//! Texture decoding (XNA 3.1 `SurfaceFormat`) and a minimal PNG writer.
//!
//! `SurfaceFormat` values are from the decompiled XNA 3.1 enum: Color = 1
//! (D3DFMT_A8R8G8B8, bytes B,G,R,A), Dxt1 = 28, Dxt3 = 30, Dxt5 = 32. These
//! are the only formats Magicka's 2D textures use besides Dxt3, which is
//! supported for completeness. Block decoding follows the public
//! S3TC/BC1-BC3 specification.

pub const COLOR: i32 = 1;
pub const DXT1: i32 = 28;
pub const DXT3: i32 = 30;
pub const DXT5: i32 = 32;

/// Expected byte size of one mip level, or `None` for unsupported formats.
pub fn mip_size(format: i32, width: usize, height: usize) -> Option<usize> {
    let blocks = width.div_ceil(4).max(1) * height.div_ceil(4).max(1);
    match format {
        COLOR => Some(width * height * 4),
        DXT1 => Some(blocks * 8),
        DXT3 | DXT5 => Some(blocks * 16),
        _ => None,
    }
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u8;
    let g = ((c >> 5) & 63) as u8;
    let b = (c & 31) as u8;
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}

/// Four RGBA palette entries of a BC1 color block. `three_color` enables the
/// c0 <= c1 punch-through mode (DXT1 only).
fn color_palette(block: &[u8], allow_three_color: bool) -> [[u8; 4]; 4] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let (a, b) = (rgb565(c0), rgb565(c1));
    let mix = |wa: u16, wb: u16, d: u16| -> [u8; 4] {
        let f = |i: usize| ((a[i] as u16 * wa + b[i] as u16 * wb) / d) as u8;
        [f(0), f(1), f(2), 255]
    };
    if c0 > c1 || !allow_three_color {
        [
            [a[0], a[1], a[2], 255],
            [b[0], b[1], b[2], 255],
            mix(2, 1, 3),
            mix(1, 2, 3),
        ]
    } else {
        [
            [a[0], a[1], a[2], 255],
            [b[0], b[1], b[2], 255],
            mix(1, 1, 2),
            [0, 0, 0, 0],
        ]
    }
}

fn dxt5_alpha(block: &[u8]) -> [u8; 16] {
    let (a0, a1) = (block[0] as u16, block[1] as u16);
    let mut table = [0u8; 8];
    table[0] = a0 as u8;
    table[1] = a1 as u8;
    if a0 > a1 {
        for i in 1..7u16 {
            table[i as usize + 1] = (((7 - i) * a0 + i * a1) / 7) as u8;
        }
    } else {
        for i in 1..5u16 {
            table[i as usize + 1] = (((5 - i) * a0 + i * a1) / 5) as u8;
        }
        table[6] = 0;
        table[7] = 255;
    }
    let mut bits = 0u64;
    for (i, b) in block[2..8].iter().enumerate() {
        bits |= (*b as u64) << (8 * i);
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = table[((bits >> (3 * i)) & 7) as usize];
    }
    out
}

/// Decode one mip level to tightly packed RGBA8. Returns `None` for an
/// unsupported format or a data size that does not match.
pub fn decode_rgba(format: i32, width: usize, height: usize, data: &[u8]) -> Option<Vec<u8>> {
    if mip_size(format, width, height)? != data.len() {
        return None;
    }
    let mut out = vec![0u8; width * height * 4];
    if format == COLOR {
        for (o, i) in out
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(data.as_chunks::<4>().0)
        {
            o.copy_from_slice(&[i[2], i[1], i[0], i[3]]);
        }
        return Some(out);
    }
    let block_bytes = if format == DXT1 { 8 } else { 16 };
    let bw = width.div_ceil(4).max(1);
    for (bi, block) in data.chunks_exact(block_bytes).enumerate() {
        let (bx, by) = ((bi % bw) * 4, (bi / bw) * 4);
        let (color, alpha): (&[u8], Option<[u8; 16]>) = match format {
            DXT1 => (block, None),
            DXT3 => {
                let mut a = [0u8; 16];
                for (i, v) in a.iter_mut().enumerate() {
                    let nib = (block[i / 2] >> (4 * (i % 2))) & 15;
                    *v = nib * 17;
                }
                (&block[8..], Some(a))
            }
            _ => (&block[8..], Some(dxt5_alpha(block))),
        };
        let palette = color_palette(color, format == DXT1);
        let idx = u32::from_le_bytes([color[4], color[5], color[6], color[7]]);
        for p in 0..16 {
            let (x, y) = (bx + p % 4, by + p / 4);
            if x >= width || y >= height {
                continue;
            }
            let mut px = palette[((idx >> (2 * p)) & 3) as usize];
            if let Some(a) = alpha {
                px[3] = a[p];
            }
            let o = (y * width + x) * 4;
            out[o..o + 4].copy_from_slice(&px);
        }
    }
    Some(out)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

/// Encode RGBA8 as PNG (uncompressed deflate; simple and dependency-free).
pub fn encode_png(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(height * (width * 4 + 1));
    for row in rgba.chunks_exact(width * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut z = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = raw.chunks(65535).collect();
    for (i, c) in chunks.iter().enumerate() {
        z.push((i + 1 == chunks.len()) as u8);
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    if chunks.is_empty() {
        z.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8], body: &[u8]| {
        png.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(body);
        png.extend_from_slice(&c);
        png.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    png
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dxt1_solid_and_punch_through() {
        // c0 = pure red (0xF800) > c1 = pure blue (0x001F); all indices 0 -> red.
        let block = [0x00, 0xF8, 0x1F, 0x00, 0, 0, 0, 0];
        let px = decode_rgba(DXT1, 4, 4, &block).unwrap();
        assert_eq!(&px[..4], &[255, 0, 0, 255]);
        // c0 <= c1 and index 3 -> transparent black.
        let block = [0x1F, 0x00, 0x00, 0xF8, 0xFF, 0xFF, 0xFF, 0xFF];
        let px = decode_rgba(DXT1, 4, 4, &block).unwrap();
        assert_eq!(&px[..4], &[0, 0, 0, 0]);
    }

    #[test]
    fn dxt5_alpha_endpoints() {
        let mut block = [0u8; 16];
        block[0] = 200; // a0
        block[1] = 100; // a1, all indices 0 -> 200
        block[8..10].copy_from_slice(&0xFFFFu16.to_le_bytes()); // white
        let px = decode_rgba(DXT5, 4, 4, &block).unwrap();
        assert_eq!(&px[..4], &[255, 255, 255, 200]);
    }

    #[test]
    fn color_is_bgra_in_memory() {
        let px = decode_rgba(COLOR, 1, 1, &[1, 2, 3, 4]).unwrap();
        assert_eq!(px, vec![3, 2, 1, 4]);
    }

    #[test]
    fn mip_sizes_round_up_to_blocks() {
        assert_eq!(mip_size(DXT1, 1, 1), Some(8));
        assert_eq!(mip_size(DXT5, 6, 2), Some(32));
        assert_eq!(mip_size(COLOR, 3, 2), Some(24));
    }

    #[test]
    fn png_has_valid_structure() {
        let png = encode_png(2, 1, &[255, 0, 0, 255, 0, 255, 0, 128]);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert!(png.ends_with(&[0xAE, 0x42, 0x60, 0x82])); // IEND CRC
    }
}
