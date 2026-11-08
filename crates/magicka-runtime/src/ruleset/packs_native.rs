//! PackMan's pack lists, read from the user's `Magicka.exe` at load time.
//!
//! The thirteen ItemPacks and six MagickPacks are compiled into the game's
//! .NET executable (`Magicka.Levels.Packs.PackMan::.ctor`, with the item
//! pools as string arrays filled in `Magicka.DRM.HashTable::.cctor`). This
//! module reads them statically: a minimal ECMA-335 metadata reader and CIL
//! decoder, the same rules as the research extractor
//! `tools/extract_survival_packs.py`. Nothing in the executable is run, and
//! nothing from it is stored in source.
use super::packs::{Pack, PackCatalogue, PackKind, PacksError};

type Result<T> = std::result::Result<T, PacksError>;

fn bad(what: impl Into<String>) -> PacksError {
    PacksError::Executable(what.into())
}

fn u16_at(raw: &[u8], at: usize) -> Result<u16> {
    raw.get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| bad("truncated executable"))
}

fn u32_at(raw: &[u8], at: usize) -> Result<u32> {
    raw.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| bad("truncated executable"))
}

/// ECMA-335 II.24.2.4 compressed unsigned integer: (value, length).
fn compressed(raw: &[u8], at: usize) -> Result<(usize, usize)> {
    let b0 = *raw.get(at).ok_or_else(|| bad("truncated heap"))? as usize;
    let byte = |i: usize| {
        raw.get(at + i)
            .map(|b| *b as usize)
            .ok_or_else(|| bad("truncated heap"))
    };
    Ok(if b0 & 0x80 == 0 {
        (b0, 1)
    } else if b0 & 0xC0 == 0x80 {
        (((b0 & 0x3F) << 8) | byte(1)?, 2)
    } else {
        (
            ((b0 & 0x1F) << 24) | (byte(1)? << 16) | (byte(2)? << 8) | byte(3)?,
            4,
        )
    })
}

// Metadata table numbers used by name.
const TYPE_REF: usize = 0x01;
const TYPE_DEF: usize = 0x02;
const METHOD_DEF: usize = 0x06;
const MEMBER_REF: usize = 0x0A;
const NESTED_CLASS: usize = 0x29;

/// A column of a metadata table (II.22).
#[derive(Clone, Copy)]
enum Col {
    U16,
    U32,
    Str,
    Guid,
    Blob,
    Table(usize),
    Coded(CodedKind),
}

/// Coded index kinds (II.24.2.6): tag bits and the tables they select.
#[derive(Clone, Copy)]
enum CodedKind {
    TypeDefOrRef,
    HasConstant,
    HasCustomAttribute,
    HasFieldMarshal,
    HasDeclSecurity,
    MemberRefParent,
    HasSemantics,
    MethodDefOrRef,
    MemberForwarded,
    Implementation,
    CustomAttributeType,
    ResolutionScope,
    TypeOrMethodDef,
}

impl CodedKind {
    fn layout(self) -> (u32, &'static [usize]) {
        match self {
            CodedKind::TypeDefOrRef => (2, &[0x02, 0x01, 0x1B]),
            CodedKind::HasConstant => (2, &[0x04, 0x08, 0x17]),
            CodedKind::HasCustomAttribute => (
                5,
                &[
                    0x06, 0x04, 0x01, 0x02, 0x08, 0x09, 0x0A, 0x00, 0x0E, 0x17, 0x14, 0x11, 0x1A,
                    0x1B, 0x20, 0x23, 0x26, 0x27, 0x28, 0x2A, 0x2C, 0x2B,
                ],
            ),
            CodedKind::HasFieldMarshal => (1, &[0x04, 0x08]),
            CodedKind::HasDeclSecurity => (2, &[0x02, 0x06, 0x20]),
            CodedKind::MemberRefParent => (3, &[0x02, 0x01, 0x1A, 0x06, 0x1B]),
            CodedKind::HasSemantics => (1, &[0x14, 0x17]),
            CodedKind::MethodDefOrRef => (1, &[0x06, 0x0A]),
            CodedKind::MemberForwarded => (1, &[0x04, 0x06]),
            CodedKind::Implementation => (2, &[0x26, 0x23, 0x27]),
            // Tags 2 and 3 only; the other tags are unused.
            CodedKind::CustomAttributeType => (3, &[0x06, 0x0A]),
            CodedKind::ResolutionScope => (2, &[0x00, 0x1A, 0x23, 0x01]),
            CodedKind::TypeOrMethodDef => (1, &[0x02, 0x06]),
        }
    }

