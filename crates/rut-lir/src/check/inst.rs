//! The monomorphization driver (RFC 0031 SS2): the instantiation queue,
//! vtable construction (RFC 0012 SS1 - trait members never devirtualize),
//! exports, and module-let emission.

use crate::lir::FnCompiler;
use rut_core::binary::FuncCode;
use rut_core::types::*;
use super::*;

impl<'a> Ctx<'a> {
    // ---- monomorphization driver (RFC 0031 §2: generic calls enter the
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
        self.queue.push(inst);
        fid
    }

    pub(crate) fn inst_name(&self, inst: &Inst) -> String {
        match inst.key {
            FnKey::Free(n) => self.name(n).to_string(),
            FnKey::Method { data, name } => format!("{}${}", self.name(data), self.name(name)),
            FnKey::ImplMethod { idx, name, .. } => {
                let im = &self.impls[idx];
                if im.inherent {
                    // inherent impl on a native builtin class: no trait id
                    format!("{}${}", self.name(im.trait_name), self.name(name))
                } else {
                    let tid = im.trait_id;
                    let tname = self.name(self.traits[tid as usize].name);
                    format!("{}#${}${}", tname, idx, self.name(name))
                }
            }
            FnKey::Lambda(node) => format!("lambda@{}", node.0),
            FnKey::ForOfEmit { body, .. } => format!("forof@{}", body.0),
        }
    }

    pub fn compile_queue(&mut self, root: Inst) -> TcResult<()> {
        self.ensure_inst(root);
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
            FnCompiler::compile(self, &inst, fid)?;
        }
        Ok(())
    }

    /// after all instantiations: fill per-(type × trait) vtables from the
    /// registered impls (nominal satisfaction, RFC 0012 §4 — a slot is
    /// filled exactly when an impl exists). Engine-named contracts
    /// (`Iterator`) flow through the same registry. Each slot's method
    /// compiles here (with its transitive calls) so every reachable slot
    /// carries a real function id.
    pub fn build_vtables(&mut self) -> Vec<Vec<Option<u32>>> {
        // (type, trait, method slot, inst) — collected first, compiled
        // after, so queue-driven interning cannot mutate what we walk.
        // Prim-target impl methods also queue their concrete-ABI twin
        // (`extra`) — vtable rows bind the SLOT variant only, but a
        // bare-receiver static call (and the exported surface) needs
        // the concrete one compiled too.
        let mut fills: Vec<(TypeId, u32, Inst)> = Vec::new();
        let mut extra: Vec<Inst> = Vec::new();
        for (idx, im) in self.impls.iter().enumerate() {
            if im.inherent {
                continue; // inherent methods dispatch statically, never a slot
            }
            let dual = self.impl_is_dual_abi(idx);
            let tdesc = self.trait_by_id(im.trait_id).clone();
            match im.target_data.clone() {
                None => {
                    for (midx, tm) in tdesc.methods.iter().enumerate() {
                        let Some(slot) = self.trait_slot(im.trait_id, midx as u32) else {
                            continue;
                        };
                        if !im.methods.iter().any(|(n, _)| *n == tm.name) {
                            continue;
                        }
                        fills.push((im.target, slot, Inst {
                            key: self.impl_method_key(idx, tm.name, true),
                            subst: vec![],
                            trait_origins: vec![],
                        }));
                        if dual {
                            extra.push(Inst {
                                key: self.impl_method_key(idx, tm.name, false),
                                subst: vec![],
                                trait_origins: vec![],
                            });
                        }
                    }
                }
                Some((dname, params)) => {
                    // generic target: fill every concrete instantiation
                    // already in the table (`Vec<i32>`, …)
                    let insts: Vec<(TypeId, Vec<(IdentId, TypeId)>)> = self
                        .inst_data
                        .iter()
                        .filter(|(_, (d, _))| *d == dname)
                        .map(|(ty, (_, args))| (*ty, params.iter().cloned().zip(args.iter().cloned()).collect()))
                        .collect();
                    for (ty, env) in insts {
                        for (midx, tm) in tdesc.methods.iter().enumerate() {
                            let Some(slot) = self.trait_slot(im.trait_id, midx as u32) else {
                                continue;
                            };
                            if !im.methods.iter().any(|(n, _)| *n == tm.name) {
                                continue;
                            }
                            fills.push((ty, slot, Inst {
                                key: self.impl_method_key(idx, tm.name, true),
                                subst: env.clone(),
                                trait_origins: vec![],
                            }));
                        }
                    }
                }
            }
        }
        for (_, _, inst) in &fills {
            let _ = self.compile_queue(inst.clone());
        }
        for inst in extra {
            let _ = self.compile_queue(inst);
        }

        let total_slots: usize = self.traits.iter().map(|t| t.methods.len()).sum();
        let mut vt = vec![Vec::new(); self.types.types.len()];
        for t in vt.iter_mut() {
            *t = vec![None; total_slots];
        }
        for (ty, slot, inst) in fills {
            if let Some(&fid) = self.inst_map.get(&inst) {
                vt[self.types.dense(ty) as usize][slot as usize] = Some(fid);
            }
        }
        vt
    }

    // ---- module lets (RFC 0003 §1: load-time expressions only) ----

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
                        "module `let` initializers must be load-time expressions — literals only in this build (RFC 0003 §1, RFC 0033 §3)",
                    );
                }
            }
            let _ = ty;
        }
    }
}
