//! KIR1 container (experimental).
//!
//! This module provides a minimal implementation for Stage 1:
//! - File header + section table
//! - STRINGS section (interned UTF-8)
//! - IR section (IrModule encoded with interned strings)
//!
//! Policy:
//! - This is a proposal implementation; format may change.
//! - Do not use for long-term stable artifacts yet.

use crate::ast;
use crate::ir::{CastTarget, IrCaseArm, IrExpr, IrItem, IrLiteral, IrModule, IrPattern};
use crate::types::{Constraint, Scheme, Ty};

const MAGIC: [u8; 4] = *b"KIR1";
const VERSION_MAJOR: u16 = 0;
const VERSION_MINOR: u16 = 1;

const SECTION_STRINGS: u32 = 1;
const SECTION_INTERFACE: u32 = 4;
const SECTION_IR: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KsifModule {
    pub module_name: String,
    /// Exported value schemes: name -> Scheme
    pub values: Vec<(String, Scheme)>,
    /// Dependency manifest: (module_name, sha256_hex_hash_of_ksif_bytes)
    /// Used to detect when dependencies have changed and trigger rebuilds.
    pub dependencies: Vec<(String, String)>,
}

pub fn encode_ksif_module(module: &KsifModule) -> Vec<u8> {
    let mut interner = StringInterner::default();
    interner.intern(&module.module_name);
    for (name, scheme) in &module.values {
        interner.intern(name);
        collect_strings_scheme(scheme, &mut interner);
    }
    for (dep_name, _hash) in &module.dependencies {
        interner.intern(dep_name);
    }

    let strings_payload = encode_strings_section(&interner);
    let interface_payload = encode_interface_section(module, &interner);

    let header_len = 4 + 2 + 2 + 4 + 8 + 8 + 4;
    let crc_len = 4;
    let file_header_len = header_len + crc_len;

    let section_count: u32 = 2;
    let section_entry_len = 4 + 8 + 8;
    let section_table_len = (section_count as usize) * section_entry_len;

    let strings_off = (file_header_len + section_table_len) as u64;
    let interface_off = strings_off + strings_payload.len() as u64;

    let sections = vec![
        SectionEntry {
            section_id: SECTION_STRINGS,
            offset: strings_off,
            length: strings_payload.len() as u64,
        },
        SectionEntry {
            section_id: SECTION_INTERFACE,
            offset: interface_off,
            length: interface_payload.len() as u64,
        },
    ];

    let file_len = (interface_off + interface_payload.len() as u64) as u64;
    let mut out = Vec::with_capacity(file_len as usize);

    out.extend_from_slice(&MAGIC);
    write_u16(&mut out, VERSION_MAJOR);
    write_u16(&mut out, VERSION_MINOR);
    write_u32(&mut out, 0);
    write_u64(&mut out, file_len);
    write_u64(&mut out, file_header_len as u64);
    write_u32(&mut out, section_count);
    write_u32(&mut out, 0);

    for s in &sections {
        write_u32(&mut out, s.section_id);
        write_u64(&mut out, s.offset);
        write_u64(&mut out, s.length);
    }

    out.extend_from_slice(&strings_payload);
    out.extend_from_slice(&interface_payload);

    debug_assert_eq!(out.len() as u64, file_len);
    out
}

pub fn decode_ksif_module(mut input: &[u8]) -> Kir1Result<KsifModule> {
    let file = input;

    if input.len() < 4 {
        return Err(Kir1Error::Msg("unexpected EOF".to_string()));
    }
    if input[..4] != MAGIC {
        return Err(Kir1Error::Msg("invalid magic".to_string()));
    }
    input = &input[4..];

    let major = read_u16(&mut input)?;
    let minor = read_u16(&mut input)?;
    if major != VERSION_MAJOR {
        return Err(Kir1Error::Msg(format!("unsupported KIR1 major: {major}")));
    }
    if minor != VERSION_MINOR {
        return Err(Kir1Error::Msg(format!("unsupported KIR1 minor: {minor}")));
    }
    let _flags = read_u32(&mut input)?;
    let file_len = read_u64(&mut input)?;
    let section_table_off = read_u64(&mut input)?;
    let section_count = read_u32(&mut input)?;
    let _header_crc = read_u32(&mut input)?;

    if file_len == 0 {
        return Err(Kir1Error::Msg("invalid file_len".to_string()));
    }
    if file_len as usize != file.len() {
        return Err(Kir1Error::Msg("file_len mismatch".to_string()));
    }
    if section_table_off as usize > file.len() {
        return Err(Kir1Error::Msg("section_table_off out of range".to_string()));
    }

    let section_entry_len = 4 + 8 + 8;
    let needed = (section_count as usize)
        .checked_mul(section_entry_len)
        .ok_or_else(|| Kir1Error::Msg("section table too large".to_string()))?;
    let start = section_table_off as usize;
    let end = start
        .checked_add(needed)
        .ok_or_else(|| Kir1Error::Msg("section table out of range".to_string()))?;
    if end > file.len() {
        return Err(Kir1Error::Msg("section table out of range".to_string()));
    }

    let mut sec_input = &file[start..end];
    let mut sections = Vec::with_capacity(section_count as usize);
    for _ in 0..section_count {
        let section_id = read_u32(&mut sec_input)?;
        let offset = read_u64(&mut sec_input)?;
        let length = read_u64(&mut sec_input)?;
        sections.push(SectionEntry {
            section_id,
            offset,
            length,
        });
    }

    let strings_entry = sections
        .iter()
        .find(|s| s.section_id == SECTION_STRINGS)
        .ok_or_else(|| Kir1Error::Msg("missing STRINGS section".to_string()))?
        .clone();
    let interface_entry = sections
        .iter()
        .find(|s| s.section_id == SECTION_INTERFACE)
        .ok_or_else(|| Kir1Error::Msg("missing INTERFACE section".to_string()))?
        .clone();

    let mut interner = decode_strings_section(slice_section(file, &strings_entry)?)?;
    let ksif = decode_interface_section(slice_section(file, &interface_entry)?, &mut interner)?;
    Ok(ksif)
}

#[derive(Debug)]
pub enum Kir1Error {
    Msg(String),
}

impl From<String> for Kir1Error {
    fn from(value: String) -> Self {
        Kir1Error::Msg(value)
    }
}

type Kir1Result<T> = Result<T, Kir1Error>;

#[derive(Debug, Clone)]
struct SectionEntry {
    section_id: u32,
    offset: u64,
    length: u64,
}

