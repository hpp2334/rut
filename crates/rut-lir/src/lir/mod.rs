//! Body compilation —RFC 0031 §2/§3 fused into one walk (M1
//! simplification: bidirectional inference + emission together; the
//! separate SSA-ish HIR of RFC 0031 §3 is a later pass). Output: typed
//! register bytecode per function (RFC 0032), monomorphized (RFC 0013 §2).

use rut_ast::ast::*;
use crate::check::{Ctx, FnKey, Inst, TcResult};
use rut_lexer::span::Span;
use rut_core::binary::ConstVal;
use rut_core::ops::*;
use std::collections::HashMap;
use rut_core::sym;
use rut_core::types::*;

mod call;
mod compile;
mod expr;
mod generic;
mod intrinsic;
mod lit;
mod ops;
mod peephole;
mod slice;
mod sroa;
mod stmt;
mod utf8;

const NEST_MAX: u32 = 1024;

/// The per-function operand pools (RFC 0032): ops address their variadic
/// `Reg` lists and `BrTable` arms by `(off, len)` span into these tables
/// instead of owning a `Vec` — that is what keeps `Op` at 16 bytes. Lists
/// are interned (deduplicated) at emission; rewriters that remap registers
/// re-intern, so sharing stays consistent. The empty list is the span
/// `(0, 0)` and is never stored.
pub(crate) struct Pools {
    pub argv: Vec<Reg>,
    argv_ix: HashMap<Vec<Reg>, u32>,
    pub labels: Vec<Label>,
}

impl Pools {
    fn new() -> Pools {
        Pools { argv: Vec::new(), argv_ix: HashMap::new(), labels: Vec::new() }
    }

    /// Intern an argument list; returns its `(off, argc)` span.
    pub(crate) fn args(&mut self, a: &[Reg]) -> (u32, u16) {
        if a.is_empty() {
            return (0, 0);
        }
        debug_assert!(a.len() <= u16::MAX as usize, "operand list longer than the register file");
        if let Some(&off) = self.argv_ix.get(a) {
            return (off, a.len() as u16);
        }
        let off = self.argv.len() as u32;
        self.argv.extend_from_slice(a);
        self.argv_ix.insert(a.to_vec(), off);
        (off, a.len() as u16)
    }

    /// `[recv] ++ args` — the callee's whole parameter list in one span
    /// (`CallM`/`CallI`: the receiver is `argv[0]`).
    pub(crate) fn recv_args(&mut self, recv: Reg, a: &[Reg]) -> (u32, u16) {
        let mut l = Vec::with_capacity(a.len() + 1);
        l.push(recv);
        l.extend_from_slice(a);
        self.args(&l)
    }

    /// Intern a branch table; returns its `(off, count)` span.
    pub(crate) fn table(&mut self, t: &[Label]) -> (u32, u16) {
        if t.is_empty() {
            return (0, 0);
        }
        // branch tables are rare (`when` chains); no dedup map for them —
        // identical tables simply share nothing
        let off = self.labels.len() as u32;
        self.labels.extend_from_slice(t);
        (off, t.len() as u16)
    }

    pub(crate) fn slice(&self, off: u32, len: u16) -> &[Reg] {
        &self.argv[off as usize..off as usize + len as usize]
    }
}

#[derive(Clone)]
pub(crate) struct Local {
    name: IdentId,
    reg: u16,
    ty: TypeId,
    is_mut: bool,
    /// for-c induction variables are loop-owned (RFC 0008 §1)
    loop_var: bool,
    /// origin counting (RFC 0012 §5): the concrete types a trait-typed
    /// binding is known to hold. Single origin ⇒ static dispatch;
    /// empty ⇒ unknown/multiple ⇒ vtable. Only direct constructions
    /// (a widening let, a copied binding, a specialized parameter)
    /// populate it — conservative by construction.
    origins: Vec<TypeId>,
}

pub struct FnCompiler<'a, 'b> {
    ctx: &'b mut Ctx<'a>,
    regs: Vec<TypeId>,
    code: Vec<Op>,
    pools: Pools,
    spans: Vec<(u32, u32)>,
    locals: Vec<Local>,
    ret_ty: TypeId,
    self_ty: Option<TypeId>,
    subst: Vec<(IdentId, TypeId)>,
    current_class: Option<IdentId>,
    depth: u32,
    loops: Vec<(u32, u32)>, // (continue label, break label)
    labels: Vec<Option<u32>>,
    fixups: Vec<(usize, u32, bool)>, // (op index, label id, is_else_target)
    span: u32,
    /// the register holding the value produced by the last compile_expr
    last_reg: u16,
    /// while inlining a `Slice` accessor (RFC 0005), reads of this local
    /// (`self`) use the receiver register directly — no copy, so element
    /// access pays no per-access ref copy
    inline_self: Option<(IdentId, u16)>,
    /// while inlining a returned method body: `(result reg, end label)` —
    /// `return` writes here and jumps, instead of emitting `Op::Ret`
    inline_ret: Option<(u16, u32)>,
    /// class methods currently being inlined — a recursion guard
    inline_stack: Vec<(IdentId, IdentId)>,
    /// inside a desugared `for..of` emit closure (RFC 0012 §6):
    /// `break` → `return false`, `continue` → `return true`
    emit_closure: bool,
    /// Type-union bounds in scope for THIS instantiation (RFC 0043 §3,
    /// native-fastpath phase 1): fn/method bounds plus — for methods of a
    /// generic class — the class's own `requires`, filtered to
    /// union-SPELLED bounds (trait bounds stay admission-only). Read by
    /// the method-call capability gate.
    union_bounds: HashMap<IdentId, NodeHandle<AnyTy>>,
    /// locals bound from a union-bounded generic (param/let/field-copy
    /// provenance): local name → the bounded generic
    union_syms: HashMap<IdentId, IdentId>,
}

