//! Module binary — RFC 0033 §1: versioned, deterministic, hash-stable
//! serialization of the linked module. v1 links a single module: type ids
//! in the binary are program-global (link-time rebase, RFC 0035 §1, lands
//! with multi-module).

use crate::ops::*;
use crate::types::{FieldInfo, PrimTy, Repr, RutType, TyKind, TypeId, TypeTable};

// ---- the loaded program ----

#[derive(Clone, Debug)]
pub struct TraitMethod {
    pub name: String,
    pub params: Vec<TypeId>,
    pub ret: TypeId,
}

#[derive(Clone, Debug)]
pub struct TraitDesc {
    pub name: String,
    pub methods: Vec<TraitMethod>,
}

#[derive(Clone, Debug)]
pub struct FuncCode {
    pub name: String,
    pub params: Vec<TypeId>,
    pub ret: TypeId,
    /// method receiver type (params[0]) — Some for methods
    pub is_method: bool,
    pub n_captures: u32,
    pub regs: Vec<TypeId>,
    pub code: Vec<Op>,
    /// pc → source byte offset (RFC 0036 — symbolication data)
    pub spans: Vec<(u32, u32)>,
    /// host-function binding (RFC 0022/0026/0029): `Some(name)` when the
    /// function has no rut body — `Op::Call` dispatches to the embedder's
    /// impl registered under `name`.
    pub host: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConstVal {
    I64(i64),
    F64(f64),
    Bool(bool),
    Char(char),
    Str(String),
    /// a `type_id<T>()` constant — a module-local `TypeId` at rest, rebased
    /// to the global type table at link (RFC 0035 §1)
    TypeId(TypeId),
}

/// One exported function in a module's surface (RFC 0029 DeclIr sketch):
/// the importable name, its signature, and its module-local id.
#[derive(Clone, Debug, Default)]
pub struct SurfaceFn {
    pub name: String,
    pub params: Vec<TypeId>,
    pub ret: TypeId,
    /// module-local function id (the exporter's)
    pub local: u32,
    /// `Some` for a compiler-lowered intrinsic (`std:math` wrapping/
    /// saturating/checked and `abs`/`min`/`max`/`signum`): there is no
    /// `FuncCode`, and the declared signature is a placeholder — `rut-lir`
    /// types the call from its arguments (RFC 0032 §1.1 R2).
    pub intrinsic: Option<crate::ops::Intrinsic>,
}

/// One exported constant in a module's surface — `std:math::PI` and
/// friends. `bits` is the raw scalar payload (f64 bits, i64 bits, …) the
/// importer materializes with `ConstRaw`.
#[derive(Clone, Debug, Default)]
pub struct SurfaceConst {
    pub name: String,
    pub ty: TypeId,
    pub bits: u64,
}

/// One exported type: its importable name and module-local id.
#[derive(Clone, Debug, Default)]
pub struct SurfaceType {
    pub name: String,
    /// local id within the exporter's own type block
    pub local: u32,
    /// records: a `class` (no outside literal) vs a `struct`
    pub is_class: bool,
    /// a generic template (`Vec<T>`): a consumer must monomorphize it, so
    /// the graph compiler source-inlines the module rather than linking it
    pub is_generic: bool,
}

/// A builtin container published by `std:core`'s native surface (RFC 0028):
/// the type constructor is the compiler's own — the NAME resolves only once
/// the importer wrote `import { .. } from "std:core"`. The prelude is
/// imported, never ambient.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTy {
    /// `Array<T>` — the heap array cell (RFC 0005)
    Array,
    /// `Opaque` — the erasure box (RFC 0014); a boot-table type whose name
    /// is import-gated like the containers
    Opaque,
}

/// A builtin interface published by `std:core`'s native surface (RFC 0028):
/// registered on first reference, exactly like a declared trait — but only
/// for importers that named it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeIface {
    /// `Disposal { fn dispose(mut self) -> unit }` (RFC 0011/0016)
    Disposal,
    /// `Index<T>` — the random-access contract (RFC 0012)
    Index,
    /// `Iterator<T>` — the cursor contract (RFC 0012)
    Iterator,
}