#[derive(Default)]
struct StringInterner {
    strings: Vec<String>,
    index: std::collections::HashMap<String, u32>,
}

impl StringInterner {
    fn intern(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.index.get(s) {
            return id;
        }
        let id = self.strings.len() as u32;
        self.strings.push(s.to_string());
        self.index.insert(s.to_string(), id);
        id
    }

    fn get(&self, id: u32) -> Kir1Result<&str> {
        self.strings
            .get(id as usize)
            .map(|s| s.as_str())
            .ok_or_else(|| Kir1Error::Msg(format!("invalid StringId: {id}")))
    }
}

pub fn encode_kir1_module(module: &IrModule) -> Vec<u8> {
    // Stage 1: compute string table by walking IR.
    let mut interner = StringInterner::default();
    collect_strings_module(module, &mut interner);

    let strings_payload = encode_strings_section(&interner);
    let ir_payload = encode_ir_section(module, &interner);

    // Layout: header (fixed 32 bytes) + section table + payloads.
    // Header is written first with placeholders, then section table, then payloads.
    let header_len = 4 + 2 + 2 + 4 + 8 + 8 + 4; // without crc for now
    let crc_len = 4;
    let file_header_len = header_len + crc_len;

    let section_count: u32 = 2;
    let section_entry_len = 4 + 8 + 8; // id, offset, length
    let section_table_len = (section_count as usize) * section_entry_len;

    let strings_off = (file_header_len + section_table_len) as u64;
    let ir_off = strings_off + strings_payload.len() as u64;

    let sections = vec![
        SectionEntry {
            section_id: SECTION_STRINGS,
            offset: strings_off,
            length: strings_payload.len() as u64,
        },
        SectionEntry {
            section_id: SECTION_IR,
            offset: ir_off,
            length: ir_payload.len() as u64,
        },
    ];

    let file_len = (ir_off + ir_payload.len() as u64) as u64;

    let mut out = Vec::with_capacity(file_len as usize);

    // Header
    out.extend_from_slice(&MAGIC);
    write_u16(&mut out, VERSION_MAJOR);
    write_u16(&mut out, VERSION_MINOR);
    write_u32(&mut out, 0); // flags
    write_u64(&mut out, file_len);
    write_u64(&mut out, file_header_len as u64); // section_table_off
    write_u32(&mut out, section_count);

    // header_crc32 placeholder (Stage 1: 0)
    write_u32(&mut out, 0);

    // Section table (minimal)
    for s in &sections {
        write_u32(&mut out, s.section_id);
        write_u64(&mut out, s.offset);
        write_u64(&mut out, s.length);
    }

    // Payloads
    out.extend_from_slice(&strings_payload);
    out.extend_from_slice(&ir_payload);

    out
}

pub fn decode_kir1_module(mut input: &[u8]) -> Kir1Result<IrModule> {
    let file = input;

    // Header
    let magic = read_bytes_4(&mut input)?;
    if magic != MAGIC {
        return Err(Kir1Error::Msg("invalid magic".to_string()));
    }
    let major = read_u16(&mut input)?;
    let minor = read_u16(&mut input)?;
    if major != VERSION_MAJOR {
        return Err(Kir1Error::Msg(format!("unsupported KIR1 major: {major}")));
    }
    if minor > VERSION_MINOR {
        return Err(Kir1Error::Msg(format!("unsupported KIR1 minor: {minor}")));
    }
    let _flags = read_u32(&mut input)?;
    let file_len = read_u64(&mut input)?;
    let section_table_off = read_u64(&mut input)?;
    let section_count = read_u32(&mut input)?;
    let _crc = read_u32(&mut input)?;

    if file_len as usize == 0 {
        return Err(Kir1Error::Msg("invalid file_len".to_string()));
    }

    if (file_len as usize) != file.len() {
        return Err(Kir1Error::Msg("file_len mismatch".to_string()));
    }

    if section_table_off as usize > file.len() {
        return Err(Kir1Error::Msg("section_table_off out of range".to_string()));
    }

    // Parse section table directly from `file`.
    let mut st = &file[section_table_off as usize..];
    let mut sections = Vec::with_capacity(section_count as usize);
    for _ in 0..section_count {
        let id = read_u32(&mut st)?;
        let off = read_u64(&mut st)?;
        let len = read_u64(&mut st)?;
        sections.push(SectionEntry {
            section_id: id,
            offset: off,
            length: len,
        });
    }

    let strings_entry = sections
        .iter()
        .find(|s| s.section_id == SECTION_STRINGS)
        .ok_or_else(|| Kir1Error::Msg("missing STRINGS section".to_string()))?
        .clone();
    let ir_entry = sections
        .iter()
        .find(|s| s.section_id == SECTION_IR)
        .ok_or_else(|| Kir1Error::Msg("missing IR section".to_string()))?
        .clone();

    let strings_payload = slice_section(file, &strings_entry)?;
    let mut interner = decode_strings_section(strings_payload)?;

    let ir_payload = slice_section(file, &ir_entry)?;
    let module = decode_ir_section(ir_payload, &mut interner)?;

    Ok(module)
}

fn slice_section<'a>(file: &'a [u8], s: &SectionEntry) -> Kir1Result<&'a [u8]> {
    let off = s.offset as usize;
    let len = s.length as usize;
    file.get(off..off + len)
        .ok_or_else(|| Kir1Error::Msg(format!("section out of range: {}", s.section_id)))
}

fn collect_strings_module(module: &IrModule, interner: &mut StringInterner) {
    for it in &module.items {
        match it {
            IrItem::Binding { name, expr } => {
                interner.intern(name);
                collect_strings_expr(expr, interner);
            }
        }
    }
}

fn collect_strings_scheme(scheme: &Scheme, interner: &mut StringInterner) {
    collect_strings_ty(&scheme.ty, interner);
    for c in &scheme.constraints {
        collect_strings_constraint(c, interner);
    }
}

fn collect_strings_ty(ty: &Ty, interner: &mut StringInterner) {
    match ty {
        Ty::Var(_) => {}
        Ty::Con(name) => {
            interner.intern(name);
        }
        Ty::List(t) => collect_strings_ty(t, interner),
        Ty::Tuple(ts) => ts.iter().for_each(|t| collect_strings_ty(t, interner)),
        Ty::Record(fields) => {
            for (k, t) in fields {
                interner.intern(k);
                collect_strings_ty(t, interner);
            }
        }
        Ty::RecordOpen(fields, rest) => {
            for (k, t) in fields {
                interner.intern(k);
                collect_strings_ty(t, interner);
            }
            collect_strings_ty(rest, interner);
        }
        Ty::App { head, args } => {
            collect_strings_ty(head, interner);
            for a in args {
                collect_strings_ty(a, interner);
            }
        }
        Ty::Func(a, b) => {
            collect_strings_ty(a, interner);
            collect_strings_ty(b, interner);
        }
    }
}

