//! Template materialization (v22) — the link lane's monomorphization.
//!
//! A template body is a `FuncCode` compiled once against `#<param>`
//! placeholder rows. Materializing it for a concrete argument list is a
//! pure data pass over the owner program: substitute every TypeId
//! (structural rebuild — equal shapes intern onto the rows the program
//! already carries), rewrite the repr bytes the substitution changed,
//! re-run the repr-keyed peephole folds, and remap the callee fids (the
//! caller drives those: concrete fids stay, template fids cascade to
//! their own materializations). The result is byte-equal to the
//! directory-compiled instantiation — the pinned law that makes the
//! in-binary templates a true replacement for the retired source
//! riding.
//!
//! The engine lives beside the compiler because it needs the exact
//! naming conventions the fused compiler mints (`Array<T>`, `?T`,
//! `(A, B)`, `Base<args>`); the interning dedups on (name, kind), so a
//! re-materialized shape always lands on the row a fresh compile would
//! have produced.

use std::collections::HashMap;

use rut_core::binary::{FuncCode, InstFn, InstFnKind, InstTy, Program};
use rut_core::ops::Op;
use rut_core::types::{arr_elem_repr, FieldInfo, RutType, TyKind, TypeId, TypeTable};
use rut_core::Interner;

/// A type substitution over an owner program's table: `#<param>`
/// placeholder rows → the request's concrete arguments, extended
/// structurally to every row the body mentions (a `SOpt<#T>` row
/// materializes to the `SOpt<JsonI64>` row, minting it — and its
/// owner-anchored ledger row — on first use).
pub struct Substitution<'a> {
    pub tt: &'a mut TypeTable,
    pub interner: &'a mut Interner,
    map: HashMap<TypeId, TypeId>,
    /// instantiation-shaped rows minted during the substitution — the
    /// caller appends them to the program's `inst_types` (owner-anchored
    /// identity, so link unifies mirrors onto ONE row program-wide)
    pub minted_inst_tys: Vec<InstTy>,
    /// the owner pkg spec the minted rows anchor to
    owner: String,
    /// base decl name text → the pkg that owns it, spelled the way the
    /// program's own ledger rows spell it (`pouch@<hash>`) — read off the
    /// window's stub/cascade rows. A minted `Vec<i32>` row must claim
    /// `pouch@…#Vec<…>` — the key a fresh compile claims — or link sees
    /// two ununified mirrors of one instantiation.
    foreign_owners: HashMap<String, String>,
}