    /// (table, 1-based row) of a coded value.
    fn decode(self, value: u32) -> Option<(usize, usize)> {
        let (bits, _) = self.layout();
        let tag = (value & ((1 << bits) - 1)) as usize;
        let tables: &[usize] = match self {
            CodedKind::MemberRefParent => &[0x02, 0x01, 0x1A, 0x06, 0x1B],
            CodedKind::TypeDefOrRef => &[0x02, 0x01, 0x1B],
            CodedKind::MethodDefOrRef => &[0x06, 0x0A],
            _ => return None,
        };
        tables.get(tag).map(|t| (*t, (value >> bits) as usize))
    }
}

fn schema(table: usize) -> Option<&'static [Col]> {
    use CodedKind::*;
    use Col::*;
    Some(match table {
        0x00 => &[U16, Str, Guid, Guid, Guid],
        0x01 => &[Coded(ResolutionScope), Str, Str],
        0x02 => &[U32, Str, Str, Coded(TypeDefOrRef), Table(0x04), Table(0x06)],
        0x03 => &[Table(0x04)],
        0x04 => &[U16, Str, Blob],
        0x05 => &[Table(0x06)],
        0x06 => &[U32, U16, U16, Str, Blob, Table(0x08)],
        0x07 => &[Table(0x08)],
        0x08 => &[U16, U16, Str],
        0x09 => &[Table(0x02), Coded(TypeDefOrRef)],
        0x0A => &[Coded(MemberRefParent), Str, Blob],
        0x0B => &[U16, Coded(HasConstant), Blob],
        0x0C => &[Coded(HasCustomAttribute), Coded(CustomAttributeType), Blob],
        0x0D => &[Coded(HasFieldMarshal), Blob],
        0x0E => &[U16, Coded(HasDeclSecurity), Blob],
        0x0F => &[U16, U32, Table(0x02)],
        0x10 => &[U32, Table(0x04)],
        0x11 => &[Blob],
        0x12 => &[Table(0x02), Table(0x14)],
        0x13 => &[Table(0x14)],
        0x14 => &[U16, Str, Coded(TypeDefOrRef)],
        0x15 => &[Table(0x02), Table(0x17)],
        0x16 => &[Table(0x17)],
        0x17 => &[U16, Str, Blob],
        0x18 => &[U16, Table(0x06), Coded(HasSemantics)],
        0x19 => &[Table(0x02), Coded(MethodDefOrRef), Coded(MethodDefOrRef)],
        0x1A => &[Str],
        0x1B => &[Blob],
        0x1C => &[U16, Coded(MemberForwarded), Str, Table(0x1A)],
        0x1D => &[U32, Table(0x04)],
        0x1E => &[U32, U32],
        0x1F => &[U32],
        0x20 => &[U32, U16, U16, U16, U16, U32, Blob, Str, Str],
        0x21 => &[U32],
        0x22 => &[U32, U32, U32],
        0x23 => &[U16, U16, U16, U16, U32, Blob, Str, Str, Blob],
        0x24 => &[U32, Table(0x23)],
        0x25 => &[U32, U32, U32, Table(0x23)],
        0x26 => &[U32, Str, Blob],
        0x27 => &[U32, U32, Str, Str, Coded(Implementation)],
        0x28 => &[U32, U32, Str, Coded(Implementation)],
        0x29 => &[Table(0x02), Table(0x02)],
        0x2A => &[U16, U16, Coded(TypeOrMethodDef), Str],
        0x2B => &[Coded(MethodDefOrRef), Blob],
        0x2C => &[Table(0x2A), Coded(TypeDefOrRef)],
        _ => return None,
    })
}

/// A .NET executable's metadata: tables, heaps and section mapping.
struct Assembly<'a> {
    raw: &'a [u8],
    /// (virtual address, virtual size, raw offset, raw size) per section.
    sections: Vec<(u32, u32, u32, u32)>,
    strings: usize,
    user_strings: usize,
    tables: Vec<Vec<Vec<u32>>>,
}

