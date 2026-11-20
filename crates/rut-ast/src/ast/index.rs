//! TreeIndex — the tree's derived geometry: a parent row per node and
//! the cursor rows (every live node, sorted by span). Plain data over
//! [`AnyHandle`]s — `Send + Sync` rides `Ast`. Cold parses leave the
//! index LAZY (`Ast`'s `OnceCell`); the edit lane (Phase 3) maintains
//! it across edits. Construction walks every child position from the
//! root — the one enumeration, driven by the slot enums.

use crate::ast::expr::{
    ArrayLitNode, ArrayRepeatNode, AssignNode, AsyncBlockNode, AwaitNode, BinaryNode, BlockNode,
    CallNode, CastNode, FPartAst, FStrNode, FieldNode, IfExprNode, IndexNode, IsNode, LambdaNode,
    MethodNode, PathNode, StructLitNode, TupleNode, UnaryNode, WhenArmNode, WhenExprNode,
};
use crate::ast::handle::AnyHandle;
use crate::ast::item::{
    AliasNode, BuiltinImplNode, BuiltinPrimitiveNode, BuiltinTyNode, ClassNode, EnumNode,
    FieldDeclNode, FnNode, ImplNode, InterfaceNode, MethodDeclNode, ModuleLetNode, ModuleNode,
    ParamNode, StructNode, SurfaceFnNode, SurfaceStructNode,
};
use crate::ast::pat::{PatLitNode, PatPathNode};
use crate::ast::stmt::{
    ForCNode, ForOfNode, IfNode, LetStmtNode, ReturnNode, WhenStmtNode, WhileNode,
};
use crate::ast::ty::{TyArrayNode, TyFnNode, TyOptNode, TyPathNode, TyTupleNode, TyUnionNode};
use crate::Ast;
use rut_lexer::span::Span;

pub(crate) struct TreeIndex {
    /// parents[kind][idx] — the parent's address. The root's row stays
    /// `None`, so do dead holes.
    parents: Vec<Vec<Option<AnyHandle>>>,
    /// every live node, sorted by (lo, hi, kind, idx) — the cursor's
    /// deepest-match search scans the `lo <= pos` window
    rows: Vec<(Span, AnyHandle)>,
}

impl TreeIndex {
    /// the one construction: DFS from the root over every child list.
    /// An explicit worklist (no host recursion — the same contract as
    /// the parser's frame machine).
    pub(crate) fn build(ast: &Ast) -> TreeIndex {
        let mut idx = TreeIndex {
            parents: Vec::new(),
            rows: Vec::new(),
        };
        let mut stack = vec![ast.root().any()];
        while let Some(h) = stack.pop() {
            idx.rows.push((ast.span_of(h), h));
            for child in children_of(ast, h) {
                idx.set_parent(child, h);
                stack.push(child);
            }
        }
        idx.rows.sort_by(|(sa, ha), (sb, hb)| {
            (sa.lo, sa.hi, ha.kind, ha.idx).cmp(&(sb.lo, sb.hi, hb.kind, hb.idx))
        });
        idx
    }

    fn set_parent(&mut self, child: AnyHandle, parent: AnyHandle) {
        let k = child.kind as usize;
        if self.parents.len() <= k {
            self.parents.resize(k + 1, Vec::new());
        }
        let row = &mut self.parents[k];
        if row.len() <= child.idx as usize {
            row.resize(child.idx as usize + 1, None);
        }
        row[child.idx as usize] = Some(parent);
    }

    /// the node's parent — `None` at the root and for stale addresses
    pub(crate) fn parent(&self, h: AnyHandle) -> Option<AnyHandle> {
        *self.parents.get(h.kind as usize)?.get(h.idx as usize)?
    }

    /// the deepest node whose span contains `pos` — the smallest span
    /// with `lo <= pos < hi` (ties break on the sorted row order, so
    /// the answer is deterministic). The rows are sorted by `lo`, so
    /// the scan stops at the first row past the window.
    pub(crate) fn node_at(&self, pos: u32) -> Option<AnyHandle> {
        let mut best: Option<(Span, AnyHandle)> = None;
        for &(s, h) in &self.rows {
            if s.lo > pos {
                break;
            }
            if pos < s.hi && best.is_none_or(|(bs, _)| s.hi - s.lo < bs.hi - bs.lo) {
                best = Some((s, h));
            }
        }
        best.map(|(_, h)| h)
    }
}

