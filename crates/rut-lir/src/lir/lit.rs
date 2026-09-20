//! Composite literals: f-strings (the concat desugaring, RFC 0007 SS2),
//! struct literals (every field initialized, RFC 0009), array literals,
//! and closures (by-value captures; RFC 0013 SS1 with the v1 capture
//! scan).

use crate::check::TcResult;
use rut_ast::ast::{ExprKind, Lit};
use rut_core::binary::ConstVal;
use rut_core::ops::*;
use rut_core::types::*;
use super::*;

/// A `[v; n]` fill whose value is a compile-time all-zero literal —
/// `nil`, `0` (any int suffix), `0.0`, `false`. The array block arrives
/// zeroed, so these constructions need no fill loop. `-0.0` (a `Neg` of
/// a literal) does NOT match: its bits are not zero.
fn is_zero_fill(e: &ExprKind) -> bool {
    match e {
        ExprKind::Lit(Lit::Nil) => true,
        ExprKind::Lit(Lit::Int(0, _)) => true,
        // f64-bit form of 0.0/0f32 — all-zero bits
        ExprKind::Lit(Lit::Float(0, _)) => true,
        ExprKind::Lit(Lit::Bool(false)) => true,
        _ => false,
    }
}

impl<'a, 'b> FnCompiler<'a, 'b> {
    // ---- f-strings: the concat desugaring (RFC 0007 §2) ----

    pub(crate) fn compile_fstr(&mut self, parts: Vec<FPartAst>, _expected: Option<TypeId>, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        let part_regs = self.compile_fstr_parts(&parts, sp)?;
        // An f-string with a single part needs no concatenation — the part
        // already is the result (`f"{x}"` is `x` after the `Str` above).
        if part_regs.len() == 1 {
            self.last_reg = part_regs[0];
            return Ok(TY_STR);
        }
        let dst = self.new_reg(TY_STR);
        { let (argv_off, argc) = self.pool_args(&(part_regs)); self.emit(Op::CallNat { nat: Nat::Concat, recv: NOREG, argv_off, argc, dst: dst }, sp.lo); }
        Ok(TY_STR)
    }

    /// The accumulator form `s = f"{s}{..}"`: build the concat with `acc`
    /// (the accumulator's own register) as BOTH the first operand and the
    /// destination. `compile_expr` would otherwise copy the local into a
    /// fresh register, so the VM could never append in place and every step
    /// would copy the whole prefix — O(n^2) for a string built in a loop.
    /// With `dst == args[0]` the VM appends into the uniquely-owned cell
    /// (geometric growth, amortized O(1)). `parts[..skip]` is the
    /// accumulator; `parts[skip..]` are compiled normally.
    pub(crate) fn compile_fstr_into(
        &mut self,
        parts: &[FPartAst],
        skip: usize,
        acc: u16,
        _expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let mut part_regs = vec![acc];
        part_regs.extend(self.compile_fstr_parts(&parts[skip..], sp)?);
        { let (argv_off, argc) = self.pool_args(&(part_regs)); self.emit(Op::CallNat { nat: Nat::Concat, recv: NOREG, argv_off, argc, dst: acc }, sp.lo); }
        Ok(TY_STR)
    }

    /// If `value` is `f"{name}{rest..}"` — the accumulator's own name as the
    /// first hole — lower it into `acc` via `compile_fstr_into` and return
    /// `true`. Bails when the name reappears in a later part: it would alias
    /// the cell (defeating the in-place append), and a nested reassignment
    /// would change evaluation order.
    pub(crate) fn try_accumulate_fstr(
        &mut self,
        value: NodeHandle<AnyExpr>,
        name: IdentId,
        acc: u16,
        sp: rut_lexer::span::Span,
    ) -> TcResult<bool> {
        let ExprKind::FStr { parts } = self.ctx.ast.expr(value).clone() else {
            return Ok(false);
        };
        if parts.len() < 2 {
            return Ok(false);
        }
        let FPartAst::Hole(e0) = &parts[0] else { return Ok(false) };
        let ExprKind::Path { segs } = self.ctx.ast.expr(*e0).clone() else {
            return Ok(false);
        };
        if segs.len() != 1 || segs[0].name != name || !segs[0].generics.is_empty() {
            return Ok(false);
        }
        for p in &parts[1..] {
            if let FPartAst::Hole(e) = p {
                let mut names = Vec::new();
                self.scan_names(e.id(), &mut names);
                if names.contains(&name) {
                    return Ok(false);
                }
            }
        }
        self.compile_fstr_into(&parts, 1, acc, Some(TY_STR), sp)?;
        Ok(true)
    }