/// The importable surface a module publishes (traits still to come).
#[derive(Clone, Debug, Default)]
pub struct Surface {
    /// The qualified-access head for a namespace module (`std:math` ->
    /// `Math`): members are reached as `<namespace>.<member>`. `None`
    /// when the module has no namespace form. Routing is name-generic:
    /// the compiler binds the head only when the importer wrote it.
    pub namespace: Option<String>,
    pub funcs: Vec<SurfaceFn>,
    /// exported constants (native modules: `std:math`)
    pub consts: Vec<SurfaceConst>,
    /// the exporter's non-boot type descriptors, verbatim; ids inside are
    /// packed with the exporter's scope (RFC 0035 §1)
    pub types: Vec<crate::types::RutType>,
    /// scope -> local offset inside `types` for each block it carries
    pub scope_blocks: Vec<(crate::id::ScopeId, u32)>,
    /// exported (pub) type names
    pub type_exports: Vec<SurfaceType>,
    /// builtin container names (`std:core` only): name -> constructor
    pub native_types: Vec<(String, NativeTy)>,
    /// builtin interface names (`std:core` only): name -> contract
    pub native_ifaces: Vec<(String, NativeIface)>,
    /// compiler-lowered builtin function names (`std:core` only) — no
    /// `FuncCode`; the bodies are rut-lir lowering, reached only through
    /// the import binding
    pub native_fns: Vec<String>,
}

/// The `std:core` prelude function names (RFC 0028), in surface order.
pub const CORE_FNS: &[&str] = &[
    "downcast", "assert", "panic",
    "make_ptr", "on_drop",
    "string_join",
];

impl Surface {
    /// The `std:core` prelude surface (RFC 0028): the builtin containers,
    /// the builtin interfaces, and the compiler-lowered functions. One
    /// source of truth — the driver mounts it (`mount_std_core`), the
    /// compiler hints from it, and `rut/std-core/core.d.rut` mirrors it
    /// for the LSP (kept true to the implementation by test).
    pub fn core() -> Surface {
        Surface {
            native_types: vec![
                ("Array".to_string(), NativeTy::Array),
                ("Opaque".to_string(), NativeTy::Opaque),
            ],
            native_ifaces: vec![("Iterator".to_string(), NativeIface::Iterator)],
            native_fns: CORE_FNS.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }
}

/// A `std:core` builtin container by name, if it is one.
pub fn core_native_type(name: &str) -> Option<NativeTy> {
    Surface::core()
        .native_types
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, k)| *k)
}

/// A `std:core` builtin interface by name, if it is one.
pub fn core_native_iface(name: &str) -> Option<NativeIface> {
    Surface::core()
        .native_ifaces
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, k)| *k)
}

/// Is `name` one of the `std:core` prelude functions?
pub fn is_core_fn(name: &str) -> bool {
    CORE_FNS.contains(&name)
}

/// Is `name` any `std:core` prelude name (type, interface, or function)?
pub fn is_core_name(name: &str) -> bool {
    is_core_fn(name) || core_native_type(name).is_some() || core_native_iface(name).is_some()
}

/// `std:core` names removed in v1.1, each with its replacement. Use
/// sites diagnose with these instead of "unknown" — a removed surface
/// explains itself (RFC 0028 v1.1).
pub const REMOVED_CORE: &[(&str, &str)] = &[
    ("own", "`own` was removed — values copy on assignment (RFC 0016 §1); share a cell via `make_ptr`"),
    ("Option", "`Option` was removed — absence is `nil` on a pointer type, or a `(T, err)` tuple (v1.1)"),
    ("Result", "`Result` was removed — errors are `(T, err)` tuples; an empty err is success (v1.1)"),
    ("char", "`char` was removed — codepoints are `u32`: `s.code()` reads one, `str.from_code(n)` builds one (RFC 0004 v1.1)"),
    ("Index", "`Index` was removed — indexing is builtin over `[T]`/`Vec`/`str`/`bytes`; give the type real `len`/indexing members or a `buf`+`len` shape (RFC 0012 v1.1)"),
    ("Disposal", "`Disposal` was removed — attach cleanups with `on_drop` (RFC 0016 v1.1)"),
    ("string_len", "`string_len(s)` was removed — use `s.len()` (RFC 0004 v1.1)"),
    ("string_encode", "`string_encode(s)` was removed — use `s.encode()` (RFC 0004 v1.1)"),
    ("bytes_len", "`bytes_len(b)` was removed — use `b.len()` (RFC 0004 v1.1)"),
    ("bytes_decode", "`bytes_decode(b)` was removed — use `b.decode()` (RFC 0004 v1.1)"),
    ("bytes_from", "`bytes_from(a)` was removed — use `bytes.from(a)` (RFC 0004 v1.1)"),
    ("bytes_zeroed", "`bytes_zeroed(n)` was removed — use `bytes.zeroed(n)` (RFC 0004 v1.1)"),
];

/// The v1.1 removal message for `name`, if it is a removed `std:core` name.
pub fn removed_core(name: &str) -> Option<&'static str> {
    REMOVED_CORE
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, msg)| *msg)
}