impl<'a> Substitution<'a> {
    pub fn new(
        tt: &'a mut TypeTable,
        interner: &'a mut Interner,
        params: Vec<(TypeId, TypeId)>,
        owner: String,
    ) -> Substitution<'a> {
        Substitution {
            tt,
            interner,
            map: params.into_iter().collect(),
            minted_inst_tys: Vec::new(),
            owner,
            foreign_owners: HashMap::new(),
        }
    }

    /// Anchor minted identity rows to the DECL's home pkg: the ledger
    /// rows this program already carries name every foreign generic the
    /// window touches (a `HashSet$new` stub row's owner IS HashSet's
    /// pkg, namespaced). `own` rows win (a local decl's text maps to
    /// `owner` itself, the caller's default).
    pub fn with_ledger_owners(mut self, rows: &[InstFn]) -> Self {
        for r in rows {
            let name = match &r.kind {
                InstFnKind::Free { name, .. } => *name,
                InstFnKind::Method { data, .. } => *data,
                InstFnKind::HostThunk { .. } => continue,
            };
            self.foreign_owners
                .entry(self.interner.name(name).to_string())
                .or_insert_with(|| self.interner.name(r.owner).to_string());
        }
        self
    }

    pub(crate) fn name_of(&self, t: TypeId) -> &str {
        self.interner.name(self.tt.type_at(t).name)
    }

    /// A row-name TEXT's row: the structural shapes' names are interned
    /// (every minted row spells its own name), so the lookup answers —
    /// a miss names an arg text no row ever carried (doctored template).
    pub(crate) fn resolve_text(&mut self, text: &str) -> Option<TypeId> {
        let (bare, opt) = match text.strip_prefix('?') {
            Some(b) => (b, true),
            None => (text, false),
        };
        let id = self.interner.lookup(bare)?;
        let row = self.tt.dense_id_of_name(id)?;
        if opt {
            Some(self.mk_opt(row))
        } else {
            Some(row)
        }
    }

    /// The (substituted) name text of a row, run through `run`.
    fn run_name(&mut self, row: TypeId) -> String {
        let t = self.run(row);
        self.name_of(t).to_string()
    }

    /// Intern a STRUCTURAL row by name first: the anonymous
    /// instantiations' spells (`?T`, `Array<T>`, `fn(..) -> ..`,
    /// `(A, B)`, and the nominal `Base<args>` rebuilds) are GENERATED
    /// from the structure, so the name fully determines the row — two
    /// substitutions reaching the same shape through different leaf
    /// copies (the surface's pre-attribution template row vs the
    /// window's compilation of it) must land on ONE row or the
    /// materialized bodies disagree with their own records. The
    /// compiler-MINTED shapes (`#cell@x@1` and friends) keep the
    /// `(name, kind)` dedup: their names are positional, not
    /// structural — the template's placeholder-world cell and the
    /// concrete cell are two different rows sharing one name.
    fn intern_structural(&mut self, ty: RutType) -> TypeId {
        if let Some(id) = self.tt.dense_id_of_name(ty.name) {
            return id;
        }
        self.tt.intern(ty)
    }

    fn mk_opt(&mut self, elem: TypeId) -> TypeId {
        // the boot shortcuts mirror the compiler's `mk_opt` (the legal-
        // host-returns phase): those spell the shared boot rows
        use rut_core::types::*;
        match elem {
            TY_STR => return TY_OPT_STR,
            TY_BYTES => return TY_OPT_BYTES,
            TY_OPAQUE => return TY_OPT_OPAQUE,
            _ => {}
        }
        let name = self.interner.intern(&format!("?{}", self.name_of(elem)));
        self.intern_structural(RutType { name, kind: TyKind::Opt { elem } })
    }

    fn mk_array(&mut self, elem: TypeId) -> TypeId {
        let name = self.interner.intern(&format!("Array<{}>", self.name_of(elem)));
        self.intern_structural(RutType { name, kind: TyKind::Array { elem } })
    }

    fn mk_weak(&mut self, elem: TypeId) -> TypeId {
        let name = self.interner.intern(&format!("Weak<{}>", self.name_of(elem)));
        self.intern_structural(RutType { name, kind: TyKind::Weak { elem } })
    }

    fn mk_fn_ty(&mut self, params: Vec<TypeId>, ret: TypeId) -> TypeId {
        let ps: Vec<String> = params.iter().map(|&p| self.name_of(p).to_string()).collect();
        let name = self.interner.intern(&format!("fn({}) -> {}", ps.join(", "), self.name_of(ret)));
        self.intern_structural(RutType { name, kind: TyKind::Fn { params, ret } })
    }

    /// Split a name's top-level argument list (`Base<a, b>` / `(a, b)`).
    pub(crate) fn split_args(text: &str) -> Option<(String, Vec<String>)> {
        let (base, rest) = text.split_once('<')?;
        let args_text = rest.strip_suffix('>')?;
        if base.starts_with('(') {
            return None;
        }
        Some((base.to_string(), split_top_commas(args_text)))
    }

    /// The substituted row for `t` — `None` when `t` mentions no
    /// parameter (it maps to itself).
    fn rebuild(&mut self, t: TypeId) -> Option<TypeId> {
        if t == u32::MAX {
            return None;
        }
        match self.tt.kind(t).clone() {
            TyKind::Opt { elem } => {
                let ne = self.run(elem);
                (ne != elem).then(|| self.mk_opt(ne))
            }
            TyKind::Array { elem } => {
                let ne = self.run(elem);
                (ne != elem).then(|| self.mk_array(ne))
            }
            TyKind::Weak { elem } => {
                let ne = self.run(elem);
                (ne != elem).then(|| self.mk_weak(ne))
            }
            TyKind::Fn { params, ret } => {
                let np: Vec<TypeId> = params.iter().map(|&p| self.run(p)).collect();
                let nr = self.run(ret);
                (np != params || nr != ret).then(|| self.mk_fn_ty(np, nr))
            }
            TyKind::Data { fields } => {
                let nf: Vec<FieldInfo> = fields
                    .iter()
                    .map(|f| FieldInfo { name: f.name, ty: self.run(f.ty) })
                    .collect();
                let fields_changed = nf.iter().zip(fields.iter()).any(|(n, o)| n.ty != o.ty);
                if !fields_changed {
                    // the parameter may still ride the NAME's argument
                    // spell: `HashMap<#K, #V>` stores `opaque` handles —
                    // the fields never mention K/V, the substitution
                    // lives entirely in the name. Rebuild the name
                    // through the substitution; a row whose every arg
                    // holds passes as itself. (The tuple and
                    // compiler-minted spells carry no separate argument
                    // list — their fields are the whole story — so they
                    // stay as themselves here.)
                    let text = self.name_of(t).to_string();
                    let Some((base, args)) = Self::split_args(&text) else {
                        return None;
                    };
                    let mut parts = Vec::new();
                    let mut cargs = Vec::new();
                    let mut name_changed = false;
                    for piece in &args {
                        let Some(row) = self.resolve_text(piece) else {
                            return None; // an arg that is not a row: not ours to substitute
                        };
                        let concrete = self.run(row);
                        let spelled = self.name_of(concrete).to_string();
                        if spelled != *piece {
                            name_changed = true;
                        }
                        parts.push(spelled);
                        cargs.push(concrete);
                    }
                    if !name_changed {
                        return None;
                    }
                    let new_text = format!("{base}<{}>", parts.join(", "));
                    let name = self.interner.intern(&new_text);
                    let row =
                        self.intern_structural(RutType { name, kind: TyKind::Data { fields: nf } });
                    // the owner-anchored identity row: link unifies every
                    // mirror of this instantiation onto ONE program-wide
                    // row — the anchor is the DECL's home pkg, the exact
                    // key a fresh compile claims (see the field-driven
                    // path below). The args are the SUBSTITUTED rows —
                    // the placeholder spellings would claim the
                    // template-era key and the merge would fold this
                    // mint onto the template row.
                    if let Some(base_id) = self.interner.lookup(&base) {
                        let owner_text = self
                            .foreign_owners
                            .get(&base)
                            .cloned()
                            .unwrap_or_else(|| self.owner.clone());
                        self.minted_inst_tys.push(InstTy {
                            owner: self.interner.intern(&owner_text),
                            decl: base_id,
                            args: cargs,
                            ty: row,
                        });
                    }
                    return Some(row);
                }
                // the row's identity: rebuild the NAME's argument list
                // through the substitution (the nominal instantiation law
                // — `SOpt<#T>` materializes to `SOpt<JsonI64>`, the exact
                // text a fresh compile would mint)
                let text = self.name_of(t).to_string();
                let new_text = if text.starts_with('(') {
                    // a tuple: `(A, B)` — substitute each member
                    let inner = text.strip_prefix('(')?.strip_suffix(')')?.to_string();
                    let mut parts = Vec::new();
                    for piece in split_top_commas(&inner) {
                        let row = self.resolve_text(&piece)?;
                        parts.push(self.run_name(row));
                    }
                    format!("({})", parts.join(", "))
                } else if let Some((base, args)) = Self::split_args(&text) {
                    let mut parts = Vec::new();
                    for piece in args {
                        let row = self.resolve_text(&piece)?;
                        parts.push(self.run_name(row));
                    }
                    format!("{base}<{}>", parts.join(", "))
                } else {
                    // a compiler-minted shape whose name is no
                    // instantiation spell (`#cell@x@1` — a captured
                    // variable's cell): the name is positional and
                    // substitution-free, the FIELDS are what mention the
                    // parameter — keep the name, re-intern the rebuilt
                    // kind with the `(name, kind)` dedup. NOT the
                    // name-first dedup: the template's own
                    // placeholder-world cell shares this name, and the
                    // concrete cell is a different row (a different
                    // field structure — the whole point of the
                    // substitution).
                    text
                };
                let name = self.interner.intern(&new_text);
                let row = self.tt.intern(RutType { name, kind: TyKind::Data { fields: nf } });
                // the owner-anchored identity row: link unifies every
                // mirror of this instantiation onto ONE program-wide row.
                // The anchor is the DECL's home pkg — a foreign base
                // (`Vec` inside a flow template) claims under ITS pkg's
                // namespaced spec, the exact key a fresh compile claims.
                if let Some((base, _)) = Self::split_args(&new_text) {
                    if let Some(base_id) = self.interner.lookup(&base) {
                        // re-derive the concrete args from the new name
                        let (_, args) = Self::split_args(&new_text).expect("just split");
                        let cargs: Vec<TypeId> = args
                            .iter()
                            .filter_map(|a| self.resolve_text(a))
                            .collect();
                        if cargs.len() == args.len() {
                            let owner_text = self
                                .foreign_owners
                                .get(&base)
                                .cloned()
                                .unwrap_or_else(|| self.owner.clone());
                            self.minted_inst_tys.push(InstTy {
                                owner: self.interner.intern(&owner_text),
                                decl: base_id,
                                args: cargs,
                                ty: row,
                            });
                        }
                    }
                }
                Some(row)
            }
            _ => None,
        }
    }

    /// The substituted type for `t` (memoized; structural rebuilds
    /// mint-and-dedup through the table's interner).
    pub fn run(&mut self, t: TypeId) -> TypeId {
        if let Some(&m) = self.map.get(&t) {
            return m;
        }
        let out = self.rebuild(t).unwrap_or(t);
        self.map.insert(t, out);
        out
    }

    /// Pin `t` to `row` explicitly — the self-row mapping's engine.
    /// The template body's receiver spells the class's OWN bare row
    /// (`HashMap` — the record shape), whose tie to the type
    /// parameters rides the template row (`data` + `param_rows`), not
    /// any argument spell of its own; no name-arg rebuild can derive
    /// it, so the caller pins bare row → instantiation row.
    pub(crate) fn force(&mut self, t: TypeId, row: TypeId) {
        self.map.insert(t, row);
    }

    /// The nominal instantiation row for `base_text<args>`: intern the
    /// spell (name-first dedup — the exact text a fresh compile mints)
    /// and record the owner-anchored identity row so link unifies
    /// every mirror of this instantiation onto one program-wide row.
    pub(crate) fn mint_instantiation(
        &mut self,
        base_text: &str,
        args: &[TypeId],
        kind: TyKind,
    ) -> TypeId {
        let spell: Vec<String> = args.iter().map(|&a| self.name_of(a).to_string()).collect();
        let new_text = format!("{base_text}<{}>", spell.join(", "));
        let name = self.interner.intern(&new_text);
        let row = self.intern_structural(RutType { name, kind });
        if let Some(base_id) = self.interner.lookup(base_text) {
            let owner_text = self
                .foreign_owners
                .get(base_text)
                .cloned()
                .unwrap_or_else(|| self.owner.clone());
            self.minted_inst_tys.push(InstTy {
                owner: self.interner.intern(&owner_text),
                decl: base_id,
                args: args.to_vec(),
                ty: row,
            });
        }
        row
    }
}

