//! The monomorphization driver: the instantiation queue,
//! vtable construction (trait members never devirtualize),
//! exports, and module-let emission.

use crate::lir::FnCompiler;
use rut_core::binary::FuncCode;
use super::*;

impl<'a> Ctx<'a> {
    // ---- monomorphization driver (generic calls enter the
    // queue; HIR contains no generic code) ----

    pub fn ensure_inst(&mut self, inst: Inst) -> u32 {
        if let Some(&f) = self.inst_map.get(&inst) {
            return f;
        }
        // reserve the slot with a placeholder so recursion terminates
        let fid = self.funcs.len() as u32;
        self.inst_map.insert(inst.clone(), fid);
        let fname = self.intern(&self.inst_name(&inst));
        self.funcs.push(FuncCode {
            name: fname,
            params: vec![],
            ret: TY_NIL,
            is_method: false,
            n_captures: 0,
            regs: vec![],
            argv: vec![],
            labels: vec![],
            code: vec![],
            spans: vec![],
            pos: vec![],
            host_id: None,
        });
        // the owner-anchored fn ledger: link unifies instantiations
        // sharing a canonical `(owner, decl, args)` key into ONE
        // program-wide copy. Lambdas and for-of emit closures are
        // unit-local by construction (their keys name this unit's AST
        // nodes) and never referenced across packages — no row.
        let subst: Vec<TypeId> = inst.subst.iter().map(|(_, t)| *t).collect();
        let origins = inst.iface_origins.clone();
        match inst.key {
            FnKey::Lambda(_) | FnKey::ForOfEmit { .. } | FnKey::AsyncBlock(_) => {}
            FnKey::Free(n) => {
                // a mirrored FOREIGN generic fn names its owner (the
                // linkable-classes phase): the stub's row must spell the
                // same owner text the real body's row does, or the keys
                // never unify at link. Own fns keep this unit's spec.
                let origin = match self.extern_generic_fns.get(&n) {
                    Some(gf) => gf.owner.clone(),
                    None => self.own_spec.clone(),
                };
                let owner = self.intern(&origin);
                self.ledger_fns.push(rut_core::binary::InstFn {
                    owner,
                    kind: rut_core::binary::InstFnKind::Free { name: n, subst, origins },
                    fid,
                });
            }
            FnKey::Method { data, name, .. } => {
                // the carried-type law: a mirror over a REQUESTER-CARRIED
                // row names the row's HOME unit (set by the mirror site),
                // so link unifies the stub with the real body there
                let owner = match self.pending_mirror_owner.take() {
                    Some(o) => o,
                    None => self.owner_of_data(data),
                };
                let owner = self.intern(&owner);
                self.ledger_fns.push(rut_core::binary::InstFn {
                    owner,
                    kind: rut_core::binary::InstFnKind::Method { data, name, subst, origins },
                    fid,
                });
            }
            FnKey::HostThunk(n) => {
                let owner = self.intern(&self.own_spec.clone());
                self.ledger_fns.push(rut_core::binary::InstFn {
                    owner,
                    kind: rut_core::binary::InstFnKind::HostThunk { name: n },
                    fid,
                });
            }
        }
        
        self.queue.push(inst);
        fid
    }

    /// Mint an instantiation's fn id + ledger row WITHOUT queueing a
    /// body — the mirror arm of [`Ctx::ensure_inst`] (the
    /// linkable-classes phase): a consumer's call into a used generic
    /// class's method binds a mirror fn that has no body here. The
    /// ledger row names the declaring package as owner, so LINK
    /// redirects the mirror onto the owner's compiled copy (the key
    /// canonicalizes the instantiation the same way the owner's own
    /// row spells it) and drops the bodyless stub.
    pub fn mirror_inst(&mut self, inst: Inst) -> u32 {
        if let Some(&f) = self.inst_map.get(&inst) {
            return f;
        }
        // a mirrored GENERIC fn or GENERIC-class method doubles as the
        // owner-side body request: the stub's ledger row claims nothing
        // until the owner's unit compiles the body at this
        // instantiation
        match inst.key {
            FnKey::Method { data, name } => {
                let args: Vec<TypeId> = inst.subst.iter().map(|(_, t)| *t).collect();
                if let Some(g) = self.extern_generics.get(&data).cloned() {
                    let owner = g.owner.clone();
                    self.request_inst_method(owner, data, args, name);
                } else if self.extern_inherents.iter().any(|ih| {
                    self.types.type_at(ih.target).name == data
                }) || self.extern_origins.contains_key(&data) {
                    // a plain class's GENERIC method: the row's bound
                    // name names the declaring package, and the body
                    // rides the same owner-side machinery
                    let owner = self.owner_of_data(data);
                    self.request_inst_method(owner, data, args, name);
                }
            }
            FnKey::Free(name) => {
                if let Some(gf) = self.extern_generic_fns.get(&name).cloned() {
                    let args: Vec<TypeId> = inst.subst.iter().map(|(_, t)| *t).collect();
                    self.request_generic_fn_body(gf.owner, name, args);
                }
            }
            _ => {}
        }
        let saved_queue = std::mem::take(&mut self.queue);
        let fid = self.ensure_inst(inst);
        self.queue = saved_queue;
        fid
    }