#[derive(Clone, Debug, Default)]
pub struct Program {
    pub name: String,
    /// the module's stable scope (RFC 0035 §1); `0` if unset
    pub scope: crate::id::ScopeId,
    /// exported surface, for importers (in-memory; not serialized)
    pub surface: Surface,
    pub types: TypeTable,
    pub traits: Vec<TraitDesc>,
    /// global trait-method slots: (trait id, method index) — RFC 0015 §6
    pub trait_slots: Vec<(u32, u32)>,
    /// per-type vtables: entry per type, mapping slot → func id
    pub vtables: Vec<Vec<Option<u32>>>,
    pub consts: Vec<ConstVal>,
    pub funcs: Vec<FuncCode>,
    pub exports: Vec<(String, u32)>,
}

impl Program {
    pub fn export(&self, name: &str) -> Option<u32> {
        self.exports.iter().find(|(n, _)| n == name).map(|(_, f)| *f)
    }
    pub fn slot_of(&self, trait_id: u32, method: u32) -> Option<u32> {
        self.trait_slots
            .iter()
            .position(|&(t, m)| t == trait_id && m == method)
            .map(|i| i as u32)
    }
}

// ---- encoding ----

pub const MAGIC: &[u8; 4] = b"RUTC";
pub const VERSION: u32 = 1;

pub fn encode(prog: &Program) -> Vec<u8> {
    let mut e = Enc::default();
    e.bytes(MAGIC);
    e.u32(VERSION);
    e.str(&prog.name);

    // types
    e.u32(prog.types.types.len() as u32);
    for t in &prog.types.types {
        e.str(&t.name);
        encode_kind(&mut e, &t.kind);
    }
    // traits
    e.u32(prog.traits.len() as u32);
    for t in &prog.traits {
        e.str(&t.name);
        e.u32(t.methods.len() as u32);
        for m in &t.methods {
            e.str(&m.name);
            e.tys(&m.params);
            e.u32(m.ret);
        }
    }
    // trait slots
    e.u32(prog.trait_slots.len() as u32);
    for &(t, m) in &prog.trait_slots {
        e.u32(t);
        e.u32(m);
    }
    // vtables: entries as (ty, [(slot, func)])
    let vt_entries: Vec<(u32, Vec<(u32, u32)>)> = prog
        .vtables
        .iter()
        .enumerate()
        .filter(|(_, vt)| vt.iter().any(|f| f.is_some()))
        .map(|(ty, vt)| {
            (
                ty as u32,
                vt.iter()
                    .enumerate()
                    .filter_map(|(s, f)| f.map(|f| (s as u32, f)))
                    .collect(),
            )
        })
        .collect();
    e.u32(vt_entries.len() as u32);
    for (ty, slots) in &vt_entries {
        e.u32(*ty);
        e.u32(slots.len() as u32);
        for (s, f) in slots {
            e.u32(*s);
            e.u32(*f);
        }
    }
    // consts
    e.u32(prog.consts.len() as u32);
    for c in &prog.consts {
        match c {
            ConstVal::I64(v) => {
                e.u8(0);
                e.i64(*v);
            }
            ConstVal::F64(v) => {
                e.u8(1);
                e.f64(*v);
            }
            ConstVal::Bool(v) => {
                e.u8(2);
                e.u8(*v as u8);
            }
            ConstVal::Char(v) => {
                e.u8(3);
                e.u32(*v as u32);
            }
            ConstVal::Str(s) => {
                e.u8(4);
                e.str(s);
            }
            ConstVal::TypeId(t) => {
                e.u8(5);
                e.u32(*t);
            }
        }
    }
    // funcs
    e.u32(prog.funcs.len() as u32);
    for f in &prog.funcs {
        e.str(&f.name);
        e.tys(&f.params);
        e.u32(f.ret);
        e.u8(f.is_method as u8);
        e.u32(f.n_captures);
        e.tys(&f.regs);
        e.u32(f.code.len() as u32);
        for op in &f.code {
            encode_op(&mut e, op);
        }
        e.u32(f.spans.len() as u32);
        for (pc, lo) in &f.spans {
            e.u32(*pc);
            e.u32(*lo);
        }
        e.u8(f.host.is_some() as u8);
        if let Some(h) = &f.host {
            e.str(h);
        }
    }
    // exports
    e.u32(prog.exports.len() as u32);
    for (n, f) in &prog.exports {
        e.str(n);
        e.u32(*f);
    }
    e.out
}