fn collect_strings_constraint(c: &Constraint, interner: &mut StringInterner) {
    match c {
        Constraint::Show(t) | Constraint::ShowRow(t) | Constraint::Eq(t) | Constraint::EqRow(t) => {
            collect_strings_ty(t, interner)
        }
        Constraint::Class { class, ty } => {
            interner.intern(&class.name);
            collect_strings_ty(ty, interner);
        }
        Constraint::Lacks { label, row } => {
            interner.intern(label);
            collect_strings_ty(row, interner);
        }
    }
}

fn encode_interface_section(module: &KsifModule, interner: &StringInterner) -> Vec<u8> {
    // INTERFACE payload format (v0.1):
    // - module_name: StringId
    // - value_count: varu32
    // - repeated value entries:
    //   - name: StringId
    //   - scheme: Scheme
    // - optional deps block:
    //   - marker: [u8; 4] = "DEPS"
    //   - dep_count: varu32
    //   - repeated dependency entries:
    //     - module_name: StringId
    //     - hash_len: varu32
    //     - hash_bytes: [u8; hash_len]
    //
    // The marker avoids accidentally interpreting trailing bytes as deps.
    let mut out = Vec::new();
    let module_name_id = interner
        .index
        .get(&module.module_name)
        .copied()
        .expect("module_name must be interned");
    write_varu32(&mut out, module_name_id);
    write_varu32(&mut out, module.values.len() as u32);
    for (name, scheme) in &module.values {
        let name_id = interner
            .index
            .get(name)
            .copied()
            .expect("export name must be interned");
        write_varu32(&mut out, name_id);
        encode_scheme(&mut out, scheme, interner);
    }
    // Encode dependencies (tagged)
    out.extend_from_slice(b"DEPS");
    write_varu32(&mut out, module.dependencies.len() as u32);
    for (dep_name, hash_hex) in &module.dependencies {
        let dep_name_id = interner
            .index
            .get(dep_name)
            .copied()
            .expect("dep name must be interned");
        write_varu32(&mut out, dep_name_id);
        // Store hash as hex string (simpler, more debuggable)
        let hash_bytes = hash_hex.as_bytes();
        write_varu32(&mut out, hash_bytes.len() as u32);
        out.extend_from_slice(hash_bytes);
    }
    out
}

fn decode_interface_section(
    mut input: &[u8],
    interner: &mut StringInterner,
) -> Kir1Result<KsifModule> {
    let module_name_id = read_varu32(&mut input)?;
    let module_name = interner.get(module_name_id)?.to_string();
    let value_count = read_varu32(&mut input)? as usize;
    let mut values = Vec::with_capacity(value_count);
    for _ in 0..value_count {
        let name_id = read_varu32(&mut input)?;
        let name = interner.get(name_id)?.to_string();
        let scheme = decode_scheme(&mut input, interner)?;
        values.push((name, scheme));
    }
    // Decode dependencies (tagged; may not exist in older .ksif files)
    let dependencies = if input.is_empty() {
        Vec::new()
    } else if input.starts_with(b"DEPS") {
        input = &input[4..];
        let dep_count = read_varu32(&mut input)? as usize;
        let mut deps = Vec::with_capacity(dep_count);
        for _ in 0..dep_count {
            let dep_name_id = read_varu32(&mut input)?;
            let dep_name = interner.get(dep_name_id)?.to_string();
            let hash_len = read_varu32(&mut input)? as usize;
            if input.len() < hash_len {
                return Err(Kir1Error::Msg("unexpected EOF reading hash".to_string()));
            }
            let hash_bytes = &input[..hash_len];
            let hash_hex = String::from_utf8(hash_bytes.to_vec())
                .map_err(|_| Kir1Error::Msg("invalid UTF-8 in hash".to_string()))?;
            input = &input[hash_len..];
            deps.push((dep_name, hash_hex));
        }
        if !input.is_empty() {
            return Err(Kir1Error::Msg("trailing bytes in INTERFACE".to_string()));
        }
        deps
    } else {
        return Err(Kir1Error::Msg(
            "unsupported INTERFACE trailing bytes (missing DEPS marker)".to_string(),
        ));
    };
    Ok(KsifModule {
        module_name,
        values,
        dependencies,
    })
}

fn encode_scheme(out: &mut Vec<u8>, scheme: &Scheme, interner: &StringInterner) {
    write_varu32(out, scheme.vars.len() as u32);
    for v in &scheme.vars {
        write_varu32(out, *v);
    }
    write_varu32(out, scheme.constraints.len() as u32);
    for c in &scheme.constraints {
        encode_constraint(out, c, interner);
    }
    encode_ty(out, &scheme.ty, interner);
}

fn decode_scheme(input: &mut &[u8], interner: &mut StringInterner) -> Kir1Result<Scheme> {
    let vars_len = read_varu32(input)? as usize;
    let mut vars = Vec::with_capacity(vars_len);
    for _ in 0..vars_len {
        vars.push(read_varu32(input)?);
    }
    let c_len = read_varu32(input)? as usize;
    let mut constraints = Vec::with_capacity(c_len);
    for _ in 0..c_len {
        constraints.push(decode_constraint(input, interner)?);
    }
    let ty = decode_ty(input, interner)?;
    Ok(Scheme {
        vars,
        constraints,
        ty,
    })
}

fn encode_constraint(out: &mut Vec<u8>, c: &Constraint, interner: &StringInterner) {
    match c {
        Constraint::Show(t) => {
            write_u8(out, 1);
            encode_ty(out, t, interner);
        }
        Constraint::ShowRow(t) => {
            write_u8(out, 2);
            encode_ty(out, t, interner);
        }
        Constraint::Eq(t) => {
            write_u8(out, 3);
            encode_ty(out, t, interner);
        }
        Constraint::EqRow(t) => {
            write_u8(out, 4);
            encode_ty(out, t, interner);
        }
        Constraint::Class { class, ty } => {
            write_u8(out, 5);
            let id = interner
                .index
                .get(&class.name)
                .copied()
                .expect("constraint class name must be interned");
            write_varu32(out, id);
            encode_ty(out, ty, interner);
        }
        Constraint::Lacks { label, row } => {
            write_u8(out, 6);
            let id = interner
                .index
                .get(label)
                .copied()
                .expect("constraint label must be interned");
            write_varu32(out, id);
            encode_ty(out, row, interner);
        }
    }
}