/// The template STABILITY refusal (v22, the pack-side law): a template
/// body's emission must have followed the MECHANICAL rule — every
/// repr-carrying op bakes the repr its (template-side) type operands
/// resolve to. A different byte means emission branched on the concrete
/// T (the body would lower wrongly for some instantiation, and the
/// materialization's repr rewrite — which recomputes the same
/// mechanical rule at the substituted types — could not repair it).
/// Answers `(fn name, construct)`.
pub fn template_stability_violation(prog: &Program) -> Option<(String, String)> {
    let interner = &prog.interner;
    let tt = &prog.types;
    for fc in prog.funcs.iter().skip(prog.template_first_fid as usize) {
        let reg_ty = |r: u16| -> TypeId { fc.regs.get(r as usize).copied().unwrap_or(rut_core::types::TY_NIL) };
        let field_ty = |t: TypeId, field: u32| -> Option<TypeId> {
            match tt.kind(t) {
                TyKind::Data { fields } => fields.get(field as usize).map(|f| f.ty),
                _ => None,
            }
        };
        let arr_elem = |t: TypeId| -> Option<TypeId> {
            match tt.kind(t) {
                TyKind::Array { elem } => Some(*elem),
                _ => None,
            }
        };
        for op in &fc.code {
            let bad = |what: &str| Some((interner.name(fc.name).to_string(), what.to_string()));
            match op {
                Op::GetF { obj, field, repr, .. } | Op::SetF { obj, field, repr, .. } => {
                    let want = field_ty(reg_ty(*obj), *field).map(|f| tt.repr_of(f));
                    if let Some(w) = want {
                        if w != *repr {
                            return bad("a field access whose baked repr is not the field type's");
                        }
                    }
                }
                Op::ArrNew { ty, repr, .. } => {
                    if let Some(e) = arr_elem(*ty) {
                        if arr_elem_repr(tt, e) != *repr {
                            return bad("an array op whose baked repr is not the element type's");
                        }
                    }
                }
                Op::ArrGet { arr, repr, .. } | Op::ArrSet { arr, repr, .. } => {
                    if let Some(e) = arr_elem(reg_ty(*arr)) {
                        if arr_elem_repr(tt, e) != *repr {
                            return bad("an array element op whose baked repr is not the element type's");
                        }
                    }
                }
                Op::ArrGetF { obj, field, repr, .. } | Op::ArrSetF { obj, field, repr, .. } => {
                    let want = field_ty(reg_ty(*obj), *field).and_then(arr_elem).map(|e| arr_elem_repr(tt, e));
                    if let Some(w) = want {
                        if w != *repr {
                            return bad("a field-array op whose baked repr is not the element type's");
                        }
                    }
                }
                _ => {}
            }
        }
    }
    None
}

