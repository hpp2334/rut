//! Link — merge module binaries into one program (RFC 0035 §1).
//!
//! At rest every module's `TypeId`s, function ids, trait ids and trait
//! slots are module-local. Linking rebases them into the global tables:
//! all modules share the same fixed **boot type prefix** (`TypeTable::boot`),
//! and each module's remaining types are appended after it. Function ids,
//! trait ids, trait-method slots and const-pool indices are offset; every
//! `TypeId` reachable from a type, trait, const, function or op is remapped
//! (RFC 0033 §1: "type_id<T>() constants are re-based with everything else").
//!
//! This pass is pure data — nothing runs — and is the compile/link half of
//! RFC 0035 §1; cyclic imports stay the loader's concern.

use crate::binary::{ConstVal, FuncCode, Program, TraitDesc, TraitMethod};
use crate::ops::Op;
use crate::types::{RutType, TyKind, TypeId, TypeTable};

/// A link failure — a load error, never a runtime trap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkError(pub String);

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for LinkError {}

/// Flatten a single module's packed `(scope, local)` ids into dense global
/// ids — the form the VM and the binary carry. Infallible: with one module
/// there is nothing to resolve across scopes.
pub fn flatten(prog: Program) -> Program {
    link(vec![prog]).expect("flatten: single-module link cannot fail")
}

/// Merge `modules` (in import order) into one [`Program`].
pub fn link(modules: Vec<Program>) -> Result<Program, LinkError> {
    if modules.is_empty() {
        return Err(LinkError("link: no modules".into()));
    }
    let mut seen = std::collections::HashSet::new();
    for m in &modules {
        if !seen.insert(m.name.clone()) {
            return Err(LinkError(format!("link: duplicate module `{}`", m.name)));
        }
    }

    let boot = boot_len();
    let mut out = Program {
        types: TypeTable::boot(),
        vtables: vec![Vec::new(); boot],
        ..Default::default()
    };
    out.name = modules
        .first()
        .map(|m| m.name.clone())
        .unwrap_or_default();

    // scope -> global dense base of that scope's type block (boot = 0)
    let mut scope_base: std::collections::HashMap<crate::id::ScopeId, u32> =
        std::collections::HashMap::new();
    scope_base.insert(crate::id::BOOT_SCOPE, 0);
    // scope -> global base of that scope's function block
    let mut func_scope_base: std::collections::HashMap<crate::id::ScopeId, u32> =
        std::collections::HashMap::new();
    func_scope_base.insert(crate::id::BOOT_SCOPE, 0);

    let mut func_off: u32 = 0;
    let mut const_off: u32 = 0;

    for m in modules {
        let nfuncs = m.funcs.len() as u32;
        let nconsts = m.consts.len() as u32;

        // where this module's OWN types land, and where its boot prefix ends
        let base = out.types.types.len() as u32;
        let m_boot = if m.types.packed { m.types.boot_len } else { boot as u32 };
        let packed = m.types.packed;
        // imported blocks carried in the table belong to their own scopes and
        // were appended by those modules — link only this module's own block
        let own_base = if packed {
            m.types
                .scope_base
                .get(m.types.scope as usize)
                .copied()
                .unwrap_or(m_boot)
        } else {
            m_boot
        };
        if packed {
            scope_base.insert(m.types.scope, base);
        }
        func_scope_base.insert(m.scope, func_off);
        // function ids: `0`-scoped are this module's own (dense); other
        // scopes name an imported module's block
        let map_func = |f: u32| -> u32 {
            if crate::id::scope_of(f) == crate::id::BOOT_SCOPE {
                f + func_off
            } else {
                func_scope_base
                    .get(&crate::id::scope_of(f))
                    .copied()
                    .unwrap_or(0)
                    + crate::id::local_of(f)
            }
        };
        // packed `(scope, local)` (compiler) or dense (pre-link) -> global dense
        let map = |id: TypeId| -> TypeId {
            if id == u32::MAX {
                return id; // TY_ANY sentinel — never a real type
            }
            if packed {
                let s = crate::id::scope_of(id);
                if s == crate::id::BOOT_SCOPE {
                    crate::id::local_of(id)
                } else {
                    scope_base.get(&s).copied().unwrap_or(0) + crate::id::local_of(id)
                }
            } else if id < m_boot {
                id
            } else {
                base + (id - m_boot)
            }
        };

        for t in m.types.types.iter().skip(own_base as usize) {
            out.types.types.push(RutType {
                name: t.name.clone(),
                kind: remap_kind(&t.kind, &map),
            });
        }
        out.vtables.resize(out.types.types.len(), Vec::new());

        // traits: trait ids are appended; methods' types are remapped
        let trait_off = out.traits.len() as u32;
        for tr in m.traits {
            out.traits.push(TraitDesc {
                name: tr.name,
                methods: tr
                    .methods
                    .into_iter()
                    .map(|tm| TraitMethod {
                        name: tm.name,
                        params: tm.params.into_iter().map(|p| map(p)).collect(),
                        ret: map(tm.ret),
                    })
                    .collect(),
            });
        }

        // trait-method slots are appended; `slot` operands index this table
        let slot_off = out.trait_slots.len() as u32;
        for (t, meth) in m.trait_slots {
            out.trait_slots.push((t + trait_off, meth));
        }

        // per-type vtables: keyed by type, slot-indexed, value = func id
        for (i, vt) in m.vtables.into_iter().enumerate() {
            if (i as u32) < own_base {
                continue;
            }
            let gi = (base + (i as u32 - own_base)) as usize;
            if gi >= out.vtables.len() {
                out.vtables.resize(gi + 1, Vec::new());
            }
            let mut nv = vec![None; out.trait_slots.len()];
            for (si, f) in vt.into_iter().enumerate() {
                if let Some(fid) = f {
                    nv[slot_off as usize + si] = Some(map_func(fid));
                }
            }
            out.vtables[gi] = nv;
        }

        // consts: `type_id` entries are rebased
        for c in m.consts {
            out.consts.push(match c {
                ConstVal::TypeId(t) => ConstVal::TypeId(map(t)),
                other => other,
            });
        }

        // funcs: signatures, register types and every op operand
        for f in m.funcs {
            out.funcs.push(FuncCode {
                name: f.name,
                params: f.params.into_iter().map(|p| map(p)).collect(),
                ret: map(f.ret),
                is_method: f.is_method,
                n_captures: f.n_captures,
                regs: f.regs.into_iter().map(|r| map(r)).collect(),
                code: f
                    .code
                    .into_iter()
                    .map(|op| remap_op(op, &map, &map_func, slot_off, trait_off, const_off))
                    .collect(),
                spans: f.spans,
                host: f.host,
            });
        }

        // exports: function ids
        for (n, fid) in m.exports {
            out.exports.push((n, map_func(fid)));
        }

        func_off += nfuncs;
        const_off += nconsts;
    }

    Ok(out)
}

