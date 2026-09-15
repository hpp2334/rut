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
            ret: TY_UNIT,
            is_method: false,
            n_captures: 0,
            regs: vec![],
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
    pub fn build_vtables(&mut self) -> Vec<Vec<Option<u32>>> {
        let mut vt = vec![Vec::new(); self.types.types.len()];
        let total_slots: usize = self.traits.iter().map(|t| t.methods.len()).sum();
        for t in vt.iter_mut() {
            *t = vec![None; total_slots];
        }
        for (idx, im) in self.impls.iter().enumerate() {
            for (mname, _) in &im.methods {
                let trait_id = im.trait_id;
                let tdesc = &self.traits[trait_id as usize];
                if let Some(midx) = tdesc.methods.iter().position(|m| m.name == self.name(*mname)) {
                    let slot = self.trait_slot(trait_id, midx as u32).unwrap();
                    let inst = Inst {
                        key: FnKey::ImplMethod { idx, name: *mname },
                        subst: vec![],
                    };
                    if let Some(&fid) = self.inst_map.get(&inst) {
                        vt[self.types.dense(im.target) as usize][slot as usize] = Some(fid);
                    }
                }
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