pub(crate) fn split_top_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for ch in s.chars() {
        match ch {
            '<' | '(' => {
                depth += 1;
                cur.push(ch);
            }
            '>' | ')' => {
                depth = depth.saturating_sub(1);
                cur.push(ch);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur = String::new();
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur.trim().to_string());
    }
    out
}

/// The interface-world half of a materialization (T2): a template body
/// bakes `CallI` slots and `IsIface` row ids against the PACK's iface
/// rows — and rut mints one row per exact type-argument tuple
/// (`mk_iface_inst`), so a generic interface param compiles against the
/// PLACEHOLDER row (`Readable<#T>`) while the consumer's fills and
/// dispatches speak the CONCRETE row (`Readable<i32>`). These maps
/// re-target those operands through the substitution: each slot's row
/// name re-spells with the substituted arguments (identity for every
/// row the substitution cannot change — non-generic interfaces and
/// already-concrete instantiations), the re-spelled row is found or
/// MINTED in the owner's table (method signatures substituted from the
/// placeholder row's descriptor, slots appended, vtable rows resized),
/// and the slot maps onto the substituted row's numbering. The owner's
/// extended table re-lays onto the merged enumeration at link — exactly
/// as every other fn the materialization appends.
pub(crate) struct IfaceRemaps {
    /// pack-local `CallI` slot → owner-local slot
    pub slots: HashMap<u32, u32>,
    /// pack-local `IsIface` row id → owner-local row id
    pub rows: HashMap<u32, u32>,
}

