//! Windows animated cursors (`Cursors/*.ani`, loaded by Game.CreateCursor
//! through LoadCursorFromFile): RIFF `ACON` with an `anih` header, optional
//! `rate`/`seq ` chunks and a `LIST fram` of `icon` chunks, each a CUR/ICO
//! file whose image is a BMP DIB (XOR colour + AND mask) or PNG.

/// One decoded cursor frame, straight RGBA.
#[derive(Clone, Debug, PartialEq)]
pub struct CursorFrame {
    pub width: u32,
    pub height: u32,
    pub hotspot: (u16, u16),
    pub rgba: Vec<u8>,
}

/// The animation: frames, the step sequence (frame indices) and each
/// step's duration in jiffies (1/60 s).
#[derive(Clone, Debug, PartialEq)]
pub struct AnimatedCursor {
    pub frames: Vec<CursorFrame>,
    pub sequence: Vec<usize>,
    pub rates: Vec<u32>,
}

fn u16_at(b: &[u8], o: usize) -> Result<u16, String> {
    b.get(o..o + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| format!("truncated at {o}"))
}
fn u32_at(b: &[u8], o: usize) -> Result<u32, String> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| format!("truncated at {o}"))
}

/// Parse an `.ani` file.
pub fn parse(bytes: &[u8]) -> Result<AnimatedCursor, String> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"ACON") {
        return Err("not a RIFF ACON file".into());
    }
    let (mut frames, mut sequence, mut rates) = (Vec::new(), Vec::new(), Vec::new());
    let (mut steps, mut default_rate) = (0usize, 0u32);
    let mut chunks = vec![(12usize, bytes.len())];
    while let Some((mut at, end)) = chunks.pop() {
        while at + 8 <= end {
            let id = &bytes[at..at + 4];
            let size = u32_at(bytes, at + 4)? as usize;
            let body = at + 8;
            let body_end = (body + size).min(end);
            match id {
                b"anih" => {
                    steps = u32_at(bytes, body + 8)? as usize;
                    default_rate = u32_at(bytes, body + 28)?;
                }
                b"rate" => {
                    rates = (0..size / 4)
                        .map(|i| u32_at(bytes, body + i * 4))
                        .collect::<Result<_, _>>()?;
                }
                b"seq " => {
                    sequence = (0..size / 4)
                        .map(|i| u32_at(bytes, body + i * 4).map(|v| v as usize))
                        .collect::<Result<_, _>>()?;
                }
                b"LIST" => chunks.push((body + 4, body_end)),
                b"icon" => frames.push(parse_cur(&bytes[body..body_end])?),
                _ => {}
            }
            at = body + size + (size & 1);
        }
    }
    if frames.is_empty() {
        return Err("no icon frames".into());
    }
    if sequence.is_empty() {
        sequence = (0..steps.max(frames.len()))
            .map(|i| i % frames.len())
            .collect();
    }
    if rates.is_empty() {
        rates = vec![default_rate.max(1); sequence.len()];
    }
    Ok(AnimatedCursor {
        frames,
        sequence,
        rates,
    })
}

/// The first (largest listed first) image of a CUR/ICO file.
pub fn parse_cur(b: &[u8]) -> Result<CursorFrame, String> {
    let count = u16_at(b, 4)?;
    if count == 0 {
        return Err("empty cursor directory".into());
    }
    let kind = u16_at(b, 2)?;
    let entry = 6;
    let w = b[entry] as u32;
    let h = b[entry + 1] as u32;
    let (hx, hy) = if kind == 2 {
        (u16_at(b, entry + 4)?, u16_at(b, entry + 6)?)
    } else {
        (0, 0)
    };
    let size = u32_at(b, entry + 8)? as usize;
    let offset = u32_at(b, entry + 12)? as usize;
    let image = b
        .get(offset..offset + size)
        .ok_or("cursor image out of bounds")?;
    if image.starts_with(b"\x89PNG") {
        return Err("PNG cursor frames are not supported".into());
    }
    let (width, height, rgba) = decode_dib(image)?;
    let _ = (w, h);
    Ok(CursorFrame {
        width,
        height,
        hotspot: (hx, hy),
        rgba,
    })
}

/// BITMAPINFOHEADER DIB with doubled height: bottom-up XOR rows (1/4/8/24/
/// 32 bpp) followed by the 1 bpp AND mask.
fn decode_dib(d: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let header = u32_at(d, 0)? as usize;
    let width = u32_at(d, 4)? as i32;
    let height = u32_at(d, 8)? as i32 / 2;
    let bpp = u16_at(d, 14)? as usize;
    let mut colors = u32_at(d, 32)? as usize;
    if colors == 0 && bpp <= 8 {
        colors = 1 << bpp;
    }
    let (w, h) = (width as usize, height as usize);
    let palette = header;
    let xor = palette + colors * 4;
    let xor_stride = (w * bpp).div_ceil(32) * 4;
    let and = xor + xor_stride * h;
    let and_stride = w.div_ceil(32) * 4;
    let mut rgba = vec![0u8; w * h * 4];
    let mut any_alpha = false;
    for y in 0..h {
        let row = xor + (h - 1 - y) * xor_stride;
        for x in 0..w {
            let (r, g, bl, a) = match bpp {
                32 => {
                    let p = row + x * 4;
                    let px = d.get(p..p + 4).ok_or("truncated pixels")?;
                    (px[2], px[1], px[0], px[3])
                }
                24 => {
                    let p = row + x * 3;
                    let px = d.get(p..p + 3).ok_or("truncated pixels")?;
                    (px[2], px[1], px[0], 255)
                }
                1 | 4 | 8 => {
                    let bit = x * bpp;
                    let byte = *d.get(row + bit / 8).ok_or("truncated pixels")?;
                    let shift = 8 - bpp - (bit % 8);
                    let index = ((byte >> shift) as usize) & ((1 << bpp) - 1);
                    let p = palette + index * 4;
                    let c = d.get(p..p + 4).ok_or("truncated palette")?;
                    (c[2], c[1], c[0], 255)
                }
                _ => return Err(format!("unsupported cursor depth {bpp}")),
            };
            if bpp == 32 && a != 0 {
                any_alpha = true;
            }
            let o = (y * w + x) * 4;
            rgba[o..o + 4].copy_from_slice(&[r, g, bl, a]);
        }
    }
    // Without per-pixel alpha the AND mask decides transparency.
    if !any_alpha {
        for y in 0..h {
            let row = and + (h - 1 - y) * and_stride;
            for x in 0..w {
                let byte = d.get(row + x / 8).copied().unwrap_or(0);
                let transparent = byte & (0x80 >> (x % 8)) != 0;
                rgba[(y * w + x) * 4 + 3] = if transparent { 0 } else { 255 };
            }
        }
    }
    Ok((w as u32, h as u32, rgba))
}
