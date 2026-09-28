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

// ---- load-time rebase (the compiled-bundle mount) ----
//
// A decoded `.rutc` carries its compiler-emitted packed `(scope, local)`
// ids verbatim, but NOT the type table's scope bookkeeping (`packed`,
// `scope`, `scope_base` — the wire's surface section carries the same
// blocks, so the loader rebuilds the bookkeeping from it). A mount
// assigns the program a fresh scope and rewrites every scope half
// through a map; boot ids are global by construction and pass through.

/// Walk every packed `TypeId` a program carries: the type table's
/// descriptors, trait methods, consts, function signatures and op
/// operands, and the surface rows. The scope halves of these ids are
/// the foreign references a rebase must rewrite.
fn walk_type_ids(prog: &Program, f: &mut impl FnMut(TypeId)) {
    fn kind(kind: &TyKind, f: &mut impl FnMut(TypeId)) {
        match kind {
            TyKind::Nil
            | TyKind::Prim(_)
            | TyKind::Str
            | TyKind::Bytes
            | TyKind::Opaque
            | TyKind::Trace
            | TyKind::StrBuf
            | TyKind::DisposalContext
            | TyKind::Enum { .. }
            | TyKind::TraitObj { .. } => {}
            TyKind::Array { elem } | TyKind::Weak { elem } | TyKind::Opt { elem } => f(*elem),
            TyKind::Data { fields } => for fl in fields {
                f(fl.ty);
            },
            TyKind::Fn { params, ret } => {
                for p in params {
                    f(*p);
                }
                f(*ret);
            }
        }
    }
    // op operands that carry a TypeId — the read-only twin of `remap_op`'s
    // type arms (func ids are walked separately below)
    fn op_tys(op: &Op, f: &mut impl FnMut(TypeId)) {
        match op {
            Op::NewCell { ty, .. }
            | Op::MakeRecord { ty, .. }
            | Op::Own { ty, .. }
            | Op::MakeOpt { ty, .. }
            | Op::WeakNew { ty, .. }
            | Op::ArrNew { ty, .. }
            | Op::ArrLit { ty, .. }
            | Op::EnumNew { ty, .. }
            | Op::IsType { want: ty, .. }
            | Op::Unbox { ty, .. }
            | Op::Box { ty, .. } => f(*ty),
            _ => {}
        }
    }
    let mut id = |t: TypeId| f(t);
    for t in &prog.types.types {
        kind(&t.kind, &mut id);
    }
    for tr in &prog.traits {
        for m in &tr.methods {
            for &p in &m.params {
                id(p);
            }
            id(m.ret);
        }
    }
    for c in &prog.consts {
        if let ConstVal::TypeId(t) = c {
            id(*t);
        }
    }
    for fc in &prog.funcs {
        for &p in &fc.params {
            id(p);
        }
        id(fc.ret);
        for &r in &fc.regs {
            id(r);
        }
        for op in &fc.code {
            op_tys(op, &mut id);
        }
    }
    let s = &prog.surface;
    for f in &s.funcs {
        for &p in &f.params {
            id(p);
        }
        id(f.ret);
    }
    for c in &s.consts {
        id(c.ty);
    }
    for t in &s.types {
        kind(&t.kind, &mut id);
    }
    for im in &s.impls {
        id(im.target);
    }
    for (t, _, _) in &s.native_impls {
        id(*t);
    }
}