    /// Compile f-string parts into registers: a literal becomes a `str`
    /// constant, a hole its value (converted through `Nat::Str` unless it is
    /// already a `str`, RFC 0007 §2).
    fn compile_fstr_parts(&mut self, parts: &[FPartAst], sp: rut_lexer::span::Span) -> TcResult<Vec<u16>> {
        let mut part_regs: Vec<u16> = Vec::new();
        for p in parts {
            match p {
                FPartAst::Lit(s) => {
                    let k = self.konst(ConstVal::Str(s.clone()));
                    let reg = self.new_reg(TY_STR);
                    self.emit(Op::Const { dst: reg, k: k as u32 }, sp.lo);
                    part_regs.push(reg);
                }
                FPartAst::Hole(e) => {
                    let mut t = self.compile_expr(*e, None)?;
                    // `{p}` formats the pointee (RFC 0012 §6): deref the
                    // box before formatting — any element type
                    let lo = self.ctx.ast.span(e.id()).lo;
                    if let TyKind::Opt { elem } = self.ctx.types.kind(t).clone() {
                        let src = self.last_reg;
                        let d = self.new_reg(elem);
                        self.emit(Op::GetF { dst: d, obj: src, field: 0, repr: self.ctx.types.repr_of(elem) }, lo);
                        t = elem;
                    }
                    self.check_formattable(t, self.ctx.ast.span(e.id()))?;
                    let src = self.last_reg;
                    if t == TY_STR {
                        // already a string: `Str` is a pure alias (RFC 0007 §2)
                        // — skip the call and use the value directly.
                        part_regs.push(src);
                    } else {
                        let sreg = self.new_reg(TY_STR);
                        { let (argv_off, argc) = self.pool_args(&(vec![src])); self.emit(Op::CallNat { nat: Nat::Str, recv: NOREG, argv_off, argc, dst: sreg }, sp.lo); }
                        part_regs.push(sreg);
                    }
                }
            }
        }
        Ok(part_regs)
    }

    // ---- literals: struct / array ----