    pub(crate) fn inst_name(&self, inst: &Inst) -> String {
        match inst.key {
            FnKey::Free(n) => self.name(n).to_string(),
            FnKey::Method { data, name } => format!("{}${}", self.name(data), self.name(name)),
            FnKey::Lambda(node) => format!("lambda@{}", node.0),
            FnKey::AsyncBlock(node) => format!("async@{}", node.0),
            FnKey::ForOfEmit { body, .. } => format!("forof@{}", body.0),
            FnKey::HostThunk(name) => format!("host@{}", self.name(name)),
        }
    }

    pub fn compile_queue(&mut self, root: Inst) -> TcResult<()> {
        self.ensure_inst(root);
        self.drain_queue()
    }

    /// Drain the instantiation queue without a root — the library
    /// shape's compile entry (a pkg may seed method bodies via owner
    /// requests and carry no `main`/`entry fn`/class-method roots of
    /// its own).
    pub fn drain_queue(&mut self) -> TcResult<()> {
        let mut guard = 0;
        while let Some(inst) = self.queue.pop() {
            guard += 1;
            if guard > 100_000 {
                self.err(self.ast.span(self.ast.root.id()), "monomorphization queue exploded (>100k instantiations)");
                return Err(());
            }
            if self.diags.len() > 64 {
                return Err(());
            }
            let fid = self.inst_map[&inst];
            let fid = self.inst_map[&inst];
            
            if FnCompiler::compile(self, &inst, fid).is_err() {
                
                return Err(());
            }
        }
        Ok(())
    }

    /// after all instantiations: materialize the demanded itables.
    /// Each fill is a (concrete type × interface) pair a boxing site
    /// PROVED (structural satisfaction at a widen); the pair's row
    /// binds every interface member slot to the concrete type's own
    /// inherent member — the member compiles here (with its transitive
    /// calls) so every reachable slot carries a real function id. A
    /// fill body that cannot compile is a unit error — the diag is
    /// already in `ctx.diags`; swallowing it here would ship the
    /// reserved empty fn and fail verification instead.
    pub fn build_vtables(&mut self) -> TcResult<Vec<Vec<Option<u32>>>> {
        // (type, slot, inst) — collected first, compiled after, so
        // queue-driven interning cannot mutate what we walk. Generic
        // classes' fills sort on the dense type id (deterministic
        // emission — the same source always emits the same binary).
        let mut fills: Vec<(TypeId, u32, Inst)> = Vec::new();
        let mut pairs = self.iface_fills.clone();
        pairs.sort_by_key(|(ty, _)| self.types.dense(*ty));
        for (concrete, iface_id) in pairs {
            let desc = self.iface_by_id(iface_id).clone();
            for (midx, tm) in desc.methods.iter().enumerate() {
                let Some(slot) = self.iface_slot(iface_id, midx as u32) else {
                    continue;
                };
                let Some(member) = self.find_inherent_member(concrete, tm.name) else {
                    continue; // the site's satisfaction check already spoke
                };
                let inst = match member {
                    crate::check::impls::MemberSrc::Local { env, data, .. } => Inst {
                        key: FnKey::Method { data, name: tm.name },
                        subst: env,
                        iface_origins: vec![],
                    },
                    crate::check::impls::MemberSrc::Extern { subst, data, .. } => {
                        // the exporter's compiled fn, mirrored: the stub's
                        // ledger row names the declaring package, link
                        // redirects it onto the owner's copy. The fill
                        // STILL lands (the vtable row binds the slot to
                        // the mirror's fn id) — skipping the push left
                        // every (concrete × iface) pair whose members
                        // live in another module with an EMPTY slot,
                        // and the first dispatch trapped.
                        let inst = Inst {
                            key: FnKey::Method { data, name: tm.name },
                            subst,
                            iface_origins: vec![],
                        };
                        self.mirror_inst(inst.clone());
                        inst
                    }
                };
                fills.push((concrete, slot, inst));
            }
        }
        for (_, _, inst) in &fills {
            self.compile_queue(inst.clone())?;
        }

        let total_slots: usize = self.ifaces.iter().map(|t| t.methods.len()).sum();
        let mut vt = vec![Vec::new(); self.types.types.len()];
        for t in vt.iter_mut() {
            *t = vec![None; total_slots];
        }
        for (ty, slot, inst) in fills {
            if let Some(&fid) = self.inst_map.get(&inst) {
                vt[self.types.dense(ty) as usize][slot as usize] = Some(fid);
            }
        }
        // the async weave's engine-minted fills: the hidden
        // frame's `Future::yield` row and the sleep future's engine-
        // backed row — no AST method nodes, so the walk above can't
        // see them; their (type, slot, fid) fills were recorded at mint.
        // The fills RESTORE after applying: the rows are idempotent
        // (same fid rewrites the same slot), and a later pass — the
        // driver builds vtables twice around the fill-body drain — must
        // see them again.
        let pending = std::mem::take(&mut self.extra_vtable_fills);
        for (ty, slot, fid) in &pending {
            vt[self.types.dense(*ty) as usize][*slot as usize] = Some(*fid);
        }
        self.extra_vtable_fills = pending;
        Ok(vt)
    }