fn decode_constraint(input: &mut &[u8], interner: &mut StringInterner) -> Kir1Result<Constraint> {
    let tag = read_u8(input)?;
    match tag {
        1 => Ok(Constraint::Show(decode_ty(input, interner)?)),
        2 => Ok(Constraint::ShowRow(decode_ty(input, interner)?)),
        3 => Ok(Constraint::Eq(decode_ty(input, interner)?)),
        4 => Ok(Constraint::EqRow(decode_ty(input, interner)?)),
        5 => {
            let class_id = read_varu32(input)?;
            let class = interner.get(class_id)?.to_string();
            let ty = decode_ty(input, interner)?;
            Ok(Constraint::Class {
                class: ast::ClassId::dummy(class),
                ty,
            })
        }
        6 => {
            let label_id = read_varu32(input)?;
            let label = interner.get(label_id)?.to_string();
            let row = decode_ty(input, interner)?;
            Ok(Constraint::Lacks { label, row })
        }
        other => Err(Kir1Error::Msg(format!("unknown Constraint tag: {other}"))),
    }
}

fn encode_ty(out: &mut Vec<u8>, ty: &Ty, interner: &StringInterner) {
    match ty {
        Ty::Var(v) => {
            write_u8(out, 1);
            write_varu32(out, *v);
        }
        Ty::Con(name) => {
            write_u8(out, 2);
            let id = interner
                .index
                .get(name)
                .copied()
                .expect("Ty::Con name must be interned");
            write_varu32(out, id);
        }
        Ty::List(t) => {
            write_u8(out, 3);
            encode_ty(out, t, interner);
        }
        Ty::Tuple(ts) => {
            write_u8(out, 4);
            write_varu32(out, ts.len() as u32);
            for t in ts {
                encode_ty(out, t, interner);
            }
        }
        Ty::Record(fields) => {
            write_u8(out, 5);
            write_varu32(out, fields.len() as u32);
            for (k, t) in fields {
                let kid = interner
                    .index
                    .get(k)
                    .copied()
                    .expect("record label must be interned");
                write_varu32(out, kid);
                encode_ty(out, t, interner);
            }
        }
        Ty::RecordOpen(fields, rest) => {
            write_u8(out, 6);
            write_varu32(out, fields.len() as u32);
            for (k, t) in fields {
                let kid = interner
                    .index
                    .get(k)
                    .copied()
                    .expect("record label must be interned");
                write_varu32(out, kid);
                encode_ty(out, t, interner);
            }
            encode_ty(out, rest, interner);
        }
        Ty::App { head, args } => {
            write_u8(out, 7);
            encode_ty(out, head, interner);
            write_varu32(out, args.len() as u32);
            for a in args {
                encode_ty(out, a, interner);
            }
        }
        Ty::Func(a, b) => {
            write_u8(out, 8);
            encode_ty(out, a, interner);
            encode_ty(out, b, interner);
        }
    }
}

fn decode_ty(input: &mut &[u8], interner: &mut StringInterner) -> Kir1Result<Ty> {
    let tag = read_u8(input)?;
    match tag {
        1 => Ok(Ty::Var(read_varu32(input)?)),
        2 => {
            let id = read_varu32(input)?;
            Ok(Ty::Con(interner.get(id)?.to_string()))
        }
        3 => Ok(Ty::List(Box::new(decode_ty(input, interner)?))),
        4 => {
            let n = read_varu32(input)? as usize;
            let mut ts = Vec::with_capacity(n);
            for _ in 0..n {
                ts.push(decode_ty(input, interner)?);
            }
            Ok(Ty::Tuple(ts))
        }
        5 => {
            let n = read_varu32(input)? as usize;
            let mut fields = Vec::with_capacity(n);
            for _ in 0..n {
                let kid = read_varu32(input)?;
                let k = interner.get(kid)?.to_string();
                let t = decode_ty(input, interner)?;
                fields.push((k, t));
            }
            Ok(Ty::Record(fields))
        }
        6 => {
            let n = read_varu32(input)? as usize;
            let mut fields = Vec::with_capacity(n);
            for _ in 0..n {
                let kid = read_varu32(input)?;
                let k = interner.get(kid)?.to_string();
                let t = decode_ty(input, interner)?;
                fields.push((k, t));
            }
            let rest = decode_ty(input, interner)?;
            Ok(Ty::RecordOpen(fields, Box::new(rest)))
        }
        7 => {
            let head = decode_ty(input, interner)?;
            let n = read_varu32(input)? as usize;
            let mut args = Vec::with_capacity(n);
            for _ in 0..n {
                args.push(decode_ty(input, interner)?);
            }
            Ok(Ty::App {
                head: Box::new(head),
                args,
            })
        }
        8 => {
            let a = decode_ty(input, interner)?;
            let b = decode_ty(input, interner)?;
            Ok(Ty::Func(Box::new(a), Box::new(b)))
        }
        other => Err(Kir1Error::Msg(format!("unknown Ty tag: {other}"))),
    }
}

fn collect_strings_expr(expr: &IrExpr, interner: &mut StringInterner) {
    match expr {
        IrExpr::Unit => {}
        IrExpr::Integer(s) | IrExpr::Float64(s) | IrExpr::String(s) | IrExpr::Var(s) => {
            interner.intern(s);
        }
        IrExpr::Bool(_) | IrExpr::Char(_) => {}
        IrExpr::Lambda { params, body } => {
            for p in params {
                interner.intern(p);
            }
            collect_strings_expr(body, interner);
        }
        IrExpr::Apply { func, args } => {
            collect_strings_expr(func, interner);
            for a in args {
                collect_strings_expr(a, interner);
            }
        }
        IrExpr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_strings_expr(cond, interner);
            collect_strings_expr(then_branch, interner);
            collect_strings_expr(else_branch, interner);
        }
        IrExpr::Let { bindings, body } => {
            for (n, e) in bindings {
                interner.intern(n);
                collect_strings_expr(e, interner);
            }
            collect_strings_expr(body, interner);
        }
        IrExpr::Case { expr, arms } => {
            collect_strings_expr(expr, interner);
            for a in arms {
                collect_strings_case_arm(a, interner);
            }
        }
        IrExpr::IoBind {
            action,
            param,
            body,
        } => {
            collect_strings_expr(action, interner);
            interner.intern(param);
            collect_strings_expr(body, interner);
        }
        IrExpr::IoThen { first, then_expr } => {
            collect_strings_expr(first, interner);
            collect_strings_expr(then_expr, interner);
        }
        IrExpr::Cons { head, tail } => {
            collect_strings_expr(head, interner);
            collect_strings_expr(tail, interner);
        }
        IrExpr::List(xs) | IrExpr::Tuple(xs) => {
            for x in xs {
                collect_strings_expr(x, interner);
            }
        }
        IrExpr::Record(fields) => {
            for (k, v) in fields {
                interner.intern(k);
                collect_strings_expr(v, interner);
            }
        }
        IrExpr::CheckedCast { expr, target: _ } => {
            collect_strings_expr(expr, interner);
        }
    }
}