/// The scope ids a program's packed ids reference, boot excluded: its
/// own block plus every used (foreign) block. `types` and the surface
/// carry the block keys directly; ops and descriptors carry them in id
/// halves — including function references, whose scope half names the
/// exporting module.
pub fn foreign_scopes(prog: &Program) -> std::collections::BTreeSet<crate::id::ScopeId> {
    let mut out = std::collections::BTreeSet::new();
    let mut push = |t: TypeId| {
        let s = crate::id::scope_of(t);
        if s != crate::id::BOOT_SCOPE {
            out.insert(s);
        }
    };
    walk_type_ids(prog, &mut push);
    for &(s, _) in &prog.surface.scope_blocks {
        if s != crate::id::BOOT_SCOPE {
            out.insert(s);
        }
    }
    for t in &prog.surface.type_exports {
        if let Some(s) = t.scope {
            if s != crate::id::BOOT_SCOPE {
                out.insert(s);
            }
        }
    }
    for fc in &prog.funcs {
        for op in &fc.code {
            match op {
                Op::Call { func, .. } | Op::CallM { func, .. } | Op::MakeClosure { func, .. } => {
                    let s = crate::id::scope_of(*func);
                    if s != crate::id::BOOT_SCOPE {
                        out.insert(s);
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// The scope a (packed) program interns its own types into: the carried
/// scope block with the greatest base offset. The own block always
/// closes the table (used blocks are appended first), so the maximum is
/// the own scope; a tie means an empty own block, whose scope no id
/// references, so either tie member answers.
pub fn own_scope(prog: &Program) -> Option<crate::id::ScopeId> {
    prog.surface
        .scope_blocks
        .iter()
        .copied()
        .filter(|&(s, _)| s != crate::id::BOOT_SCOPE)
        .max_by_key(|&(_, off)| off)
        .map(|(s, _)| s)
}

/// Rewrite every packed `(scope, local)` id's scope half through `map`
/// (boot ids pass through — they are global by construction), and
/// rebuild the type table's packed bookkeeping from the carried surface
/// blocks: `packed = true`, the program's scope, and each block's dense
/// base. A decoded program needs exactly this — the wire carries the
/// ids and the blocks but not the bookkeeping — and a mount calls it
/// once with the pack-time → load-time scope map. Dense (post-link)
/// programs must not be rebased: their ids are not scope-qualified.
pub fn rebase(mut prog: Program, map: &impl Fn(crate::id::ScopeId) -> crate::id::ScopeId) -> Program {
    let rb_t = |t: TypeId| -> TypeId {
        let s = crate::id::scope_of(t);
        if s == crate::id::BOOT_SCOPE {
            t
        } else {
            crate::id::pack(map(s), crate::id::local_of(t))
        }
    };
    let rb_f = |fid: u32| -> u32 {
        let s = crate::id::scope_of(fid);
        if s == crate::id::BOOT_SCOPE {
            fid
        } else {
            crate::id::pack(map(s), crate::id::local_of(fid))
        }
    };
    let nm = |n: IdentId| n;
    let tm = |t: u32| t;
    for t in prog.types.types.iter_mut() {
        t.kind = remap_kind(&t.kind, &rb_t, &nm, &tm);
    }
    for tr in prog.traits.iter_mut() {
        for m in tr.methods.iter_mut() {
            m.params = m.params.iter().map(|&p| rb_t(p)).collect();
            m.ret = rb_t(m.ret);
        }
    }
    for c in prog.consts.iter_mut() {
        if let ConstVal::TypeId(t) = c {
            *t = rb_t(*t);
        }
    }
    for fc in prog.funcs.iter_mut() {
        fc.params = fc.params.iter().map(|&p| rb_t(p)).collect();
        fc.ret = rb_t(fc.ret);
        fc.regs = fc.regs.iter().map(|&r| rb_t(r)).collect();
        fc.code = std::mem::take(&mut fc.code)
            .into_iter()
            .map(|op| remap_op(op, &rb_t, &rb_f, &tm, &tm, 0))
            .collect();
    }
    // dense module-local tables (vtables, disposal rows, exports, trait
    // slots) carry no scope halves — link rebases them by position
    let s = &mut prog.surface;
    for f in s.funcs.iter_mut() {
        f.params = f.params.iter().map(|&p| rb_t(p)).collect();
        f.ret = rb_t(f.ret);
    }
    for c in s.consts.iter_mut() {
        c.ty = rb_t(c.ty);
    }
    for t in s.types.iter_mut() {
        t.kind = remap_kind(&t.kind, &rb_t, &nm, &tm);
    }
    for &(scope, off) in &s.scope_blocks {
        let mapped = if scope == crate::id::BOOT_SCOPE { scope } else { map(scope) };
        set_scope_base(&mut prog.types, mapped, off);
    }
    for im in s.impls.iter_mut() {
        im.target = rb_t(im.target);
    }
    for (t, _, _) in s.native_impls.iter_mut() {
        *t = rb_t(*t);
    }
    // a decoded program's `scope` field is unset (the wire carries the
    // ids and the blocks, not the field) — the own scope comes from the
    // carried blocks, exactly like every own id's scope half
    let own = own_scope(&prog).unwrap_or(prog.scope);
    prog.scope = map(own);
    // the packed bookkeeping: the carried blocks are local offsets from
    // the boot prefix, so each block's dense base is boot_len + offset
    let boot_len = crate::types::TypeTable::boot().types.len() as u32;
    prog.types.packed = true;
    prog.types.boot_len = boot_len;
    prog.types.scope = prog.scope;
    prog
}

/// Grow `scope_base` to fit `scope` and record the block's dense base.
fn set_scope_base(tt: &mut TypeTable, scope: crate::id::ScopeId, off: u32) {
    let boot_len = TypeTable::boot().types.len() as u32;
    let need = scope as usize + 1;
    if tt.scope_base.len() < need {
        tt.scope_base.resize(need, 0);
    }
    tt.scope_base[scope as usize] = boot_len + off;
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
        disposal_impls: vec![None; boot],
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

    for mut m in modules {
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
        out.disposal_impls.resize(out.types.types.len(), None);

        // a module-local type index -> its global dense id. Boot-prefix
        // ids are global by construction (a boot id packs to itself); a
        // used block's dense index resolves through its scope's global
        // base; a module's own block offsets by its base. The vtable rows
        // and the disposal rows key the same ids.
        let gi_of = |i: u32| -> Option<u32> {
            if i < own_base {
                if i < m_boot {
                    // boot prefix: global by construction
                    return Some(i);
                }
                // a used block's dense index -> its scope -> global base
                let mut owner: Option<(crate::id::ScopeId, u32)> = None;
                for (s, &b) in m.types.scope_base.iter().enumerate() {
                    if s == crate::id::BOOT_SCOPE as usize || b < m_boot || b > i {
                        continue;
                    }
                    match owner {
                        Some((_, best)) if best > b => {}
                        _ => owner = Some((s as crate::id::ScopeId, b)),
                    }
                }
                let (s, b) = owner?;
                let gb = *scope_base.get(&s)?;
                Some(gb + (i - b))
            } else {
                Some(base + (i - own_base))
            }
        };

        // per-type vtables: keyed by type, slot-indexed, value = func id.
        // Rows for this module's OWN types are assigned; rows for types
        // this module only USES merge — an impl may live in a different
        // module than the type (RFC 0012 §2), and its fills must reach
        // the global row the owner module laid down.
        for (i, vt) in m.vtables.into_iter().enumerate() {
            let Some(gi) = gi_of(i as u32) else { continue };
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

        // per-type disposal rows: `(target type, dispose func id)` — the
        // same remap the vtable loop applies, func ids rebased like every
        // other func ref. A second fill is unreachable: the duplicate-
        // impl check above rejected a repeated (Disposal, type) pair.
        for (i, fid) in std::mem::take(&mut m.disposal_impls).into_iter().enumerate() {
            let Some(fid) = fid else { continue };
            let Some(gi) = gi_of(i as u32) else { continue };
            if gi as usize >= out.disposal_impls.len() {
                out.disposal_impls.resize(gi as usize + 1, None);
            }
            out.disposal_impls[gi as usize] = Some(map_func(fid));
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
                pos: f.pos,
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
        TyKind::Weak { elem } => TyKind::Weak { elem: map(*elem) },
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
        // the trace snapshot carries no type ids — the boot type is global
        TyKind::Trace => TyKind::Trace,
        // the builder carries no type ids — the boot type is global
        TyKind::StrBuf => TyKind::StrBuf,
        // the disposal context carries no type ids — the boot type is global
        TyKind::DisposalContext => TyKind::DisposalContext,
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
        Op::WeakNew { dst, src, ty } => Op::WeakNew { dst, src, ty: map(ty) },
        Op::WeakUpgrade { recv, dst, ty } => Op::WeakUpgrade { recv, dst, ty: map(ty) },
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
            pos: vec![],
            host_id: None,
        });
        p.exports.push((main, 0));
        p.vtables = vec![Vec::new(); p.types.types.len()];
        p.disposal_impls = vec![None; p.types.types.len()];
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
            pos: vec![],
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
