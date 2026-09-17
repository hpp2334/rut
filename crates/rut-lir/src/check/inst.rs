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
        self.funcs.push(FuncCode {
            name: self.inst_name(&inst),
            params: vec![],
            ret: TY_NIL,
            is_method: false,
            n_captures: 0,
            regs: vec![],
            argv: vec![],
            labels: vec![],
            code: vec![],
            spans: vec![],
            host: None,
        });
        self.queue.push(inst);
        fid
    }

    pub(crate) fn inst_name(&self, inst: &Inst) -> String {
        match inst.key {
            FnKey::Free(n) => self.name(n).to_string(),
            FnKey::Method { data, name } => format!("{}${}", self.name(data), self.name(name)),
            FnKey::ImplMethod { idx, name } => {
                let tid = self.impls[idx].trait_id;
                format!("{}#${}${}", self.traits[tid as usize].name, idx, self.name(name))
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

    /// after all instantiations: fill per-type vtables from impl blocks
    /// vtables synthesize per (type × interface) — the duck-typed law
    /// (RFC 0012 v1.1): a concrete data type fills the slots whose
    /// interface methods its OWN methods match by shape. Satisfying
    /// methods compile here (with their transitive calls) so every
    /// reachable slot carries a real function id.
    pub fn build_vtables(&mut self) -> Vec<Vec<Option<u32>>> {
        let mut targets: Vec<TypeId> = self
            .datas
            .iter()
            .filter(|(_, d)| d.generics.is_empty())
            .map(|(_, d)| d.ty)
            .collect();
        targets.extend(self.inst_data.keys().cloned());

        let mut fills: Vec<(TypeId, u32, Inst)> = Vec::new();
        for ty in &targets {
            for trait_id in 0..self.traits.len() as u32 {
                if !self.duck_satisfies(*ty, trait_id) {
                    continue;
                }
                let (dname, args) = match self.inst_data.get(ty) {
                    Some((d, a)) => (*d, a.clone()),
                    None => match self.datas.iter().find(|(_, d)| d.ty == *ty) {
                        Some((n, d)) => (*n, Vec::new()),
                        _ => continue,
                    },
                };
                let Some((_, d)) = self.datas.iter().find(|(n, _)| n == &dname) else {
                    continue;
                };
                let d = d.clone();
                let tdesc = self.trait_by_id(trait_id).clone();
                let env: Vec<(IdentId, TypeId)> =
                    d.generics.iter().cloned().zip(args.iter().cloned()).collect();
                for (midx, tm) in tdesc.methods.iter().enumerate() {
                    let Some((mname, _)) =
                        d.methods.iter().find(|(n, _)| self.name(*n) == tm.name)
                    else {
                        continue;
                    };
                    let Some(slot) = self.trait_slot(trait_id, midx as u32) else {
                        continue;
                    };
                    let inst = Inst {
                        key: FnKey::Method { data: dname, name: *mname },
                        subst: env.clone(),
                    };
                    let _ = self.compile_queue(inst.clone());
                    fills.push((*ty, slot, inst));
                }
            }
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