    pub(crate) fn compile_struct(&mut self, ty: NodeHandle<AnyTy>, fields: Vec<(IdentId, NodeHandle<AnyExpr>)>, _expected: Option<TypeId>, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        // `Self` binds inside class bodies (RFC 0010 §1)
        let sty = self.resolve_type_now(ty);
        // resolve the record: a local generic instantiation, a local record,
        // or a used one
        let inst = self.ctx.inst_data.get(&sty).cloned();
        let local = self.ctx.datas.iter().find(|(_, d)| d.ty == sty).map(|(n, d)| (*n, d.clone()));
        let (dname, kind, field_list, class_subst): (
            IdentId,
            crate::check::DataKind,
            Vec<(IdentId, TypeId, Option<NodeHandle<AnyExpr>>)>,
            Vec<(IdentId, TypeId)>,
        ) = if let Some((dname, cargs)) = inst {
            let d = self.ctx.find_data(dname).cloned().unwrap();
            let class_subst: Vec<(IdentId, TypeId)> =
                d.generics.iter().cloned().zip(cargs.iter().cloned()).collect();
            let field_nodes = match self.ctx.ast.item(d.node) {
                ItemKind::Dataclass { fields, .. } | ItemKind::Class { fields, .. } => fields.clone(),
                _ => Vec::new(),
            };
            let saved_subst = std::mem::replace(&mut self.subst, class_subst.clone());
            let saved_self = self.self_ty;
            self.self_ty = Some(sty);
            let mut list = Vec::new();
            for f in &field_nodes {
                let fd = self.ctx.ast.field_decl(*f);
                let fty = self.resolve_type_now(fd.ty);
                list.push((fd.name, fty, fd.init));
            }
            self.subst = saved_subst;
            self.self_ty = saved_self;
            (dname, d.kind, list, class_subst)
        } else if let Some((dname, d)) = local {
            let list = d.fields.iter().map(|(n, t, i, _)| (*n, *t, *i)).collect();
            (dname, d.kind, list, Vec::new())
        } else {
            return self.compile_struct_extern(sty, fields, sp);
        };
        // classes have no outside literal (RFC 0010 §1); the Self {} literal
        // is legal only inside the class body
        let inside_body = self.current_class == Some(dname);
        if kind == crate::check::DataKind::Class && !inside_body {
            self.ctx.err(sp, format!(
                "classes have no instance literal —construct through a class method (`{}.new(..)`, RFC 0010 §1)",
                self.ctx.name(dname)
            ));
            return Err(());
        }
        // every field initialized (any order, by name) or has an initializer
        // (RFC 0009); collect each field value in a register, then mint the
        // whole record with one MakeRecord (no NewCell/SetF/MovRef sequence)
        let mut val_regs: Vec<Option<u16>> = vec![None; field_list.len()];
        let mut set: Vec<bool> = vec![false; field_list.len()];
        for (fname, v) in &fields {
            let Some(fidx) = field_list.iter().position(|(n, _, _)| n == fname) else {
                self.ctx.err(self.ctx.ast.span(v.id()), format!(
                    "`{}` has no field `{}`", self.ctx.name(dname), self.ctx.name(*fname)
                ));
                return Err(());
            };
            let fty = field_list[fidx].1;
            let t = self.compile_expr(*v, Some(fty))?;
            if t != fty {
                self.ctx.err(self.ctx.ast.span(v.id()), format!(
                    "field `{}` is `{}`, found `{}`",
                    self.ctx.name(*fname), self.ctx.type_name(fty), self.ctx.type_name(t)
                ));
            }
            val_regs[fidx] = Some(self.last_reg);
            set[fidx] = true;
        }
        for (fidx, (fname, fty, init)) in field_list.iter().enumerate() {
            if !set[fidx] {
                match init {
                    None => {
                        // zero-value defaults (RFC 0007): an omitted field
                        // takes its type's zero value
                        let z = self.zero_value(*fty, sp)?;
                        val_regs[fidx] = Some(z);
                        continue;
                    }
                    Some(init) => {
                        // initializers are class code: resolve under the class subst
                        let saved_subst = std::mem::replace(&mut self.subst, class_subst.clone());
                        let saved_self = self.self_ty;
                        self.self_ty = Some(sty);
                        let t = self.compile_expr(*init, Some(*fty));
                        self.subst = saved_subst;
                        self.self_ty = saved_self;
                        let t = t?;
                        if t != *fty {
                            self.ctx.err(self.ctx.ast.span(init.id()), "field initializer type mismatch");
                        }
                        val_regs[fidx] = Some(self.last_reg);
                    }
                }
            }
        }
        let vals: Vec<u16> = val_regs
            .into_iter()
            .map(|r| r.expect("checked: every field is initialized"))
            .collect();
        let dst = self.new_reg(sty);
        { let (argv_off, argc) = self.pool_args(&(vals)); self.emit(Op::MakeRecord { dst: dst, ty: sty, argv_off, argc }, sp.lo); }
        Ok(sty)
    }

    /// The zero value of a type (RFC 0007): `0`/`0.0`/`false`, the empty
    /// string, `nil` for pointers, member 0 for enums, the all-zero record
    /// for records.
    pub(crate) fn zero_value(&mut self, ty: TypeId, sp: rut_lexer::span::Span) -> TcResult<u16> {
        let reg = self.new_reg(ty);
        match self.ctx.types.kind(ty).clone() {
            TyKind::Prim(_) | TyKind::Nil | TyKind::Opt { .. } => {
                self.emit(Op::ConstRaw { dst: reg, bits: 0 }, sp.lo);
            }
            TyKind::Str => {
                let k = self.konst(ConstVal::Str(String::new()));
                self.emit(Op::Const { dst: reg, k: k as u32 }, sp.lo);
            }
            TyKind::Enum { .. } => {
                self.emit(Op::EnumNew { dst: reg, ty, member: 0 }, sp.lo);
            }
            TyKind::Data { fields } => {
                let mut vals = Vec::with_capacity(fields.len());
                for f in &fields {
                    vals.push(self.zero_value(f.ty, sp)?);
                }
                { let (argv_off, argc) = self.pool_args(&(vals)); self.emit(Op::MakeRecord { dst: reg, ty: ty, argv_off, argc }, sp.lo); }
            }
            _ => {
                self.ctx.err(sp, format!(
                    "a literal must initialize `{}` —it has no zero value (RFC 0009)",
                    self.ctx.type_name(ty)
                ));
                return Err(());
            }
        }
        Ok(reg)
    }

