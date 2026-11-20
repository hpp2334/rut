//! TreeIndex — the tree's derived geometry: a parent row per node and
//! the cursor rows (every live node, sorted by span). Plain data over
//! [`AnyHandle`]s — `Send + Sync` rides `Ast`. Cold parses leave the
//! index LAZY (`Ast`'s `OnceCell`); the edit lane invalidates it and
//! the next read rebuilds — the same deterministic walk, so the index
//! content is ≡ a cold parse's (pinned in `tests` below). Construction
//! walks every child position from the root — the one enumeration,
//! driven by the slot enums.

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
    /// with `lo <= pos < hi`. The rows are sorted by `lo`, so the scan
    /// stops at the first row past the window; equal-size spans (an
    /// item ending at EOF ties with the root module) break on the
    /// sorted row order — the LATER row wins, so the item beats the
    /// root that wraps it.
    pub(crate) fn node_at(&self, pos: u32) -> Option<AnyHandle> {
        let mut best: Option<(Span, AnyHandle)> = None;
        for &(s, h) in &self.rows {
            if s.lo > pos {
                break;
            }
            if pos < s.hi && best.is_none_or(|(bs, _)| s.hi - s.lo <= bs.hi - bs.lo) {
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

#[cfg(test)]
mod tests {
    use super::TreeIndex;
    use crate::document::Doc;
    use crate::parser::Mode;
    use rut_lexer::source::Edit;
    use std::collections::BTreeMap;

    /// one index row's canonical name — ((lo, hi), kind): slot indexes
    /// differ between an edited tree and a cold rebuild, the shape may
    /// not
    type Row = ((u32, u32), &'static str);

    /// the index's SHAPE — cursor rows and parent rows, projected
    fn shape(idx: &TreeIndex, ast: &crate::Ast) -> (Vec<Row>, BTreeMap<Row, Row>) {
        let name_span = |h: crate::AnyHandle| -> Row {
            let s = ast.span_of(h);
            ((s.lo, s.hi), h.kind_name())
        };
        let mut rows: Vec<Row> = idx.rows.iter().map(|&(_, h)| name_span(h)).collect();
        rows.sort();
        let mut parents: BTreeMap<Row, Row> = BTreeMap::new();
        for &(_, h) in &idx.rows {
            if let Some(p) = idx.parent(h) {
                parents.insert(name_span(h), name_span(p));
            }
        }
        (rows, parents)
    }

    /// the edit ≡ cold law at the index level: after every edit, the
    /// rebuilt index equals the cold parse's — cursor rows AND parent
    /// rows, exactly — and the cursor answers agree at every byte.
    fn assert_index_edit_cold(src0: &str, edits: &[&str]) {
        let mut doc = Doc::parse(Mode::Impl, src0.to_string());
        let mut old = src0.to_string();
        for (i, text) in edits.iter().enumerate() {
            doc.edit(Edit::between(&old, text));
            old = (*text).to_string();
            let cold = Doc::parse(Mode::Impl, text.to_string());
            let (edited_rows, edited_parents) = shape(doc.ast().index(), doc.ast());
            let (cold_rows, cold_parents) = shape(cold.ast().index(), cold.ast());
            assert_eq!(edited_rows, cold_rows, "step {i}: cursor rows differ");
            assert_eq!(edited_parents, cold_parents, "step {i}: parent rows differ");
            for pos in 0..=doc.len() {
                let ea = doc
                    .ast()
                    .node_at(pos)
                    .map(|h| (h.kind_name(), doc.ast().span_of(h)));
                let ca = cold
                    .ast()
                    .node_at(pos)
                    .map(|h| (h.kind_name(), cold.ast().span_of(h)));
                assert_eq!(ea, ca, "step {i}: node_at({pos}) differs");
            }
        }
    }

    #[test]
    fn edited_index_equals_the_cold_rebuild_on_split_merge_delete() {
        // split: an item appended mid-file (two items where one was)
        assert_index_edit_cold(
            "let a = 1;\nlet b = 2;\nlet c = 3;\n",
            &["let a = 1;\nlet b = 2;\nlet d = 4;\nlet c = 3;\n"],
        );
        // merge + delete: two items collapse into one, then vanish
        assert_index_edit_cold(
            "let a = 1;\nlet b = 2;\nlet c = 3;\n",
            &["let a = 1;\nlet b = 2; let c = 3;\n", "let a = 1;\n"],
        );
        // the empty document, and back
        assert_index_edit_cold("let a = 1;\n", &["\n", "let x = 0;\n"]);
    }

    #[test]
    fn edited_index_equals_the_cold_rebuild_through_fences_and_docs() {
        // a broken item (the fence is a real node in the index) beside
        // good ones, a doc comment that must RE-ATTACH after the edit,
        // and deep nesting after the boundary
        let src0 = "let ok = 1;\noops here\n\n/// adds\nfn inc(x: i64) -> i64 { x + 1 }\n";
        let step1 = "let ok = 1;\nlet more = true;\n\n/// adds\nfn inc(x: i64) -> i64 { x + 1 }\n";
        // the doc chain re-grafted onto a CHANGED head
        let step2 =
            "let ok = 1;\nlet more = true;\n\n/// doubles\nfn inc(x: i64) -> i64 { x * 2 }\n";
        assert_index_edit_cold(src0, &[step1, step2]);
    }

    #[test]
    fn edited_index_equals_the_cold_rebuild_on_literal_and_eof_edits() {
        // the window re-anchors inside a raw string; whitespace-only
        // growth at EOF rides the module-span corner
        let src0 = "let s = r#\"\nline one\nline two\n\"#;\nlet n = 2;\n";
        let step1 = "let s = r#\"\nline one\nline TWO\n\"#;\nlet n = 2;\n";
        let step2 = "let s = r#\"\nline one\nline TWO\n\"#;\nlet n = 2;   \n";
        assert_index_edit_cold(src0, &[step1, step2]);
    }
}