/// Number of shared boot types every module's table starts with.
fn boot_len() -> usize {
    TypeTable::boot().types.len()
}

/// Remap the `TypeId`s inside a type descriptor.
fn remap_kind(kind: &TyKind, map: &impl Fn(TypeId) -> TypeId) -> TyKind {
    match kind {
        TyKind::Unit | TyKind::Prim(_) | TyKind::Str | TyKind::Bytes | TyKind::Opaque => {
            kind.clone()
        }
        TyKind::Array { elem } => TyKind::Array { elem: map(*elem) },
        TyKind::Enum { members } => TyKind::Enum { members: members.clone() },
        TyKind::Option { elem } => TyKind::Option { elem: map(*elem) },
        TyKind::Result { ok, err } => TyKind::Result { ok: map(*ok), err: map(*err) },
        TyKind::Data { fields } => TyKind::Data {
            fields: fields
                .iter()
                .map(|f| crate::types::FieldInfo {
                    name: f.name.clone(),
                    ty: map(f.ty),
                })
                .collect(),
        },
        // trait ids are NOT TypeIds; remapped with the trait table
        TyKind::TraitObj { trait_id } => TyKind::TraitObj { trait_id: *trait_id },
        TyKind::Fn { params, ret } => TyKind::Fn {
            params: params.iter().map(|&p| map(p)).collect(),
            ret: map(*ret),
        },
        TyKind::Ptr { elem } => TyKind::Ptr { elem: map(*elem) },
    }
}