    /// Record literal for a USED struct (RFC 0035 §1): the layout is
    /// the copied type descriptor; field names compare by string (separate
    /// ASTs intern separately), and field defaults from other modules are not carried.
    fn compile_struct_extern(
        &mut self,
        sty: TypeId,
        fields: Vec<(IdentId, NodeHandle<AnyExpr>)>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let TyKind::Data { fields: desc } = self.ctx.types.kind(sty).clone() else {
            self.ctx.err(sp, format!("`{}` is not a struct/class of this module", self.ctx.type_name(sty)));
            return Err(());
        };
        if self.ctx.extern_classes.contains(&sty) {
            self.ctx.err(sp, format!(
                "classes have no instance literal — construct through a class method (`{}.new(..)`, RFC 0010 §1)",
                self.ctx.type_name(sty)
            ));
            return Err(());
        }
        let n = desc.len();
        let mut val_regs: Vec<Option<u16>> = vec![None; n];
        for (fname, v) in &fields {
            let Some(fidx) = desc.iter().position(|f| f.name == *fname) else {
                self.ctx.err(self.ctx.ast.span(v.id()), format!(
                    "`{}` has no field `{}`", self.ctx.type_name(sty), self.ctx.name(*fname)
                ));
                return Err(());
            };
            let fty = desc[fidx].ty;
            let t = self.compile_expr(*v, Some(fty))?;
            if t != fty {
                self.ctx.err(self.ctx.ast.span(v.id()), format!(
                    "field `{}` is `{}`, found `{}`",
                    self.ctx.name(*fname), self.ctx.type_name(fty), self.ctx.type_name(t)
                ));
            }
            val_regs[fidx] = Some(self.last_reg);
        }
        if let Some(missing) = desc
            .iter()
            .zip(&val_regs)
            .find(|(_, r)| r.is_none())
            .map(|(f, _)| f.name)
        {
            self.ctx.err(sp, format!(
                "record `{}` from another module must initialize every field — `{}` is missing (cross-module field defaults are not carried, RFC 0035 §1)",
                self.ctx.type_name(sty), self.ctx.name(missing)
            ));
            return Err(());
        }
        let vals: Vec<u16> = val_regs.into_iter().map(|r| r.unwrap()).collect();
        let dst = self.new_reg(sty);
        { let (argv_off, argc) = self.pool_args(&(vals)); self.emit(Op::MakeRecord { dst: dst, ty: sty, argv_off, argc }, sp.lo); }
        Ok(sty)
    }

    pub(crate) fn compile_array_lit(&mut self, elems: Vec<NodeHandle<AnyExpr>>, expected: Option<TypeId>, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        // `[e1, .., en] : [T]` (RFC 0005 §9, RFC 0007 §1); T from expected or the
        // first element; uncontextualized int elements default to i32
        let elem_hint = match expected.map(|e| self.ctx.types.kind(e).clone()) {
            Some(TyKind::Array { elem }) => Some(elem),
            _ => None,
        };
        let mut eregs = Vec::new();
        let mut ety = elem_hint;
        for (i, e) in elems.iter().enumerate() {
            let t = self.compile_expr(*e, if i == 0 { ety } else { ety })?;
            if i == 0 {
                // keep the expected element when the first element widens to it
                ety = match ety {
                    Some(u) if self.widens(t, u) => Some(u),
                    _ => Some(t),
                };
            } else if let Some(u) = ety {
                // RFC 0012 §2: heterogeneous elements unify through trait objects
                if self.widens(u, t) {
                    ety = Some(t);
                } else if !self.widens(t, u) {
                    self.ctx.err(self.ctx.ast.span(e.id()), "array literal elements must agree on one type");
                }
            }
            eregs.push(self.last_reg);
        }
        let elem = ety.unwrap_or(TY_I32);
        let aty = self.ctx.mk_array(elem);
        let dst = self.new_reg(aty);
        { let (argv_off, argc) = self.pool_args(&(eregs)); self.emit(Op::ArrLit { dst: dst, ty: aty, argv_off, argc }, sp.lo); }
        Ok(aty)
    }