impl<'a> Assembly<'a> {
    fn parse(raw: &'a [u8]) -> Result<Self> {
        if raw.get(..2) != Some(b"MZ") {
            return Err(bad("not a Windows executable"));
        }
        let pe = u32_at(raw, 0x3C)? as usize;
        if raw.get(pe..pe + 4) != Some(b"PE\0\0") {
            return Err(bad("not a PE executable"));
        }
        let coff = pe + 4;
        let sections = u16_at(raw, coff + 2)? as usize;
        let optional = coff + 20;
        let optional_size = u16_at(raw, coff + 16)? as usize;
        let directories = match u16_at(raw, optional)? {
            0x10B => optional + 96,
            0x20B => optional + 112,
            _ => return Err(bad("unknown PE optional header")),
        };
        let mut table = Vec::new();
        let first = optional + optional_size;
        for s in 0..sections {
            let at = first + 40 * s;
            table.push((
                u32_at(raw, at + 12)?,
                u32_at(raw, at + 8)?,
                u32_at(raw, at + 20)?,
                u32_at(raw, at + 16)?,
            ));
        }
        let mut asm = Assembly {
            raw,
            sections: table,
            strings: 0,
            user_strings: 0,
            tables: Vec::new(),
        };
        let clr_rva = u32_at(raw, directories + 14 * 8)?;
        if clr_rva == 0 {
            return Err(bad("not a .NET executable"));
        }
        let clr = asm.offset(clr_rva)?;
        let metadata = asm.offset(u32_at(raw, clr + 8)?)?;
        if raw.get(metadata..metadata + 4) != Some(b"BSJB") {
            return Err(bad("missing .NET metadata"));
        }
        let version_len = u32_at(raw, metadata + 12)? as usize;
        let mut p = metadata + 16 + version_len + 2;
        let streams = u16_at(raw, p)?;
        p += 2;
        let (mut tables_at, mut blob_seen) = (None, false);
        for _ in 0..streams {
            let offset = u32_at(raw, p)? as usize;
            p += 8;
            let end = raw[p..]
                .iter()
                .position(|b| *b == 0)
                .ok_or_else(|| bad("bad stream name"))?;
            let name = &raw[p..p + end];
            p = (p + end + 4) & !3;
            match name {
                b"#~" | b"#-" => tables_at = Some(metadata + offset),
                b"#Strings" => asm.strings = metadata + offset,
                b"#US" => asm.user_strings = metadata + offset,
                b"#Blob" => blob_seen = true,
                _ => {}
            }
        }
        let tables_at = tables_at.ok_or_else(|| bad("missing metadata tables"))?;
        if asm.strings == 0 || asm.user_strings == 0 || !blob_seen {
            return Err(bad("missing metadata heaps"));
        }
        asm.read_tables(tables_at)?;
        Ok(asm)
    }

    fn read_tables(&mut self, at: usize) -> Result<()> {
        let raw = self.raw;
        let heaps = raw[at + 6];
        let valid = u32_at(raw, at + 8)? as u64 | (u32_at(raw, at + 12)? as u64) << 32;
        let mut p = at + 24;
        let mut rows = vec![0usize; 64];
        for (t, count) in rows.iter_mut().enumerate() {
            if valid >> t & 1 == 1 {
                *count = u32_at(raw, p)? as usize;
                p += 4;
            }
        }
        if heaps & 0x40 != 0 {
            p += 4;
        }
        let heap = |bit: u8| if heaps & bit != 0 { 4 } else { 2 };
        let index = |t: usize| if rows[t] < 0x10000 { 2 } else { 4 };
        let width = |col: Col| match col {
            Col::U16 => 2,
            Col::U32 => 4,
            Col::Str => heap(1),
            Col::Guid => heap(2),
            Col::Blob => heap(4),
            Col::Table(t) => index(t),
            Col::Coded(kind) => {
                let (bits, tables) = kind.layout();
                let most = tables.iter().map(|t| rows[*t]).max().unwrap_or(0);
                if most < 1 << (16 - bits) {
                    2
                } else {
                    4
                }
            }
        };
        self.tables = vec![Vec::new(); 64];
        for t in 0..64 {
            if rows[t] == 0 {
                continue;
            }
            let cols = schema(t).ok_or_else(|| bad(format!("unknown metadata table {t:#x}")))?;
            let widths: Vec<usize> = cols.iter().map(|c| width(*c)).collect();
            for _ in 0..rows[t] {
                let mut row = Vec::with_capacity(widths.len());
                for w in &widths {
                    row.push(if *w == 2 {
                        u16_at(raw, p)? as u32
                    } else {
                        u32_at(raw, p)?
                    });
                    p += w;
                }
                self.tables[t].push(row);
            }
        }
        for indirect in [0x03, 0x05] {
            if !self.tables[indirect].is_empty() {
                return Err(bad("unoptimized metadata is not supported"));
            }
        }
        Ok(())
    }

