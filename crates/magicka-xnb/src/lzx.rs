//! LZX decompressor, as used by XNA's XMemCompress for compressed `.xnb` files.
//!
//! Implements the LZX bitstream as published in libmspack's `lzxd.c` and
//! MonoGame's `LzxDecoder` (block types verbatim/aligned/uncompressed,
//! pretree delta-coded Huffman lengths, R0..R2 repeated offsets, Intel E8
//! translation). Nothing here is Magicka-specific; XNB uses a 64 KiB window.
//!
//! Frames are decoded one at a time via [`LzxDecoder::decompress_frame`]. The
//! bit buffer is reset at every call, matching XNB where every compressed
//! block starts byte-aligned. Decoder state (window, trees, repeated offsets,
//! partially consumed LZX block) persists across calls.

use std::fmt;

const MIN_MATCH: usize = 2;
const MAX_MATCH: usize = 257;
const NUM_CHARS: usize = 256;
const BLOCKTYPE_VERBATIM: u8 = 1;
const BLOCKTYPE_ALIGNED: u8 = 2;
const BLOCKTYPE_UNCOMPRESSED: u8 = 3;
const PRETREE_NUM_ELEMENTS: usize = 20;
const ALIGNED_NUM_ELEMENTS: usize = 8;
const NUM_PRIMARY_LENGTHS: usize = 7;
const NUM_SECONDARY_LENGTHS: usize = 249;
/// Slack after length tables: run-length codes may write a little past `last`.
const LENTABLE_SAFETY: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LzxError {
    InvalidWindowBits(u32),
    InvalidBlockType(u8),
    /// The bitstream contained a code that is not in the current Huffman tree.
    BadHuffmanCode(&'static str),
    /// A Huffman length table is over-subscribed (not a valid prefix code).
    InvalidHuffmanTable(&'static str),
    LengthTableOverrun,
    /// Decoding would run past the end of the sliding window.
    WindowOverrun,
    /// A match continued past the end of the current LZX block.
    BlockOverrun,
    /// Input ended inside an uncompressed block.
    Truncated,
    FrameTooLarge {
        frame: usize,
        window: usize,
    },
}

impl fmt::Display for LzxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for LzxError {}

/// 16-bit little-endian words, bits consumed MSB-first (LZX convention).
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    bits_left: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            buf: 0,
            bits_left: 0,
        }
    }

    fn byte(&mut self) -> u32 {
        // Reading past the end yields zeros; the decoder may look ahead a
        // few bits beyond the last symbol of a block.
        let b = self.data.get(self.pos).copied().unwrap_or(0);
        self.pos += 1;
        b as u32
    }

    fn ensure(&mut self, n: u32) {
        while self.bits_left < n {
            let lo = self.byte();
            let hi = self.byte();
            self.buf |= ((hi << 8) | lo) << (16 - self.bits_left);
            self.bits_left += 16;
        }
    }

    fn peek(&self, n: u32) -> u32 {
        self.buf >> (32 - n)
    }

    fn remove(&mut self, n: u32) {
        self.buf <<= n;
        self.bits_left -= n;
    }

    fn read(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        self.ensure(n);
        let v = self.peek(n);
        self.remove(n);
        v
    }

    fn reset_bits(&mut self) {
        self.buf = 0;
        self.bits_left = 0;
    }

    fn raw_u32_le(&mut self) -> Result<u32, LzxError> {
        let b = self
            .data
            .get(self.pos..self.pos + 4)
            .ok_or(LzxError::Truncated)?;
        self.pos += 4;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// Canonical Huffman decoder (codes assigned by increasing length, then by
/// symbol index; max code length 16).
#[derive(Clone, Default)]
struct Huffman {
    counts: [u16; 17],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lens: &[u8], name: &'static str) -> Result<Self, LzxError> {
        let mut counts = [0u16; 17];
        for &l in lens {
            if l > 16 {
                return Err(LzxError::InvalidHuffmanTable(name));
            }
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        // Reject over-subscribed tables; incomplete tables are legal in LZX
        // (e.g. a length tree that is never used).
        let mut left: i32 = 1;
        for &c in &counts[1..] {
            left = (left << 1) - c as i32;
            if left < 0 {
                return Err(LzxError::InvalidHuffmanTable(name));
            }
        }
        let mut offs = [0u16; 17];
        for l in 1..16 {
            offs[l + 1] = offs[l] + counts[l];
        }
        let total = offs[16] + counts[16];
        let mut symbols = vec![0u16; total as usize];
        for (sym, &l) in lens.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = sym as u16;
                offs[l as usize] += 1;
            }
        }
        Ok(Self { counts, symbols })
    }

    fn decode(&self, br: &mut BitReader, name: &'static str) -> Result<usize, LzxError> {
        br.ensure(16);
        let bits = br.peek(16);
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..=16u32 {
            code |= ((bits >> (16 - len)) & 1) as i32;
            let count = self.counts[len as usize] as i32;
            if code - first < count {
                br.remove(len);
                return Ok(self.symbols[(index + code - first) as usize] as usize);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err(LzxError::BadHuffmanCode(name))
    }
}

pub struct LzxDecoder {
    window: Vec<u8>,
    window_size: usize,
    window_posn: usize,
    frame_posn: usize,
    r: [u32; 3],
    main_elements: usize,
    header_read: bool,
    block_type: u8,
    block_length: usize,
    block_remaining: usize,
    intel_filesize: i32,
    intel_curpos: i32,
    intel_started: bool,
    frames_read: u32,
    maintree_len: Vec<u8>,
    length_len: Vec<u8>,
    maintree: Huffman,
    length: Huffman,
    aligned: Huffman,
    extra_bits: [u8; 51],
    position_base: [u32; 51],
}

impl LzxDecoder {
    /// `window_bits` is 15..=21; XNB always uses 16.
    pub fn new(window_bits: u32) -> Result<Self, LzxError> {
        let posn_slots = match window_bits {
            15 => 30,
            16 => 32,
            17 => 34,
            18 => 36,
            19 => 38,
            20 => 42,
            21 => 50,
            _ => return Err(LzxError::InvalidWindowBits(window_bits)),
        };
        let window_size = 1usize << window_bits;
        let main_elements = NUM_CHARS + posn_slots * 8;
        let mut extra_bits = [0u8; 51];
        let mut j = 0u8;
        let mut i = 0;
        while i < 50 {
            extra_bits[i] = j;
            extra_bits[i + 1] = j;
            if i != 0 && j < 17 {
                j += 1;
            }
            i += 2;
        }
        let mut position_base = [0u32; 51];
        let mut acc = 0u32;
        for i in 0..51 {
            position_base[i] = acc;
            acc += 1 << extra_bits[i];
        }
        Ok(Self {
            // Extra room past the window for a match that overshoots the end
            // of the final frame in the window.
            window: vec![0; window_size + MAX_MATCH + 1],
            window_size,
            window_posn: 0,
            frame_posn: 0,
            r: [1, 1, 1],
            main_elements,
            header_read: false,
            block_type: 0,
            block_length: 0,
            block_remaining: 0,
            intel_filesize: 0,
            intel_curpos: 0,
            intel_started: false,
            frames_read: 0,
            maintree_len: vec![0; main_elements + LENTABLE_SAFETY],
            length_len: vec![0; NUM_SECONDARY_LENGTHS + LENTABLE_SAFETY],
            maintree: Huffman::default(),
            length: Huffman::default(),
            aligned: Huffman::default(),
            extra_bits,
            position_base,
        })
    }

    fn read_lengths(
        br: &mut BitReader,
        lens: &mut [u8],
        first: usize,
        last: usize,
    ) -> Result<(), LzxError> {
        let mut pre_len = [0u8; PRETREE_NUM_ELEMENTS];
        for l in pre_len.iter_mut() {
            *l = br.read(4) as u8;
        }
        let pre = Huffman::new(&pre_len, "pretree")?;
        let mut x = first;
        let put = |lens: &mut [u8], x: &mut usize, v: u8| -> Result<(), LzxError> {
            *lens.get_mut(*x).ok_or(LzxError::LengthTableOverrun)? = v;
            *x += 1;
            Ok(())
        };
        while x < last {
            let z = pre.decode(br, "pretree")?;
            match z {
                17 => {
                    for _ in 0..br.read(4) + 4 {
                        put(lens, &mut x, 0)?;
                    }
                }
                18 => {
                    for _ in 0..br.read(5) + 20 {
                        put(lens, &mut x, 0)?;
                    }
                }
                19 => {
                    let run = br.read(1) + 4;
                    let z = pre.decode(br, "pretree")?;
                    let cur = *lens.get(x).ok_or(LzxError::LengthTableOverrun)? as usize;
                    let v = ((cur + 17 - z) % 17) as u8;
                    for _ in 0..run {
                        put(lens, &mut x, v)?;
                    }
                }
                _ => {
                    let v = ((lens[x] as usize + 17 - z) % 17) as u8;
                    put(lens, &mut x, v)?;
                }
            }
        }
        Ok(())
    }

    fn read_main_and_length_trees(&mut self, br: &mut BitReader) -> Result<(), LzxError> {
        Self::read_lengths(br, &mut self.maintree_len, 0, NUM_CHARS)?;
        Self::read_lengths(br, &mut self.maintree_len, NUM_CHARS, self.main_elements)?;
        self.maintree = Huffman::new(&self.maintree_len[..self.main_elements], "maintree")?;
        if self.maintree_len[0xE8] != 0 {
            self.intel_started = true;
        }
        Self::read_lengths(br, &mut self.length_len, 0, NUM_SECONDARY_LENGTHS)?;
        self.length = Huffman::new(&self.length_len[..NUM_SECONDARY_LENGTHS], "length")?;
        Ok(())
    }

    fn read_block_header(&mut self, br: &mut BitReader) -> Result<(), LzxError> {
        if self.block_type == BLOCKTYPE_UNCOMPRESSED {
            if self.block_length & 1 == 1 {
                br.pos += 1; // uncompressed blocks are padded to 16 bits
            }
            br.reset_bits();
        }
        self.block_type = br.read(3) as u8;
        let hi = br.read(16) as usize;
        let lo = br.read(8) as usize;
        self.block_length = (hi << 8) | lo;
        self.block_remaining = self.block_length;
        match self.block_type {
            BLOCKTYPE_ALIGNED => {
                let mut lens = [0u8; ALIGNED_NUM_ELEMENTS];
                for l in lens.iter_mut() {
                    *l = br.read(3) as u8;
                }
                self.aligned = Huffman::new(&lens, "aligned")?;
                self.read_main_and_length_trees(br)
            }
            BLOCKTYPE_VERBATIM => self.read_main_and_length_trees(br),
            BLOCKTYPE_UNCOMPRESSED => {
                self.intel_started = true;
                // Align to the next 16-bit word; if already aligned the
                // format still inserts 16 bits of padding.
                br.ensure(16);
                if br.bits_left > 16 {
                    br.pos -= 2;
                }
                br.reset_bits();
                for k in 0..3 {
                    self.r[k] = br.raw_u32_le()?;
                }
                Ok(())
            }
            t => Err(LzxError::InvalidBlockType(t)),
        }
    }

    /// Decode a match offset slot into an absolute offset, updating R0..R2.
    fn match_offset(
        &mut self,
        br: &mut BitReader,
        slot: usize,
        aligned: bool,
    ) -> Result<usize, LzxError> {
        let off = match slot {
            0 => return Ok(self.r[0] as usize),
            1 => {
                self.r.swap(0, 1);
                return Ok(self.r[0] as usize);
            }
            2 => {
                self.r.swap(0, 2);
                return Ok(self.r[0] as usize);
            }
            _ => {
                let extra = self.extra_bits[slot] as u32;
                let base = self.position_base[slot] - 2;
                if !aligned {
                    base + br.read(extra)
                } else if extra > 3 {
                    let verbatim = br.read(extra - 3);
                    let al = self.aligned.decode(br, "aligned")? as u32;
                    base + (verbatim << 3) + al
                } else if extra == 3 {
                    base + self.aligned.decode(br, "aligned")? as u32
                } else if extra > 0 {
                    base + br.read(extra)
                } else {
                    1
                }
            }
        };
        self.r[2] = self.r[1];
        self.r[1] = self.r[0];
        self.r[0] = off;
        Ok(off as usize)
    }

    /// Decompress one frame. `input` is one XNB compressed block; `out.len()`
    /// is the frame's uncompressed size (0x8000 except possibly the last).
    pub fn decompress_frame(&mut self, input: &[u8], out: &mut [u8]) -> Result<(), LzxError> {
        let ws = self.window_size;
        let frame_start = self.frame_posn;
        let frame_end = frame_start + out.len();
        if frame_end > ws {
            return Err(LzxError::FrameTooLarge {
                frame: out.len(),
                window: ws,
            });
        }
        let mut br = BitReader::new(input);
        if !self.header_read {
            if br.read(1) != 0 {
                let hi = br.read(16);
                let lo = br.read(16);
                self.intel_filesize = ((hi << 16) | lo) as i32;
            }
            self.header_read = true;
        }

        while self.window_posn < frame_end {
            if self.block_remaining == 0 {
                self.read_block_header(&mut br)?;
                continue;
            }
            let run = self.block_remaining.min(frame_end - self.window_posn);
            self.block_remaining -= run;
            let mut this_run = run as isize;
            match self.block_type {
                BLOCKTYPE_VERBATIM | BLOCKTYPE_ALIGNED => {
                    let aligned = self.block_type == BLOCKTYPE_ALIGNED;
                    while this_run > 0 {
                        let main = self.maintree.decode(&mut br, "maintree")?;
                        if main < NUM_CHARS {
                            self.window[self.window_posn] = main as u8;
                            self.window_posn += 1;
                            this_run -= 1;
                            continue;
                        }
                        let main = main - NUM_CHARS;
                        let mut len = main & NUM_PRIMARY_LENGTHS;
                        if len == NUM_PRIMARY_LENGTHS {
                            len += self.length.decode(&mut br, "length")?;
                        }
                        len += MIN_MATCH;
                        let off = self.match_offset(&mut br, main >> 3, aligned)?;
                        if off == 0 || off > ws {
                            return Err(LzxError::WindowOverrun);
                        }
                        let dst = self.window_posn;
                        if dst + len > self.window.len() {
                            return Err(LzxError::WindowOverrun);
                        }
                        if dst >= off {
                            // Byte-wise copy: source and destination may overlap.
                            let src = dst - off;
                            for k in 0..len {
                                self.window[dst + k] = self.window[src + k];
                            }
                        } else {
                            let src = dst + ws - off;
                            for k in 0..len {
                                self.window[dst + k] = self.window[(src + k) % ws];
                            }
                        }
                        self.window_posn += len;
                        this_run -= len as isize;
                    }
                    if this_run < 0 {
                        // The last match ran into the next frame.
                        let over = (-this_run) as usize;
                        if over > self.block_remaining {
                            return Err(LzxError::BlockOverrun);
                        }
                        self.block_remaining -= over;
                    }
                }
                BLOCKTYPE_UNCOMPRESSED => {
                    let src = br
                        .data
                        .get(br.pos..br.pos + run)
                        .ok_or(LzxError::Truncated)?;
                    self.window[self.window_posn..self.window_posn + run].copy_from_slice(src);
                    br.pos += run;
                    self.window_posn += run;
                }
                t => return Err(LzxError::InvalidBlockType(t)),
            }
        }

        out.copy_from_slice(&self.window[frame_start..frame_end]);
        self.intel_e8(out);
        self.frames_read += 1;

        self.frame_posn = frame_end;
        if self.frame_posn == ws {
            self.frame_posn = 0;
            // Move any overshoot past the window end back to its start.
            let over = self.window_posn - ws;
            self.window.copy_within(ws..ws + over, 0);
            self.window_posn = over;
        }
        Ok(())
    }

    fn intel_e8(&mut self, out: &mut [u8]) {
        let n = out.len();
        if !(self.intel_started && self.intel_filesize != 0 && self.frames_read < 32768 && n > 10) {
            self.intel_curpos = self.intel_curpos.wrapping_add(n as i32);
            return;
        }
        let filesize = self.intel_filesize;
        let mut curpos = self.intel_curpos;
        let mut i = 0;
        while i < n - 10 {
            if out[i] != 0xE8 {
                i += 1;
                curpos += 1;
                continue;
            }
            let abs = i32::from_le_bytes([out[i + 1], out[i + 2], out[i + 3], out[i + 4]]);
            if abs >= -curpos && abs < filesize {
                let rel = if abs >= 0 {
                    abs - curpos
                } else {
                    abs + filesize
                };
                out[i + 1..i + 5].copy_from_slice(&rel.to_le_bytes());
            }
            i += 5;
            curpos += 5;
        }
        self.intel_curpos = self.intel_curpos.wrapping_add(n as i32);
    }

    /// True if the stream header requested Intel E8 call translation.
    pub fn uses_intel_e8(&self) -> bool {
        self.intel_filesize != 0
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Test-only LZX bit writer: MSB-first bits packed into 16-bit LE words.
    pub(crate) struct BitWriter {
        pub out: Vec<u8>,
        acc: u32,
        n: u32,
    }

    impl BitWriter {
        pub fn new() -> Self {
            Self {
                out: Vec::new(),
                acc: 0,
                n: 0,
            }
        }
        pub fn put(&mut self, value: u32, bits: u32) {
            for i in (0..bits).rev() {
                self.acc = (self.acc << 1) | ((value >> i) & 1);
                self.n += 1;
                if self.n == 16 {
                    self.out.extend_from_slice(&(self.acc as u16).to_le_bytes());
                    self.acc = 0;
                    self.n = 0;
                }
            }
        }
        /// Pad to a word boundary.
        pub fn flush(&mut self) {
            if self.n > 0 {
                let pad = 16 - self.n;
                self.put(0, pad);
            }
        }
    }

    #[test]
    fn canonical_huffman_codes() {
        // lens: sym0=2, sym1=1, sym2=3, sym3=3 -> codes 1:"0" 0:"10" 2:"110" 3:"111"
        let h = Huffman::new(&[2, 1, 3, 3], "t").unwrap();
        let mut w = BitWriter::new();
        for (code, bits) in [(0b0, 1), (0b10, 2), (0b110, 3), (0b111, 3)] {
            w.put(code, bits);
        }
        w.flush();
        let mut br = BitReader::new(&w.out);
        let got: Vec<usize> = (0..4).map(|_| h.decode(&mut br, "t").unwrap()).collect();
        assert_eq!(got, vec![1, 0, 2, 3]);
    }

    #[test]
    fn oversubscribed_table_rejected() {
        assert!(Huffman::new(&[1, 1, 1], "t").is_err());
    }

    #[test]
    fn extra_bits_and_position_base_match_spec() {
        let d = LzxDecoder::new(16).unwrap();
        assert_eq!(&d.extra_bits[..8], &[0, 0, 0, 0, 1, 1, 2, 2]);
        assert_eq!(&d.position_base[..8], &[0, 1, 2, 3, 4, 6, 8, 12]);
        assert_eq!(d.main_elements, 256 + 32 * 8);
    }

    #[test]
    fn uncompressed_block() {
        let payload = b"hello, lzx!"; // odd length exercises padding logic
        let mut w = BitWriter::new();
        w.put(0, 1); // no intel E8
        w.put(BLOCKTYPE_UNCOMPRESSED as u32, 3);
        w.put((payload.len() >> 8) as u32, 16);
        w.put((payload.len() & 0xFF) as u32, 8);
        w.flush(); // 28 bits -> 32: the decoder rewinds into this boundary
        let mut bytes = w.out;
        for r in [1u32, 1, 1] {
            bytes.extend_from_slice(&r.to_le_bytes());
        }
        bytes.extend_from_slice(payload);
        let mut d = LzxDecoder::new(16).unwrap();
        let mut out = vec![0; payload.len()];
        d.decompress_frame(&bytes, &mut out).unwrap();
        assert_eq!(&out, payload);
    }

    /// Writes a pretree header with every element at length 5 (code == symbol)
    /// followed by `count` deltas that set each length to `target` from 0.
    fn write_flat_lengths(w: &mut BitWriter, count: usize, target: u32) {
        for _ in 0..PRETREE_NUM_ELEMENTS {
            w.put(5, 4);
        }
        let z = (17 - target) % 17; // new = (old - z) mod 17 with old = 0
        for _ in 0..count {
            w.put(z, 5);
        }
    }

    #[test]
    fn verbatim_block_literals_and_matches() {
        // Main tree: all 512 symbols length 9 (code == symbol).
        // Length tree: all 249 symbols length 8 (code == symbol).
        let mut w = BitWriter::new();
        w.put(0, 1);
        w.put(BLOCKTYPE_VERBATIM as u32, 3);
        w.put(0, 16);
        w.put(15, 8);
        write_flat_lengths(&mut w, 256, 9);
        write_flat_lengths(&mut w, 256, 9);
        write_flat_lengths(&mut w, 249, 8);
        for c in b"abc" {
            w.put(*c as u32, 9);
        }
        // Match: slot 4 (1 extra bit, base 4 -> offset 2 + bit), length header 7
        // + footer 1 -> length 10, extra bit 1 -> offset 3.
        w.put((256 + (4 << 3) + 7) as u32, 9);
        w.put(1, 8); // length footer
        w.put(1, 1); // verbatim offset bit
                     // Repeat match R0 (=3), length header 0 -> length 2.
        w.put(256, 9);
        w.flush();
        let mut d = LzxDecoder::new(16).unwrap();
        let mut out = vec![0; 15];
        d.decompress_frame(&w.out, &mut out).unwrap();
        assert_eq!(&out, b"abcabcabcabcabc");
    }
}