impl<'a, 'b> FnCompiler<'a, 'b> {

    // ---- infrastructure ----

    pub(crate) fn new_reg(&mut self, ty: TypeId) -> u16 {
        // the file must stay below the NOREG sentinel (u16::MAX) — optional
        // operands use it as "no register" (rut_core::ops::NOREG)
        assert!(self.regs.len() < NOREG as usize, "register file overflow");
        self.regs.push(ty);
        let r = (self.regs.len() - 1) as u16;
        self.last_reg = r;
        r
    }

    /// Coerce a computed register into a slot's type (RFC 0044): a `?T`
    /// slot takes the boxed handle — the mirror of the read-side deref.
    /// The register is returned unchanged when no coercion applies.
    pub(crate) fn coerce_to(&mut self, t: TypeId, to: TypeId, reg: u16, sp_lo: u32) -> u16 {
        if t != to {
            if let TyKind::Opt { elem } = self.ctx.types.kind(to).clone() {
                if elem == t {
                    let dst = self.new_reg(to);
                    self.emit(Op::MakeOpt { dst, src: reg, ty: to }, sp_lo);
                    return dst;
                }
            }
        }
        reg
    }

    /// Move a value into `dst` under the sharing law (RFC 0044): every
    /// cell type binds a reference (MovRef — retain new, release old);
    /// primitives and `fn` values copy their immediate slot (Mov).
    /// Binding is O(1) for records, arrays, `str`, `?T` — a share, never
    /// a copy.
    pub(crate) fn mov_slot(&mut self, dst: u16, src: u16, ty: TypeId, sp_lo: u32) {
        if self.ctx.types.is_ref(ty) {
            self.emit(Op::MovRef { dst, src }, sp_lo);
        } else {
            self.emit(Op::Mov { dst, src }, sp_lo);
        }
    }

    /// Concrete → trait-slot widening at a value boundary (RFC 0012 §4).
    /// A by-value concrete (a scalar — records/`str`/`bytes` already are
    /// cell handles) is BOXED: `Op::Box` carries the concrete type, so the
    /// slot always holds a cell handle — every ref op on the slot is safe
    /// and the vtable dispatch reads the cell's own type. The mirror is
    /// the prim-target impl method's prologue unbox (the slot ABI). A
    /// no-op for slot-typed and ref-typed values.
    pub(crate) fn widen_to_slot(&mut self, t: TypeId, to: TypeId, sp_lo: u32) {
        if t != to
            && matches!(self.ctx.types.kind(to), TyKind::TraitObj { .. })
            && !matches!(self.ctx.types.kind(t), TyKind::TraitObj { .. })
            && !self.ctx.types.is_ref(t)
        {
            let src = self.last_reg;
            let dst = self.new_reg(to);
            self.emit(Op::Box { dst, val: src, ty: t }, sp_lo);
        }
    }

    /// `p.m(..)` / `p[i]` auto-deref (RFC 0005): a nullable used as a
    /// receiver or indexee loads its payload cell first — at the
    /// payload's OWN repr (a scalar payload reads its bits, a cell
    /// payload retains; RFC 0044). TRANSITIVE: a `??T` receiver derefs
    /// twice. Returns the (type, register) to continue from.
    pub(crate) fn deref_for_use(&mut self, ty: TypeId, reg: u16, sp_lo: u32) -> (TypeId, u16) {
        let mut ty = ty;
        let mut cur = reg;
        while let TyKind::Opt { elem } = self.ctx.types.kind(ty).clone() {
            let d = self.new_reg(elem);
            let repr = self.ctx.types.repr_of(elem);
            self.emit(Op::GetF { dst: d, obj: cur, field: 0, repr }, sp_lo);
            ty = elem;
            cur = d;
        }
        (ty, cur)
    }