    fn offset(&self, rva: u32) -> Result<usize> {
        self.sections
            .iter()
            .find(|(va, vsize, _, rsize)| rva >= *va && rva < va + (*vsize).max(*rsize))
            .map(|(va, _, raw, _)| (rva - va + raw) as usize)
            .ok_or_else(|| bad(format!("address {rva:#x} outside the executable")))
    }

    fn row(&self, table: usize, index: usize) -> Result<&[u32]> {
        index
            .checked_sub(1)
            .and_then(|i| self.tables[table].get(i))
            .map(Vec::as_slice)
            .ok_or_else(|| bad(format!("bad metadata row {table:#x}/{index}")))
    }

    fn string(&self, index: u32) -> Result<String> {
        let start = self.strings + index as usize;
        let rest = self
            .raw
            .get(start..)
            .ok_or_else(|| bad("bad string index"))?;
        let end = rest
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| bad("bad string"))?;
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    }

    /// A `ldstr` literal from the #US heap.
    fn user_string(&self, token: u32) -> Result<String> {
        let at = self.user_strings + (token & 0xFF_FFFF) as usize;
        let (len, header) = compressed(self.raw, at)?;
        let bytes = self
            .raw
            .get(at + header..at + header + len.saturating_sub(1))
            .ok_or_else(|| bad("bad user string"))?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        Ok(String::from_utf16_lossy(&units))
    }

    /// `Namespace.Name` of a TypeDef, with `/` for nested types.
    fn type_def_name(&self, index: usize) -> Result<String> {
        let row = self.row(TYPE_DEF, index)?;
        let name = self.string(row[1])?;
        if let Some(outer) = self.tables[NESTED_CLASS]
            .iter()
            .find(|r| r[0] as usize == index)
        {
            return Ok(format!("{}/{name}", self.type_def_name(outer[1] as usize)?));
        }
        let namespace = self.string(row[2])?;
        Ok(if namespace.is_empty() {
            name
        } else {
            format!("{namespace}.{name}")
        })
    }

    fn type_ref_name(&self, index: usize) -> Result<String> {
        let row = self.row(TYPE_REF, index)?;
        let (name, namespace) = (self.string(row[1])?, self.string(row[2])?);
        Ok(if namespace.is_empty() {
            name
        } else {
            format!("{namespace}.{name}")
        })
    }

    /// The type a TypeDef/TypeRef token names.
    fn type_name(&self, token: u32) -> Result<String> {
        let index = (token & 0xFF_FFFF) as usize;
        match (token >> 24) as usize {
            TYPE_DEF => self.type_def_name(index),
            TYPE_REF => self.type_ref_name(index),
            _ => Ok(String::new()),
        }
    }

    /// The TypeDef that declares a MethodDef (MethodList ranges).
    fn method_owner(&self, method: usize) -> Result<String> {
        let types = &self.tables[TYPE_DEF];
        let owner = types
            .iter()
            .rposition(|t| t[5] as usize <= method)
            .ok_or_else(|| bad("method without a type"))?;
        self.type_def_name(owner + 1)
    }

    /// `Type::Member` of a MethodDef, MemberRef or Field token.
    fn member_name(&self, token: u32) -> Result<String> {
        let index = (token & 0xFF_FFFF) as usize;
        match (token >> 24) as usize {
            METHOD_DEF => Ok(format!(
                "{}::{}",
                self.method_owner(index)?,
                self.string(self.row(METHOD_DEF, index)?[3])?
            )),
            MEMBER_REF => {
                let row = self.row(MEMBER_REF, index)?;
                let parent = match CodedKind::MemberRefParent.decode(row[0]) {
                    Some((t, i)) if t == TYPE_DEF || t == TYPE_REF => {
                        self.type_name(((t as u32) << 24) | i as u32)?
                    }
                    _ => String::new(),
                };
                Ok(format!("{parent}::{}", self.string(row[1])?))
            }
            _ => Ok(String::new()),
        }
    }

    /// The MethodDef token of `Type::Method` (exactly one overload).
    fn method(&self, owner: &str, name: &str) -> Result<u32> {
        let mut found = None;
        for (i, row) in self.tables[METHOD_DEF].iter().enumerate() {
            if self.string(row[3])? == name && self.method_owner(i + 1)? == owner {
                if found.is_some() {
                    return Err(bad(format!("several {owner}::{name}")));
                }
                found = Some(((METHOD_DEF as u32) << 24) | (i + 1) as u32);
            }
        }
        found.ok_or_else(|| bad(format!("{owner}::{name} not found")))
    }

    /// The decoded body of a MethodDef.
    fn body(&self, token: u32) -> Result<Vec<Instr>> {
        if (token >> 24) as usize != METHOD_DEF {
            return Err(bad("call into another assembly"));
        }
        let rva = self.row(METHOD_DEF, (token & 0xFF_FFFF) as usize)?[0];
        if rva == 0 {
            return Err(bad("method without a body"));
        }
        let at = self.offset(rva)?;
        let first = *self.raw.get(at).ok_or_else(|| bad("bad method body"))?;
        let (code, size) = if first & 3 == 2 {
            (at + 1, (first >> 2) as usize)
        } else {
            let header = u16_at(self.raw, at)?;
            (
                at + (header >> 12) as usize * 4,
                u32_at(self.raw, at + 4)? as usize,
            )
        };
        let code = self
            .raw
            .get(code..code + size)
            .ok_or_else(|| bad("bad method body"))?;
        decode(code)
    }
}