fn collect_strings_pattern(p: &IrPattern, interner: &mut StringInterner) {
    match p {
        IrPattern::Var(s) => {
            interner.intern(s);
        }
        IrPattern::Wildcard => {}
        IrPattern::Literal(l) => collect_strings_literal(l, interner),
        IrPattern::Tuple(ps) | IrPattern::List(ps) => {
            for p in ps {
                collect_strings_pattern(p, interner);
            }
        }
        IrPattern::Record(fields) => {
            for (k, v) in fields {
                interner.intern(k);
                collect_strings_pattern(v, interner);
            }
        }
        IrPattern::RecordLoose(fields, tail) => {
            for (k, v) in fields {
                interner.intern(k);
                collect_strings_pattern(v, interner);
            }
            if let Some(t) = tail {
                interner.intern(t);
            }
        }
        IrPattern::Cons(h, t) | IrPattern::Or(h, t) => {
            collect_strings_pattern(h, interner);
            collect_strings_pattern(t, interner);
        }
        IrPattern::Constructor { name, args } => {
            interner.intern(name);
            for a in args {
                collect_strings_pattern(a, interner);
            }
        }
        IrPattern::As(n, p) => {
            interner.intern(n);
            collect_strings_pattern(p, interner);
        }
        IrPattern::View(p, e) => {
            collect_strings_pattern(p, interner);
            collect_strings_expr(e, interner);
        }
    }
}

fn collect_strings_case_arm(a: &IrCaseArm, interner: &mut StringInterner) {
    collect_strings_pattern(&a.pat, interner);
    if let Some(g) = &a.guard {
        collect_strings_expr(g, interner);
    }
    collect_strings_expr(&a.body, interner);
}

fn collect_strings_literal(l: &IrLiteral, interner: &mut StringInterner) {
    match l {
        IrLiteral::Unit => {}
        IrLiteral::Integer(s) | IrLiteral::Float64(s) | IrLiteral::String(s) => {
            interner.intern(s);
        }
        IrLiteral::Bool(_) | IrLiteral::Char(_) => {}
    }
}

fn encode_strings_section(interner: &StringInterner) -> Vec<u8> {
    let mut out = Vec::new();
    write_varu32(&mut out, interner.strings.len() as u32);
    for s in &interner.strings {
        let bytes = s.as_bytes();
        write_varu32(&mut out, bytes.len() as u32);
        out.extend_from_slice(bytes);
    }
    out
}

fn decode_strings_section(mut input: &[u8]) -> Kir1Result<StringInterner> {
    let n = read_varu32(&mut input)? as usize;
    let mut strings = Vec::with_capacity(n);
    for _ in 0..n {
        let len = read_varu32(&mut input)? as usize;
        let bytes = read_bytes(&mut input, len)?;
        let s = std::str::from_utf8(bytes)
            .map_err(|e| Kir1Error::Msg(format!("invalid utf8: {e}")))?
            .to_string();
        strings.push(s);
    }
    if !input.is_empty() {
        return Err(Kir1Error::Msg("trailing bytes in STRINGS".to_string()));
    }
    let mut index = std::collections::HashMap::new();
    for (i, s) in strings.iter().enumerate() {
        index.insert(s.clone(), i as u32);
    }
    Ok(StringInterner { strings, index })
}

fn encode_ir_section(module: &IrModule, interner: &StringInterner) -> Vec<u8> {
    let mut out = Vec::new();
    write_varu32(&mut out, module.items.len() as u32);
    for it in &module.items {
        match it {
            IrItem::Binding { name, expr } => {
                write_u8(&mut out, 0);
                write_varu32(&mut out, interner.index[name]);
                encode_expr(&mut out, expr, interner);
            }
        }
    }
    out
}

fn decode_ir_section(mut input: &[u8], interner: &mut StringInterner) -> Kir1Result<IrModule> {
    let n = read_varu32(&mut input)? as usize;
    let mut items = Vec::with_capacity(n);
    for _ in 0..n {
        let tag = read_u8(&mut input)?;
        match tag {
            0 => {
                let name_id = read_varu32(&mut input)?;
                let name = interner.get(name_id)?.to_string();
                let expr = decode_expr(&mut input, interner)?;
                items.push(IrItem::Binding { name, expr });
            }
            other => return Err(Kir1Error::Msg(format!("unknown IrItem tag: {other}"))),
        }
    }
    if !input.is_empty() {
        return Err(Kir1Error::Msg("trailing bytes in IR".to_string()));
    }
    Ok(IrModule { items })
}

fn encode_literal(out: &mut Vec<u8>, lit: &IrLiteral, interner: &StringInterner) {
    match lit {
        IrLiteral::Unit => write_u8(out, 0),
        IrLiteral::Integer(s) => {
            write_u8(out, 1);
            write_varu32(out, interner.index[s]);
        }
        IrLiteral::Float64(s) => {
            write_u8(out, 2);
            write_varu32(out, interner.index[s]);
        }
        IrLiteral::Bool(b) => {
            write_u8(out, 3);
            write_u8(out, if *b { 1 } else { 0 });
        }
        IrLiteral::String(s) => {
            write_u8(out, 4);
            write_varu32(out, interner.index[s]);
        }
        IrLiteral::Char(c) => {
            write_u8(out, 5);
            write_u32(out, *c as u32);
        }
    }
}

