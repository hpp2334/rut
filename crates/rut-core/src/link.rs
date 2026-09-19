//! Link — merge module binaries into one program (RFC 0035 §1).
//!
//! At rest every module's `TypeId`s, function ids, trait ids and trait
//! slots are module-local. Linking rebases them into the global tables:
//! all modules share the same fixed **boot type prefix** (`TypeTable::boot`),
//! and each module's remaining types are appended after it. Function ids
//! and const-pool indices are offset; every `TypeId` reachable from a
//! type, trait, const, function or op is remapped (RFC 0033 §1: "type_id<T>()
//! constants are re-based with everything else").
//!
//! **Trait ids are global, not offset** (RFC 0012 §5): a trait bound as an
//! extern in one module and declared in another is ONE trait — the merged
//! table dedups by (mapped) name, so every module's `TraitObj` ids,
//! `IsTrait` probes and trait-method slots land on the same global trait.
//! Trait-method slots re-lay through the merged table's enumeration.
//!
//! Impl registrations (`surface.impls`) merge here too: a duplicate
//! `(trait, type)` pair — unobservable per-module, since trait impls may
//! live in any module (RFC 0012 §2) — is a link error. Vtable fills for
//! types a module merely *uses* merge into the owner module's row, so an
//! impl registered in any module reaches every call site.
//!
//! This pass is pure data — nothing runs — and is the compile/link half of
//! RFC 0035 §1; cyclic uses stay the loader's concern.

use crate::binary::{ConstVal, FuncCode, Program, TraitDesc, TraitMethod};
use crate::ops::Op;
use crate::sym::IdentId;
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