fn encode_kind(e: &mut Enc, k: &TyKind) {
    match k {
        TyKind::Unit => e.u8(0),
        TyKind::Prim(p) => {
            e.u8(1);
            e.u8(match p {
                PrimTy::U8 => 0, PrimTy::U16 => 1, PrimTy::U32 => 2, PrimTy::U64 => 3,
                PrimTy::I8 => 4, PrimTy::I16 => 5, PrimTy::I32 => 6, PrimTy::I64 => 7,
                PrimTy::F32 => 8, PrimTy::F64 => 9, PrimTy::Bool => 10, PrimTy::Char => 11,
            });
        }
        TyKind::Str => e.u8(2),
        TyKind::Bytes => e.u8(12),
        TyKind::Array { elem } => {
            e.u8(4);
            e.u32(*elem);
        }
        TyKind::Enum { members } => {
            e.u8(5);
            e.u32(members.len() as u32);
            for (n, v) in members {
                e.str(n);
                e.i64(*v);
            }
        }
        TyKind::Data { fields } => {
            e.u8(8);
            e.u32(fields.len() as u32);
            for f in fields {
                e.str(&f.name);
                e.u32(f.ty);
            }
        }
        TyKind::TraitObj { trait_id } => {
            e.u8(9);
            e.u32(*trait_id);
        }
        TyKind::Opaque => e.u8(10),
        TyKind::Fn { params, ret } => {
            e.u8(11);
            e.tys(params);
            e.u32(*ret);
        }
        TyKind::Ptr { elem } => {
            e.u8(13);
            e.u32(*elem);
        }
    }
}

pub fn decode(bytes: &[u8]) -> Result<Program, String> {
    let mut d = Dec { b: bytes, pos: 0 };
    let mut magic = [0u8; 4];
    d.bytes(&mut magic)?;
    if &magic != MAGIC {
        return Err("not a rut module binary (bad magic)".into());
    }
    let ver = d.u32()?;
    if ver != VERSION {
        return Err(format!("unsupported module binary version {ver}"));
    }
    let name = d.str()?;
    let ntypes = d.u32()? as usize;
    let mut types = TypeTable::default();
    types.types.reserve(ntypes);
    for _ in 0..ntypes {
        let tname = d.str()?;
        let kind = decode_kind(&mut d)?;
        types.types.push(RutType { name: tname, kind });
    }
    let ntraits = d.u32()? as usize;
    let mut traits = Vec::with_capacity(ntraits);
    for _ in 0..ntraits {
        let tname = d.str()?;
        let nmethods = d.u32()? as usize;
        let mut methods = Vec::with_capacity(nmethods);
        for _ in 0..nmethods {
            let mname = d.str()?;
            let params = d.tys()?;
            let ret = d.u32()?;
            methods.push(TraitMethod { name: mname, params, ret });
        }
        traits.push(TraitDesc { name: tname, methods });
    }
    let nslots = d.u32()? as usize;
    let mut trait_slots = Vec::with_capacity(nslots);
    for _ in 0..nslots {
        let t = d.u32()?;
        let m = d.u32()?;
        trait_slots.push((t, m));
    }
    let nvt = d.u32()? as usize;
    let mut vtables = vec![Vec::new(); types.types.len()];
    for _ in 0..nvt {
        let ty = d.u32()? as usize;
        let n = d.u32()? as usize;
        let mut vt = vec![None; trait_slots.len()];
        for _ in 0..n {
            let s = d.u32()? as usize;
            let f = d.u32()?;
            vt[s] = Some(f);
        }
        vtables[ty] = vt;
    }
    let nconsts = d.u32()? as usize;
    let mut consts = Vec::with_capacity(nconsts);
    for _ in 0..nconsts {
        consts.push(match d.u8()? {
            0 => ConstVal::I64(d.i64()?),
            1 => ConstVal::F64(d.f64()?),
            2 => ConstVal::Bool(d.u8()? != 0),
            3 => ConstVal::Char(char::from_u32(d.u32()?).unwrap_or('\0')),
            4 => ConstVal::Str(d.str()?),
            5 => ConstVal::TypeId(d.u32()?),
            t => return Err(format!("bad const tag {t}")),
        });
    }
    let nfuncs = d.u32()? as usize;
    let mut funcs = Vec::with_capacity(nfuncs);
    for _ in 0..nfuncs {
        let fname = d.str()?;
        let params = d.tys()?;
        let ret = d.u32()?;
        let is_method = d.u8()? != 0;
        let n_captures = d.u32()?;
        let regs = d.tys()?;
        let ncode = d.u32()? as usize;
        let mut code = Vec::with_capacity(ncode);
        for _ in 0..ncode {
            code.push(decode_op(&mut d)?);
        }
        let nspans = d.u32()? as usize;
        let mut spans = Vec::with_capacity(nspans);
        for _ in 0..nspans {
            let pc = d.u32()?;
            let lo = d.u32()?;
            spans.push((pc, lo));
        }
        let host = if d.u8()? != 0 { Some(d.str()?) } else { None };
        funcs.push(FuncCode { name: fname, params, ret, is_method, n_captures, regs, code, spans, host });
    }
    let nexp = d.u32()? as usize;
    let mut exports = Vec::with_capacity(nexp);
    for _ in 0..nexp {
        let n = d.str()?;
        let f = d.u32()?;
        exports.push((n, f));
    }
    Ok(Program { name, types, traits, trait_slots, vtables, consts, funcs, exports, ..Default::default() })
}