    /// Deref a nullable at a value use site: `v: ?T` reads as `T` (RFC
    /// 0012 §6) — the payload slot at the pointee's own repr, transitively
    /// for `??T`. Callers decide which positions deref: value-expected
    /// positions via the compile_expr funnel, arithmetic operands, and
    /// `==` against the pointee type. `p.f`/`p.m()` deref at their own
    /// sites and `?T == ?T` compares payloads at the shared cell.
    pub(crate) fn deref_ptr(&mut self, ty: TypeId, reg: u16, sp_lo: u32) -> (TypeId, u16) {
        let mut ty = ty;
        let mut cur = reg;
        while let TyKind::Opt { elem } = self.ctx.types.kind(ty).clone() {
            let d = self.new_reg(elem);
            let repr = self.ctx.types.repr_of(elem);
            self.emit(Op::GetF { dst: d, obj: cur, field: 0, repr }, sp_lo);
            ty = elem;
            cur = d;
        }
        (ty, cur)
    }

    /// Intern an argument list into the function's operand pool.
    pub(crate) fn pool_args(&mut self, a: &[Reg]) -> (u32, u16) {
        self.pools.args(a)
    }

    /// Intern `[recv] ++ args` (the callee's whole parameter list).
    pub(crate) fn pool_recv_args(&mut self, recv: Reg, a: &[Reg]) -> (u32, u16) {
        self.pools.recv_args(recv, a)
    }

    pub(crate) fn emit(&mut self, op: Op, span_lo: u32) {
        self.spans.push((self.code.len() as u32, span_lo));
        self.code.push(op);
    }

    pub(crate) fn new_label(&mut self) -> u32 {
        self.labels.push(None);
        (self.labels.len() - 1) as u32
    }
    pub(crate) fn bind(&mut self, l: u32) {
        self.labels[l as usize] = Some(self.code.len() as u32);
    }
    pub(crate) fn resolve_labels(&mut self) {
        for (op_idx, l, is_else) in std::mem::take(&mut self.fixups) {
            let target = self.labels[l as usize].unwrap_or(0);
            match &mut self.code[op_idx] {
                Op::Jmp { target: t } => *t = target,
                Op::Br { then_t, else_t, .. } => {
                    if is_else {
                        *else_t = target;
                    } else {
                        *then_t = target;
                    }
                }
                Op::BrTable { table_off, count, default, .. } => {
                    if is_else {
                        *default = target;
                    } else if *count > 0 {
                        self.pools.labels[*table_off as usize] = target;
                    }
                }
                _ => unreachable!(),
            }
        }
    }
    pub(crate) fn jmp(&mut self, l: u32) {
        self.fixups.push((self.code.len(), l, false));
        self.emit(Op::Jmp { target: 0 }, self.span);
    }
    pub(crate) fn br(&mut self, cond: u16, then_l: u32, else_l: u32) {
        self.fixups.push((self.code.len(), then_l, false));
        self.fixups.push((self.code.len(), else_l, true));
        self.emit(Op::Br { cond, then_t: 0, else_t: 0 }, self.span);
    }

    pub(crate) fn resolve_type_now(&mut self, node: NodeHandle<AnyTy>) -> TypeId {
        // `Self` (bare or under `?`, at any structural depth — the
        // rut-json batch phase 1) binds to the enclosing type inside
        // method bodies
        self.ctx.resolve_sig_ty_deep(node, &self.subst, self.self_ty)
    }

    pub(crate) fn lookup(&self, name: IdentId) -> Option<&Local> {
        self.locals.iter().rev().find(|l| l.name == name)
    }

    /// The origin set of a binding (origin counting, RFC 0012 §5): the
    /// concrete types a trait-typed local is known to hold. Empty =
    /// unknown/multiple.
    pub(crate) fn origins_of(&self, name: IdentId) -> Vec<TypeId> {
        self.locals
            .iter()
            .rev()
            .find(|l| l.name == name)
            .map(|l| l.origins.clone())
            .unwrap_or_default()
    }

    /// Record a binding's origins after a write: a concrete value pins
    /// the origin; a trait-typed value from an untracked source erases
    /// it (branch merges, cross-function values — conservative).
    pub(crate) fn set_origins(&mut self, name: IdentId, origins: Vec<TypeId>) {
        if let Some(l) = self.locals.iter_mut().rev().find(|l| l.name == name) {
            l.origins = origins;
        }
    }

    pub(crate) fn konst(&mut self, v: ConstVal) -> u16 {
        // dedup
        if let Some(i) = self.ctx.consts.iter().position(|c| *c == v) {
            return i as u16;
        }
        self.ctx.consts.push(v);
        (self.ctx.consts.len() - 1) as u16
    }

    pub(crate) fn enter(&mut self) -> bool {
        self.depth += 1;
        if self.depth > NEST_MAX {
            self.ctx.err(
                Span::new(self.span, self.span + 1),
                "expression nesting too deep",
            );
            return false;
        }
        true
    }
    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
    }

}