    // ---- module lets (load-time expressions only) ----

    /// The per-type disposal rows (the cell-death dispatch): entry per
    /// (dense) type — `Some(dispose func id)` when the type carries a
    /// `[disposal]`-marked member, None otherwise. The engine's release
    /// path reads this table at refcount zero; no vtable and no trait
    /// lookup — the marker IS the designated slot (v20). The member
    /// queues eagerly at its impl (no call site exists); a generic
    /// target was refused at collect — no static row.
    pub fn disposal_impls(&mut self) -> Vec<Option<u32>> {
        let mut out = vec![None; self.types.types.len()];
        let disposal = sym::DISPOSAL_MARKER;
        let datas = self.datas.clone();
        for (dname, d) in datas {
            for (n, mnode) in &d.methods {
                if self.ast.method_decl(*mnode).marker != Some(disposal) {
                    continue;
                }
                let key = Inst {
                    key: FnKey::Method { data: dname, name: *n },
                    subst: vec![],
                    iface_origins: vec![],
                };
                if let Some(&fid) = self.inst_map.get(&key) {
                    out[self.types.dense(d.ty) as usize] = Some(fid);
                }
                break;
            }
        }
        out
    }

    /// The dense-type set carrying a `[disposal]` member (the SROA
    /// guard): a record of one of these types observes its own lifetime
    /// — the marked member runs at refcount zero — so the optimizer must
    /// not delete its mint. Derived from the DECLARATIONS, so it is
    /// stable at every point of the monomorphization queue.
    pub fn disposal_dense_set(&self) -> std::collections::HashSet<TypeId> {
        let mut set = std::collections::HashSet::new();
        let disposal = sym::DISPOSAL_MARKER;
        for (_, d) in &self.datas {
            if d.methods
                .iter()
                .any(|(_, m)| self.ast.method_decl(*m).marker == Some(disposal))
            {
                set.insert(d.ty);
            }
        }
        set
    }

    pub fn compile_module_lets(&mut self) {
        let entries: Vec<(IdentId, Option<NodeHandle<AnyTy>>, NodeHandle<AnyExpr>)> = self.lets.clone();
        for (_, ty, init) in entries {
            let sp = self.ast.span(init.id());
            match self.ast.expr(init) {
                ExprKind::Lit(Lit::Int(v, sfx)) => {
                    let _ = (v, sfx);
                    // typed by annotation or default i32; store raw
                }
                ExprKind::Lit(Lit::Float(_, _)) | ExprKind::Lit(Lit::Str(_)) | ExprKind::Lit(Lit::Bool(_)) => {}
                _ => {
                    self.err(
                        sp,
                        "module `let` initializers must be load-time expressions — literals only in this build",
                    );
                }
            }
            let _ = ty;
        }
    }
}

