//! XACT 3 wave banks (`.xwb`, version 45) and wave decoding (PCM and
//! MS-ADPCM). xWMA entries can be wrapped for an external decoder. See
//! research/audio-xact.md.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XactError {
    Truncated(&'static str),
    BadSignature,
    UnsupportedVersion(u32),
    CompactFormat,
    UnsupportedCodec(WaveCodec),
    BadAdpcm(&'static str),
}

pub type XResult<T> = std::result::Result<T, XactError>;

fn u16_at(b: &[u8], o: usize) -> XResult<u16> {
    b.get(o..o + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or(XactError::Truncated("u16"))
}
fn u32_at(b: &[u8], o: usize) -> XResult<u32> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(XactError::Truncated("u32"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveCodec {
    Pcm,
    Xma,
    Adpcm,
    Wma,
}

/// WAVEBANKMINIWAVEFORMAT: tag:2, channels:3, rate:18, block align:8,
/// bits:1 (1 = 16-bit PCM).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MiniFormat {
    pub codec: WaveCodec,
    pub channels: u16,
    pub sample_rate: u32,
    pub block_align: u16,
    pub sixteen_bit: bool,
}
impl MiniFormat {
    pub fn decode(v: u32) -> Self {
        Self {
            codec: [
                WaveCodec::Pcm,
                WaveCodec::Xma,
                WaveCodec::Adpcm,
                WaveCodec::Wma,
            ][(v & 3) as usize],
            channels: ((v >> 2) & 7) as u16,
            sample_rate: (v >> 5) & 0x3_ffff,
            block_align: ((v >> 23) & 0xff) as u16,
            sixteen_bit: v >> 31 != 0,
        }
    }
    /// Bytes per MS-ADPCM block: (wBlockAlign + 22) × channels.
    pub fn adpcm_block_bytes(&self) -> usize {
        (self.block_align as usize + 22) * self.channels as usize
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WaveEntry {
    pub flags: u32,
    /// Duration in samples (28 bits).
    pub duration: u32,
    pub format: MiniFormat,
    /// Absolute byte range in the bank file.
    pub offset: usize,
    pub length: usize,
    pub loop_start: u32,
    pub loop_length: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WaveBank {
    pub name: String,
    pub flags: u32,
    pub entries: Vec<WaveEntry>,
    pub entry_names: Vec<String>,
}

/// Decoded interleaved 16-bit samples.
#[derive(Debug, Clone, PartialEq)]
pub struct Pcm {
    pub channels: u16,
    pub sample_rate: u32,
    pub samples: Vec<i16>,
}
impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }
}

impl WaveBank {
    /// Reconstruct an xWMA RIFF stream from an entry and its native packet
    /// seek table. Mini-format conversion follows Microsoft's DirectXTK
    /// WaveBankReader; no audio data or packet sizes are synthesized.
    pub fn xwma(&self, b: &[u8], index: usize) -> XResult<Vec<u8>> {
        let entry = self
            .entries
            .get(index)
            .ok_or(XactError::Truncated("wave index"))?;
        let f = entry.format;
        if f.codec != WaveCodec::Wma {
            return Err(XactError::UnsupportedCodec(f.codec));
        }
        const ALIGN: [u16; 17] = [
            929, 1487, 1280, 2230, 8917, 8192, 4459, 5945, 2304, 1536, 1485, 1008, 2731, 4096,
            6827, 5462, 1280,
        ];
        const RATE: [u32; 7] = [12000, 24000, 4000, 6000, 8000, 20000, 2500];
        let align = *ALIGN
            .get((f.block_align & 31) as usize)
            .ok_or(XactError::Truncated("WMA alignment index"))?;
        let rate = *RATE
            .get((f.block_align >> 5) as usize)
            .ok_or(XactError::Truncated("WMA rate index"))?;
        let seek_offset = u32_at(b, 28)? as usize;
        let seek_len = u32_at(b, 32)? as usize;
        let seek = b
            .get(
                seek_offset
                    ..seek_offset
                        .checked_add(seek_len)
                        .ok_or(XactError::Truncated("seek table"))?,
            )
            .ok_or(XactError::Truncated("seek table"))?;
        let relative = u32_at(seek, index * 4)?;
        if relative == u32::MAX {
            return Err(XactError::Truncated("missing WMA seek table"));
        }
        let offset = relative as usize + self.entries.len() * 4;
        let count = u32_at(seek, offset)? as usize;
        let packet_bytes = count
            .checked_mul(4)
            .ok_or(XactError::Truncated("packet table"))?;
        let packets = seek
            .get(
                offset + 4
                    ..(offset + 4)
                        .checked_add(packet_bytes)
                        .ok_or(XactError::Truncated("packet table"))?,
            )
            .ok_or(XactError::Truncated("packet table"))?;
        let data = b
            .get(
                entry.offset
                    ..entry
                        .offset
                        .checked_add(entry.length)
                        .ok_or(XactError::Truncated("wave data"))?,
            )
            .ok_or(XactError::Truncated("wave data"))?;
        let mut fmt = Vec::with_capacity(18);
        fmt.extend_from_slice(&(if f.sixteen_bit { 0x162u16 } else { 0x161u16 }).to_le_bytes());
        fmt.extend_from_slice(&f.channels.to_le_bytes());
        fmt.extend_from_slice(&f.sample_rate.to_le_bytes());
        fmt.extend_from_slice(&rate.to_le_bytes());
        fmt.extend_from_slice(&align.to_le_bytes());
        fmt.extend_from_slice(&16u16.to_le_bytes());
        fmt.extend_from_slice(&0u16.to_le_bytes());
        let mut out = b"RIFF\0\0\0\0XWMA".to_vec();
        for (tag, bytes) in [
            (b"fmt ", fmt.as_slice()),
            (b"dpds", packets),
            (b"data", data),
        ] {
            out.extend_from_slice(tag);
            let size = u32::try_from(bytes.len()).map_err(|_| XactError::Truncated("RIFF size"))?;
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(bytes);
            if bytes.len() % 2 != 0 {
                out.push(0);
            }
        }
        let size = u32::try_from(out.len() - 8).map_err(|_| XactError::Truncated("RIFF size"))?;
        out[4..8].copy_from_slice(&size.to_le_bytes());
        Ok(out)
    }

    /// Parse the header, bank data and entry metadata of a v45 bank.
    pub fn parse(b: &[u8]) -> XResult<Self> {
        if b.get(0..4) != Some(b"WBND") {
            return Err(XactError::BadSignature);
        }
        let version = u32_at(b, 4)?;
        if version != 45 {
            return Err(XactError::UnsupportedVersion(version));
        }
        // dwHeaderVersion at 8, then five (offset, length) segments.
        let segment = |i: usize| -> XResult<(usize, usize)> {
            Ok((
                u32_at(b, 12 + 8 * i)? as usize,
                u32_at(b, 16 + 8 * i)? as usize,
            ))
        };
        let (bank, _) = segment(0)?;
        let (meta, _) = segment(1)?;
        let (names, names_len) = segment(3)?;
        let (data, _) = segment(4)?;
        let flags = u32_at(b, bank)?;
        let count = u32_at(b, bank + 4)? as usize;
        let name_bytes = b
            .get(bank + 8..bank + 72)
            .ok_or(XactError::Truncated("bank name"))?;
        let name = String::from_utf8_lossy(name_bytes.split(|&c| c == 0).next().unwrap_or(&[]))
            .into_owned();
        let meta_size = u32_at(b, bank + 72)? as usize;
        let name_size = u32_at(b, bank + 76)? as usize;
        let compact = u32_at(b, bank + 84)?;
        if flags & 0x0002_0000 != 0 {
            return Err(XactError::CompactFormat);
        }
        let _ = compact;
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let e = meta + i * meta_size;
            let flags_duration = u32_at(b, e)?;
            let format = MiniFormat::decode(u32_at(b, e + 4)?);
            let offset = data + u32_at(b, e + 8)? as usize;
            let length = u32_at(b, e + 12)? as usize;
            if b.len() < offset + length {
                return Err(XactError::Truncated("wave data"));
            }
            let (loop_start, loop_length) = if meta_size >= 24 {
                (u32_at(b, e + 16)?, u32_at(b, e + 20)?)
            } else {
                (0, 0)
            };
            entries.push(WaveEntry {
                flags: flags_duration & 0xf,
                duration: flags_duration >> 4,
                format,
                offset,
                length,
                loop_start,
                loop_length,
            });
        }
        let mut entry_names = Vec::new();
        if name_size > 0 && names_len >= name_size * count {
            for i in 0..count {
                let s = &b[names + i * name_size..names + (i + 1) * name_size];
                entry_names.push(
                    String::from_utf8_lossy(s.split(|&c| c == 0).next().unwrap_or(&[]))
                        .into_owned(),
                );
            }
        }
        Ok(Self {
            name,
            flags,
            entries,
            entry_names,
        })
    }

    /// Decode entry `index` of the bank bytes `b` to 16-bit PCM.
    pub fn decode(&self, b: &[u8], index: usize) -> XResult<Pcm> {
        let e = &self.entries[index];
        let data = &b[e.offset..e.offset + e.length];
        let f = e.format;
        let samples = match f.codec {
            WaveCodec::Pcm if f.sixteen_bit => data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| i16::from_le_bytes(*c))
                .collect(),
            WaveCodec::Pcm => data.iter().map(|&v| ((v as i16) - 128) << 8).collect(),
            WaveCodec::Adpcm => decode_ms_adpcm(data, f.channels, f.adpcm_block_bytes())?,
            other => return Err(XactError::UnsupportedCodec(other)),
        };
        Ok(Pcm {
            channels: f.channels,
            sample_rate: f.sample_rate,
            samples,
        })
    }
}

/// MS-ADPCM's seven standard coefficient pairs.
const COEF: [(i32, i32); 7] = [
    (256, 0),
    (512, -256),
    (0, 0),
    (192, 64),
    (240, 0),
    (460, -208),
    (392, -232),
];
const ADAPT: [i32; 16] = [
    230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230,
];

#[derive(Clone, Copy, Default)]
struct AdpcmChannel {
    c1: i32,
    c2: i32,
    delta: i32,
    s1: i32,
    s2: i32,
}
impl AdpcmChannel {
    fn expand(&mut self, nibble: u8) -> i16 {
        let signed = if nibble & 8 != 0 {
            nibble as i32 - 16
        } else {
            nibble as i32
        };
        let predicted = (self.s1 * self.c1 + self.s2 * self.c2) / 256;
        let sample = (predicted + signed * self.delta).clamp(-32768, 32767);
        self.s2 = self.s1;
        self.s1 = sample;
        self.delta = (ADAPT[nibble as usize] * self.delta / 256).max(16);
        sample as i16
    }
}

/// Standard MS-ADPCM block decoding (interleaved output). A trailing
/// partial block is decoded for the nibbles it contains.
pub fn decode_ms_adpcm(data: &[u8], channels: u16, block: usize) -> XResult<Vec<i16>> {
    let ch = channels as usize;
    if ch == 0 || ch > 2 || block < 7 * ch {
        return Err(XactError::BadAdpcm("format"));
    }
    let mut out = Vec::new();
    for chunk in data.chunks(block) {
        if chunk.len() < 7 * ch {
            break;
        }
        let mut state = [AdpcmChannel::default(); 2];
        let mut p = 0;
        for s in state.iter_mut().take(ch) {
            let predictor = chunk[p] as usize;
            p += 1;
            let (c1, c2) = *COEF
                .get(predictor)
                .ok_or(XactError::BadAdpcm("predictor"))?;
            s.c1 = c1;
            s.c2 = c2;
        }
        for s in state.iter_mut().take(ch) {
            s.delta = i16::from_le_bytes([chunk[p], chunk[p + 1]]) as i32;
            p += 2;
        }
        for s in state.iter_mut().take(ch) {
            s.s1 = i16::from_le_bytes([chunk[p], chunk[p + 1]]) as i32;
            p += 2;
        }
        for s in state.iter_mut().take(ch) {
            s.s2 = i16::from_le_bytes([chunk[p], chunk[p + 1]]) as i32;
            p += 2;
        }
        // The two header samples come out oldest first.
        for s in state.iter().take(ch) {
            out.push(s.s2 as i16);
        }
        for s in state.iter().take(ch) {
            out.push(s.s1 as i16);
        }
        let mut c = 0;
        for &byte in &chunk[p..] {
            for nibble in [byte >> 4, byte & 0xf] {
                out.push(state[c].expand(nibble));
                c = (c + 1) % ch;
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Sound banks (`.xsb`, tool version 45 / format 43).

/// XACT volume byte to decibels: the four-parameter curve fitted to the
/// authoring tool's table (0x00 −96, 0x14 −38, 0x5A −12, 0x8F −4, 0xB4 0,
/// 0xBF +1, 0xCA +2, 0xFF +6 dB), as used by MonoGame's XACT reader.
pub fn volume_db(v: u8) -> f32 {
    let (a, b, c, d) = (
        -96.0f64,
        0.432_254_984_608_615,
        80.174_860_029_796_3,
        67.738_521_233_404_7,
    );
    (((a - d) / (1.0 + (v as f64 / c).powf(b))) + d) as f32
}

/// One playable wave reference: (wave bank index in the sound bank's
/// table, track index in that wave bank).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaveRef {
    pub bank: u8,
    pub track: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variation {
    pub wave: WaveRef,
    pub weight: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClipEvent {
    /// Event 1/3/4/6: play one wave (or a variation playlist) with optional
    /// pitch (semitones) and volume (dB) randomisation.
    PlayWave {
        time: f32,
        random_offset: f32,
        loop_count: u8,
        waves: Vec<Variation>,
        /// Variation playlist type (bits 0..3 of the playlist flags).
        variation_type: u8,
        pitch: Option<(f32, f32)>,
        volume: Option<(f32, f32)>,
    },
    /// Any other event (volume/pitch ramps, markers): id and time only.
    Other { id: u8, time: f32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub volume_db: f32,
    pub events: Vec<ClipEvent>,
    /// The track filter from the sound's clip table, when enabled.
    pub filter: Option<ClipFilter>,
    /// The clip table's raw filter word (0 for clips parsed alone).
    pub filter_raw: u32,
}

/// XACT track filter word of a complex sound's clip table (INFERRED
/// layout, as MonoGame's XactSound reads it): low u16 = Q and flags (bit 0
/// enabled, bits 1-2 XAudio2 filter type, the rest Q), high u16 = frequency
/// in Hz. 1390 of the retail clips carry the disabled default 0x03e85dc0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipFilter {
    /// XAudio2 FilterType: 0 low-pass, 1 band-pass, 2 high-pass, 3 notch.
    pub kind: u8,
    /// The Q bits (scale UNKNOWN).
    pub q_raw: u16,
    pub frequency: f32,
}

impl ClipFilter {
    pub fn from_raw(raw: u32) -> Option<Self> {
        let flags = raw as u16;
        (flags & 1 != 0).then_some(Self {
            kind: ((flags >> 1) & 3) as u8,
            q_raw: flags >> 3,
            frequency: (raw >> 16) as f32,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    pub category: u16,
    pub volume_db: f32,
    /// Semitones.
    pub pitch: f32,
    pub priority: u8,
    /// RPC preset codes (XGS references).
    pub rpc_codes: Vec<u32>,
    pub body: SoundBody,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SoundBody {
    Simple(WaveRef),
    Complex(Vec<Clip>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CueSounds {
    Single(Sound),
    /// Variation table entries (sound or wave) with weights.
    Variations {
        table_type: u8,
        entries: Vec<(CueEntry, f32)>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum CueEntry {
    Sound(Sound),
    Wave(WaveRef),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub name: String,
    pub sounds: CueSounds,
    /// Maximum simultaneous instances; 255 = unlimited (simple cues).
    pub instance_limit: u8,
    /// Seconds.
    pub fade_in: f32,
    pub fade_out: f32,
    /// Bits 3.. = `InstanceBehavior`; bits 0-2 are the crossfade type
    /// (UNKNOWN encoding; retail uses 0, 1 and 2).
    pub instance_flags: u8,
}

/// What XACT does when a cue is played at its instance limit (INFERRED:
/// the FAudio/FACT reimplementation reads `flags >> 3`; XACT's authoring
/// order Fail To Play, Queue, Replace Oldest, Replace Quietest, Replace
/// Lowest Priority).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceBehavior {
    FailToPlay,
    Queue,
    ReplaceOldest,
    ReplaceQuietest,
    ReplaceLowestPriority,
}

impl InstanceBehavior {
    pub fn from_flags(flags: u8) -> Self {
        match flags >> 3 {
            0 => Self::FailToPlay,
            1 => Self::Queue,
            2 => Self::ReplaceOldest,
            3 => Self::ReplaceQuietest,
            _ => Self::ReplaceLowestPriority,
        }
    }
}

impl Cue {
    pub fn instance_behavior(&self) -> InstanceBehavior {
        InstanceBehavior::from_flags(self.instance_flags)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SoundBank {
    pub name: String,
    pub wave_banks: Vec<String>,
    pub cues: Vec<Cue>,
}

struct Cursor<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Cursor<'a> {
    fn at(b: &'a [u8], p: usize) -> Self {
        Self { b, p }
    }
    fn u8(&mut self) -> XResult<u8> {
        let v = *self.b.get(self.p).ok_or(XactError::Truncated("u8"))?;
        self.p += 1;
        Ok(v)
    }
    fn u16(&mut self) -> XResult<u16> {
        let v = u16_at(self.b, self.p)?;
        self.p += 2;
        Ok(v)
    }
    fn i16(&mut self) -> XResult<i16> {
        Ok(self.u16()? as i16)
    }
    fn u32(&mut self) -> XResult<u32> {
        let v = u32_at(self.b, self.p)?;
        self.p += 4;
        Ok(v)
    }
    fn f32(&mut self) -> XResult<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn skip(&mut self, n: usize) -> XResult<()> {
        if self.p + n > self.b.len() {
            return Err(XactError::Truncated("skip"));
        }
        self.p += n;
        Ok(())
    }
}

fn fixed_string(b: &[u8], at: usize, len: usize) -> XResult<String> {
    let s = b.get(at..at + len).ok_or(XactError::Truncated("name"))?;
    Ok(String::from_utf8_lossy(s.split(|&c| c == 0).next().unwrap_or(&[])).into_owned())
}

fn playlist(c: &mut Cursor<'_>) -> XResult<(Vec<Variation>, u8)> {
    let count = c.u16()?;
    let flags = c.u8()?;
    c.skip(5)?;
    let mut waves = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let track = c.u16()?;
        let bank = c.u8()?;
        let min = c.u8()?;
        let max = c.u8()?;
        waves.push(Variation {
            wave: WaveRef { bank, track },
            weight: max.saturating_sub(min) as f32,
        });
    }
    Ok((waves, flags & 0x0f))
}

fn parse_clip(b: &[u8], at: usize, clip_db: f32) -> XResult<Clip> {
    let mut c = Cursor::at(b, at);
    let n = c.u8()?;
    let mut events = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let info = c.u32()?;
        let random_offset = c.u16()? as f32 * 0.001;
        let id = (info & 0x1f) as u8;
        let time = ((info >> 5) & 0xffff) as f32 * 0.001;
        let event = match id {
            1 | 3 | 4 | 6 => {
                c.u8()?; // unknown
                c.u8()?; // event flags (release, pan, centre speaker)
                let single = if id == 1 || id == 4 {
                    let track = c.u16()?;
                    let bank = c.u8()?;
                    Some(WaveRef { bank, track })
                } else {
                    None
                };
                let loop_count = c.u8()?;
                c.u16()?; // pan angle
                c.u16()?; // pan arc
                let (mut pitch, mut volume) = (None, None);
                if id == 4 || id == 6 {
                    let min_pitch = c.i16()? as f32 / 1000.0;
                    let max_pitch = c.i16()? as f32 / 1000.0;
                    let min_volume = volume_db(c.u8()?);
                    let max_volume = volume_db(c.u8()?);
                    c.f32()?; // filter frequency min
                    c.f32()?; // filter frequency max
                    c.f32()?; // filter Q min
                    c.f32()?; // filter Q max
                    c.u8()?; // unknown
                    let flags = c.u8()?;
                    if flags & 0x04 != 0 {
                        pitch = Some((min_pitch, max_pitch));
                    }
                    if flags & 0x01 != 0 {
                        volume = Some((min_volume, max_volume));
                    }
                }
                let (waves, variation_type) = match single {
                    Some(wave) => (vec![Variation { wave, weight: 1.0 }], 0),
                    None => playlist(&mut c)?,
                };
                ClipEvent::PlayWave {
                    time,
                    random_offset,
                    loop_count,
                    waves,
                    variation_type,
                    pitch,
                    volume,
                }
            }
            _ => {
                // Unknown event payloads have unknown sizes: keep what was
                // parsed and record the stop point.
                events.push(ClipEvent::Other { id, time });
                return Ok(Clip {
                    volume_db: clip_db,
                    events,
                    filter: None,
                    filter_raw: 0,
                });
            }
        };
        events.push(event);
    }
    Ok(Clip {
        volume_db: clip_db,
        events,
        filter: None,
        filter_raw: 0,
    })
}

fn parse_sound(b: &[u8], at: usize) -> XResult<Sound> {
    let mut c = Cursor::at(b, at);
    let flags = c.u8()?;
    let category = c.u16()?;
    let volume = volume_db(c.u8()?);
    let pitch = c.i16()? as f32 / 1000.0;
    let priority = c.u8()?;
    c.u16()?; // entry length / filter
    let complex = flags & 1 != 0;
    let mut clips = 0;
    let mut simple = None;
    if complex {
        clips = c.u8()?;
    } else {
        let track = c.u16()?;
        let bank = c.u8()?;
        simple = Some(WaveRef { bank, track });
    }
    let mut rpc_codes = Vec::new();
    if flags & 0x0e != 0 {
        let start = c.p;
        let len = c.u16()? as usize;
        let n = c.u8()?;
        for _ in 0..n {
            rpc_codes.push(c.u32()?);
        }
        c.p = start + len;
    }
    if flags & 0x10 != 0 {
        let start = c.p;
        let len = c.u16()? as usize;
        c.p = start + len;
    }
    let body = if complex {
        let mut list = Vec::with_capacity(clips as usize);
        for _ in 0..clips {
            let volume = volume_db(c.u8()?);
            let offset = c.u32()? as usize;
            let filter_raw = c.u32()?;
            list.push(Clip {
                filter: ClipFilter::from_raw(filter_raw),
                filter_raw,
                ..parse_clip(b, offset, volume)?
            });
        }
        SoundBody::Complex(list)
    } else {
        SoundBody::Simple(simple.unwrap())
    };
    Ok(Sound {
        category,
        volume_db: volume,
        pitch,
        priority,
        rpc_codes,
        body,
    })
}

impl SoundBank {
    pub fn parse(b: &[u8]) -> XResult<Self> {
        if b.get(0..4) != Some(b"SDBK") {
            return Err(XactError::BadSignature);
        }
        let tool = u16_at(b, 4)? as u32;
        if tool != 45 {
            return Err(XactError::UnsupportedVersion(tool));
        }
        let mut h = Cursor::at(b, 0x12);
        h.u8()?; // platform
        let simple_count = h.u16()? as usize;
        let complex_count = h.u16()? as usize;
        h.u16()?;
        let total = h.u16()? as usize;
        let wave_bank_count = h.u8()? as usize;
        let _sounds = h.u16()?;
        let _names_len = h.u16()?;
        h.u16()?;
        let simple_at = h.u32()? as usize;
        let complex_at = h.u32()? as usize;
        let names_at = h.u32()? as usize;
        h.u32()?;
        let _variations_at = h.u32()?;
        h.u32()?;
        let wave_bank_names_at = h.u32()? as usize;
        let name = fixed_string(b, 0x4a, 64)?;
        let wave_banks = (0..wave_bank_count)
            .map(|i| fixed_string(b, wave_bank_names_at + 64 * i, 64))
            .collect::<XResult<Vec<_>>>()?;
        let mut names = Vec::with_capacity(total);
        let mut p = names_at;
        for _ in 0..total {
            let end = b[p..]
                .iter()
                .position(|&c| c == 0)
                .ok_or(XactError::Truncated("cue name"))?;
            names.push(String::from_utf8_lossy(&b[p..p + end]).into_owned());
            p += end + 1;
        }
        let mut cues = Vec::with_capacity(total);
        let mut c = Cursor::at(b, simple_at);
        for name in names.iter().take(simple_count) {
            c.u8()?;
            let sound = parse_sound(b, c.u32()? as usize)?;
            cues.push(Cue {
                name: name.clone(),
                sounds: CueSounds::Single(sound),
                instance_limit: 255,
                fade_in: 0.0,
                fade_out: 0.0,
                instance_flags: 0,
            });
        }
        let mut c = Cursor::at(b, complex_at);
        for i in 0..complex_count {
            let flags = c.u8()?;
            let sounds = if (flags >> 2) & 1 != 0 {
                let sound = parse_sound(b, c.u32()? as usize)?;
                c.u32()?;
                CueSounds::Single(sound)
            } else {
                let table = c.u32()? as usize;
                c.u32()?; // transition table
                let mut t = Cursor::at(b, table);
                let count = t.u16()?;
                let vflags = t.u16()?;
                t.skip(4)?;
                let table_type = ((vflags >> 3) & 7) as u8;
                let mut entries = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    entries.push(match table_type {
                        0 => {
                            let track = t.u16()?;
                            let bank = t.u8()?;
                            let min = t.u8()?;
                            let max = t.u8()?;
                            (
                                CueEntry::Wave(WaveRef { bank, track }),
                                max.saturating_sub(min) as f32,
                            )
                        }
                        1 => {
                            let at = t.u32()? as usize;
                            let min = t.u8()?;
                            let max = t.u8()?;
                            (
                                CueEntry::Sound(parse_sound(b, at)?),
                                max.saturating_sub(min) as f32,
                            )
                        }
                        3 => {
                            let at = t.u32()? as usize;
                            let min = t.f32()?;
                            let max = t.f32()?;
                            t.u32()?;
                            (CueEntry::Sound(parse_sound(b, at)?), (max - min).max(0.0))
                        }
                        4 => {
                            let track = t.u16()?;
                            let bank = t.u8()?;
                            (CueEntry::Wave(WaveRef { bank, track }), 1.0)
                        }
                        _ => return Err(XactError::Truncated("variation table type")),
                    });
                }
                CueSounds::Variations {
                    table_type,
                    entries,
                }
            };
            let instance_limit = c.u8()?;
            let fade_in = c.u16()? as f32 / 1000.0;
            let fade_out = c.u16()? as f32 / 1000.0;
            let instance_flags = c.u8()?;
            cues.push(Cue {
                name: names[simple_count + i].clone(),
                sounds,
                instance_limit,
                fade_in,
                fade_out,
                instance_flags,
            });
        }
        Ok(Self {
            name,
            wave_banks,
            cues,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn instance_behavior_is_the_flag_byte_above_the_crossfade_bits() {
        use super::InstanceBehavior as B;
        // Retail values: 0x00/0x02 unlimited or fail, 0x10-0x12 replace
        // oldest, 0x18/0x19 replace quietest.
        assert_eq!(B::from_flags(0x00), B::FailToPlay);
        assert_eq!(B::from_flags(0x02), B::FailToPlay);
        assert_eq!(B::from_flags(0x08), B::Queue);
        assert_eq!(B::from_flags(0x12), B::ReplaceOldest);
        assert_eq!(B::from_flags(0x19), B::ReplaceQuietest);
        assert_eq!(B::from_flags(0x20), B::ReplaceLowestPriority);
    }

    #[test]
    fn volume_curve_hits_the_tool_table() {
        for (byte, db) in [
            (0u8, -96.0f32),
            (0x5a, -12.0),
            (0x8f, -4.0),
            (0xb4, 0.0),
            (0xff, 6.0),
        ] {
            let v = super::volume_db(byte);
            assert!((v - db).abs() < 0.6, "{byte:#x}: {v}");
        }
    }
}

// ---------------------------------------------------------------------------
// Global settings (`.xgs`, tool version 45 / format 42).

#[derive(Debug, Clone, PartialEq)]
pub struct Category {
    pub name: String,
    pub max_instances: u8,
    pub fade_in: f32,
    pub fade_out: f32,
    pub instance_flags: u8,
    pub parent: u16,
    pub volume_db: f32,
    pub visibility: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Variable {
    pub name: String,
    pub flags: u8,
    pub initial: f32,
    pub min: f32,
    pub max: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RpcPoint {
    pub x: f32,
    pub y: f32,
    /// 0 linear, 1 fast, 2 slow, 3 sin/cos.
    pub kind: u8,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RpcCurve {
    /// File offset: the code sounds use to reference it.
    pub code: u32,
    pub variable: u16,
    /// 0 volume (hundredths of dB), 1 pitch (hundredths of a semitone),
    /// 2 reverb send, 3 filter frequency, 4 filter Q; higher values are DSP.
    pub parameter: u16,
    pub points: Vec<RpcPoint>,
}

impl RpcCurve {
    /// Piecewise evaluation between points (clamped at the ends). Point
    /// types other than linear are approximated linearly.
    pub fn evaluate(&self, x: f32) -> f32 {
        let p = &self.points;
        match p.len() {
            0 => 0.0,
            _ if x <= p[0].x => p[0].y,
            n if x >= p[n - 1].x => p[n - 1].y,
            _ => {
                let i = p.windows(2).position(|w| x < w[1].x).unwrap_or(0);
                let (a, b) = (p[i], p[i + 1]);
                let t = (x - a.x) / (b.x - a.x).max(f32::MIN_POSITIVE);
                a.y + (b.y - a.y) * t
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GlobalSettings {
    pub categories: Vec<Category>,
    pub variables: Vec<Variable>,
    pub rpcs: Vec<RpcCurve>,
}

fn names_at(b: &[u8], at: usize, count: usize) -> XResult<Vec<String>> {
    let mut out = Vec::with_capacity(count);
    let mut p = at;
    for _ in 0..count {
        let end = b
            .get(p..)
            .and_then(|s| s.iter().position(|&c| c == 0))
            .ok_or(XactError::Truncated("name"))?;
        out.push(String::from_utf8_lossy(&b[p..p + end]).into_owned());
        p += end + 1;
    }
    Ok(out)
}

impl GlobalSettings {
    pub fn parse(b: &[u8]) -> XResult<Self> {
        if b.get(0..4) != Some(b"XGSF") {
            return Err(XactError::BadSignature);
        }
        let tool = u16_at(b, 4)? as u32;
        if tool != 45 {
            return Err(XactError::UnsupportedVersion(tool));
        }
        let mut h = Cursor::at(b, 0x12);
        h.u8()?; // platform
        let cat_count = h.u16()? as usize;
        let var_count = h.u16()? as usize;
        h.u16()?;
        h.u16()?;
        let rpc_count = h.u16()? as usize;
        h.u16()?; // DSP presets
        h.u16()?; // DSP parameters
        let cats_at = h.u32()? as usize;
        let vars_at = h.u32()? as usize;
        h.u32()?;
        h.u32()?; // category name index
        h.u32()?;
        h.u32()?; // variable name index
        let cat_names_at = h.u32()? as usize;
        let var_names_at = h.u32()? as usize;
        let rpc_at = h.u32()? as usize;
        let cat_names = names_at(b, cat_names_at, cat_count)?;
        let var_names = names_at(b, var_names_at, var_count)?;
        let mut c = Cursor::at(b, cats_at);
        let mut categories = Vec::with_capacity(cat_count);
        for name in cat_names {
            categories.push(Category {
                name,
                max_instances: c.u8()?,
                fade_in: c.u16()? as f32 / 1000.0,
                fade_out: c.u16()? as f32 / 1000.0,
                instance_flags: c.u8()?,
                parent: c.u16()?,
                volume_db: volume_db(c.u8()?),
                visibility: c.u8()?,
            });
        }
        let mut c = Cursor::at(b, vars_at);
        let mut variables = Vec::with_capacity(var_count);
        for name in var_names {
            variables.push(Variable {
                name,
                flags: c.u8()?,
                initial: c.f32()?,
                min: c.f32()?,
                max: c.f32()?,
            });
        }
        let mut c = Cursor::at(b, rpc_at);
        let mut rpcs = Vec::with_capacity(rpc_count);
        for _ in 0..rpc_count {
            let code = c.p as u32;
            let variable = c.u16()?;
            let n = c.u8()?;
            let parameter = c.u16()?;
            let mut points = Vec::with_capacity(n as usize);
            for _ in 0..n {
                points.push(RpcPoint {
                    x: c.f32()?,
                    y: c.f32()?,
                    kind: c.u8()?,
                });
            }
            rpcs.push(RpcCurve {
                code,
                variable,
                parameter,
                points,
            });
        }
        Ok(Self {
            categories,
            variables,
            rpcs,
        })
    }
}