/// Remap every id-bearing operand of an op. Ops without ids fall through.
fn remap_op(
    op: Op,
    map: &impl Fn(TypeId) -> TypeId,
    map_func: &impl Fn(u32) -> u32,
    slot_off: u32,
    trait_off: u32,
    const_off: u32,
) -> Op {
    match op {
        Op::Const { dst, k } => Op::Const { dst, k: k + const_off },
        Op::NewCell { dst, ty } => Op::NewCell { dst, ty: map(ty) },
        Op::MakeRecord { dst, ty, vals } => Op::MakeRecord { dst, ty: map(ty), vals },
        Op::Own { dst, src, ty } => Op::Own { dst, src, ty: map(ty) },
        Op::MakePtr { dst, src, ty } => Op::MakePtr { dst, src, ty: map(ty) },
        Op::ArrNew { dst, ty, len, repr } => Op::ArrNew { dst, ty: map(ty), len, repr },
        Op::ArrLit { dst, ty, elems } => Op::ArrLit { dst, ty: map(ty), elems },
        Op::EnumNew { dst, ty, member } => Op::EnumNew { dst, ty: map(ty), member },
        Op::OptSome { dst, ty, val } => Op::OptSome { dst, ty: map(ty), val },
        Op::OptNone { dst, ty } => Op::OptNone { dst, ty: map(ty) },
        Op::ResOk { dst, ty, val } => Op::ResOk { dst, ty: map(ty), val },
        Op::ResErr { dst, ty, val } => Op::ResErr { dst, ty: map(ty), val },
        Op::IsType { dst, obj, want } => Op::IsType { dst, obj, want: map(want) },
        Op::Unbox { dst, box_, ty } => Op::Unbox { dst, box_, ty: map(ty) },
        Op::Box { dst, val, ty } => Op::Box { dst, val, ty: map(ty) },
        Op::Call { func, args, dst } => Op::Call { func: map_func(func), args, dst },
        Op::CallM { func, recv, args, dst } => {
            Op::CallM { func: map_func(func), recv, args, dst }
        }
        Op::CallI { slot, recv, args, dst } => {
            Op::CallI { slot: slot + slot_off, recv, args, dst }
        }
        Op::MakeClosure { dst, func, captures } => Op::MakeClosure {
            dst,
            func: map_func(func),
            captures,
        },
        Op::IsTrait { dst, obj, want } => Op::IsTrait { dst, obj, want: want + trait_off },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary::FuncCode;
    use crate::ops::Op;
    use crate::types::{FieldInfo, TY_I32};

    fn module(name: &str, with_point: bool) -> Program {
        let mut p = Program::default();
        p.name = name.to_string();
        p.types = TypeTable::boot();
        if with_point {
            p.types.types.push(RutType {
                name: "Point".into(),
                kind: TyKind::Data {
                    fields: vec![FieldInfo { name: "x".into(), ty: TY_I32 }],
                },
            });
            let point = (p.types.types.len() - 1) as u32;
            p.consts.push(ConstVal::TypeId(point));
        }
        p.funcs.push(FuncCode {
            name: "main".into(),
            params: vec![],
            ret: TY_I32,
            is_method: false,
            n_captures: 0,
            regs: vec![TY_I32],
            code: vec![Op::Const { dst: 0, k: 0 }, Op::Ret { val: Some(0) }],
            spans: vec![],
            host: None,
        });
        p.exports.push(("main".into(), 0));
        p.vtables = vec![Vec::new(); p.types.types.len()];
        p
    }

    #[test]
    fn links_and_rebases_types_and_funcs() {
        let boot = TypeTable::boot().types.len();
        let a = module("a", true);
        let b = module("b", true);
        let out = link(vec![a, b]).expect("link");

        // two points appended after the shared boot prefix
        assert_eq!(out.types.types.len(), boot + 2);
        // type_id consts rebased: a's Point -> boot, b's Point -> boot + 1
        assert_eq!(out.consts[0], ConstVal::TypeId(boot as u32));
        assert_eq!(out.consts[1], ConstVal::TypeId(boot as u32 + 1));
        // func/const ids offset per module
        assert_eq!(out.funcs.len(), 2);
        assert_eq!(out.exports, vec![("main".into(), 0), ("main".into(), 1)]);
        assert_eq!(out.funcs[1].code[0], Op::Const { dst: 0, k: 1 });
        // boot prefix is not duplicated
        assert_eq!(out.types.types[TY_I32 as usize].name, "i32");
    }

    #[test]
    fn duplicate_module_is_an_error() {
        let err = link(vec![module("a", false), module("a", false)]).unwrap_err();
        assert!(err.to_string().contains("duplicate module `a`"), "{err}");
    }
}