/// One CIL instruction: opcode (0xFExx for two-byte opcodes) and operand.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Instr {
    op: u16,
    arg: Arg,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Arg {
    None,
    Int(i64),
    Token(u32),
    Other,
}

const LDC_I4_M1: u16 = 0x15;
const LDC_I4_0: u16 = 0x16;
const LDC_I4_8: u16 = 0x1E;
const LDC_I4_S: u16 = 0x1F;
const LDC_I4: u16 = 0x20;
const LDSTR: u16 = 0x72;
const NEWOBJ: u16 = 0x73;
const LDSFLD: u16 = 0x7E;
const STSFLD: u16 = 0x80;
const NEWARR: u16 = 0x8D;
const STELEM_I4: u16 = 0x9E;
const STELEM_REF: u16 = 0xA2;

/// Operand size of an opcode (ECMA-335 III); `None` for unknown opcodes.
/// The switch table's length is handled by the decoder.
fn operand_size(op: u16) -> Option<usize> {
    Some(match op {
        0x0E..=0x13 | 0x1F | 0x2B..=0x37 | 0xDE => 1,
        0x20 | 0x22 | 0x38..=0x44 | 0xDD | 0x45 => 4,
        0x21 | 0x23 => 8,
        0x27..=0x29
        | 0x6F..=0x75
        | 0x79
        | 0x7B..=0x81
        | 0x8C
        | 0x8D
        | 0x8F
        | 0xA3..=0xA5
        | 0xC2
        | 0xC6
        | 0xD0 => 4,
        0x00..=0x0D
        | 0x14..=0x1E
        | 0x25
        | 0x26
        | 0x2A
        | 0x46..=0x6E
        | 0x76
        | 0x7A
        | 0x82..=0x8B
        | 0x8E
        | 0x90..=0xA2
        | 0xB3..=0xBA
        | 0xC3
        | 0xD1..=0xDC
        | 0xDF
        | 0xE0 => 0,
        0xFE06 | 0xFE07 | 0xFE15 | 0xFE16 | 0xFE1C => 4,
        0xFE09..=0xFE0E => 2,
        0xFE12 | 0xFE19 => 1,
        0xFE00..=0xFE05
        | 0xFE0F
        | 0xFE11
        | 0xFE13
        | 0xFE14
        | 0xFE17
        | 0xFE18
        | 0xFE1A
        | 0xFE1D
        | 0xFE1E => 0,
        _ => return None,
    })
}