    /// `[v; n]` — the repeat construction (RFC 0005 §9): `ArrNew` for n
    /// slots, then a fill loop storing `v` into each. A scalar/nil fill is
    /// the memset-class op (the store is a plain slot move); a ref fill
    /// copies the cell handle n times — every slot aliases the one cell.
    pub(crate) fn compile_array_repeat(
        &mut self,
        value: NodeHandle<AnyExpr>,
        count: NodeHandle<AnyExpr>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let elem_hint = match expected.map(|e| self.ctx.types.kind(e).clone()) {
            Some(TyKind::Array { elem }) => Some(elem),
            _ => None,
        };
        // the value first (its type names the element; the hint flows in
        // from the annotation — `[nil; cap]` over a `[*T]`), then the count
        let vt = self.compile_expr(value, elem_hint)?;
        let elem = match elem_hint {
            Some(u) if self.widens(vt, u) => u,
            _ => vt,
        };
        let val = self.last_reg;
        let ct = self.compile_expr(count, Some(TY_I32))?;
        if ct != TY_I32 {
            self.ctx.err(sp, format!("the repeat count must be `i32`, found `{}`", self.ctx.type_name(ct)));
        }
        let len = self.last_reg;
        let aty = self.ctx.mk_array(elem);
        let dst = self.new_reg(aty);
        self.emit(Op::ArrNew { dst, ty: aty, len, repr: rut_core::types::arr_elem_repr(&self.ctx.types, elem) }, sp.lo);
        // a `nil` fill IS the zero-fill — ArrNew alone is the memset. Any
        // all-zero literal is too (0, 0u64, 0u8, false, 0.0): the block
        // arrives zeroed, so the fill loop would rewrite zero with zero
        if is_zero_fill(&self.ctx.ast.expr(value)) {
            self.last_reg = dst;
            return Ok(aty);
        }
        // fill: `for i in 0..len { dst[i] = val }`
        let zero = self.new_reg(TY_I32);
        self.emit(Op::ConstRaw { dst: zero, bits: 0 }, sp.lo);
        let idx = self.new_reg(TY_I32);
        self.emit(Op::Mov { dst: idx, src: zero }, sp.lo);
        let one = self.new_reg(TY_I32);
        self.emit(Op::ConstRaw { dst: one, bits: 1 }, sp.lo);
        let l_head = self.new_label();
        let l_body = self.new_label();
        let l_end = self.new_label();
        self.bind(l_head);
        self.emit(Op::LoopHead, sp.lo);
        let more = self.new_reg(TY_BOOL);
        self.emit(cmpop(CmpOp::Lt, PrimTy::I32, more, idx, len), sp.lo);
        self.br(more, l_body, l_end);
        self.bind(l_body);
        let repr = rut_core::types::arr_elem_repr(&self.ctx.types, elem);
        self.emit(Op::ArrSet { arr: dst, idx, val, repr }, sp.lo);
        let next = self.new_reg(TY_I32);
        self.emit(arith(ArithOp::Add, PrimTy::I32, next, idx, one), sp.lo);
        self.emit(Op::Mov { dst: idx, src: next }, sp.lo);
        self.jmp(l_head);
        self.bind(l_end);
        self.last_reg = dst;
        Ok(aty)
    }

    // ---- closures (RFC 0013 §1 —v1 captures by value) ----