fn decode_kind(d: &mut Dec) -> Result<TyKind, String> {
    Ok(match d.u8()? {
        0 => TyKind::Unit,
        1 => TyKind::Prim(match d.u8()? {
            0 => PrimTy::U8, 1 => PrimTy::U16, 2 => PrimTy::U32, 3 => PrimTy::U64,
            4 => PrimTy::I8, 5 => PrimTy::I16, 6 => PrimTy::I32, 7 => PrimTy::I64,
            8 => PrimTy::F32, 9 => PrimTy::F64, 10 => PrimTy::Bool, 11 => PrimTy::Char,
            t => return Err(format!("bad prim tag {t}")),
        }),
        2 => TyKind::Str,
        12 => TyKind::Bytes,
        4 => TyKind::Array { elem: d.u32()? },
        5 => {
            let n = d.u32()? as usize;
            let mut members = Vec::with_capacity(n);
            for _ in 0..n {
                let m = d.str()?;
                let v = d.i64()?;
                members.push((m, v));
            }
            TyKind::Enum { members }
        }
        8 => {
            let n = d.u32()? as usize;
            let mut fields = Vec::with_capacity(n);
            for _ in 0..n {
                let name = d.str()?;
                let ty = d.u32()?;
                fields.push(FieldInfo { name, ty });
            }
            TyKind::Data { fields }
        }
        9 => TyKind::TraitObj { trait_id: d.u32()? },
        10 => TyKind::Opaque,
        11 => {
            let params = d.tys()?;
            let ret = d.u32()?;
            TyKind::Fn { params, ret }
        }
        13 => TyKind::Ptr { elem: d.u32()? },
        t => return Err(format!("bad type kind tag {t}")),
    })
}