/// every child address of the node at `h` — the one enumeration of the
/// tree's edges, slot enums and closed positions alike.
pub(crate) fn children_of(ast: &Ast, h: AnyHandle) -> Vec<AnyHandle> {
    let mut out = Vec::new();
    // items
    if let Some(n) = h.downcast::<ModuleNode>() {
        out.extend(n.get(ast).items.iter().map(|&i| i.any()));
    } else if let Some(n) = h.downcast::<AliasNode>() {
        out.push(n.get(ast).target.any());
    } else if let Some(n) = h.downcast::<ModuleLetNode>() {
        let d = n.get(ast);
        if let Some(t) = d.ty {
            out.push(t.any());
        }
        out.push(d.init.any());
    } else if let Some(n) = h.downcast::<EnumNode>() {
        out.extend(n.get(ast).members.iter().map(|m| m.any()));
    } else if let Some(n) = h.downcast::<StructNode>() {
        let d = n.get(ast);
        out.extend(d.fields.iter().map(|f| f.any()));
        out.extend(d.methods.iter().map(|m| m.any()));
    } else if let Some(n) = h.downcast::<ClassNode>() {
        let d = n.get(ast);
        out.extend(d.requires.iter().map(|&(_, t)| t.any()));
        out.extend(d.fields.iter().map(|f| f.any()));
        out.extend(d.methods.iter().map(|m| m.any()));
    } else if let Some(n) = h.downcast::<InterfaceNode>() {
        out.extend(n.get(ast).methods.iter().map(|m| m.any()));
    } else if let Some(n) = h.downcast::<ImplNode>() {
        let d = n.get(ast);
        out.extend(d.bounds.iter().map(|&(_, t)| t.any()));
        out.push(d.target.any());
        out.extend(d.methods.iter().map(|m| m.any()));
    } else if let Some(n) = h.downcast::<FnNode>() {
        let d = n.get(ast);
        out.extend(d.bounds.iter().map(|&(_, t)| t.any()));
        out.extend(d.params.iter().map(|p| p.any()));
        if let Some(r) = d.ret {
            out.push(r.any());
        }
        out.push(d.body.any());
    } else if let Some(n) = h.downcast::<SurfaceFnNode>() {
        let d = n.get(ast);
        out.extend(d.params.iter().map(|p| p.any()));
        if let Some(r) = d.ret {
            out.push(r.any());
        }
    } else if let Some(n) = h.downcast::<SurfaceStructNode>() {
        out.extend(n.get(ast).fields.iter().map(|f| f.any()));
    } else if let Some(n) = h.downcast::<BuiltinTyNode>() {
        out.extend(n.get(ast).members.iter().map(|m| m.any()));
    } else if let Some(n) = h.downcast::<BuiltinPrimitiveNode>() {
        out.extend(n.get(ast).members.iter().map(|m| m.any()));
    } else if let Some(n) = h.downcast::<BuiltinImplNode>() {
        out.extend(n.get(ast).methods.iter().map(|m| m.any()));
    }
    // members
    else if let Some(n) = h.downcast::<FieldDeclNode>() {
        let d = n.get(ast);
        out.push(d.ty.any());
        if let Some(i) = d.init {
            out.push(i.any());
        }
    } else if let Some(n) = h.downcast::<MethodDeclNode>() {
        let d = n.get(ast);
        out.extend(d.bounds.iter().map(|&(_, t)| t.any()));
        out.extend(d.params.iter().map(|p| p.any()));
        if let Some(r) = d.ret {
            out.push(r.any());
        }
        if let Some(b) = d.body {
            out.push(b.any());
        }
    } else if let Some(n) = h.downcast::<ParamNode>() {
        if let Some(t) = n.get(ast).ty {
            out.push(t.any());
        }
    }
    // statements
    else if let Some(n) = h.downcast::<LetStmtNode>() {
        let d = n.get(ast);
        if let Some(t) = d.ty {
            out.push(t.any());
        }
        out.push(d.init.any());
    } else if let Some(n) = h.downcast::<IfNode>() {
        let d = n.get(ast);
        out.push(d.cond.any());
        out.push(d.then.any());
        if let Some(e) = d.els {
            out.push(e.any());
        }
    } else if let Some(n) = h.downcast::<WhileNode>() {
        let d = n.get(ast);
        out.push(d.cond.any());
        out.push(d.body.any());
    } else if let Some(n) = h.downcast::<ForOfNode>() {
        let d = n.get(ast);
        out.push(d.iter.any());
        out.push(d.body.any());
    } else if let Some(n) = h.downcast::<ForCNode>() {
        let d = n.get(ast);
        out.push(d.init.any());
        out.push(d.cond.any());
        out.push(d.update.any());
        out.push(d.body.any());
    } else if let Some(n) = h.downcast::<ReturnNode>() {
        if let Some(v) = n.get(ast).value {
            out.push(v.any());
        }
    } else if let Some(n) = h.downcast::<WhenStmtNode>() {
        let d = n.get(ast);
        out.push(d.scrut.any());
        out.extend(d.arms.iter().map(|&a| a.any()));
    }
    // expressions
    else if let Some(n) = h.downcast::<BlockNode>() {
        let d = n.get(ast);
        out.extend(d.stmts.iter().map(|s| s.any()));
        if let Some(t) = d.tail {
            out.push(t.any());
        }
    } else if let Some(n) = h.downcast::<PathNode>() {
        out.extend(
            n.get(ast)
                .segs
                .iter()
                .flat_map(|s| s.generics.iter().copied())
                .map(|t| t.any()),
        );
    } else if let Some(n) = h.downcast::<CallNode>() {
        let d = n.get(ast);
        out.push(d.callee.any());
        out.extend(d.args.iter().map(|x| x.any()));
    } else if let Some(n) = h.downcast::<MethodNode>() {
        let d = n.get(ast);
        out.push(d.recv.any());
        out.extend(d.generics.iter().map(|t| t.any()));
        out.extend(d.args.iter().map(|x| x.any()));
    } else if let Some(n) = h.downcast::<FieldNode>() {
        out.push(n.get(ast).recv.any());
    } else if let Some(n) = h.downcast::<IndexNode>() {
        let d = n.get(ast);
        out.push(d.recv.any());
        out.push(d.idx.any());
    } else if let Some(n) = h.downcast::<UnaryNode>() {
        out.push(n.get(ast).expr.any());
    } else if let Some(n) = h.downcast::<BinaryNode>() {
        let d = n.get(ast);
        out.push(d.lhs.any());
        out.push(d.rhs.any());
    } else if let Some(n) = h.downcast::<AssignNode>() {
        let d = n.get(ast);
        out.push(d.target.any());
        out.push(d.value.any());
    } else if let Some(n) = h.downcast::<LambdaNode>() {
        let d = n.get(ast);
        out.extend(d.params.iter().map(|p| p.any()));
        if let Some(r) = d.ret {
            out.push(r.any());
        }
        out.push(d.body.any());
    } else if let Some(n) = h.downcast::<FStrNode>() {
        out.extend(n.get(ast).parts.iter().filter_map(|p| match p {
            FPartAst::Lit(_) => None,
            FPartAst::Hole(e) => Some(e.any()),
        }));
    } else if let Some(n) = h.downcast::<StructLitNode>() {
        let d = n.get(ast);
        out.push(d.ty.any());
        out.extend(d.fields.iter().map(|f| f.value.any()));
    } else if let Some(n) = h.downcast::<TupleNode>() {
        out.extend(n.get(ast).elems.iter().map(|e| e.any()));
    } else if let Some(n) = h.downcast::<ArrayLitNode>() {
        out.extend(n.get(ast).elems.iter().map(|e| e.any()));
    } else if let Some(n) = h.downcast::<ArrayRepeatNode>() {
        let d = n.get(ast);
        out.push(d.value.any());
        out.push(d.count.any());
    } else if let Some(n) = h.downcast::<WhenExprNode>() {
        let d = n.get(ast);
        out.push(d.scrut.any());
        out.extend(d.arms.iter().map(|&a| a.any()));
    } else if let Some(n) = h.downcast::<IfExprNode>() {
        let d = n.get(ast);
        out.push(d.cond.any());
        out.push(d.then.any());
        if let Some(e) = d.els {
            out.push(e.any());
        }
    } else if let Some(n) = h.downcast::<AwaitNode>() {
        out.push(n.get(ast).expr.any());
    } else if let Some(n) = h.downcast::<AsyncBlockNode>() {
        out.push(n.get(ast).body.any());
    } else if let Some(n) = h.downcast::<IsNode>() {
        let d = n.get(ast);
        out.push(d.expr.any());
        out.push(d.ty.any());
    } else if let Some(n) = h.downcast::<CastNode>() {
        let d = n.get(ast);
        out.push(d.expr.any());
        out.push(d.ty.any());
    }
    // types
    else if let Some(n) = h.downcast::<TyPathNode>() {
        out.extend(
            n.get(ast)
                .segs
                .iter()
                .flat_map(|s| s.generics.iter().copied())
                .map(|t| t.any()),
        );
    } else if let Some(n) = h.downcast::<TyFnNode>() {
        let d = n.get(ast);
        out.extend(d.params.iter().map(|t| t.any()));
        out.push(d.ret.any());
    } else if let Some(n) = h.downcast::<TyOptNode>() {
        out.push(n.get(ast).inner.any());
    } else if let Some(n) = h.downcast::<TyArrayNode>() {
        out.push(n.get(ast).elem.any());
    } else if let Some(n) = h.downcast::<TyTupleNode>() {
        out.extend(n.get(ast).elems.iter().map(|t| t.any()));
    } else if let Some(n) = h.downcast::<TyUnionNode>() {
        out.extend(n.get(ast).elems.iter().map(|t| t.any()));
    }
    // patterns
    else if let Some(n) = h.downcast::<PatLitNode>() {
        out.push(n.get(ast).expr.any());
    } else if let Some(n) = h.downcast::<PatPathNode>() {
        out.extend(
            n.get(ast)
                .segs
                .iter()
                .flat_map(|s| s.generics.iter().copied())
                .map(|t| t.any()),
        );
    }
    // arms + recovery (WhenArmNode: pats, body — ErrorNode: none)
    else if let Some(n) = h.downcast::<WhenArmNode>() {
        let d = n.get(ast);
        out.extend(d.pats.iter().map(|p| p.any()));
        out.push(d.body.any());
    }
    out
}