/// Build [`IfaceRemaps`] for one template body. Collected and resolved
/// BEFORE `materialize_fn` runs — the row re-spelling needs the
/// substitution, which `materialize_fn` itself holds.
pub(crate) fn build_iface_remaps(
    sub: &mut Substitution,
    ifaces: &mut Vec<rut_core::binary::IfaceDesc>,
    iface_slots: &mut Vec<(u32, u32)>,
    vtables: &mut Vec<Vec<Option<u32>>>,
    fc: &FuncCode,
) -> Result<IfaceRemaps, String> {
    let mut wanted_slots: Vec<u32> = Vec::new();
    let mut wanted_rows: Vec<u32> = Vec::new();
    for op in &fc.code {
        match op {
            Op::CallI { slot, .. } => wanted_slots.push(*slot),
            Op::IsIface { want, .. } => wanted_rows.push(*want),
            _ => {}
        }
    }
    // the SIGNATURE's interface rows ride the demand law too: a ret
    // typed `Readable<T>` pairs the returned payload with the concrete
    // row at the return boundary (the T2b fill), and an iface-typed
    // local/reg names the row the body SPEAKS — want every one so the
    // map resolves each
    for t in fc.params.iter().chain(std::iter::once(&fc.ret)).chain(fc.regs.iter()) {
        if let rut_core::types::TyKind::IfaceObj { iface_id } = sub.tt.kind(*t) {
            wanted_rows.push(*iface_id);
        }
    }
    wanted_slots.sort_unstable();
    wanted_slots.dedup();
    wanted_rows.sort_unstable();
    wanted_rows.dedup();
    let mut remaps = IfaceRemaps { slots: HashMap::new(), rows: HashMap::new() };
    for want in wanted_rows {
        let row = resolve_iface_row(sub, ifaces, iface_slots, vtables, want)?;
        remaps.rows.insert(want, row);
    }
    for slot in wanted_slots {
        let Some(&(row, method)) = iface_slots.get(slot as usize) else {
            return Err(format!(
                "template body names interface slot {slot} outside its table — \
                 corrupt or doctored, re-pack the directory"
            ));
        };
        let new_row = resolve_iface_row(sub, ifaces, iface_slots, vtables, row)?;
        remaps.rows.entry(row).or_insert(new_row);
        let new_slot = iface_slots
            .iter()
            .position(|&(r, m)| r == new_row && m == method)
            .unwrap_or(slot as usize) as u32;
        remaps.slots.insert(slot, new_slot);
    }
    Ok(remaps)
}

/// The substituted iface ROW for a pack-local row id: re-spell the row
/// name's argument list through the substitution. An unchanged spell is
/// the identity row; a changed spell finds or mints the row under the
/// same `Base<args>` naming law the compiler's `mk_iface_inst` uses, so
/// a consumer session's own compile and this materialization land on
/// ONE row after link's by-name trait dedup.
fn resolve_iface_row(
    sub: &mut Substitution,
    ifaces: &mut Vec<rut_core::binary::IfaceDesc>,
    iface_slots: &mut Vec<(u32, u32)>,
    vtables: &mut Vec<Vec<Option<u32>>>,
    row: u32,
) -> Result<u32, String> {
    let Some(desc) = ifaces.get(row as usize) else {
        return Err(format!(
            "template body names interface row {row} outside its table — \
             corrupt or doctored, re-pack the directory"
        ));
    };
    let name_text = sub.interner.name(desc.name).to_string();
    let Some((base, args)) = Substitution::split_args(&name_text) else {
        return Ok(row); // a bare interface (no argument list): identity
    };
    let mut parts = Vec::with_capacity(args.len());
    for piece in &args {
        let Some(r) = sub.resolve_text(piece) else {
            return Err(format!(
                "interface row `{name_text}` names argument `{piece}` that is no row — \
                 corrupt or doctored, re-pack the directory"
            ));
        };
        let concrete = sub.run(r);
        parts.push(sub.name_of(concrete).to_string());
    }
    let new_text = format!("{base}<{}>", parts.join(", "));
    if new_text == name_text {
        return Ok(row);
    }
    let name_id = sub.interner.intern(&new_text);
    if let Some(id) = ifaces.iter().position(|d| d.name == name_id) {
        return Ok(id as u32);
    }
    // mint: the descriptor's method signatures substitute with the row
    // (the same substitution a fresh compile at these arguments spells)
    let methods: Vec<rut_core::binary::IfaceMethod> = ifaces[row as usize]
        .methods
        .iter()
        .map(|m| rut_core::binary::IfaceMethod {
            name: m.name,
            params: m.params.iter().map(|&p| sub.run(p)).collect(),
            ret: sub.run(m.ret),
        })
        .collect();
    let new_row = ifaces.len() as u32;
    ifaces.push(rut_core::binary::IfaceDesc { name: name_id, methods });
    for m in 0..ifaces[new_row as usize].methods.len() as u32 {
        iface_slots.push((new_row, m));
    }
    let total = iface_slots.len();
    for vt in vtables.iter_mut() {
        vt.resize(total, None);
    }
    Ok(new_row)
}