    pub(crate) fn compile_lambda(
        &mut self,
        lambda_node: NodeId,
        params: Vec<NodeHandle<AnyParam>>,
        ret: Option<NodeHandle<AnyTy>>,
        body: NodeHandle<AnyExpr>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        let (eptys, eret) = match expected.map(|e| self.ctx.types.kind(e).clone()) {
            Some(TyKind::Fn { params, ret }) => (params, ret),
            _ => (Vec::new(), TY_NIL),
        };
        // param types: annotations first, then the expected fn type
        let mut param_tys = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let ty = match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { ty: Some(t), .. }) => self.resolve_type_now(*t),
                MemberKind::Param(ParamData { ty: None, .. }) => {
                    if let Some(&t) = eptys.get(i) {
                        t
                    } else {
                        self.ctx.err(self.ctx.ast.span(p.id()), "lambda parameter needs a type annotation (or an expected fn type)");
                        TY_I32
                    }
                }
                _ => {
                    self.ctx.err(self.ctx.ast.span(p.id()), "lambdas take no `self`");
                    TY_I32
                }
            };
            param_tys.push(ty);
        }
        let ptys = param_tys;
        let ret_ty = ret.map(|r| self.resolve_type_now(r)).unwrap_or(eret);
        // capture scan: free names that resolve to enclosing locals
        let mut referenced = Vec::new();
        self.scan_names(body.id(), &mut referenced);
        let lambda_param_names: Vec<IdentId> = params
            .iter()
            .filter_map(|p| match self.ctx.ast.param(*p) {
                MemberKind::Param(ParamData { name, .. }) => Some(*name),
                _ => None,
            })
            .collect();
        let mut caps: Vec<(IdentId, TypeId, bool, u16)> = Vec::new();
        for n in referenced {
            if lambda_param_names.contains(&n) {
                continue;
            }
            if let Some(l) = self.lookup(n).cloned() {
                // copy the CURRENT value into a capture register (by value);
                // the capture inherits the binding's mutability so writes
                // through a captured `let mut` stay legal (RFC 0044 — the
                // old pointer exception is gone)
                let cap_reg = self.new_reg(l.ty);
                if self.ctx.types.is_ref(l.ty) {
                    self.emit(Op::MovRef { dst: cap_reg, src: l.reg }, sp.lo);
                } else {
                    self.emit(Op::Mov { dst: cap_reg, src: l.reg }, sp.lo);
                }
                caps.push((n, l.ty, l.is_mut, cap_reg));
            }
        }
        // register the synthetic fn: params = declared ++ captures
                let node_id = self.ctx.ast.span(body.id()).lo; // not unique per node —use body NodeId instead
        let _ = node_id;
        // the Lambda node id: find it by body —the caller passes parts; the
        // lambda node is the PARENT of body. Store captures keyed by the
        // body node; FnKey::Lambda uses the body node id (unique).
        // record the resolved signature for the body compilation
        self.ctx.lambda_sigs.insert(lambda_node, (ptys.clone(), ret_ty));
        self.ctx.lambda_info.insert(lambda_node, caps.iter().map(|(n, t, m, _)| (*n, *t, *m)).collect());
        let inst = crate::check::Inst { key: crate::check::FnKey::Lambda(lambda_node), subst: vec![], trait_origins: vec![] };
        let fid = self.ctx.ensure_inst(inst);
        let fty = self.ctx.mk_fn_ty(ptys.clone(), ret_ty);
        let dst = self.new_reg(fty);
        { let (argv_off, argc) = self.pool_args(&(caps.iter().map(|(_, _, _, r)| *r).collect::<Vec<_>>())); self.emit(Op::MakeClosure { dst: dst, func: fid, argv_off, argc }, sp.lo,); }
        Ok(fty)
    }

    /// collect every single-segment path name under `node` (capture scan).
    /// Generic walk over the arena by `NodeId` — it must cross statement and
    /// expression categories freely; type subtrees carry no value names.
    pub(crate) fn scan_names(&mut self, node: NodeId, out: &mut Vec<IdentId>) {
        if out.len() > 4096 {
            return;
        }
        let kids = |n: NodeId, out: &mut Vec<IdentId>, s: &mut Self| s.scan_names(n, out);
        match self.ctx.ast.kind(node).clone() {
            // items / members / patterns / types carry no value names
            Kind::Item(_) | Kind::Member(_) | Kind::Pat(_) | Kind::Type(_) => {}
            Kind::Stmt(StmtKind::LetStmt { init, .. }) => kids(init.id(), out, self),
            Kind::Stmt(StmtKind::If { cond, then, els }) => {
                kids(cond.id(), out, self);
                kids(then.id(), out, self);
                if let Some(e) = els {
                    match e {
                        ElseBranch::If(h) => kids(h.id(), out, self),
                        ElseBranch::Block(h) => kids(h.id(), out, self),
                    }
                }
            }
            Kind::Stmt(StmtKind::While { cond, body }) => {
                kids(cond.id(), out, self);
                kids(body.id(), out, self);
            }
            Kind::Stmt(StmtKind::ForOf { iter, body, .. }) => {
                kids(iter.id(), out, self);
                kids(body.id(), out, self);
            }
            Kind::Stmt(StmtKind::ForC { init, cond, update, body, .. }) => {
                kids(init.id(), out, self);
                kids(cond.id(), out, self);
                kids(update.id(), out, self);
                kids(body.id(), out, self);
            }
            Kind::Stmt(StmtKind::Return { value }) => {
                if let Some(v) = value {
                    kids(v.id(), out, self);
                }
            }
            Kind::Stmt(StmtKind::WhenStmt { scrut, arms }) => {
                kids(scrut.id(), out, self);
                for a in arms {
                    kids(a.id(), out, self);
                }
            }
            Kind::Stmt(StmtKind::ExprStmt(e)) => kids(e.id(), out, self),
            Kind::Stmt(StmtKind::Break | StmtKind::Continue) => {}
            Kind::Arm(ArmKind::WhenArm { pats, body }) => {
                for p in pats {
                    kids(p.id(), out, self);
                }
                kids(body.id(), out, self);
            }
            Kind::Arm(ArmKind::SelectArm { fut, body, .. }) => {
                kids(fut.id(), out, self);
                kids(body.id(), out, self);
            }
            Kind::Expr(ExprKind::Block { stmts }) => {
                for s in stmts {
                    kids(s.id(), out, self);
                }
            }
            Kind::Expr(ExprKind::Path { segs }) => {
                // the HEAD of any path chain is a name candidate — a
                // multi-seg chain (`acc.total`) names its head local just
                // like a bare path does; lookup filters non-locals
                if !segs.is_empty() && out.len() < 4096 && !out.contains(&segs[0].name) {
                    out.push(segs[0].name);
                }
            }
            Kind::Expr(ExprKind::Lit(_)) => {}
            Kind::Expr(ExprKind::Call { callee, args }) => {
                kids(callee.id(), out, self);
                for a in args {
                    kids(a.id(), out, self);
                }
            }
            Kind::Expr(ExprKind::Method { recv, args, .. }) => {
                kids(recv.id(), out, self);
                for a in args {
                    kids(a.id(), out, self);
                }
            }
            Kind::Expr(ExprKind::Field { recv, .. }) => kids(recv.id(), out, self),
            Kind::Expr(ExprKind::Index { recv, idx }) => {
                kids(recv.id(), out, self);
                kids(idx.id(), out, self);
            }
            Kind::Expr(ExprKind::Unary { expr, .. }) => kids(expr.id(), out, self),
            Kind::Expr(ExprKind::Binary { lhs, rhs, .. }) => {
                kids(lhs.id(), out, self);
                kids(rhs.id(), out, self);
            }
            Kind::Expr(ExprKind::Assign { target, value, .. }) => {
                kids(target.id(), out, self);
                kids(value.id(), out, self);
            }
            Kind::Expr(ExprKind::Lambda { body: b, .. }) => kids(b.id(), out, self),
            Kind::Expr(ExprKind::Try { expr }) | Kind::Expr(ExprKind::Await { expr }) => kids(expr.id(), out, self),
            Kind::Expr(ExprKind::Select { arms }) => {
                for a in arms {
                    kids(a.id(), out, self);
                }
            }
            Kind::Expr(ExprKind::Is { expr, .. }) => kids(expr.id(), out, self),
            Kind::Expr(ExprKind::Cast { expr, .. }) => kids(expr.id(), out, self),
            Kind::Expr(ExprKind::FStr { parts }) => {
                for p in parts {
                    if let FPartAst::Hole(e) = p {
                        kids(e.id(), out, self);
                    }
                }
            }
            Kind::Expr(ExprKind::Struct { fields, .. }) => {
                for (_, v) in fields {
                    kids(v.id(), out, self);
                }
            }
            Kind::Expr(ExprKind::ArrayLit { elems }) => {
                for e in elems {
                    kids(e.id(), out, self);
                }
            }
            Kind::Expr(ExprKind::ArrayRepeat { value, count }) => {
                kids(value.id(), out, self);
                kids(count.id(), out, self);
            }
            Kind::Expr(ExprKind::Tuple { elems }) => {
                for e in elems {
                    kids(e.id(), out, self);
                }
            }
            Kind::Expr(ExprKind::WhenExpr { scrut, arms }) => {
                kids(scrut.id(), out, self);
                for a in arms {
                    kids(a.id(), out, self);
                }
            }
        }
    }
}