/// Merge `modules` (in use order) into one [`Program`].
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

    // the global trait table: mapped trait name -> global trait id. A
    // trait bound as an extern and declared in its owner are ONE trait —
    // dedup by name keeps every module's `TraitObj` ids, `IsTrait` wants
    // and slots pointing at the same global trait (RFC 0012 §5).
    let mut global_trait: std::collections::HashMap<IdentId, u32> =
        std::collections::HashMap::new();
    // global slot index: (global trait id, method) -> global slot
    let mut global_slot: std::collections::HashMap<(u32, u32), u32> =
        std::collections::HashMap::new();
    // (global trait id, global target type) -> the module that registered
    // the impl — a second registration is the link-time duplicate error
    // (RFC 0012 §2: the pair is only detectable here)
    let mut impl_owner: std::collections::HashMap<(u32, u32), String> =
        std::collections::HashMap::new();

    for m in modules {
        let nfuncs = m.funcs.len() as u32;
        let nconsts = m.consts.len() as u32;

        // where this module's OWN types land, and where its boot prefix ends
        let base = out.types.types.len() as u32;
        let m_boot = if m.types.packed { m.types.boot_len } else { boot as u32 };
        let packed = m.types.packed;
        // used blocks carried in the table belong to their own scopes and
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

        // names: merge this module's interner into the output's — the name
        // rebase mirrors the type rebase (RFC 0035 §1). Well-known ids are
        // the same in every interner and pass through untouched.
        let m_interner = m.interner;
        let wk = m_interner.well_known_len() as usize;
        let mut name_map: Vec<IdentId> = Vec::with_capacity(m_interner.names().len());
        for (i, n) in m_interner.names().iter().enumerate() {
            name_map.push(if i < wk {
                IdentId(i as u32)
            } else {
                out.interner.intern(n)
            });
        }
        let nm = |id: IdentId| -> IdentId {
            name_map.get(id.0 as usize).copied().unwrap_or(id)
        };
        // function ids: `0`-scoped are this module's own (dense); other
        // scopes name a used module's block
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

        // traits: the GLOBAL table dedups by mapped name (see the module
        // comment) — every trait id this module carries remaps through
        // `tmap`, and its slots re-lay through the merged enumeration
        let mut tmap: Vec<u32> = Vec::with_capacity(m.traits.len());
        for tr in m.traits.iter() {
            let key = nm(tr.name);
            let gid = match global_trait.get(&key) {
                Some(&g) => {
                    // one trait everywhere: the copies must agree on the
                    // member set (the slot layout hangs off it). Method
                    // SIGNATURES may differ in packed-id spelling (a
                    // consumer's copy normalizes trait-object types), so
                    // the check is count + names.
                    let prev = &out.traits[g as usize];
                    let same = prev.methods.len() == tr.methods.len()
                        && prev
                            .methods
                            .iter()
                            .zip(tr.methods.iter())
                            .all(|(a, b)| a.name == nm(b.name));
                    if !same {
                        return Err(LinkError(format!(
                            "link: trait `{}` has different members across modules",
                            out.interner.name(key)
                        )));
                    }
                    g
                }
                None => {
                    let g = out.traits.len() as u32;
                    let methods: Vec<TraitMethod> = tr
                        .methods
                        .iter()
                        .map(|tm| TraitMethod {
                            name: nm(tm.name),
                            params: tm.params.iter().map(|&p| map(p)).collect(),
                            ret: map(tm.ret),
                        })
                        .collect();
                    for meth in 0..methods.len() as u32 {
                        global_slot.insert((g, meth), out.trait_slots.len() as u32);
                        out.trait_slots.push((g, meth));
                    }
                    out.traits.push(TraitDesc { name: key, methods });
                    global_trait.insert(key, g);
                    g
                }
            };
            tmap.push(gid);
        }
        let tm = |id: u32| -> u32 { tmap.get(id as usize).copied().unwrap_or(id) };

        // trait-method slots: re-laid through the global enumeration — a
        // module's slot `s` names `(trait, method)` in ITS table, which
        // maps to the global trait and the global slot
        let mut slot_map: Vec<u32> = Vec::with_capacity(m.trait_slots.len());
        for (t, meth) in m.trait_slots.iter() {
            let g = tmap.get(*t as usize).copied().unwrap_or(*t);
            match global_slot.get(&(g, *meth)) {
                Some(&s) => slot_map.push(s),
                None => {
                    return Err(LinkError(format!(
                        "link: module `{}` names trait slot ({t}, {meth}) outside its trait table",
                        m.name
                    )));
                }
            }
        }
        // every compiler-emitted slot indexes its own table; the fallback
        // is unreachable for well-formed programs
        let sm = |slot: u32| -> u32 {
            slot_map.get(slot as usize).copied().unwrap_or(slot)
        };

        // impl registrations: duplicate (trait, type) pairs are a LINK
        // error (RFC 0012 §2). Generic-target impls stay per-module (the
        // target is a template) and are not exported.
        for im in m.surface.impls.iter() {
            let Some(&tg) = global_trait.get(&nm(im.trait_name)) else {
                continue; // the trait table above covers every declared trait
            };
            let key = (tg, map(im.target));
            match impl_owner.get(&key) {
                Some(owner) => {
                    let tname = out
                        .traits
                        .get(tg as usize)
                        .map(|t| out.interner.name(t.name).to_string())
                        .unwrap_or_else(|| "?".to_string());
                    let target = out.types.types.get(key.1 as usize)
                        .map(|t| out.interner.name(t.name).to_string())
                        .unwrap_or_else(|| "?".to_string());
                    return Err(LinkError(format!(
                        "link: duplicate impl `({}, {})` — `{}` and `{}` both register it (RFC 0012 §2: one impl per (trait, type) pair per program)",
                        tname, target, owner, m.name
                    )));
                }
                None => {
                    impl_owner.insert(key, m.name.clone());
                }
            }
        }

        for t in m.types.types.iter().skip(own_base as usize) {
            out.types.types.push(RutType {
                name: nm(t.name),
                kind: remap_kind(&t.kind, &map, &nm, &tm),
            });
        }
        out.vtables.resize(out.types.types.len(), Vec::new());

        // per-type vtables: keyed by type, slot-indexed, value = func id.
        // Rows for this module's OWN types are assigned; rows for types
        // this module only USES merge — an impl may live in a different
        // module than the type (RFC 0012 §2), and its fills must reach
        // the global row the owner module laid down.
        for (i, vt) in m.vtables.into_iter().enumerate() {
            let gi: u32 = if (i as u32) < own_base {
                if (i as u32) < m_boot {
                    // boot prefix: ids are global by construction (a boot
                    // id packs to itself, RFC 0035 §1) — a primitive
                    // trait-impl target's fill merges into the global
                    // boot row (RFC 0012 §2)
                    i as u32
                } else {
                    // a used block's dense index -> its scope -> global base
                    let mut owner: Option<(crate::id::ScopeId, u32)> = None;
                    for (s, &b) in m.types.scope_base.iter().enumerate() {
                        if s == crate::id::BOOT_SCOPE as usize || b < m_boot || b > i as u32 {
                            continue;
                        }
                        match owner {
                            Some((_, best)) if best > b => {}
                            _ => owner = Some((s as crate::id::ScopeId, b)),
                        }
                    }
                    let Some((s, b)) = owner else { continue };
                    let Some(&gb) = scope_base.get(&s) else { continue };
                    gb + (i as u32 - b)
                }
            } else {
                base + (i as u32 - own_base)
            };
            let gi = gi as usize;
            if gi >= out.vtables.len() {
                out.vtables.resize(gi + 1, Vec::new());
            }
            let row = &mut out.vtables[gi];
            if row.len() < out.trait_slots.len() {
                row.resize(out.trait_slots.len(), None);
            }
            for (si, f) in vt.into_iter().enumerate() {
                if let Some(fid) = f {
                    row[sm(si as u32) as usize] = Some(map_func(fid));
                }
            }
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
                name: nm(f.name),
                params: f.params.into_iter().map(|p| map(p)).collect(),
                ret: map(f.ret),
                is_method: f.is_method,
                n_captures: f.n_captures,
                regs: f.regs.into_iter().map(|r| map(r)).collect(),
                // pools are function-local: spans move verbatim, untouched
                argv: f.argv,
                labels: f.labels,
                code: f
                    .code
                    .into_iter()
                    .map(|op| remap_op(op, &map, &map_func, &sm, &tm, const_off))
                    .collect(),
                spans: f.spans,
                host_id: f.host_id.map(nm),
            });
        }

        // exports: function ids
        for (n, fid) in m.exports {
            out.exports.push((nm(n), map_func(fid)));
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

/// Remap the `TypeId`s and names inside a type descriptor. `tm` maps a
/// module-local trait id to the global one (`TyKind::TraitObj` carries a
/// trait id, not a `TypeId` — RFC 0015 §6).
fn remap_kind(
    kind: &TyKind,
    map: &impl Fn(TypeId) -> TypeId,
    nm: &impl Fn(IdentId) -> IdentId,
    tm: &impl Fn(u32) -> u32,
) -> TyKind {
    match kind {
        TyKind::Nil | TyKind::Prim(_) | TyKind::Str | TyKind::Bytes | TyKind::Opaque => {
            kind.clone()
        }
        TyKind::Array { elem } => TyKind::Array { elem: map(*elem) },
        TyKind::Enum { members } => TyKind::Enum {
            members: members
                .iter()
                .map(|(n, v)| (nm(*n), *v))
                .collect(),
        },
        TyKind::Data { fields } => TyKind::Data {
            fields: fields
                .iter()
                .map(|f| crate::types::FieldInfo {
                    name: nm(f.name),
                    ty: map(f.ty),
                })
                .collect(),
        },
        TyKind::TraitObj { trait_id } => TyKind::TraitObj { trait_id: tm(*trait_id) },
        TyKind::Fn { params, ret } => TyKind::Fn {
            params: params.iter().map(|&p| map(p)).collect(),
            ret: map(*ret),
        },
        TyKind::Opt { elem } => TyKind::Opt { elem: map(*elem) },
    }
}

/// Remap every id-bearing operand of an op. Ops without ids fall through.
/// `sm` re-lays trait-method slots through the global enumeration; `tm`
/// maps trait ids (`IsTrait` wants) to the global table.
fn remap_op(
    op: Op,
    map: &impl Fn(TypeId) -> TypeId,
    map_func: &impl Fn(u32) -> u32,
    sm: &impl Fn(u32) -> u32,
    tm: &impl Fn(u32) -> u32,
    const_off: u32,
) -> Op {
    match op {
        Op::Const { dst, k } => Op::Const { dst, k: k + const_off },
        Op::NewCell { dst, ty } => Op::NewCell { dst, ty: map(ty) },
        Op::MakeRecord { dst, ty, argv_off, argc } => Op::MakeRecord { dst, ty: map(ty), argv_off, argc },
        Op::Own { dst, src, ty } => Op::Own { dst, src, ty: map(ty) },
        Op::MakeOpt { dst, src, ty } => Op::MakeOpt { dst, src, ty: map(ty) },
        Op::ArrNew { dst, ty, len, repr } => Op::ArrNew { dst, ty: map(ty), len, repr },
        Op::ArrLit { dst, ty, argv_off, argc } => Op::ArrLit { dst, ty: map(ty), argv_off, argc },
        Op::EnumNew { dst, ty, member } => Op::EnumNew { dst, ty: map(ty), member },
        Op::IsType { dst, obj, want } => Op::IsType { dst, obj, want: map(want) },
        Op::Unbox { dst, box_, ty } => Op::Unbox { dst, box_, ty: map(ty) },
        Op::Box { dst, val, ty } => Op::Box { dst, val, ty: map(ty) },
        Op::Call { func, argv_off, argc, dst } => Op::Call { func: map_func(func), argv_off, argc, dst },
        Op::CallM { func, argv_off, argc, dst } => {
            Op::CallM { func: map_func(func), argv_off, argc, dst }
        }
        Op::CallI { slot, argv_off, argc, dst } => {
            Op::CallI { slot: sm(slot), argv_off, argc, dst }
        }
        Op::MakeClosure { dst, func, argv_off, argc } => Op::MakeClosure {
            dst,
            func: map_func(func),
            argv_off,
            argc,
        },
        Op::IsTrait { dst, obj, want } => Op::IsTrait { dst, obj, want: tm(want) },
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary::FuncCode;
    use crate::ops::Op;
    use crate::sym::MAIN;
    use crate::types::{FieldInfo, TY_I32};

    fn module(name: &str, with_point: bool) -> Program {
        let mut p = Program::default();
        p.name = name.to_string();
        p.types = TypeTable::boot();
        if with_point {
            let point = p.interner.intern("Point");
            let x = p.interner.intern("x");
            p.types.types.push(RutType {
                name: point,
                kind: TyKind::Data {
                    fields: vec![FieldInfo { name: x, ty: TY_I32 }],
                },
            });
            let pid = (p.types.types.len() - 1) as u32;
            p.consts.push(ConstVal::TypeId(pid));
        }
        let main = p.interner.intern("main");
        p.funcs.push(FuncCode {
            name: main,
            params: vec![],
            ret: TY_I32,
            is_method: false,
            n_captures: 0,
            regs: vec![TY_I32],
            argv: vec![],
            labels: vec![],
            code: vec![Op::Const { dst: 0, k: 0 }, Op::Ret { val: Some(0) }],
            spans: vec![],
            host_id: None,
        });
        p.exports.push((main, 0));
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
        assert_eq!(out.exports, vec![(MAIN, 0), (MAIN, 1)]);
        assert_eq!(out.funcs[1].code[0], Op::Const { dst: 0, k: 1 });
        // boot prefix is not duplicated
        assert_eq!(out.type_name(TY_I32), "i32");
        // the name rebase merged both interners: both Points resolve,
        // and each module's "x" field name is one shared symbol
        assert_eq!(out.type_name(boot as u32), "Point");
        let fields = match &out.types.types[boot].kind {
            TyKind::Data { fields } => fields.clone(),
            _ => panic!("expected a record"),
        };
        assert_eq!(out.name_of(fields[0].name), "x");
    }

    #[test]
    fn duplicate_module_is_an_error() {
        let err = link(vec![module("a", false), module("a", false)]).unwrap_err();
        assert!(err.to_string().contains("duplicate module `a`"), "{err}");
    }

    /// A module carrying one trait `Shape` (1 method) and naming it from
    /// an op. `slot`/`want` are module-local ids; both copies declare the
    /// "same" trait, so link must dedup them into ONE global trait.
    fn trait_module(name: &str) -> Program {
        let mut p = Program::default();
        p.name = name.to_string();
        p.types = TypeTable::boot();
        let shape = p.interner.intern("Shape");
        let area = p.interner.intern("area");
        p.traits.push(TraitDesc {
            name: shape,
            methods: vec![TraitMethod { name: area, params: vec![], ret: TY_I32 }],
        });
        p.trait_slots.push((0, 0));
        let main = p.interner.intern("main");
        p.funcs.push(FuncCode {
            name: main,
            params: vec![],
            ret: TY_I32,
            is_method: false,
            n_captures: 0,
            regs: vec![TY_I32],
            argv: vec![],
            labels: vec![],
            code: vec![
                Op::CallI { slot: 0, argv_off: 0, argc: 0, dst: 0 },
                Op::IsTrait { dst: 0, obj: 0, want: 0 },
                Op::Ret { val: Some(0) },
            ],
            spans: vec![],
            host_id: None,
        });
        p.exports.push((main, 0));
        p
    }

    #[test]
    fn same_trait_across_modules_merges_into_one_global_trait() {
        // a: declares the trait; b: binds it as an extern (its own copy of
        // the descriptor) and calls through a slot + probe
        let mut a = trait_module("a");
        a.surface.impls.push(crate::binary::SurfaceImpl {
            trait_name: a.interner.intern("Shape"),
            target: 0, // irrelevant here
            methods: vec![],
            methods_concrete: vec![],
        });
        let b = trait_module("b");
        let out = link(vec![a, b]).expect("link");
        assert_eq!(out.traits.len(), 1, "one global Shape");
        assert_eq!(out.trait_slots, vec![(0, 0)], "one global slot");
        for f in &out.funcs {
            assert!(
                f.code.iter().all(|op| match op {
                    Op::CallI { slot, .. } => *slot == 0,
                    Op::IsTrait { want, .. } => *want == 0,
                    _ => true,
                }),
                "slots and trait ids land on the global trait: {f:?}"
            );
        }
    }

    #[test]
    fn duplicate_impl_pair_is_a_link_error() {
        // module `a` (scope 1) declares Point and registers (Shape, Point);
        // module `b` (scope 2) binds a's Point and registers the same pair.
        // Per-module compiles cannot see each other, so the collision is
        // detectable only here (RFC 0012 §2).
        let boot = TypeTable::boot().types.len() as u32;
        let mut mk = |name: &str, scope: crate::id::ScopeId, target: TypeId| {
            let mut p = trait_module(name);
            p.scope = scope;
            let point = p.interner.intern("Point");
            // packed table carrying `a`'s block + own block (a real
            // module's layout: boot, used blocks, own types)
            p.types = TypeTable::boot_scoped(scope);
            p.types.types.push(RutType { name: point, kind: TyKind::Data { fields: vec![] } });
            p.types.scope_base.resize(3, 0);
            p.types.scope_base[1] = boot;
            p.types.scope_base[2] = boot;
            p.surface.impls.push(crate::binary::SurfaceImpl {
                trait_name: p.interner.intern("Shape"),
                target,
                methods: vec![],
                methods_concrete: vec![],
            });
            p
        };
        let a = mk("a", 1, crate::id::pack(1, 0));
        let b = mk("b", 2, crate::id::pack(1, 0));
        let err = link(vec![a, b]).unwrap_err();
        assert!(
            err.to_string().contains("duplicate impl `(Shape, Point)`"),
            "{err}"
        );
        assert!(err.to_string().contains("`a` and `b`"), "{err}");
    }
}