fn decode_literal(input: &mut &[u8], interner: &mut StringInterner) -> Kir1Result<IrLiteral> {
    let tag = read_u8(input)?;
    match tag {
        0 => Ok(IrLiteral::Unit),
        1 => {
            let id = read_varu32(input)?;
            Ok(IrLiteral::Integer(interner.get(id)?.to_string()))
        }
        2 => {
            let id = read_varu32(input)?;
            Ok(IrLiteral::Float64(interner.get(id)?.to_string()))
        }
        3 => {
            let b = read_u8(input)?;
            Ok(IrLiteral::Bool(match b {
                0 => false,
                1 => true,
                _ => return Err(Kir1Error::Msg("invalid bool byte".to_string())),
            }))
        }
        4 => {
            let id = read_varu32(input)?;
            Ok(IrLiteral::String(interner.get(id)?.to_string()))
        }
        5 => {
            let v = read_u32(input)?;
            let c =
                std::char::from_u32(v).ok_or_else(|| Kir1Error::Msg("invalid char".to_string()))?;
            Ok(IrLiteral::Char(c))
        }
        other => Err(Kir1Error::Msg(format!("unknown IrLiteral tag: {other}"))),
    }
}

fn encode_pattern(out: &mut Vec<u8>, pat: &IrPattern, interner: &StringInterner) {
    match pat {
        IrPattern::Var(s) => {
            write_u8(out, 0);
            write_varu32(out, interner.index[s]);
        }
        IrPattern::Wildcard => write_u8(out, 1),
        IrPattern::Literal(lit) => {
            write_u8(out, 2);
            encode_literal(out, lit, interner);
        }
        IrPattern::Tuple(ps) => {
            write_u8(out, 3);
            write_varu32(out, ps.len() as u32);
            for p in ps {
                encode_pattern(out, p, interner);
            }
        }
        IrPattern::List(ps) => {
            write_u8(out, 4);
            write_varu32(out, ps.len() as u32);
            for p in ps {
                encode_pattern(out, p, interner);
            }
        }
        IrPattern::Record(fields) => {
            write_u8(out, 5);
            write_varu32(out, fields.len() as u32);
            for (k, v) in fields {
                write_varu32(out, interner.index[k]);
                encode_pattern(out, v, interner);
            }
        }
        IrPattern::RecordLoose(fields, tail) => {
            write_u8(out, 6);
            write_varu32(out, fields.len() as u32);
            for (k, v) in fields {
                write_varu32(out, interner.index[k]);
                encode_pattern(out, v, interner);
            }
            encode_opt_string_id(out, tail.as_deref(), interner);
        }
        IrPattern::Cons(h, t) => {
            write_u8(out, 7);
            encode_pattern(out, h, interner);
            encode_pattern(out, t, interner);
        }
        IrPattern::Constructor { name, args } => {
            write_u8(out, 8);
            write_varu32(out, interner.index[name]);
            write_varu32(out, args.len() as u32);
            for a in args {
                encode_pattern(out, a, interner);
            }
        }
        IrPattern::Or(a, b) => {
            write_u8(out, 9);
            encode_pattern(out, a, interner);
            encode_pattern(out, b, interner);
        }
        IrPattern::As(name, p) => {
            write_u8(out, 10);
            write_varu32(out, interner.index[name]);
            encode_pattern(out, p, interner);
        }
        IrPattern::View(pat, expr) => {
            write_u8(out, 11);
            encode_pattern(out, pat, interner);
            encode_expr(out, expr, interner);
        }
    }
}

fn decode_pattern(input: &mut &[u8], interner: &mut StringInterner) -> Kir1Result<IrPattern> {
    let tag = read_u8(input)?;
    match tag {
        0 => {
            let id = read_varu32(input)?;
            Ok(IrPattern::Var(interner.get(id)?.to_string()))
        }
        1 => Ok(IrPattern::Wildcard),
        2 => Ok(IrPattern::Literal(decode_literal(input, interner)?)),
        3 => {
            let n = read_varu32(input)? as usize;
            let mut ps = Vec::with_capacity(n);
            for _ in 0..n {
                ps.push(decode_pattern(input, interner)?);
            }
            Ok(IrPattern::Tuple(ps))
        }
        4 => {
            let n = read_varu32(input)? as usize;
            let mut ps = Vec::with_capacity(n);
            for _ in 0..n {
                ps.push(decode_pattern(input, interner)?);
            }
            Ok(IrPattern::List(ps))
        }
        5 => {
            let n = read_varu32(input)? as usize;
            let mut fields = Vec::with_capacity(n);
            for _ in 0..n {
                let k = read_varu32(input)?;
                let v = decode_pattern(input, interner)?;
                fields.push((interner.get(k)?.to_string(), v));
            }
            Ok(IrPattern::Record(fields))
        }
        6 => {
            let n = read_varu32(input)? as usize;
            let mut fields = Vec::with_capacity(n);
            for _ in 0..n {
                let k = read_varu32(input)?;
                let v = decode_pattern(input, interner)?;
                fields.push((interner.get(k)?.to_string(), v));
            }
            let tail = decode_opt_string_id(input, interner)?;
            Ok(IrPattern::RecordLoose(fields, tail))
        }
        7 => {
            let h = decode_pattern(input, interner)?;
            let t = decode_pattern(input, interner)?;
            Ok(IrPattern::Cons(Box::new(h), Box::new(t)))
        }
        8 => {
            let name = read_varu32(input)?;
            let n = read_varu32(input)? as usize;
            let mut args = Vec::with_capacity(n);
            for _ in 0..n {
                args.push(decode_pattern(input, interner)?);
            }
            Ok(IrPattern::Constructor {
                name: interner.get(name)?.to_string(),
                args,
            })
        }
        9 => {
            let a = decode_pattern(input, interner)?;
            let b = decode_pattern(input, interner)?;
            Ok(IrPattern::Or(Box::new(a), Box::new(b)))
        }
        10 => {
            let name = read_varu32(input)?;
            let p = decode_pattern(input, interner)?;
            Ok(IrPattern::As(interner.get(name)?.to_string(), Box::new(p)))
        }
        11 => {
            let pat = decode_pattern(input, interner)?;
            let expr = decode_expr(input, interner)?;
            Ok(IrPattern::View(Box::new(pat), Box::new(expr)))
        }
        other => Err(Kir1Error::Msg(format!("unknown IrPattern tag: {other}"))),
    }
}

fn encode_case_arm(out: &mut Vec<u8>, arm: &IrCaseArm, interner: &StringInterner) {
    encode_pattern(out, &arm.pat, interner);
    match &arm.guard {
        None => write_u8(out, 0),
        Some(g) => {
            write_u8(out, 1);
            encode_expr(out, g, interner);
        }
    }
    encode_expr(out, &arm.body, interner);
}