/// Materialize one template body: substitute TypeIds, remap callee fids
/// through `map_fid`, re-target the interface-slot operands through
/// `ifc`, rewrite the repr bytes the substitution changed, and re-run
/// the repr-keyed folds — the byte-equality law's engine.
pub fn materialize_fn(
    sub: &mut Substitution,
    fc: &FuncCode,
    map_fid: &mut dyn FnMut(u32) -> u32,
    ifc: &IfaceRemaps,
) -> FuncCode {
    let mut out = FuncCode {
        name: fc.name,
        params: fc.params.iter().map(|&t| sub.run(t)).collect(),
        ret: sub.run(fc.ret),
        is_method: fc.is_method,
        n_captures: fc.n_captures,
        regs: fc.regs.iter().map(|&t| sub.run(t)).collect(),
        argv: fc.argv.clone(),
        labels: fc.labels.clone(),
        code: Vec::with_capacity(fc.code.len()),
        spans: fc.spans.clone(),
        pos: fc.pos.clone(),
        host_id: fc.host_id,
    };
    for op in &fc.code {
        out.code.push(remap_op(sub, op, map_fid, ifc));
    }
    rewrite_reprs(sub.tt, &mut out);
    rut_lir::lir::refold_optprim(&mut out, sub.tt);
    out
}