fn is_token(op: u16) -> bool {
    matches!(op, 0x27..=0x29 | 0x6F..=0x75 | 0x79 | 0x7B..=0x81 | 0x8C | 0x8D | 0x8F
        | 0xA3..=0xA5 | 0xC2 | 0xC6 | 0xD0 | 0xFE06 | 0xFE07 | 0xFE15 | 0xFE16 | 0xFE1C)
}

fn decode(code: &[u8]) -> Result<Vec<Instr>> {
    let mut out = Vec::new();
    let mut p = 0;
    while p < code.len() {
        let mut op = code[p] as u16;
        p += 1;
        if op == 0xFE {
            op = 0xFE00 | *code.get(p).ok_or_else(|| bad("truncated opcode"))? as u16;
            p += 1;
        }
        let size = operand_size(op).ok_or_else(|| bad(format!("unknown opcode {op:#x}")))?;
        let operand = code
            .get(p..p + size)
            .ok_or_else(|| bad("truncated operand"))?;
        let arg = match (op, size) {
            (LDC_I4_S, _) => Arg::Int(operand[0] as i8 as i64),
            (LDC_I4, _) => Arg::Int(i32::from_le_bytes(operand.try_into().unwrap()) as i64),
            (LDC_I4_M1, _) => Arg::Int(-1),
            (LDC_I4_0..=LDC_I4_8, _) => Arg::Int((op - LDC_I4_0) as i64),
            (LDSTR, _) => Arg::Token(u32::from_le_bytes(operand.try_into().unwrap())),
            (_, 4) if is_token(op) => Arg::Token(u32::from_le_bytes(operand.try_into().unwrap())),
            (_, 0) => Arg::None,
            _ => Arg::Other,
        };
        p += size;
        if op == 0x45 {
            let count = u32::from_le_bytes(operand.try_into().unwrap()) as usize;
            p += 4 * count;
        }
        out.push(Instr { op, arg });
    }
    Ok(out)
}

fn integer(instr: &Instr) -> Result<i64> {
    match instr.arg {
        Arg::Int(n) if matches!(instr.op, LDC_I4_M1..=LDC_I4) => Ok(n),
        _ => Err(bad("expected an integer constant")),
    }
}

fn index(instr: &Instr, len: usize) -> Result<usize> {
    usize::try_from(integer(instr)?)
        .ok()
        .filter(|i| *i < len)
        .ok_or_else(|| bad("index out of range"))
}

fn token(instr: &Instr) -> Result<u32> {
    match instr.arg {
        Arg::Token(t) => Ok(t),
        _ => Err(bad("expected a token operand")),
    }
}

/// Item template name of a pool path: the file stem, lowercase.
fn item_name(path: &str) -> String {
    let path = path.replace('\\', "/");
    let file = path.rsplit('/').next().unwrap_or(&path);
    let stem = match file.rfind('.') {
        Some(dot) if dot > 0 => &file[..dot],
        _ => file,
    };
    stem.to_ascii_lowercase()
}