fn decode_case_arm(input: &mut &[u8], interner: &mut StringInterner) -> Kir1Result<IrCaseArm> {
    let pat = decode_pattern(input, interner)?;
    let has_guard = read_u8(input)?;
    let guard = match has_guard {
        0 => None,
        1 => Some(decode_expr(input, interner)?),
        _ => return Err(Kir1Error::Msg("invalid guard byte".to_string())),
    };
    let body = decode_expr(input, interner)?;
    Ok(IrCaseArm { pat, guard, body })
}

fn encode_expr(out: &mut Vec<u8>, expr: &IrExpr, interner: &StringInterner) {
    #![allow(clippy::too_many_lines)]
    match expr {
        IrExpr::Unit => write_u8(out, 0),
        IrExpr::Integer(s) => {
            write_u8(out, 1);
            write_varu32(out, interner.index[s]);
        }
        IrExpr::Float64(s) => {
            write_u8(out, 2);
            write_varu32(out, interner.index[s]);
        }
        IrExpr::Bool(b) => {
            write_u8(out, 3);
            write_u8(out, if *b { 1 } else { 0 });
        }
        IrExpr::String(s) => {
            write_u8(out, 4);
            write_varu32(out, interner.index[s]);
        }
        IrExpr::Char(c) => {
            write_u8(out, 5);
            write_u32(out, *c as u32);
        }
        IrExpr::Var(s) => {
            write_u8(out, 6);
            write_varu32(out, interner.index[s]);
        }
        IrExpr::Lambda { params, body } => {
            write_u8(out, 7);
            write_varu32(out, params.len() as u32);
            for p in params {
                write_varu32(out, interner.index[p]);
            }
            encode_expr(out, body, interner);
        }
        IrExpr::Apply { func, args } => {
            write_u8(out, 8);
            encode_expr(out, func, interner);
            write_varu32(out, args.len() as u32);
            for a in args {
                encode_expr(out, a, interner);
            }
        }
        IrExpr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            write_u8(out, 9);
            encode_expr(out, cond, interner);
            encode_expr(out, then_branch, interner);
            encode_expr(out, else_branch, interner);
        }
        IrExpr::Let { bindings, body } => {
            write_u8(out, 10);
            write_varu32(out, bindings.len() as u32);
            for (name, expr) in bindings {
                write_varu32(out, interner.index[name]);
                encode_expr(out, expr, interner);
            }
            encode_expr(out, body, interner);
        }
        IrExpr::Case { expr, arms } => {
            write_u8(out, 11);
            encode_expr(out, expr, interner);
            write_varu32(out, arms.len() as u32);
            for a in arms {
                encode_case_arm(out, a, interner);
            }
        }
        IrExpr::IoBind {
            action,
            param,
            body,
        } => {
            write_u8(out, 12);
            encode_expr(out, action, interner);
            write_varu32(out, interner.index[param]);
            encode_expr(out, body, interner);
        }
        IrExpr::IoThen { first, then_expr } => {
            write_u8(out, 13);
            encode_expr(out, first, interner);
            encode_expr(out, then_expr, interner);
        }
        IrExpr::Cons { head, tail } => {
            write_u8(out, 14);
            encode_expr(out, head, interner);
            encode_expr(out, tail, interner);
        }
        IrExpr::List(xs) => {
            write_u8(out, 15);
            write_varu32(out, xs.len() as u32);
            for x in xs {
                encode_expr(out, x, interner);
            }
        }
        IrExpr::Tuple(xs) => {
            write_u8(out, 16);
            write_varu32(out, xs.len() as u32);
            for x in xs {
                encode_expr(out, x, interner);
            }
        }
        IrExpr::Record(fields) => {
            write_u8(out, 17);
            write_varu32(out, fields.len() as u32);
            for (k, v) in fields {
                write_varu32(out, interner.index[k]);
                encode_expr(out, v, interner);
            }
        }
        IrExpr::CheckedCast { expr, target } => {
            write_u8(out, 18);
            encode_expr(out, expr, interner);
            write_u8(
                out,
                match target {
                    CastTarget::I32 => 0,
                    CastTarget::I64 => 1,
                    CastTarget::F32 => 2,
                    CastTarget::F64 => 3,
                },
            );
        }
    }
}