fn encode_op(e: &mut Enc, op: &Op) {
    match op {
        Op::Mov { dst, src } => { e.u8(0); e.u16(*dst); e.u16(*src); }
        Op::MovRef { dst, src } => { e.u8(1); e.u16(*dst); e.u16(*src); }
        Op::Const { dst, k } => { e.u8(2); e.u16(*dst); e.u32(*k); }
        Op::ConstRaw { dst, bits } => { e.u8(3); e.u16(*dst); e.u64(*bits); }
        Op::Not { dst, a } => { e.u8(8); e.u16(*dst); e.u16(*a); }
        Op::AddF { prim, dst, a, b } => { e.u8(50); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::SubF { prim, dst, a, b } => { e.u8(51); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::MulF { prim, dst, a, b } => { e.u8(52); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::DivF { prim, dst, a, b } => { e.u8(53); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::ModF { prim, dst, a, b } => { e.u8(54); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::NegF { prim, dst, a } => { e.u8(55); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); }
        Op::EqF { dst, a, b } => { e.u8(56); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::NeF { dst, a, b } => { e.u8(57); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::LtF { dst, a, b } => { e.u8(58); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::GtF { dst, a, b } => { e.u8(59); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::LeF { dst, a, b } => { e.u8(60); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::GeF { dst, a, b } => { e.u8(61); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::AddI { prim, dst, a, b } => { e.u8(62); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::SubI { prim, dst, a, b } => { e.u8(63); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::MulI { prim, dst, a, b } => { e.u8(64); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::DivI { prim, dst, a, b } => { e.u8(65); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::ModI { prim, dst, a, b } => { e.u8(66); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::WAddI { prim, dst, a, b } => { e.u8(67); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::WSubI { prim, dst, a, b } => { e.u8(68); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::WMulI { prim, dst, a, b } => { e.u8(69); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::AndI { prim, dst, a, b } => { e.u8(72); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::OrI { prim, dst, a, b } => { e.u8(73); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::XorI { prim, dst, a, b } => { e.u8(74); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::ShlI { prim, dst, a, b } => { e.u8(75); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::ShrI { prim, dst, a, b } => { e.u8(76); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::WrapShlI { prim, dst, a, b } => { e.u8(77); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::EqI { prim, dst, a, b } => { e.u8(78); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::NeI { prim, dst, a, b } => { e.u8(79); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::LtI { prim, dst, a, b } => { e.u8(80); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::GtI { prim, dst, a, b } => { e.u8(81); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::LeI { prim, dst, a, b } => { e.u8(82); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::GeI { prim, dst, a, b } => { e.u8(83); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::NegI { prim, dst, a } => { e.u8(84); e.u8(prim.to_u8()); e.u16(*dst); e.u16(*a); }
        Op::StrCmp { eq, dst, a, b } => { e.u8(10); e.u8(*eq as u8); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::RefEq { eq, dst, a, b } => { e.u8(11); e.u8(*eq as u8); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::Jmp { target } => { e.u8(12); e.u32(*target); }
        Op::Br { cond, then_t, else_t } => { e.u8(13); e.u16(*cond); e.u32(*then_t); e.u32(*else_t); }
        Op::BrTable { idx, table, default } => {
            e.u8(14); e.u16(*idx); e.u32(table.len() as u32);
            for t in table { e.u32(*t); }
            e.u32(*default);
        }
        Op::Call { func, args, dst } => { e.u8(15); e.u32(*func); e.u16s(args); e.u8opt(dst); }
        Op::CallM { func, recv, args, dst } => { e.u8(16); e.u32(*func); e.u16(*recv); e.u16s(args); e.u8opt(dst); }
        Op::CallI { slot, recv, args, dst } => { e.u8(17); e.u32(*slot); e.u16(*recv); e.u16s(args); e.u8opt(dst); }
        Op::CallNat { nat, recv, args, dst } => { e.u8(18); e.u8(*nat as u8); e.u8opt(recv); e.u16s(args); e.u8opt(dst); }
        Op::CallFn { fval, args, dst } => { e.u8(19); e.u16(*fval); e.u16s(args); e.u8opt(dst); }
        Op::Ret { val } => { e.u8(20); e.u8opt(val); }
        Op::NewCell { dst, ty } => { e.u8(21); e.u16(*dst); e.u32(*ty); }
        Op::MakeRecord { dst, ty, vals } => { e.u8(49); e.u16(*dst); e.u32(*ty); e.u16s(vals); }
        Op::GetF { dst, obj, field, repr } => { e.u8(22); e.u16(*dst); e.u16(*obj); e.u32(*field); e.u8(repr.to_u8()); }
        Op::SetF { obj, field, val, repr } => { e.u8(23); e.u16(*obj); e.u32(*field); e.u16(*val); e.u8(repr.to_u8()); }
        Op::Own { dst, src, ty } => { e.u8(24); e.u16(*dst); e.u16(*src); e.u32(*ty); }
        Op::MakePtr { dst, src, ty } => { e.u8(89); e.u16(*dst); e.u16(*src); e.u32(*ty); }
        Op::CloneVal { dst, src, ty } => { e.u8(91); e.u16(*dst); e.u16(*src); e.u32(*ty); }
        Op::ValEq { dst, a, b, ty, eq } => { e.u8(92); e.u16(*dst); e.u16(*a); e.u16(*b); e.u32(*ty); e.u8(*eq as u8); }
        Op::OnDrop { obj, cleanup } => { e.u8(90); e.u16(*obj); e.u16(*cleanup); }
        Op::ArrNew { dst, ty, len, repr } => { e.u8(25); e.u16(*dst); e.u32(*ty); e.u16(*len); e.u8(repr.to_u8()); }
        Op::ArrLit { dst, ty, elems } => { e.u8(26); e.u16(*dst); e.u32(*ty); e.u16s(elems); }
        Op::ArrGet { dst, arr, idx, repr } => { e.u8(27); e.u16(*dst); e.u16(*arr); e.u16(*idx); e.u8(repr.to_u8()); }
        Op::ArrSet { arr, idx, val, repr } => { e.u8(28); e.u16(*arr); e.u16(*idx); e.u16(*val); e.u8(repr.to_u8()); }
        Op::EnumNew { dst, ty, member } => { e.u8(29); e.u16(*dst); e.u32(*ty); e.u32(*member); }
        Op::TidOf { dst, obj } => { e.u8(38); e.u16(*dst); e.u16(*obj); }
        Op::IsType { dst, obj, want } => { e.u8(39); e.u16(*dst); e.u16(*obj); e.u32(*want); }
        Op::IsTrait { dst, obj, want } => { e.u8(40); e.u16(*dst); e.u16(*obj); e.u32(*want); }
        Op::Unbox { dst, box_, ty } => { e.u8(41); e.u16(*dst); e.u16(*box_); e.u32(*ty); }
        Op::Box { dst, val, ty } => { e.u8(42); e.u16(*dst); e.u16(*val); e.u32(*ty); }
        Op::MakeClosure { dst, func, captures } => { e.u8(43); e.u16(*dst); e.u32(*func); e.u16s(captures); }
        Op::Panic { msg } => { e.u8(44); e.u16(*msg); }
        Op::Assert { cond, msg } => { e.u8(45); e.u16(*cond); e.u8opt(msg); }
        Op::LoopHead => e.u8(46),
        Op::Conv { dst, src, from, to } => { e.u8(47); e.u16(*dst); e.u16(*src); e.u8(from.to_u8()); e.u8(to.to_u8()); }
        Op::StrCharAt { dst, s, idx } => { e.u8(48); e.u16(*dst); e.u16(*s); e.u16(*idx); }
        Op::ArrayCmp { eq, dst, a, b } => { e.u8(85); e.u8(*eq as u8); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::ArrGetF { dst, obj, field, idx, repr } => { e.u8(87); e.u16(*dst); e.u16(*obj); e.u32(*field); e.u16(*idx); e.u8(repr.to_u8()); }
        Op::ArrSetF { obj, field, idx, val, repr } => { e.u8(88); e.u16(*obj); e.u32(*field); e.u16(*idx); e.u16(*val); e.u8(repr.to_u8()); }
        Op::ArrGetRef { dst, arr, idx, ty } => { e.u8(30); e.u16(*dst); e.u16(*arr); e.u16(*idx); e.u32(*ty); }
    }
}

fn decode_op(d: &mut Dec) -> Result<Op, String> {
    Ok(match d.u8()? {
        0 => Op::Mov { dst: d.u16()?, src: d.u16()? },
        1 => Op::MovRef { dst: d.u16()?, src: d.u16()? },
        2 => Op::Const { dst: d.u16()?, k: d.u32()? },
        3 => Op::ConstRaw { dst: d.u16()?, bits: d.u64()? },
        8 => Op::Not { dst: d.u16()?, a: d.u16()? },
        10 => Op::StrCmp { eq: d.u8()? != 0, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        11 => Op::RefEq { eq: d.u8()? != 0, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        12 => Op::Jmp { target: d.u32()? },
        13 => Op::Br { cond: d.u16()?, then_t: d.u32()?, else_t: d.u32()? },
        14 => {
            let idx = d.u16()?;
            let n = d.u32()? as usize;
            let mut table = Vec::with_capacity(n);
            for _ in 0..n {
                table.push(d.u32()?);
            }
            let default = d.u32()?;
            Op::BrTable { idx, table, default }
        }
        15 => Op::Call { func: d.u32()?, args: d.u16s()?, dst: d.u8opt()? },
        16 => Op::CallM { func: d.u32()?, recv: d.u16()?, args: d.u16s()?, dst: d.u8opt()? },
        17 => Op::CallI { slot: d.u32()?, recv: d.u16()?, args: d.u16s()?, dst: d.u8opt()? },
        18 => Op::CallNat { nat: nat(d.u8()?)?, recv: d.u8opt()?, args: d.u16s()?, dst: d.u8opt()? },
        19 => Op::CallFn { fval: d.u16()?, args: d.u16s()?, dst: d.u8opt()? },
        20 => Op::Ret { val: d.u8opt()? },
        21 => Op::NewCell { dst: d.u16()?, ty: d.u32()? },
        22 => Op::GetF { dst: d.u16()?, obj: d.u16()?, field: d.u32()?, repr: repr(d.u8()?)? },
        23 => Op::SetF { obj: d.u16()?, field: d.u32()?, val: d.u16()?, repr: repr(d.u8()?)? },
        24 => Op::Own { dst: d.u16()?, src: d.u16()?, ty: d.u32()? },
        89 => Op::MakePtr { dst: d.u16()?, src: d.u16()?, ty: d.u32()? },
        91 => Op::CloneVal { dst: d.u16()?, src: d.u16()?, ty: d.u32()? },
        92 => Op::ValEq { dst: d.u16()?, a: d.u16()?, b: d.u16()?, ty: d.u32()?, eq: d.u8()? != 0 },
        90 => Op::OnDrop { obj: d.u16()?, cleanup: d.u16()? },
        25 => Op::ArrNew { dst: d.u16()?, ty: d.u32()?, len: d.u16()?, repr: repr(d.u8()?)? },
        26 => Op::ArrLit { dst: d.u16()?, ty: d.u32()?, elems: d.u16s()? },
        27 => Op::ArrGet { dst: d.u16()?, arr: d.u16()?, idx: d.u16()?, repr: repr(d.u8()?)? },
        28 => Op::ArrSet { arr: d.u16()?, idx: d.u16()?, val: d.u16()?, repr: repr(d.u8()?)? },
        29 => Op::EnumNew { dst: d.u16()?, ty: d.u32()?, member: d.u32()? },
        38 => Op::TidOf { dst: d.u16()?, obj: d.u16()? },
        39 => Op::IsType { dst: d.u16()?, obj: d.u16()?, want: d.u32()? },
        40 => Op::IsTrait { dst: d.u16()?, obj: d.u16()?, want: d.u32()? },
        41 => Op::Unbox { dst: d.u16()?, box_: d.u16()?, ty: d.u32()? },
        42 => Op::Box { dst: d.u16()?, val: d.u16()?, ty: d.u32()? },
        43 => Op::MakeClosure { dst: d.u16()?, func: d.u32()?, captures: d.u16s()? },
        44 => Op::Panic { msg: d.u16()? },
        45 => Op::Assert { cond: d.u16()?, msg: d.u8opt()? },
        46 => Op::LoopHead,
        47 => Op::Conv { dst: d.u16()?, src: d.u16()?, from: prim(d.u8()?)?, to: prim(d.u8()?)? },
        48 => Op::StrCharAt { dst: d.u16()?, s: d.u16()?, idx: d.u16()? },
        49 => Op::MakeRecord { dst: d.u16()?, ty: d.u32()?, vals: d.u16s()? },
        50 => Op::AddF { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        51 => Op::SubF { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        52 => Op::MulF { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        53 => Op::DivF { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        54 => Op::ModF { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        55 => Op::NegF { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()? },
        56 => Op::EqF { dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        57 => Op::NeF { dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        58 => Op::LtF { dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        59 => Op::GtF { dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        60 => Op::LeF { dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        61 => Op::GeF { dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        62 => Op::AddI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        63 => Op::SubI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        64 => Op::MulI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        65 => Op::DivI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        66 => Op::ModI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        67 => Op::WAddI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        68 => Op::WSubI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        69 => Op::WMulI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        72 => Op::AndI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        73 => Op::OrI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        74 => Op::XorI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        75 => Op::ShlI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        76 => Op::ShrI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        77 => Op::WrapShlI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        78 => Op::EqI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        79 => Op::NeI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        80 => Op::LtI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        81 => Op::GtI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        82 => Op::LeI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        83 => Op::GeI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        84 => Op::NegI { prim: prim(d.u8()?)?, dst: d.u16()?, a: d.u16()? },
        85 => Op::ArrayCmp { eq: d.u8()? != 0, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        87 => Op::ArrGetF { dst: d.u16()?, obj: d.u16()?, field: d.u32()?, idx: d.u16()?, repr: repr(d.u8()?)? },
        88 => Op::ArrSetF { obj: d.u16()?, field: d.u32()?, idx: d.u16()?, val: d.u16()?, repr: repr(d.u8()?)? },
        30 => Op::ArrGetRef { dst: d.u16()?, arr: d.u16()?, idx: d.u16()?, ty: d.u32()? },
        t => return Err(format!("bad opcode {t}")),
    })
}

fn prim(b: u8) -> Result<PrimTy, String> {
    PrimTy::from_u8(b).ok_or_else(|| format!("bad prim tag {b}"))
}

fn repr(b: u8) -> Result<Repr, String> {
    Repr::from_u8(b).ok_or_else(|| format!("bad repr tag {b}"))
}

fn nat(b: u8) -> Result<Nat, String> {
    Ok(match b {
        0 => Nat::Str, 1 => Nat::Concat, 2 => Nat::StrLen,
        3 => Nat::ArrLen, 4 => Nat::StrJoin, 5 => Nat::StrSlice,
        _ => return Err("bad nat tag".into()),
    })
}

// ---- little-endian encoder/decoder ----

#[derive(Default)]
struct Enc {
    out: Vec<u8>,
}
impl Enc {
    fn bytes(&mut self, b: &[u8]) {
        self.out.extend_from_slice(b);
    }
    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u32(s.len() as u32);
        self.bytes(s.as_bytes());
    }
    fn tys(&mut self, v: &[TypeId]) {
        self.u32(v.len() as u32);
        for t in v {
            self.u32(*t);
        }
    }
    fn u16s(&mut self, v: &[u16]) {
        self.u32(v.len() as u32);
        for x in v {
            self.u16(*x);
        }
    }
    fn u8opt(&mut self, v: &Option<u16>) {
        match v {
            Some(x) => {
                self.u8(1);
                self.u16(*x);
            }
            None => self.u8(0),
        }
    }
}

struct Dec<'a> {
    b: &'a [u8],
    pos: usize,
}
impl<'a> Dec<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.pos + n > self.b.len() {
            return Err("truncated module binary".into());
        }
        let s = &self.b[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn bytes(&mut self, out: &mut [u8]) -> Result<(), String> {
        out.copy_from_slice(self.take(out.len())?);
        Ok(())
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64, String> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }
    fn i64(&mut self) -> Result<i64, String> {
        Ok(self.u64()? as i64)
    }
    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_bits(self.u64()?))
    }
    fn str(&mut self) -> Result<String, String> {
        let n = self.u32()? as usize;
        let b = self.take(n)?;
        String::from_utf8(b.to_vec()).map_err(|_| "bad utf8 in binary".into())
    }
    fn tys(&mut self) -> Result<Vec<TypeId>, String> {
        let n = self.u32()? as usize;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.u32()?);
        }
        Ok(v)
    }
    fn u16s(&mut self) -> Result<Vec<u16>, String> {
        let n = self.u32()? as usize;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.u16()?);
        }
        Ok(v)
    }
    fn u8opt(&mut self) -> Result<Option<u16>, String> {
        Ok(if self.u8()? != 0 { Some(self.u16()?) } else { None })
    }
}
