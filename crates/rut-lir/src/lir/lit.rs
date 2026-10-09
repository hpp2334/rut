//! Composite literals: f-strings (the concat desugaring),
//! struct literals (every field initialized), array literals,
//! and closures (by-value captures with the v1 capture
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
    // ---- f-strings: the concat desugaring ----

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
    /// already a `str`).
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
                    // `{p}` formats the pointee: deref the
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
                        // already a string: `Str` is a pure alias
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

    // ---- the newtype call construction ----
    //
    // `JsonI64(64)`, `Tail([1, 2])`, `Converter<i32>("64")` — the
    // positional one-field class's compiler-provided constructor. THE
    // manufacture mechanism for everything satisfaction can't reach: a
    // wrapper comes into being only where a constructor is spelled —
    // never auto-inserted, never forwarded (the wrapper exposes exactly
    // its own members). The construction mints a real cell (MakeRecord).

    /// The LOCAL newtype's construction (the decl is this module's).
    pub(crate) fn compile_newtype_ctor(
        &mut self,
        name: IdentId,
        d: &crate::check::DataDecl,
        generics: Vec<NodeHandle<AnyTy>>,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        if args.len() != 1 {
            self.ctx.err(sp, format!(
                "`{}`(..) takes exactly one wrapped value",
                self.ctx.name(name)
            ));
            return Err(());
        }
        // the wrapped type, as the decl spells it — the synthesized
        // `inner` field's node (the desugared decl's single field)
        let field_node = match self.ctx.ast.item(d.node) {
            ItemKind::Class { fields, .. } if fields.len() == 1 => fields[0],
            _ => {
                self.ctx.err(sp, format!("`{}` is not a newtype class", self.ctx.name(name)));
                return Err(());
            }
        };
        let fd = self.ctx.ast.field_decl(field_node);
        if d.generics.is_empty() {
            // the concrete wrapper: the wrapped value checks against the
            // field type as written, one cell mints
            let fty = self.ctx.resolve_type(fd.ty, &[]);
            let t = self.compile_expr(args[0], Some(fty))?;
            if !self.widens(t, fty) {
                self.ctx.err(self.ctx.ast.span(args[0].id()), format!(
                    "the wrapped value is `{}`, `{}` expected",
                    self.ctx.type_name(t), self.ctx.type_name(fty)
                ));
            }
            self.widen_to_slot(t, fty, sp.lo);
            let val = self.last_reg;
            return Ok(self.mint_newtype(d.ty, val, sp));
        }
        // the generic wrapper: explicit type args first (all-or-nothing —
        // the call spells ALL binders or none), else the binders the
        // wrapped argument determines (the free-fn unification rule)
        let (inst, fty, arg_ty) = if !generics.is_empty() {
            if generics.len() != d.generics.len() {
                self.ctx.err(sp, format!(
                    "`{}`<..> takes {} type argument(s), {} given — spell ALL binders or none",
                    self.ctx.name(name), d.generics.len(), generics.len()
                ));
                return Err(());
            }
            let inst_args: Vec<TypeId> = generics.iter().map(|g| self.resolve_type_now(*g)).collect();
            let inst = self.ctx.mk_data_inst(name, inst_args.clone(), sp);
            let env: Vec<(IdentId, TypeId)> = d
                .generics
                .iter()
                .cloned()
                .zip(inst_args.iter().cloned())
                .collect();
            let fty = self.ctx.resolve_type(fd.ty, &env);
            let t = self.compile_expr(args[0], Some(fty))?;
            (inst, fty, t)
        } else {
            // the binders the wrapped argument determines — the wrapped
            // type resolved under the `#<param>` placeholder env (the
            // template row's spelling) against the argument, the same
            // TypeId-level unification the used-newtype arm runs
            // (structural: `Vec<#T>` against `Vec<str>` binds `T := str`)
            let env: Vec<(IdentId, TypeId)> = d
                .generics
                .iter()
                .map(|&g| (g, self.ctx.param_placeholder(g)))
                .collect();
            let hint = self.ctx.resolve_type(fd.ty, &env);
            let t = self.compile_expr(args[0], Some(hint))?;
            let mut subst: Vec<(IdentId, TypeId)> = Vec::new();
            if !self.unify_template_field(hint, t, &d.generics, &mut subst) {
                self.ctx.err(self.ctx.ast.span(args[0].id()), format!(
                    "the wrapped value is `{}`, the wrapped type of `{}` expected",
                    self.ctx.type_name(t),
                    self.ctx.name(name)
                ));
                return Err(());
            }
            let undetermined = d
                .generics
                .iter()
                .find(|g| !subst.iter().any(|(n, _)| n == *g))
                .cloned();
            if let Some(g) = undetermined {
                // the binder the wrapped argument could not name — the
                // constructor demands its spelling (the annotated
                // binding's own arguments show the shape, when they
                // answer)
                let wrapped = crate::check::bound_ty_str(self.ctx, fd.ty);
                let spelled = match expected.and_then(|e| self.ctx.inst_data.get(&e).cloned()) {
                    Some((ed, eargs)) if ed == name => eargs
                        .iter()
                        .map(|a| self.ctx.type_name(*a).to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    _ => d
                        .generics
                        .iter()
                        .map(|g| self.ctx.name(*g).to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                };
                self.ctx.err(sp, format!(
                    "`{}`'s parameter `{}` is not determined by `{wrapped}` — spell it: `{}<{spelled}>(..)`",
                    self.ctx.name(name),
                    self.ctx.name(g),
                    self.ctx.name(name)
                ));
                return Err(());
            }
            let inst_args: Vec<TypeId> = d
                .generics
                .iter()
                .map(|g| {
                    subst
                        .iter()
                        .find(|(n, _)| n == g)
                        .map(|(_, t)| *t)
                        .unwrap_or(TY_I32)
                })
                .collect();
            let inst = self.ctx.mk_data_inst(name, inst_args.clone(), sp);
            let env: Vec<(IdentId, TypeId)> = d
                .generics
                .iter()
                .cloned()
                .zip(inst_args.iter().cloned())
                .collect();
            let fty = self.ctx.resolve_type(fd.ty, &env);
            (inst, fty, t)
        };
        if !self.widens(arg_ty, fty) {
            self.ctx.err(self.ctx.ast.span(args[0].id()), format!(
                "the wrapped value is `{}`, `{}` expected",
                self.ctx.type_name(arg_ty),
                self.ctx.type_name(fty)
            ));
        }
        // the slot widen runs AFTER the substitution completed — a
        // placeholder-hinted lambda argument crosses at its real ABI here
        self.widen_to_slot(arg_ty, fty, sp.lo);
        let val = self.last_reg;
        Ok(self.mint_newtype(inst, val, sp))
    }

    /// The USED newtype's construction — `theirjson::TheirJson(x)`,
    /// `Wrap<i32>(5)`: the surface row's flag arms the same spelled
    /// constructor at a distance. A concrete wrapper's layout is the
    /// carried row (the `inner` field descriptor); a generic one lays
    /// the mirror instantiation out of the template's placeholder field
    /// and routes the bodies request to the owner (the linkable-classes
    /// machinery — nothing new).
    pub(crate) fn compile_extern_newtype_ctor(
        &mut self,
        name: IdentId,
        generics: Vec<NodeHandle<AnyTy>>,
        args: Vec<NodeHandle<AnyExpr>>,
        expected: Option<TypeId>,
        sp: rut_lexer::span::Span,
    ) -> TcResult<TypeId> {
        if args.len() != 1 {
            self.ctx.err(sp, format!(
                "`{}`(..) takes exactly one wrapped value",
                self.ctx.name(name)
            ));
            return Err(());
        }
        // the generic form first: the template answers (a generic class
        // rides `extern_generics` too — the generic arm answers before
        // the concrete row)
        if let Some(g) = self.ctx.extern_generics.get(&name).cloned() {
            let template_field = match self.ctx.types.kind(g.template) {
                TyKind::Data { fields } if fields.len() == 1 => fields[0].ty,
                _ => {
                    self.ctx.err(sp, format!("`{}` is not a newtype class", self.ctx.name(name)));
                    return Err(());
                }
            };
            // explicit type args: all-or-nothing (the call spells ALL
            // binders or none), the wrapped value against the
            // substituted field row
            if !generics.is_empty() {
                if generics.len() != g.params.len() {
                    self.ctx.err(sp, format!(
                        "`{}`<..> takes {} type argument(s), {} given — spell ALL binders or none",
                        self.ctx.name(name),
                        g.params.len(),
                        generics.len()
                    ));
                    return Err(());
                }
                let inst_args: Vec<TypeId> =
                    generics.iter().map(|gn| self.resolve_type_now(*gn)).collect();
                let inst = self.ctx.mk_data_inst(name, inst_args.clone(), sp);
                let mut env: std::collections::HashMap<String, TypeId> =
                    std::collections::HashMap::new();
                for (p, &a) in g.params.iter().zip(inst_args.iter()) {
                    env.insert(format!("#{}", self.ctx.name(*p)), a);
                }
                let fty = self.ctx.subst_template_ty(template_field, &env);
                let t = self.compile_expr(args[0], Some(fty))?;
                if !self.widens(t, fty) {
                    self.ctx.err(self.ctx.ast.span(args[0].id()), format!(
                        "the wrapped value is `{}`, `{}` expected",
                        self.ctx.type_name(t), self.ctx.type_name(fty)
                    ));
                }
                self.widen_to_slot(t, fty, sp.lo);
                let val = self.last_reg;
                return Ok(self.mint_newtype(inst, val, sp));
            }
            // inference: the wrapped argument against the template's
            // placeholder field (structural — the consumer's side of the
            // free-fn rule). The argument compiles once; the substituted
            // slot check runs after the unification completed.
            let t = self.compile_expr(args[0], None)?;
            let mut subst: Vec<(IdentId, TypeId)> = Vec::new();
            if !self.unify_template_field(template_field, t, &g.params, &mut subst) {
                self.ctx.err(self.ctx.ast.span(args[0].id()), format!(
                    "the wrapped value is `{}`, the wrapped type of `{}` expected",
                    self.ctx.type_name(t),
                    self.ctx.name(name)
                ));
                return Err(());
            }
            let undetermined = g
                .params
                .iter()
                .find(|p| !subst.iter().any(|(n, _)| n == *p))
                .cloned();
            if let Some(p) = undetermined {
                let spelled = match expected.and_then(|e| self.ctx.inst_data.get(&e).cloned()) {
                    Some((ed, eargs)) if ed == name => eargs
                        .iter()
                        .map(|a| self.ctx.type_name(*a).to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                    _ => g
                        .params
                        .iter()
                        .map(|p| self.ctx.name(*p).to_string())
                        .collect::<Vec<_>>()
                        .join(", "),
                };
                self.ctx.err(sp, format!(
                    "`{}`'s parameter `{}` is not determined by the wrapped argument — spell it: `{}<{spelled}>(..)`",
                    self.ctx.name(name),
                    self.ctx.name(p),
                    self.ctx.name(name)
                ));
                return Err(());
            }
            let inst_args: Vec<TypeId> = g
                .params
                .iter()
                .map(|p| {
                    subst
                        .iter()
                        .find(|(n, _)| n == p)
                        .map(|(_, t)| *t)
                        .unwrap_or(TY_I32)
                })
                .collect();
            let inst = self.ctx.mk_data_inst(name, inst_args.clone(), sp);
            let mut env: std::collections::HashMap<String, TypeId> = std::collections::HashMap::new();
            for (p, &a) in g.params.iter().zip(inst_args.iter()) {
                env.insert(format!("#{}", self.ctx.name(*p)), a);
            }
            let fty = self.ctx.subst_template_ty(template_field, &env);
            if !self.widens(t, fty) {
                self.ctx.err(self.ctx.ast.span(args[0].id()), format!(
                    "the wrapped value is `{}`, `{}` expected",
                    self.ctx.type_name(t), self.ctx.type_name(fty)
                ));
            }
            self.widen_to_slot(t, fty, sp.lo);
            let val = self.last_reg;
            return Ok(self.mint_newtype(inst, val, sp));
        }
        // the concrete form: the carried row IS the layout
        let ty = self.ctx.extern_types.get(&name).cloned();
        let fty = ty.and_then(|ty| match self.ctx.types.kind(ty) {
            TyKind::Data { fields } if fields.len() == 1 => Some(fields[0].ty),
            _ => None,
        });
        let Some((ty, fty)) = ty.zip(fty) else {
            self.ctx.err(sp, format!("`{}` is not a newtype class", self.ctx.name(name)));
            return Err(());
        };
        let t = self.compile_expr(args[0], Some(fty))?;
        if !self.widens(t, fty) {
            self.ctx.err(self.ctx.ast.span(args[0].id()), format!(
                "the wrapped value is `{}`, `{}` expected",
                self.ctx.type_name(t), self.ctx.type_name(fty)
            ));
        }
        self.widen_to_slot(t, fty, sp.lo);
        let val = self.last_reg;
        Ok(self.mint_newtype(ty, val, sp))
    }

    /// The construction's emission: one cell, the wrapped value its
    /// single field. A real record — eliding the wrapper cell in static
    /// chains is a later optimization lane, never a surface law.
    fn mint_newtype(&mut self, ty: TypeId, val: u16, sp: rut_lexer::span::Span) -> TypeId {
        let dst = self.new_reg(ty);
        { let (argv_off, argc) = self.pool_args(&[val]); self.emit(Op::MakeRecord { dst, ty, argv_off, argc }, sp.lo); }
        ty
    }

    /// Structural unification of a carried template field row (its
    /// `#<param>` placeholder leaves) against a concrete argument: a
    /// placeholder leaf binds its parameter; the structural wrappers
    /// (`[T]`, `?T`) recurse; everything else is exact identity (the
    /// owner-anchored mirrors count as one).
    fn unify_template_field(
        &mut self,
        field: TypeId,
        arg: TypeId,
        params: &[IdentId],
        subst: &mut Vec<(IdentId, TypeId)>,
    ) -> bool {
        let text = self
            .ctx
            .interner
            .name(self.ctx.types.type_at(field).name)
            .to_string();
        // the parameter may vanish into the HEAD's own spelling
        // (`HashSet<#T>` — the host-table field carries no `#T`): a head
        // that spells the parameters decides FIRST — the field walk
        // would return true vacuously (the fields carry no `#T` to
        // bind)
        if text.contains('<') && self.unify_name_args(field, arg, params, subst) {
            return true;
        }
        if let Some(stripped) = text.strip_prefix('#') {
            if let Some(id) = self.ctx.lookup_name(stripped) {
                if params.contains(&id) {
                    if let Some(e) = subst.iter_mut().find(|(n, _)| *n == id) {
                        return self.same_ty(e.1, arg);
                    }
                    subst.push((id, arg));
                    return true;
                }
            }
        }
        match (self.ctx.types.kind(field).clone(), self.ctx.types.kind(arg).clone()) {
            (TyKind::Array { elem: a }, TyKind::Array { elem: b }) => {
                self.unify_template_field(a, b, params, subst)
            }
            (TyKind::Opt { elem: a }, TyKind::Opt { elem: b }) => {
                self.unify_template_field(a, b, params, subst)
            }
            // two instantiations of one generic class unify element-wise
            // (`Vec<#T>` against `Vec<JsonI64>` — the wrapper-construction
            // inference at a consumer); the FIELD-WISE walk answers even
            // when neither row is in `inst_data` (a template's stamped
            // fields — the local construction's hint)
            (TyKind::Data { fields: fa }, TyKind::Data { fields: fb })
                if fa.len() == fb.len()
                    && self.same_head_text(field, arg) =>
            {
                fa.iter().zip(fb.iter()).all(|(x, y)| {
                    x.name == y.name && self.unify_template_field(x.ty, y.ty, params, subst)
                })
            }
            _ if self.unify_inst_pair(field, arg, params, subst) => true,
            _ => self.same_ty(field, arg),
        }
    }

    /// Two same-head instantiations whose parameters survive only in
    /// the ROW NAMES: `HashSet<#T>` against `HashSet<str>` binds
    /// `T := str` off the spelled argument texts.
    fn unify_name_args(&mut self, field: TypeId, arg: TypeId, params: &[IdentId], subst: &mut Vec<(IdentId, TypeId)>) -> bool {
        let ftext = self.ctx.interner.name(self.ctx.types.type_at(field).name).to_string();
        let atext = self.ctx.interner.name(self.ctx.types.type_at(arg).name).to_string();
        let Some((fh, fargs)) = ftext.split_once('<') else { return false };
        let Some((ah, aargs)) = atext.split_once('<') else { return false };
        let Some(fargs) = fargs.strip_suffix('>') else { return false };
        let Some(aargs) = aargs.strip_suffix('>') else { return false };
        if fh != ah {
            return false;
        }
        let fs: Vec<&str> = fargs.split(',').map(|x| x.trim()).collect();
        let as_: Vec<&str> = aargs.split(',').map(|x| x.trim()).collect();
        if fs.len() != as_.len() {
            return false;
        }
        for (f, a) in fs.iter().zip(as_.iter()) {
            let Some(stripped) = f.strip_prefix('#') else {
                if f != a {
                    return false;
                }
                continue;
            };
            let Some(id) = self.ctx.lookup_name(stripped) else { return false };
            if !params.contains(&id) {
                return false;
            }
            let Some(ty) = self.ctx.interner.lookup(a).and_then(|iid| self.ctx.types.dense_id_of_name(iid)) else {
                return false;
            };
            if let Some(e) = subst.iter_mut().find(|(n, _)| *n == id) {
                if e.1 != ty {
                    return false;
                }
            } else {
                subst.push((id, ty));
            }
        }
        true
    }

    /// Do two rows spell the same generic head (`Vec<..>` — the base
    /// name text, before the type arguments)?
    fn same_head_text(&self, a: TypeId, b: TypeId) -> bool {
        let ta = self.ctx.interner.name(self.ctx.types.type_at(a).name).to_string();
        let tb = self.ctx.interner.name(self.ctx.types.type_at(b).name).to_string();
        let base = |t: &str| t.split('<').next().unwrap_or(t).to_string();
        base(&ta) == base(&tb)
    }

    /// `field` and `arg` are instantiations of the SAME generic class:
    /// unify their arguments element-wise. `false` when either side is
    /// not an instantiation or the heads differ.
    fn unify_inst_pair(&mut self, field: TypeId, arg: TypeId, params: &[IdentId], subst: &mut Vec<(IdentId, TypeId)>) -> bool {
        let (Some((fd, fargs)), Some((ad, aargs))) = (self.ctx.inst_data.get(&field).cloned(), self.ctx.inst_data.get(&arg).cloned()) else {
            return false;
        };
        if fd != ad || fargs.len() != aargs.len() {
            return false;
        }
        for (f, a) in fargs.iter().zip(aargs.iter()) {
            if !self.unify_template_field(*f, *a, params, subst) {
                return false;
            }
        }
        true
    }

    pub(crate) fn compile_struct(&mut self, ty: NodeHandle<AnyTy>, fields: Vec<(IdentId, NodeHandle<AnyExpr>)>, expected: Option<TypeId>, sp: rut_lexer::span::Span) -> TcResult<TypeId> {
        // `Self` binds inside class bodies. A generic head
        // spelled WITHOUT its arguments (`M { .. }` where `M` is generic)
        // takes them from the expected instantiation — the literal types
        // against its annotation or return (`let m: M<i64, ?Item> = M { .. }`)
        let sty = match self.ctx.ast.ty(ty) {
            TypeKind::TyPath { segs } if segs.len() == 1 && segs[0].generics.is_empty() => {
                let arity = self
                    .ctx
                    .find_data(segs[0].name)
                    .map(|d| d.generics.len())
                    .or_else(|| self.ctx.extern_generics.get(&segs[0].name).map(|g| g.params.len()));
                match (arity, expected) {
                    (Some(n), Some(e)) if n > 0
                        && self.ctx.inst_data.get(&e).map(|(d, _)| *d) == Some(segs[0].name) =>
                    {
                        e
                    }
                    _ => self.resolve_type_now(ty),
                }
            }
            _ => self.resolve_type_now(ty),
        };
        // resolve the record: a local generic instantiation, a local record,
        // or a used one
        let inst = self.ctx.inst_data.get(&sty).cloned();
        let local = self.ctx.datas.iter().find(|(_, d)| d.ty == sty).map(|(n, d)| (*n, d.clone()));
        let (dname, kind, field_list, class_subst): (
            IdentId,
            crate::check::DataKind,
            Vec<(IdentId, TypeId, Option<NodeHandle<AnyExpr>>)>,
            Vec<(IdentId, TypeId)>,
        ) = if let Some((dname, cargs)) = inst.clone() {
            match self.ctx.find_data(dname).cloned() {
                Some(d) => {
                    let class_subst: Vec<(IdentId, TypeId)> =
                        d.generics.iter().cloned().zip(cargs.iter().cloned()).collect();
                    let field_nodes = match self.ctx.ast.item(d.node) {
                        ItemKind::Struct { fields, .. } | ItemKind::Class { fields, .. } => fields.clone(),
                        _ => Vec::new(),
                    };
                    let saved_subst = std::mem::replace(&mut self.subst, class_subst.clone());
                    let saved_self = self.self_ty;
                    self.self_ty = Some(sty);
                    // the field types resolve in the DECL's module (phase 3)
                    let saved_mod = self.ctx.cur_mod.clone();
                    self.ctx.cur_mod = self.ctx.mod_of(d.node.id()).to_string();
                    let mut list = Vec::new();
                    for f in &field_nodes {
                        let fd = self.ctx.ast.field_decl(*f);
                        let fty = self.resolve_type_now(fd.ty);
                        list.push((fd.name, fty, fd.init));
                    }
                    self.ctx.cur_mod = saved_mod;
                    self.subst = saved_subst;
                    self.self_ty = saved_self;
                    (dname, d.kind, list, class_subst)
                }
                // a foreign generic's mirror: the row IS the layout — its
                // fields resolved under the caller's substitution at the
                // instantiation, no declaring body in sight
                None => {
                    let TyKind::Data { fields: desc } = self.ctx.types.kind(sty) else {
                        self.ctx.err(sp, format!("`{}` is not a record of this module", self.ctx.type_name(sty)));
                        return Err(());
                    };
                    let desc = desc.clone();
                    (
                        dname,
                        crate::check::DataKind::Struct,
                        desc.into_iter().map(|f| (f.name, f.ty, None)).collect(),
                        Vec::new(),
                    )
                }
            }
        } else if let Some((dname, d)) = local {
            let list = d.fields.iter().map(|(n, t, i, _)| (*n, *t, *i)).collect();
            (dname, d.kind, list, Vec::new())
        } else {
            return self.compile_struct_extern(sty, fields, sp);
        };
        // classes have no outside literal — RELAXED in the
        // two-pkg store batch: the store's and the app's boot turns
        // construct their containers directly (`World { .. }`, the spikes'
        // `Source<str> { .. }`), so the instance literal is the
        // constructor surface everywhere now. The every-field-covered law
        // below still holds; struct vs class changes nothing at the
        // literal any more (`kind` stays for the extern path).
        let _ = kind;
        // every field initialized (any order, by name) or has an initializer;
        // collect each field value in a register, then mint the
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
                        // zero-value defaults: an omitted field
                        // takes its type's zero value
                        let z = self.zero_value(*fty, sp)?;
                        val_regs[fidx] = Some(z);
                        continue;
                    }
                    Some(init) => {
                        // initializers are class code: resolve under the class subst,
                        // in the decl's module (phase 3)
                        let saved_subst = std::mem::replace(&mut self.subst, class_subst.clone());
                        let saved_self = self.self_ty;
                        let saved_mod = self.ctx.cur_mod.clone();
                        if let Some((_, d)) = self.ctx.datas.iter().find(|(n, _)| *n == dname) {
                            self.ctx.cur_mod = self.ctx.mod_of(d.node.id()).to_string();
                        }
                        self.self_ty = Some(sty);
                        let t = self.compile_expr(*init, Some(*fty));
                        self.ctx.cur_mod = saved_mod;
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

    /// The zero value of a type: `0`/`0.0`/`false`, the empty
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
                    "a literal must initialize `{}` —it has no zero value",
                    self.ctx.type_name(ty)
                ));
                return Err(());
            }
        }
        Ok(reg)
    }

    /// Record literal for a USED struct: the layout is
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
                "classes have no instance literal — construct through a class method (`{}.new(..)`)",
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
                "record `{}` from another module must initialize every field — `{}` is missing (cross-module field defaults are not carried)",
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
        // `[e1, .., en] : [T]`; T from expected or the
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
                // heterogeneous elements unify through trait objects
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

    /// `[v; n]` — the repeat construction: `ArrNew` for n
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

    // ---- closures (v1 captures by value) ----

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
        let mut caps: Vec<Capture> = Vec::new();
        let mut cap_regs: Vec<u16> = Vec::new();
        for n in referenced {
            if lambda_param_names.contains(&n) {
                continue;
            }
            if let Some(l) = self.lookup(n).cloned() {
                if l.cell.is_some() {
                    // promoted: pool the SHARED CELL — the closure's
                    // parameter carries the cell handle, reads and
                    // writes route through it, both frames stay linked
                    // for the binding's whole scope
                    caps.push(Capture { name: n, ty: l.ty, is_mut: l.is_mut, cell: l.cell, origins: l.origins.clone() });
                    cap_regs.push(l.reg);
                } else {
                    // the immediate-slot copy: primitives/nil/fn copy
                    // their slot, ref-headed values cross as handles
                    // (the capture inherits the binding's mutability so
                    // writes through a captured `let mut` stay legal)
                    let cap_reg = self.read_local(&l, sp.lo);
                    caps.push(Capture { name: n, ty: l.ty, is_mut: l.is_mut, cell: None, origins: l.origins.clone() });
                    cap_regs.push(cap_reg);
                }
            }
        }
        // register the synthetic fn: params = declared ++ captures
                let node_id = self.ctx.ast.span(body.id()).lo; // not unique per node —use body NodeId instead
        let _ = node_id;
        // the Lambda node id: find it by body —the caller passes parts; the
        // lambda node is the PARENT of body. Store captures keyed by the
        // body node; FnKey::Lambda uses the body node id (unique).
        // record the resolved signature for the body compilation —
        // param/ret types AND the creation site's substitution (the
        // body compiler re-arms its type env from it, so a lambda
        // inside a generic fn/method — and any lambda IT creates —
        // resolves annotations against the enclosing generics)
        self.ctx.lambda_sigs.insert(lambda_node, (ptys.clone(), ret_ty, self.subst.clone()));
        self.ctx.lambda_info.insert(lambda_node, caps.clone());
        let inst = crate::check::Inst { key: crate::check::FnKey::Lambda(lambda_node), subst: vec![], iface_origins: vec![] };
        let fid = self.ctx.ensure_inst(inst);
        // the surface fn type spells the DECLARED params only — the
        // capture tail is ABI, never type-checked against
        let fty = self.ctx.mk_fn_ty(ptys.clone(), ret_ty);
        let dst = self.new_reg(fty);
        { let (argv_off, argc) = self.pool_args(&cap_regs); self.emit(Op::MakeClosure { dst: dst, func: fid, argv_off, argc }, sp.lo,); }
        Ok(fty)
    }

    /// collect every single-segment path name under `node` (capture scan).
    /// Generic walk over the arena by `NodeId` — it must cross statement and
    /// expression categories freely; type subtrees carry no value names.
    pub(crate) fn scan_names(&mut self, node: NodeId, out: &mut Vec<IdentId>) {        if out.len() > 4096 {
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
            Kind::Expr(ExprKind::Block { stmts }) => {
                for s in stmts {
                    kids(s.id(), out, self);
                }
            }
            // the async block's body IS in scope at its site: the mint
            // reads the captured values there, so the enclosing scan
            // descends (an `async { }` inside a for-of body makes the
            // block's reads the emit closure's captures)
            Kind::Expr(ExprKind::AsyncBlock { body }) => kids(body.id(), out, self),
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

// ---- the capture law's fn-level pre-pass ----

impl<'a, 'b> FnCompiler<'a, 'b> {
    /// The fn-level pre-pass behind the capture law: one walk over the
    /// fn body collecting NAMES only (types resolve during the fused
    /// walk, so promotion tests `is_ref` at `bind_local`):
    ///
    /// - `assigned`: every name that is the single-segment LHS of an
    ///   `Assign` (plain or compound), plus every `for..of` loop
    ///   variable — the desugar's element store IS an assignment (the
    ///   sugar law: the loop var is one variable, reassigned per
    ///   iteration, exactly like a handwritten `while`).
    /// - `captured`: every single-segment name referenced inside a
    ///   lambda body or a `for..of` body — the closure sites. A flat
    ///   set: shadowing only over-approximates, which costs a cell and
    ///   stays correct.
    pub(crate) fn capture_pre_pass(&mut self, node: NodeId) {
        let mut assigned = std::collections::HashSet::new();
        let mut captured = std::collections::HashSet::new();
        self.walk_capture_law(node, false, &mut assigned, &mut captured);
        self.assigned = assigned;
        self.captured = captured;
    }

    fn walk_capture_law(
        &mut self,
        node: NodeId,
        inside: bool,
        assigned: &mut std::collections::HashSet<IdentId>,
        captured: &mut std::collections::HashSet<IdentId>,
    ) {
        if assigned.len() + captured.len() > 8192 {
            return;
        }
        let walk = |n: NodeId,
                    inside: bool,
                    s: &mut Self,
                    a: &mut std::collections::HashSet<IdentId>,
                    c: &mut std::collections::HashSet<IdentId>| {
            s.walk_capture_law(n, inside, a, c)
        };
        match self.ctx.ast.kind(node).clone() {
            Kind::Item(_) | Kind::Member(_) | Kind::Pat(_) | Kind::Type(_) => {}
            Kind::Stmt(StmtKind::LetStmt { init, .. }) => walk(init.id(), inside, self, assigned, captured),
            Kind::Stmt(StmtKind::If { cond, then, els }) => {
                walk(cond.id(), inside, self, assigned, captured);
                walk(then.id(), inside, self, assigned, captured);
                if let Some(e) = els {
                    match e {
                        ElseBranch::If(h) => walk(h.id(), inside, self, assigned, captured),
                        ElseBranch::Block(h) => walk(h.id(), inside, self, assigned, captured),
                    }
                }
            }
            Kind::Stmt(StmtKind::While { cond, body }) => {
                walk(cond.id(), inside, self, assigned, captured);
                walk(body.id(), inside, self, assigned, captured);
            }
            Kind::Stmt(StmtKind::ForOf { var, iter, body }) => {
                // the loop var is ONE variable reassigned per iteration
                // (the sugar law) — the element store counts as an
                // assignment in BOTH frames
                assigned.insert(var);
                walk(iter.id(), inside, self, assigned, captured);
                walk(body.id(), true, self, assigned, captured);
            }
            Kind::Stmt(StmtKind::ForC { init, cond, update, body, .. }) => {
                walk(init.id(), inside, self, assigned, captured);
                walk(cond.id(), inside, self, assigned, captured);
                walk(update.id(), inside, self, assigned, captured);
                walk(body.id(), inside, self, assigned, captured);
            }
            Kind::Stmt(StmtKind::Return { value }) => {
                if let Some(v) = value {
                    walk(v.id(), inside, self, assigned, captured);
                }
            }
            Kind::Stmt(StmtKind::WhenStmt { scrut, arms }) => {
                walk(scrut.id(), inside, self, assigned, captured);
                for a in arms {
                    walk(a.id(), inside, self, assigned, captured);
                }
            }
            Kind::Stmt(StmtKind::ExprStmt(e)) => walk(e.id(), inside, self, assigned, captured),
            Kind::Stmt(StmtKind::Break | StmtKind::Continue) => {}
            Kind::Arm(ArmKind::WhenArm { pats, body }) => {
                for p in pats {
                    walk(p.id(), inside, self, assigned, captured);
                }
                walk(body.id(), inside, self, assigned, captured);
            }
            Kind::Expr(ExprKind::Block { stmts }) => {
                for s in stmts {
                    walk(s.id(), inside, self, assigned, captured);
                }
            }
            // the async block descends with the SAME `inside` law: its
            // reads are the enclosing fn's captures (the mint reads the
            // values at the site); the lambda's own law is the stricter
            // `true` arm and does not apply — the block frame is minted,
            // never deferred
            Kind::Expr(ExprKind::AsyncBlock { body }) => walk(body.id(), inside, self, assigned, captured),
            Kind::Expr(ExprKind::Path { segs }) => {
                if inside && !segs.is_empty() {
                    captured.insert(segs[0].name);
                }
            }
            Kind::Expr(ExprKind::Lit(_)) => {}
            Kind::Expr(ExprKind::Call { callee, args }) => {
                walk(callee.id(), inside, self, assigned, captured);
                for a in args {
                    walk(a.id(), inside, self, assigned, captured);
                }
            }
            Kind::Expr(ExprKind::Method { recv, args, .. }) => {
                walk(recv.id(), inside, self, assigned, captured);
                for a in args {
                    walk(a.id(), inside, self, assigned, captured);
                }
            }
            Kind::Expr(ExprKind::Field { recv, .. }) => walk(recv.id(), inside, self, assigned, captured),
            Kind::Expr(ExprKind::Index { recv, idx }) => {
                walk(recv.id(), inside, self, assigned, captured);
                walk(idx.id(), inside, self, assigned, captured);
            }
            Kind::Expr(ExprKind::Unary { expr, .. }) => walk(expr.id(), inside, self, assigned, captured),
            Kind::Expr(ExprKind::Binary { lhs, rhs, .. }) => {
                walk(lhs.id(), inside, self, assigned, captured);
                walk(rhs.id(), inside, self, assigned, captured);
            }
            Kind::Expr(ExprKind::Assign { target, value, .. }) => {
                // the single-segment LHS names an assignment; compound
                // forms (`+=` and kin) count too. A multi-segment or
                // field/index target mutates THROUGH the head — not an
                // assignment OF the binding (the shared handle already
                // crosses).
                if let ExprKind::Path { segs } = self.ctx.ast.expr(target) {
                    if segs.len() == 1 {
                        assigned.insert(segs[0].name);
                    }
                }
                walk(target.id(), inside, self, assigned, captured);
                walk(value.id(), inside, self, assigned, captured);
            }
            Kind::Expr(ExprKind::Lambda { body: b, .. }) => walk(b.id(), true, self, assigned, captured),
            Kind::Expr(ExprKind::Try { expr }) | Kind::Expr(ExprKind::Await { expr }) => {
                walk(expr.id(), inside, self, assigned, captured)
            }
            Kind::Expr(ExprKind::Is { expr, .. }) => walk(expr.id(), inside, self, assigned, captured),
            Kind::Expr(ExprKind::Cast { expr, .. }) => walk(expr.id(), inside, self, assigned, captured),
            Kind::Expr(ExprKind::FStr { parts }) => {
                for p in parts {
                    if let FPartAst::Hole(e) = p {
                        walk(e.id(), inside, self, assigned, captured);
                    }
                }
            }
            Kind::Expr(ExprKind::Struct { fields, .. }) => {
                for (_, v) in fields {
                    walk(v.id(), inside, self, assigned, captured);
                }
            }
            Kind::Expr(ExprKind::ArrayLit { elems }) => {
                for e in elems {
                    walk(e.id(), inside, self, assigned, captured);
                }
            }
            Kind::Expr(ExprKind::ArrayRepeat { value, count }) => {
                walk(value.id(), inside, self, assigned, captured);
                walk(count.id(), inside, self, assigned, captured);
            }
            Kind::Expr(ExprKind::Tuple { elems }) => {
                for e in elems {
                    walk(e.id(), inside, self, assigned, captured);
                }
            }
            Kind::Expr(ExprKind::WhenExpr { scrut, arms }) => {
                walk(scrut.id(), inside, self, assigned, captured);
                for a in arms {
                    walk(a.id(), inside, self, assigned, captured);
                }
            }
        }
    }
}