fn remap_op(sub: &mut Substitution, op: &Op, map_fid: &mut dyn FnMut(u32) -> u32, ifc: &IfaceRemaps) -> Op {
    match op {
        Op::Mov { dst, src } => Op::Mov { dst: *dst, src: *src },
        Op::MovRef { dst, src } => Op::MovRef { dst: *dst, src: *src },
        Op::Const { dst, k } => Op::Const { dst: *dst, k: *k },
        Op::ConstRaw { dst, bits } => Op::ConstRaw { dst: *dst, bits: *bits },
        Op::Not { dst, a } => Op::Not { dst: *dst, a: *a },
        Op::AddF { prim, dst, a, b } => Op::AddF { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::SubF { prim, dst, a, b } => Op::SubF { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::MulF { prim, dst, a, b } => Op::MulF { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::DivF { prim, dst, a, b } => Op::DivF { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::ModF { prim, dst, a, b } => Op::ModF { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::NegF { prim, dst, a } => Op::NegF { prim: *prim, dst: *dst, a: *a },
        Op::EqF { dst, a, b } => Op::EqF { dst: *dst, a: *a, b: *b },
        Op::NeF { dst, a, b } => Op::NeF { dst: *dst, a: *a, b: *b },
        Op::LtF { dst, a, b } => Op::LtF { dst: *dst, a: *a, b: *b },
        Op::GtF { dst, a, b } => Op::GtF { dst: *dst, a: *a, b: *b },
        Op::LeF { dst, a, b } => Op::LeF { dst: *dst, a: *a, b: *b },
        Op::GeF { dst, a, b } => Op::GeF { dst: *dst, a: *a, b: *b },
        Op::AddI { prim, dst, a, b } => Op::AddI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::SubI { prim, dst, a, b } => Op::SubI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::MulI { prim, dst, a, b } => Op::MulI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::DivI { prim, dst, a, b } => Op::DivI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::ModI { prim, dst, a, b } => Op::ModI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::WAddI { prim, dst, a, b } => Op::WAddI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::WSubI { prim, dst, a, b } => Op::WSubI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::WMulI { prim, dst, a, b } => Op::WMulI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::AndI { prim, dst, a, b } => Op::AndI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::OrI { prim, dst, a, b } => Op::OrI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::XorI { prim, dst, a, b } => Op::XorI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::ShlI { prim, dst, a, b } => Op::ShlI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::ShrI { prim, dst, a, b } => Op::ShrI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::WrapShlI { prim, dst, a, b } => Op::WrapShlI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::EqI { prim, dst, a, b } => Op::EqI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::NeI { prim, dst, a, b } => Op::NeI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::LtI { prim, dst, a, b } => Op::LtI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::GtI { prim, dst, a, b } => Op::GtI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::LeI { prim, dst, a, b } => Op::LeI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::GeI { prim, dst, a, b } => Op::GeI { prim: *prim, dst: *dst, a: *a, b: *b },
        Op::NegI { prim, dst, a } => Op::NegI { prim: *prim, dst: *dst, a: *a },
        Op::StrCmp { eq, dst, a, b } => Op::StrCmp { eq: *eq, dst: *dst, a: *a, b: *b },
        Op::RefEq { eq, dst, a, b } => Op::RefEq { eq: *eq, dst: *dst, a: *a, b: *b },
        Op::Jmp { target } => Op::Jmp { target: *target },
        Op::Br { cond, then_t, else_t } => Op::Br { cond: *cond, then_t: *then_t, else_t: *else_t },
        Op::BrTable { idx, table_off, count, default } => Op::BrTable { idx: *idx, table_off: *table_off, count: *count, default: *default },
        Op::Call { func, argv_off, argc, dst } => Op::Call { func: map_fid(*func), argv_off: *argv_off, argc: *argc, dst: *dst },
        Op::CallM { func, argv_off, argc, dst } => Op::CallM { func: map_fid(*func), argv_off: *argv_off, argc: *argc, dst: *dst },
        // T2: a template's interface dispatch bakes the PLACEHOLDER
        // row's slot (`Readable<#T>.get`); the substituted body must
        // dispatch on the CONCRETE row (`Readable<i32>.get`) — the row
        // its consumer-side fills answer on
        Op::CallI { slot, argv_off, argc, dst } => Op::CallI {
            slot: ifc.slots.get(slot).copied().unwrap_or(*slot),
            argv_off: *argv_off,
            argc: *argc,
            dst: *dst,
        },
        Op::CallNat { nat, recv, argv_off, argc, dst } => Op::CallNat { nat: *nat, recv: *recv, argv_off: *argv_off, argc: *argc, dst: *dst },
        Op::CallFn { fval, argv_off, argc, dst } => Op::CallFn { fval: *fval, argv_off: *argv_off, argc: *argc, dst: *dst },
        Op::Ret { val } => Op::Ret { val: *val },
        Op::NewCell { dst, ty } => Op::NewCell { dst: *dst, ty: sub.run(*ty) },
        Op::MakeRecord { dst, ty, argv_off, argc } => Op::MakeRecord { dst: *dst, ty: sub.run(*ty), argv_off: *argv_off, argc: *argc },
        Op::GetF { dst, obj, field, repr } => Op::GetF { dst: *dst, obj: *obj, field: *field, repr: *repr },
        Op::SetF { obj, field, val, repr } => Op::SetF { obj: *obj, field: *field, val: *val, repr: *repr },
        Op::Own { dst, src, ty } => Op::Own { dst: *dst, src: *src, ty: sub.run(*ty) },
        Op::MakeOpt { dst, src, ty } => Op::MakeOpt { dst: *dst, src: *src, ty: sub.run(*ty) },
        Op::WeakNew { dst, src, ty } => Op::WeakNew { dst: *dst, src: *src, ty: sub.run(*ty) },
        Op::WeakUpgrade { recv, dst, ty } => Op::WeakUpgrade { recv: *recv, dst: *dst, ty: sub.run(*ty) },
        Op::ArrNew { dst, ty, len, repr } => Op::ArrNew { dst: *dst, ty: sub.run(*ty), len: *len, repr: *repr },
        Op::ArrLit { dst, ty, argv_off, argc } => Op::ArrLit { dst: *dst, ty: sub.run(*ty), argv_off: *argv_off, argc: *argc },
        Op::ArrGet { dst, arr, idx, repr } => Op::ArrGet { dst: *dst, arr: *arr, idx: *idx, repr: *repr },
        Op::ArrSet { arr, idx, val, repr } => Op::ArrSet { arr: *arr, idx: *idx, val: *val, repr: *repr },
        Op::EnumNew { dst, ty, member } => Op::EnumNew { dst: *dst, ty: sub.run(*ty), member: *member },
        Op::TidOf { dst, obj } => Op::TidOf { dst: *dst, obj: *obj },
        Op::IsType { dst, obj, want } => Op::IsType { dst: *dst, obj: *obj, want: sub.run(*want) },
        // the same re-target for the interface TEST (`is` matches the
        // row id, not a slot — a stale row id answers false for a type
        // that satisfies the substituted row)
        Op::IsIface { dst, obj, want } => Op::IsIface {
            dst: *dst,
            obj: *obj,
            want: ifc.rows.get(want).copied().unwrap_or(*want),
        },
        Op::Unbox { dst, box_, ty } => Op::Unbox { dst: *dst, box_: *box_, ty: sub.run(*ty) },
        Op::Box { dst, val, ty } => Op::Box { dst: *dst, val: *val, ty: sub.run(*ty) },
        Op::MakeClosure { dst, func, argv_off, argc } => Op::MakeClosure { dst: *dst, func: map_fid(*func), argv_off: *argv_off, argc: *argc },
        Op::Panic { msg } => Op::Panic { msg: *msg },
        Op::LoopHead => Op::LoopHead,
        Op::Conv { dst, src, from, to } => Op::Conv { dst: *dst, src: *src, from: *from, to: *to },
        Op::StrCodeAt { dst, s, idx } => Op::StrCodeAt { dst: *dst, s: *s, idx: *idx },
        Op::ArrayCmp { eq, dst, a, b } => Op::ArrayCmp { eq: *eq, dst: *dst, a: *a, b: *b },
        Op::ArrGetF { dst, obj, field, idx, repr } => Op::ArrGetF { dst: *dst, obj: *obj, field: *field, idx: *idx, repr: *repr },
        Op::ArrSetF { obj, field, idx, val, repr } => Op::ArrSetF { obj: *obj, field: *field, idx: *idx, val: *val, repr: *repr },
        Op::Pad { .. } => unreachable!("diagnostic-only variant"),
    }
}

/// Recompute every repr byte — and the repr-keyed OPCODE spell — from
/// the SUBSTITUTED types. The template baked the placeholder rows' Ref
/// reprs; the concrete arguments may carry prim/fn (immediate-slot)
/// stores, and the materialized body must spell exactly what a fresh
/// compile at these types spells. The opcode half covers the one
/// ref-vs-immediate opcode pair the emitter branches on (`mov_slot`:
/// `MovRef` iff `types.is_ref(ty)`, else `Mov`): a template `MovRef`
/// whose moved value substitutes to a non-ref repr is the fresh
/// compile's `Mov` copy — left as a ref move it retains the prim bits
/// as a heap handle and aborts (the T1 crash), so the spell remaps.
/// (The converse never occurs: the template emitter spells `Mov` only
/// for types the placeholder rows can never be.)
fn rewrite_reprs(tt: &TypeTable, fc: &mut FuncCode) {
    let reg_ty = |r: u16| -> TypeId { fc.regs.get(r as usize).copied().unwrap_or(rut_core::types::TY_NIL) };
    let arr_elem = |t: TypeId| -> Option<TypeId> {
        match tt.kind(t) {
            TyKind::Array { elem } => Some(*elem),
            _ => None,
        }
    };
    for i in 0..fc.code.len() {
        // the opcode spell: the moved VALUE's substituted type decides
        // (both regs of an emitter-spelled copy share it)
        if let Op::MovRef { dst, src } = fc.code[i] {
            if !tt.is_ref(reg_ty(src)) || !tt.is_ref(reg_ty(dst)) {
                fc.code[i] = Op::Mov { dst, src };
            }
        }
        let repr = {
            match &fc.code[i] {
                Op::GetF { obj, field, .. } | Op::SetF { obj, field, .. } => {
                    let fty = match tt.kind(reg_ty(*obj)) {
                        TyKind::Data { fields } => fields.get(*field as usize).map(|f| f.ty),
                        // the opt box's payload slot (field 0): the fresh
                        // compile bakes the payload's own repr — a prim
                        // element reads Prim(p), never the box's Ref
                        TyKind::Opt { elem } if *field == 0 => Some(*elem),
                        _ => None,
                    };
                    fty.map(|f| tt.repr_of(f))
                }
                Op::ArrNew { ty, .. } => arr_elem(*ty).map(|e| arr_elem_repr(tt, e)),
                Op::ArrGet { arr, .. } | Op::ArrSet { arr, .. } => {
                    arr_elem(reg_ty(*arr)).map(|e| arr_elem_repr(tt, e))
                }
                Op::ArrGetF { obj, field, .. } | Op::ArrSetF { obj, field, .. } => {
                    let fty = match tt.kind(reg_ty(*obj)) {
                        TyKind::Data { fields } => fields.get(*field as usize).map(|f| f.ty),
                        _ => None,
                    };
                    fty.and_then(arr_elem).map(|e| arr_elem_repr(tt, e))
                }
                _ => None,
            }
        };
        if let Some(r) = repr {
            match &mut fc.code[i] {
                Op::GetF { repr, .. }
                | Op::SetF { repr, .. }
                | Op::ArrNew { repr, .. }
                | Op::ArrGet { repr, .. }
                | Op::ArrSet { repr, .. }
                | Op::ArrGetF { repr, .. }
                | Op::ArrSetF { repr, .. } => *repr = r,
                _ => {}
            }
        }
    }
}
