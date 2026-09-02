//! Module binary — RFC 0033 §1: versioned, deterministic, hash-stable
//! serialization of the linked module. v1 links a single module: type ids
//! in the binary are program-global (link-time rebase, RFC 0035 §1, lands
//! with multi-module).

use crate::heap::Slot;
use crate::ops::*;
use crate::types::{FieldInfo, PrimTy, RutType, TyKind, TypeId, TypeTable};

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
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConstVal {
    I64(i64),
    F64(f64),
    Bool(bool),
    Char(char),
    Str(String),
}

#[derive(Clone, Debug, Default)]
pub struct Program {
    pub name: String,
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
        e.u32(t.size);
        e.u32(t.align);
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
        TyKind::Vec { elem } => {
            e.u8(3);
            e.u32(*elem);
        }
        TyKind::Array { elem, len } => {
            e.u8(4);
            e.u32(*elem);
            e.u32(*len);
        }
        TyKind::Enum { members } => {
            e.u8(5);
            e.u32(members.len() as u32);
            for (n, v) in members {
                e.str(n);
                e.i64(*v);
            }
        }
        TyKind::Option { elem } => {
            e.u8(6);
            e.u32(*elem);
        }
        TyKind::Result { ok, err } => {
            e.u8(7);
            e.u32(*ok);
            e.u32(*err);
        }
        TyKind::Data { fields } => {
            e.u8(8);
            e.u32(fields.len() as u32);
            for f in fields {
                e.str(&f.name);
                e.u32(f.ty);
                e.u32(f.offset);
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
        let size = d.u32()?;
        let align = d.u32()?;
        let kind = decode_kind(&mut d)?;
        types.types.push(RutType { name: tname, kind, size, align });
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
        funcs.push(FuncCode { name: fname, params, ret, is_method, n_captures, regs, code, spans });
    }
    let nexp = d.u32()? as usize;
    let mut exports = Vec::with_capacity(nexp);
    for _ in 0..nexp {
        let n = d.str()?;
        let f = d.u32()?;
        exports.push((n, f));
    }
    Ok(Program { name, types, traits, trait_slots, vtables, consts, funcs, exports })
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
        3 => TyKind::Vec { elem: d.u32()? },
        4 => TyKind::Array { elem: d.u32()?, len: d.u32()? },
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
        6 => TyKind::Option { elem: d.u32()? },
        7 => TyKind::Result { ok: d.u32()?, err: d.u32()? },
        8 => {
            let n = d.u32()? as usize;
            let mut fields = Vec::with_capacity(n);
            for _ in 0..n {
                let name = d.str()?;
                let ty = d.u32()?;
                let offset = d.u32()?;
                fields.push(FieldInfo { name, ty, offset });
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
        t => return Err(format!("bad type kind tag {t}")),
    })
}

fn encode_op(e: &mut Enc, op: &Op) {
    match op {
        Op::Mov { dst, src } => { e.u8(0); e.u16(*dst); e.u16(*src); }
        Op::MovRef { dst, src } => { e.u8(1); e.u16(*dst); e.u16(*src); }
        Op::Const { dst, k } => { e.u8(2); e.u16(*dst); e.u32(*k); }
        Op::ConstRaw { dst, bits } => { e.u8(3); e.u16(*dst); e.u64(*bits); }
        Op::Arith { op, ty, dst, a, b } => { e.u8(4); e.u8(*op as u8); e.u32(*ty); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::Wrap { op, ty, dst, a, b } => { e.u8(5); e.u8(*op as u8); e.u32(*ty); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::Bit { op, ty, dst, a, b } => { e.u8(6); e.u8(*op as u8); e.u32(*ty); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::Cmp { op, ty, dst, a, b } => { e.u8(7); e.u8(*op as u8); e.u32(*ty); e.u16(*dst); e.u16(*a); e.u16(*b); }
        Op::Not { dst, a } => { e.u8(8); e.u16(*dst); e.u16(*a); }
        Op::Neg { ty, dst, a } => { e.u8(9); e.u32(*ty); e.u16(*dst); e.u16(*a); }
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
        Op::GetF { dst, obj, field } => { e.u8(22); e.u16(*dst); e.u16(*obj); e.u32(*field); }
        Op::SetF { obj, field, val } => { e.u8(23); e.u16(*obj); e.u32(*field); e.u16(*val); }
        Op::Own { dst, src, ty } => { e.u8(24); e.u16(*dst); e.u16(*src); e.u32(*ty); }
        Op::ArrNew { dst, ty, len } => { e.u8(25); e.u16(*dst); e.u32(*ty); e.u16(*len); }
        Op::ArrLit { dst, ty, elems } => { e.u8(26); e.u16(*dst); e.u32(*ty); e.u16s(elems); }
        Op::ArrGet { dst, arr, idx } => { e.u8(27); e.u16(*dst); e.u16(*arr); e.u16(*idx); }
        Op::ArrSet { arr, idx, val } => { e.u8(28); e.u16(*arr); e.u16(*idx); e.u16(*val); }
        Op::EnumNew { dst, ty, member } => { e.u8(29); e.u16(*dst); e.u32(*ty); e.u32(*member); }
        Op::OptSome { dst, ty, val } => { e.u8(30); e.u16(*dst); e.u32(*ty); e.u16(*val); }
        Op::OptNone { dst, ty } => { e.u8(31); e.u16(*dst); e.u32(*ty); }
        Op::ResOk { dst, ty, val } => { e.u8(32); e.u16(*dst); e.u32(*ty); e.u16(*val); }
        Op::ResErr { dst, ty, val } => { e.u8(33); e.u16(*dst); e.u32(*ty); e.u16(*val); }
        Op::SumIs { dst, v, want_err } => { e.u8(34); e.u16(*dst); e.u16(*v); e.u8(*want_err as u8); }
        Op::Unwrap { dst, v, want_err } => { e.u8(35); e.u16(*dst); e.u16(*v); e.u8(*want_err as u8); }
        Op::UnwrapOr { dst, v, default } => { e.u8(36); e.u16(*dst); e.u16(*v); e.u16(*default); }
        Op::Expect { dst, v, msg } => { e.u8(37); e.u16(*dst); e.u16(*v); e.u16(*msg); }
        Op::TidOf { dst, obj } => { e.u8(38); e.u16(*dst); e.u16(*obj); }
        Op::IsType { dst, obj, want } => { e.u8(39); e.u16(*dst); e.u16(*obj); e.u32(*want); }
        Op::IsTrait { dst, obj, want } => { e.u8(40); e.u16(*dst); e.u16(*obj); e.u32(*want); }
        Op::Unbox { dst, box_, ty } => { e.u8(41); e.u16(*dst); e.u16(*box_); e.u32(*ty); }
        Op::Box { dst, val, ty } => { e.u8(42); e.u16(*dst); e.u16(*val); e.u32(*ty); }
        Op::MakeClosure { dst, func, captures } => { e.u8(43); e.u16(*dst); e.u32(*func); e.u16s(captures); }
        Op::Panic { msg } => { e.u8(44); e.u16(*msg); }
        Op::Assert { cond, msg } => { e.u8(45); e.u16(*cond); e.u8opt(msg); }
        Op::LoopHead => e.u8(46),
        Op::Conv { dst, src, from, to } => { e.u8(47); e.u16(*dst); e.u16(*src); e.u32(*from); e.u32(*to); }
        Op::StrCharAt { dst, s, idx } => { e.u8(48); e.u16(*dst); e.u16(*s); e.u16(*idx); }
    }
}

fn decode_op(d: &mut Dec) -> Result<Op, String> {
    Ok(match d.u8()? {
        0 => Op::Mov { dst: d.u16()?, src: d.u16()? },
        1 => Op::MovRef { dst: d.u16()?, src: d.u16()? },
        2 => Op::Const { dst: d.u16()?, k: d.u32()? },
        3 => Op::ConstRaw { dst: d.u16()?, bits: d.u64()? },
        4 => Op::Arith { op: arith(d.u8()?)?, ty: d.u32()?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        5 => Op::Wrap { op: arith(d.u8()?)?, ty: d.u32()?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        6 => Op::Bit { op: bitop(d.u8()?)?, ty: d.u32()?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        7 => Op::Cmp { op: cmpop(d.u8()?)?, ty: d.u32()?, dst: d.u16()?, a: d.u16()?, b: d.u16()? },
        8 => Op::Not { dst: d.u16()?, a: d.u16()? },
        9 => Op::Neg { ty: d.u32()?, dst: d.u16()?, a: d.u16()? },
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
        22 => Op::GetF { dst: d.u16()?, obj: d.u16()?, field: d.u32()? },
        23 => Op::SetF { obj: d.u16()?, field: d.u32()?, val: d.u16()? },
        24 => Op::Own { dst: d.u16()?, src: d.u16()?, ty: d.u32()? },
        25 => Op::ArrNew { dst: d.u16()?, ty: d.u32()?, len: d.u16()? },
        26 => Op::ArrLit { dst: d.u16()?, ty: d.u32()?, elems: d.u16s()? },
        27 => Op::ArrGet { dst: d.u16()?, arr: d.u16()?, idx: d.u16()? },
        28 => Op::ArrSet { arr: d.u16()?, idx: d.u16()?, val: d.u16()? },
        29 => Op::EnumNew { dst: d.u16()?, ty: d.u32()?, member: d.u32()? },
        30 => Op::OptSome { dst: d.u16()?, ty: d.u32()?, val: d.u16()? },
        31 => Op::OptNone { dst: d.u16()?, ty: d.u32()? },
        32 => Op::ResOk { dst: d.u16()?, ty: d.u32()?, val: d.u16()? },
        33 => Op::ResErr { dst: d.u16()?, ty: d.u32()?, val: d.u16()? },
        34 => Op::SumIs { dst: d.u16()?, v: d.u16()?, want_err: d.u8()? != 0 },
        35 => Op::Unwrap { dst: d.u16()?, v: d.u16()?, want_err: d.u8()? != 0 },
        36 => Op::UnwrapOr { dst: d.u16()?, v: d.u16()?, default: d.u16()? },
        37 => Op::Expect { dst: d.u16()?, v: d.u16()?, msg: d.u16()? },
        38 => Op::TidOf { dst: d.u16()?, obj: d.u16()? },
        39 => Op::IsType { dst: d.u16()?, obj: d.u16()?, want: d.u32()? },
        40 => Op::IsTrait { dst: d.u16()?, obj: d.u16()?, want: d.u32()? },
        41 => Op::Unbox { dst: d.u16()?, box_: d.u16()?, ty: d.u32()? },
        42 => Op::Box { dst: d.u16()?, val: d.u16()?, ty: d.u32()? },
        43 => Op::MakeClosure { dst: d.u16()?, func: d.u32()?, captures: d.u16s()? },
        44 => Op::Panic { msg: d.u16()? },
        45 => Op::Assert { cond: d.u16()?, msg: d.u8opt()? },
        46 => Op::LoopHead,
        47 => Op::Conv { dst: d.u16()?, src: d.u16()?, from: d.u32()?, to: d.u32()? },
        48 => Op::StrCharAt { dst: d.u16()?, s: d.u16()?, idx: d.u16()? },
        t => return Err(format!("bad opcode {t}")),
    })
}

fn arith(b: u8) -> Result<ArithOp, String> {
    Ok(match b {
        0 => ArithOp::Add, 1 => ArithOp::Sub, 2 => ArithOp::Mul, 3 => ArithOp::Div, 4 => ArithOp::Mod,
        _ => return Err("bad arith tag".into()),
    })
}
fn bitop(b: u8) -> Result<BitOp, String> {
    Ok(match b {
        0 => BitOp::And, 1 => BitOp::Or, 2 => BitOp::Xor, 3 => BitOp::Shl, 4 => BitOp::Shr,
        _ => return Err("bad bitop tag".into()),
    })
}
fn cmpop(b: u8) -> Result<CmpOp, String> {
    Ok(match b {
        0 => CmpOp::Eq, 1 => CmpOp::Ne, 2 => CmpOp::Lt, 3 => CmpOp::Gt, 4 => CmpOp::Le, 5 => CmpOp::Ge,
        _ => return Err("bad cmp tag".into()),
    })
}
fn nat(b: u8) -> Result<Nat, String> {
    Ok(match b {
        0 => Nat::Print, 1 => Nat::Str, 2 => Nat::Concat, 3 => Nat::StrLen,
        4 => Nat::VecNew, 5 => Nat::VecZeroed, 6 => Nat::VecFrom, 7 => Nat::VecLen,
        8 => Nat::VecPush, 9 => Nat::VecPop,
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

/// A const-pool string materialized into a heap slot at load (immortal:
/// the module keeps the Slot alive for the program's lifetime).
pub fn const_to_slot(c: &ConstVal, heap: &crate::heap::Heap) -> Result<Slot, String> {
    Ok(match c {
        ConstVal::I64(v) => Slot::int(*v),
        ConstVal::F64(v) => Slot::float(*v),
        ConstVal::Bool(v) => Slot::bool(*v),
        ConstVal::Char(v) => Slot::ch(*v),
        ConstVal::Str(s) => heap
            .alloc_str(s.clone())
            .map_err(|t| format!("load: {}", t.msg))?,
    })
}