fn decode_expr(input: &mut &[u8], interner: &mut StringInterner) -> Kir1Result<IrExpr> {
    #![allow(clippy::too_many_lines)]
    let tag = read_u8(input)?;
    Ok(match tag {
        0 => IrExpr::Unit,
        1 => {
            let id = read_varu32(input)?;
            IrExpr::Integer(interner.get(id)?.to_string())
        }
        2 => {
            let id = read_varu32(input)?;
            IrExpr::Float64(interner.get(id)?.to_string())
        }
        3 => {
            let b = read_u8(input)?;
            IrExpr::Bool(match b {
                0 => false,
                1 => true,
                _ => return Err(Kir1Error::Msg("invalid bool byte".to_string())),
            })
        }
        4 => {
            let id = read_varu32(input)?;
            IrExpr::String(interner.get(id)?.to_string())
        }
        5 => {
            let v = read_u32(input)?;
            let c =
                std::char::from_u32(v).ok_or_else(|| Kir1Error::Msg("invalid char".to_string()))?;
            IrExpr::Char(c)
        }
        6 => {
            let id = read_varu32(input)?;
            IrExpr::Var(interner.get(id)?.to_string())
        }
        7 => {
            let n = read_varu32(input)? as usize;
            let mut params = Vec::with_capacity(n);
            for _ in 0..n {
                let id = read_varu32(input)?;
                params.push(interner.get(id)?.to_string());
            }
            let body = decode_expr(input, interner)?;
            IrExpr::Lambda {
                params,
                body: Box::new(body),
            }
        }
        8 => {
            let func = decode_expr(input, interner)?;
            let n = read_varu32(input)? as usize;
            let mut args = Vec::with_capacity(n);
            for _ in 0..n {
                args.push(decode_expr(input, interner)?);
            }
            IrExpr::Apply {
                func: Box::new(func),
                args,
            }
        }
        9 => {
            let c = decode_expr(input, interner)?;
            let t = decode_expr(input, interner)?;
            let e = decode_expr(input, interner)?;
            IrExpr::If {
                cond: Box::new(c),
                then_branch: Box::new(t),
                else_branch: Box::new(e),
            }
        }
        10 => {
            let n = read_varu32(input)? as usize;
            let mut binds = Vec::with_capacity(n);
            for _ in 0..n {
                let name = read_varu32(input)?;
                let expr = decode_expr(input, interner)?;
                binds.push((interner.get(name)?.to_string(), expr));
            }
            let body = decode_expr(input, interner)?;
            IrExpr::Let {
                bindings: binds,
                body: Box::new(body),
            }
        }
        11 => {
            let scrut = decode_expr(input, interner)?;
            let n = read_varu32(input)? as usize;
            let mut arms = Vec::with_capacity(n);
            for _ in 0..n {
                arms.push(decode_case_arm(input, interner)?);
            }
            IrExpr::Case {
                expr: Box::new(scrut),
                arms,
            }
        }
        12 => {
            let action = decode_expr(input, interner)?;
            let param = read_varu32(input)?;
            let body = decode_expr(input, interner)?;
            IrExpr::IoBind {
                action: Box::new(action),
                param: interner.get(param)?.to_string(),
                body: Box::new(body),
            }
        }
        13 => {
            let first = decode_expr(input, interner)?;
            let then_expr = decode_expr(input, interner)?;
            IrExpr::IoThen {
                first: Box::new(first),
                then_expr: Box::new(then_expr),
            }
        }
        14 => {
            let head = decode_expr(input, interner)?;
            let tail = decode_expr(input, interner)?;
            IrExpr::Cons {
                head: Box::new(head),
                tail: Box::new(tail),
            }
        }
        15 => {
            let n = read_varu32(input)? as usize;
            let mut xs = Vec::with_capacity(n);
            for _ in 0..n {
                xs.push(decode_expr(input, interner)?);
            }
            IrExpr::List(xs)
        }
        16 => {
            let n = read_varu32(input)? as usize;
            let mut xs = Vec::with_capacity(n);
            for _ in 0..n {
                xs.push(decode_expr(input, interner)?);
            }
            IrExpr::Tuple(xs)
        }
        17 => {
            let n = read_varu32(input)? as usize;
            let mut fields = Vec::with_capacity(n);
            for _ in 0..n {
                let k = read_varu32(input)?;
                let v = decode_expr(input, interner)?;
                fields.push((interner.get(k)?.to_string(), v));
            }
            IrExpr::Record(fields)
        }
        18 => {
            let e = decode_expr(input, interner)?;
            let t = read_u8(input)?;
            let target = match t {
                0 => CastTarget::I32,
                1 => CastTarget::I64,
                2 => CastTarget::F32,
                3 => CastTarget::F64,
                _ => return Err(Kir1Error::Msg("invalid CastTarget".to_string())),
            };
            IrExpr::CheckedCast {
                expr: Box::new(e),
                target,
            }
        }
        other => return Err(Kir1Error::Msg(format!("unknown IrExpr tag: {other}"))),
    })
}

fn encode_opt_string_id(out: &mut Vec<u8>, s: Option<&str>, interner: &StringInterner) {
    match s {
        None => write_u8(out, 0),
        Some(s) => {
            write_u8(out, 1);
            write_varu32(out, interner.index[s]);
        }
    }
}

fn decode_opt_string_id(
    input: &mut &[u8],
    interner: &mut StringInterner,
) -> Kir1Result<Option<String>> {
    let tag = read_u8(input)?;
    match tag {
        0 => Ok(None),
        1 => {
            let id = read_varu32(input)?;
            Ok(Some(interner.get(id)?.to_string()))
        }
        _ => Err(Kir1Error::Msg("invalid opt string tag".to_string())),
    }
}

// ---- primitive encoders/decoders ----

fn write_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}

fn write_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn write_varu32(out: &mut Vec<u8>, mut v: u32) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn read_u8(input: &mut &[u8]) -> Kir1Result<u8> {
    if input.is_empty() {
        return Err(Kir1Error::Msg("unexpected EOF".to_string()));
    }
    let b = input[0];
    *input = &input[1..];
    Ok(b)
}

fn read_u16(input: &mut &[u8]) -> Kir1Result<u16> {
    let bytes = read_bytes(input, 2)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(input: &mut &[u8]) -> Kir1Result<u32> {
    let bytes = read_bytes(input, 4)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(input: &mut &[u8]) -> Kir1Result<u64> {
    let bytes = read_bytes(input, 8)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn read_varu32(input: &mut &[u8]) -> Kir1Result<u32> {
    let mut shift = 0u32;
    let mut out: u32 = 0;
    loop {
        let b = read_u8(input)?;
        out |= ((b & 0x7f) as u32) << shift;
        if (b & 0x80) == 0 {
            return Ok(out);
        }
        shift += 7;
        if shift > 28 {
            return Err(Kir1Error::Msg("varu32 too large".to_string()));
        }
    }
}

fn read_bytes<'a>(input: &mut &'a [u8], n: usize) -> Kir1Result<&'a [u8]> {
    if input.len() < n {
        return Err(Kir1Error::Msg("unexpected EOF".to_string()));
    }
    let out = &input[..n];
    *input = &input[n..];
    Ok(out)
}

fn read_bytes_4(input: &mut &[u8]) -> Kir1Result<[u8; 4]> {
    let b = read_bytes(input, 4)?;
    Ok([b[0], b[1], b[2], b[3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_simple_module() {
        let m = IrModule {
            items: vec![IrItem::Binding {
                name: "main".to_string(),
                expr: IrExpr::Integer("42".to_string()),
            }],
        };

        let bytes = encode_kir1_module(&m);
        let m2 = decode_kir1_module(&bytes).expect("decode");
        assert_eq!(m, m2);
    }

    #[test]
    fn roundtrip_ksif_minimal() {
        let ksif = KsifModule {
            module_name: "A".to_string(),
            values: vec![(
                "id".to_string(),
                Scheme {
                    vars: vec![0],
                    constraints: vec![],
                    ty: Ty::Func(Box::new(Ty::Var(0)), Box::new(Ty::Var(0))),
                },
            )],
            dependencies: vec![],
        };
        let bytes = encode_ksif_module(&ksif);
        let decoded = decode_ksif_module(&bytes).unwrap();
        assert_eq!(decoded, ksif);
    }

    #[test]
    fn roundtrip_ksif_with_dependencies() {
        let ksif = KsifModule {
            module_name: "B".to_string(),
            values: vec![(
                "foo".to_string(),
                Scheme {
                    vars: vec![],
                    constraints: vec![],
                    ty: Ty::Var(0),
                },
            )],
            dependencies: vec![
                ("Prelude".to_string(), "abc123".to_string()),
                ("Data.List".to_string(), "def456".to_string()),
            ],
        };
        let bytes = encode_ksif_module(&ksif);
        let decoded = decode_ksif_module(&bytes).unwrap();
        assert_eq!(decoded, ksif);
    }
}