/// Read PackMan's packs from the game executable's bytes. Packs of the base
/// game (app ID 0) are enabled by default, as in the research extractor.
pub fn extract(raw: &[u8]) -> Result<PackCatalogue> {
    let asm = Assembly::parse(raw)?;

    // HashTable's static string arrays: `new string[n]` stored in a field,
    // then `field[i] = "..."` element by element.
    let code = asm.body(asm.method("Magicka.DRM.HashTable", ".cctor")?)?;
    let mut arrays: std::collections::HashMap<u32, Vec<Option<String>>> = Default::default();
    for (i, instr) in code.iter().enumerate() {
        if instr.op == STSFLD && i >= 2 && code[i - 1].op == NEWARR {
            if asm.type_name(token(&code[i - 1])?)? == "System.String" {
                let len = usize::try_from(integer(&code[i - 2])?).map_err(|_| bad("array size"))?;
                arrays.insert(token(instr)?, vec![None; len]);
            }
        }
        if instr.op == STELEM_REF && i >= 3 && code[i - 3].op == LDSFLD && code[i - 1].op == LDSTR {
            if let Some(values) = arrays.get_mut(&token(&code[i - 3])?) {
                let at = index(&code[i - 2], values.len())?;
                values[at] = Some(asm.user_string(token(&code[i - 1])?)?);
            }
        }
    }

    let mut packs = Vec::new();
    let (mut items, mut magicks) = (0, 0);
    let mut segment: Vec<Instr> = Vec::new();
    for instr in asm.body(asm.method("Magicka.Levels.Packs.PackMan", ".ctor")?)? {
        segment.push(instr);
        if instr.op != NEWOBJ {
            continue;
        }
        let kind = match asm.member_name(token(&instr)?)?.as_str() {
            "Magicka.Levels.Packs.ItemPack::.ctor" => PackKind::Item,
            "Magicka.Levels.Packs.MagickPack::.ctor" => PackKind::Magick,
            _ => continue,
        };
        let seg = std::mem::take(&mut segment);
        // The constructor also loads the thumbnail texture; the label is a key.
        let mut name = None;
        for s in seg.iter().filter(|s| s.op == LDSTR) {
            let text = asm.user_string(token(s)?)?;
            if text.starts_with('#') {
                name = Some(text);
                break;
            }
        }
        let name = name.ok_or_else(|| bad("pack without a label"))?;
        let at = |back: usize| {
            seg.len()
                .checked_sub(back)
                .map(|i| &seg[i])
                .ok_or_else(|| bad("short pack"))
        };
        let (app, pack_items, pack_magicks) = match kind {
            PackKind::Item => {
                // `new ItemPack(name, app, HashTable.GetX(), ...)`: the pool getter
                // returns one of the HashTable arrays.
                let getter = asm.body(token(at(2)?)?)?;
                let field = getter
                    .iter()
                    .find(|g| g.op == LDSFLD)
                    .ok_or_else(|| bad("item pool getter without a field"))?;
                let values = arrays
                    .get(&token(field)?)
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| bad("unknown item pool"))?;
                let names = values
                    .iter()
                    .map(|v| v.as_deref().map(item_name))
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| bad("incomplete item pool"))?;
                (integer(at(3)?)?, names, Vec::new())
            }
            PackKind::Magick => {
                let start = seg
                    .iter()
                    .position(|s| {
                        s.op == NEWARR
                            && token(s)
                                .and_then(|t| asm.type_name(t))
                                .is_ok_and(|n| n == "Magicka.GameLogic.Spells.MagickType")
                    })
                    .ok_or_else(|| bad("magick pack without a MagickType array"))?;
                if start < 2 {
                    return Err(bad("short magick pack"));
                }
                let len =
                    usize::try_from(integer(&seg[start - 1])?).map_err(|_| bad("array size"))?;
                let mut values = vec![None; len];
                for i in start + 1..seg.len() {
                    if seg[i].op == STELEM_I4 {
                        let slot = index(&seg[i - 2], len)?;
                        values[slot] = Some(
                            u8::try_from(integer(&seg[i - 1])?).map_err(|_| bad("magick value"))?,
                        );
                    }
                }
                let values = values
                    .into_iter()
                    .collect::<Option<Vec<_>>>()
                    .filter(|v| !v.is_empty())
                    .ok_or_else(|| bad("incomplete magick pool"))?;
                (integer(&seg[start - 2])?, Vec::new(), values)
            }
        };
        // The UI/Menu/PackThumbs cell: the two integers before `new Point(x, y)`.
        let point = seg
            .iter()
            .position(|s| {
                s.op == NEWOBJ
                    && token(s)
                        .and_then(|t| asm.member_name(t))
                        .is_ok_and(|n| n.ends_with("Microsoft.Xna.Framework.Point::.ctor"))
            })
            .filter(|p| *p >= 2)
            .ok_or_else(|| bad("pack without a thumbnail cell"))?;
        let cell =
            |i: usize| integer(&seg[i]).and_then(|n| u32::try_from(n).map_err(|_| bad("thumb")));
        let app = u32::try_from(app).map_err(|_| bad("pack app"))?;
        match kind {
            PackKind::Item => items += 1,
            PackKind::Magick => magicks += 1,
        }
        packs.push(Pack {
            kind,
            name: super::packs::pack_key(&name),
            app,
            thumb: [cell(point - 2)?, cell(point - 1)?],
            items: pack_items,
            magicks: pack_magicks,
            default_enabled: app == 0,
        });
    }
    if items != 13 || magicks != 6 {
        return Err(bad(format!(
            "expected 13 item and 6 magick packs, found {items} and {magicks}"
        )));
    }
    // AllPacks order: item packs, then magick packs.
    packs.sort_by_key(|p| p.kind == PackKind::Magick);
    Ok(PackCatalogue { packs })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_names_are_lowercase_file_stems() {
        assert_eq!(
            item_name(r"Data\Items\Wizard\Staff_Gnarled.xml"),
            "staff_gnarled"
        );
        assert_eq!(item_name("Data/Items/Wizard/sword"), "sword");
        assert_eq!(item_name("a.b.c"), "a.b");
    }

    #[test]
    fn decoder_knows_operand_sizes_and_constants() {
        // ldc.i4.s -2; ldc.i4 300; ldc.i4.m1; ldc.i4.7; switch(2) [..]; ldstr tok; ceq; ret
        let code = [
            0x1F, 0xFE, 0x20, 0x2C, 0x01, 0x00, 0x00, 0x15, 0x1D, 0x45, 2, 0, 0, 0, 1, 0, 0, 0, 2,
            0, 0, 0, 0x72, 1, 0, 0, 0x70, 0xFE, 0x01, 0x2A,
        ];
        let out = decode(&code).unwrap();
        let ints: Vec<_> = out.iter().filter_map(|i| integer(i).ok()).collect();
        assert_eq!(ints, [-2, 300, -1, 7]);
        assert_eq!(
            out[5],
            Instr {
                op: LDSTR,
                arg: Arg::Token(0x7000_0001)
            }
        );
        assert_eq!(out[6].op, 0xFE01);
        assert_eq!(out.len(), 8);
        assert!(decode(&[0x24]).is_err());
        assert!(decode(&[0x20, 1]).is_err());
    }

    /// Against the user's game: the research extractor's JSON when present
    /// (exact match), otherwise PackMan's fixed shape.
    #[test]
    fn reads_the_installed_games_packs() {
        let content = magicka_content::retail_content_dir();
        let Ok(raw) = std::fs::read(content.join("../Magicka.exe")) else {
            eprintln!("SKIP: Magicka.exe not present");
            return;
        };
        let native = extract(&raw).unwrap();
        let kinds: Vec<_> = native.packs.iter().map(|p| p.kind).collect();
        assert_eq!(kinds.iter().filter(|k| **k == PackKind::Item).count(), 13);
        assert!(kinds
            .windows(2)
            .all(|w| !(w[0] == PackKind::Magick && w[1] == PackKind::Item)));
        for pack in &native.packs {
            assert!(pack.name.starts_with('#'), "{}", pack.name);
            assert_eq!(pack.name, pack.name.to_ascii_lowercase());
            assert_eq!(pack.default_enabled, pack.app == 0);
            match pack.kind {
                PackKind::Item => assert!(!pack.items.is_empty() && pack.magicks.is_empty()),
                PackKind::Magick => assert!(pack.items.is_empty() && !pack.magicks.is_empty()),
            }
        }
        let mut names: Vec<_> = native.packs.iter().map(|p| &p.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 19);
        let pools = native.pools(&native.default_enabled());
        assert_eq!((pools.items.len(), pools.magicks.len()), (47, 26));
        let reference = PackCatalogue::default_cache(&content);
        match std::fs::read(&reference) {
            Ok(json) => assert_eq!(native, PackCatalogue::parse(&json).unwrap()),
            Err(_) => eprintln!("no research extractor output at {}", reference.display()),
        }
    }

    #[test]
    fn rejects_files_that_are_not_dotnet_executables() {
        assert!(matches!(
            extract(b"not an exe"),
            Err(PacksError::Executable(_))
        ));
        assert!(extract(&[]).is_err());
    }
}
