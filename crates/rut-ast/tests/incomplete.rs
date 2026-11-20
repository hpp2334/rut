//! The Incomplete rung and the `self`/`Self` node kinds — the tree
//! always has a REAL node at the cursor: the parser emits the partial
//! shape (absent slots) where it used to fence, and the receiver/type
//! spellings are their own kinds. The verdicts here are the pins the
//! lsp consumers walk through (`node_at` → `ancestors` → `downcast`).

mod common;

use common::parse;
use rut_ast::parser::Mode;
use rut_ast::{AnyHandle, ExprRef, ItemRef, MethodNode, StmtRef, TyRef, UseNode};

fn let_init_of(src: &str) -> (rut_ast::Doc, ExprRef) {
    let (doc, diags) = parse(src, Mode::Impl);
    assert!(
        diags.is_empty(),
        "fixture must parse clean: {src:?} -> {diags:?}"
    );
    let ast = doc.ast();
    let init = first_let_init(ast);
    (doc, init)
}

/// the first `let` initializer, free fn or impl method
fn first_let_init(ast: &rut_ast::Ast) -> ExprRef {
    for i in ast.root().items(ast) {
        match *i {
            ItemRef::Fn(h) => {
                for s in &h.get(ast).body.get(ast).stmts {
                    if let StmtRef::Let(l) = s {
                        return l.get(ast).init;
                    }
                }
            }
            ItemRef::Impl(h) => {
                for m in &h.get(ast).methods {
                    let rut_ast::MethodRef::Method(mh) = m else {
                        continue;
                    };
                    let Some(b) = mh.get(ast).body else { continue };
                    for s in &b.get(ast).stmts {
                        if let StmtRef::Let(l) = s {
                            return l.get(ast).init;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    panic!("no fn/let in the module")
}

// ---- the self / Self node kinds ----

#[test]
fn self_is_its_own_expr_kind_not_a_path() {
    let (doc, init) = let_init_of(
        "class C { r: i32; }\nimpl C { fn m(self) -> i32 { let x = self.r; return x; } }\n",
    );
    let ast = doc.ast();
    let ExprRef::Field(f) = init else {
        panic!("field access expected")
    };
    let d = f.get(ast);
    assert!(matches!(d.recv, ExprRef::Self_(_)), "{:?}", d.recv);
    let ExprRef::Self_(s) = d.recv else {
        unreachable!()
    };
    let sd = s.get(ast);
    assert!(!sd.is_ty, "the receiver spelling is not the type");
    assert_eq!(doc.text(sd.span), "self");
}

#[test]
fn self_ty_is_its_own_type_kind() {
    let (doc, _) = let_init_of(
        "class C { r: i32; }\nimpl C { fn m(self) -> Self { let x = self.r; return x; } }\n",
    );
    let ast = doc.ast();
    for i in ast.root().items(ast) {
        if let ItemRef::Impl(h) = *i {
            for m in &h.get(ast).methods {
                let rut_ast::MethodRef::Method(mh) = m else {
                    continue;
                };
                let md = mh.get(ast);
                let Some(r) = md.ret else {
                    panic!("ret expected")
                };
                assert!(matches!(r, TyRef::Self_(_)), "{r:?}");
                assert_eq!(doc.text(r.span(ast)), "Self");
                return;
            }
        }
    }
    panic!("no impl in the module");
}

#[test]
fn self_literal_head_is_the_self_ty_kind() {
    let (doc, init) = let_init_of(
        "class C { r: i32; }\nimpl C { fn m(self) -> Self { let x = Self { r: 1 }; return x.r; } }\n",
    );
    let ast = doc.ast();
    let ExprRef::Struct(s) = init else {
        panic!("struct literal expected: {:?}", init)
    };
    assert!(matches!(s.get(ast).ty, TyRef::Self_(_)));
}

// ---- the Incomplete rung ----

#[test]
fn a_missing_member_is_a_partial_method_node_at_the_cursor() {
    // `let x = obj.` — the member is being typed: the tree keeps the
    // METHOD shape with an absent member, the span covers the partial
    // region, and one diag still reports the gap
    let src = "fn f(obj: C) -> i32 { let x = obj.; return 0; }\n";
    let (doc, diags) = parse(src, Mode::Impl);
    assert_eq!(diags.len(), 1, "the member gap is still one diag");
    assert!(diags[0].msg.contains("member name"), "{:?}", diags);
    let ast = doc.ast();
    for i in ast.root().items(ast) {
        if let ItemRef::Fn(h) = *i {
            if let StmtRef::Let(l) = h.get(ast).body.get(ast).stmts[0] {
                let ExprRef::Method(m) = l.get(ast).init else {
                    panic!("partial method expected: {:?}", l.get(ast).init)
                };
                let d = m.get(ast);
                assert!(d.member.is_none(), "the member slot is absent");
                assert!(d.rparen.is_none());
                // the span covers `obj.` — the consumer probe is the
                // byte BEFORE the cursor (the last typed byte): the
                // dot's own byte lands the walk on the partial node
                let dot = src.find("obj.").map(|i| i as u32 + 4).unwrap();
                let at = ast.node_at(dot - 1).expect("a real node at the cursor");
                assert!(at.is::<MethodNode>(), "{}", at.kind_name());
                assert!(ast.is_incomplete(at));
                return;
            }
        }
    }
    panic!("no fn/let in the module");
}

#[test]
fn an_unclosed_call_records_the_absent_rparen() {
    let src = "fn f(g: fn(i32) -> i32) -> i32 { let x = g(1; return 0; }\n";
    let (doc, diags) = parse(src, Mode::Impl);
    assert_eq!(diags.len(), 1, "the missing `)` is still one diag");
    let ast = doc.ast();
    for i in ast.root().items(ast) {
        if let ItemRef::Fn(h) = *i {
            if let StmtRef::Let(l) = h.get(ast).body.get(ast).stmts[0] {
                let ExprRef::Call(c) = l.get(ast).init else {
                    panic!("call expected: {:?}", l.get(ast).init)
                };
                let d = c.get(ast);
                assert_eq!(d.args.len(), 1);
                assert!(d.rparen.is_none(), "the closer is absent");
                assert!(ast.is_incomplete(c.any()));
                return;
            }
        }
    }
    panic!("no fn/let in the module");
}

#[test]
fn a_closed_call_is_complete_with_the_rparen_span() {
    let (doc, init) = let_init_of("fn f(g: fn(i32) -> i32) -> i32 { let x = g(1); return x; }\n");
    let ast = doc.ast();
    let ExprRef::Call(c) = init else {
        panic!("call expected")
    };
    let d = c.get(ast);
    let rp = d.rparen.expect("the closer's span");
    assert_eq!(doc.text(rp), ")");
    assert!(!ast.is_incomplete(c.any()));
}

#[test]
fn a_member_call_keeps_its_member_when_only_the_args_fail() {
    // `obj.m(` — the member parsed, the args never will: the partial
    // METHOD shape carries the member for the resolver doors
    let src = "fn f(obj: C) -> i32 { let x = obj.m(; return 0; }\n";
    let (doc, diags) = parse(src, Mode::Impl);
    assert!(!diags.is_empty(), "the arg gap is diagnosed");
    let ast = doc.ast();
    for i in ast.root().items(ast) {
        if let ItemRef::Fn(h) = *i {
            if let StmtRef::Let(l) = h.get(ast).body.get(ast).stmts[0] {
                let ExprRef::Method(m) = l.get(ast).init else {
                    panic!("partial method expected: {:?}", l.get(ast).init)
                };
                let d = m.get(ast);
                let (name, nsp) = d.member().expect("the member parsed");
                assert_eq!(ast.name(name), "m");
                assert_eq!(doc.text(nsp), "m");
                assert!(d.rparen.is_none());
                assert!(ast.is_incomplete(m.any()));
                return;
            }
        }
    }
    panic!("no fn/let in the module");
}

#[test]
fn an_unclosed_use_group_is_a_partial_use_node() {
    // `use p::` at the typing edge — the partial use keeps the path
    let src = "use p::;\nlet ok = 1;\n";
    let (doc, diags) = parse(src, Mode::Impl);
    assert!(!diags.is_empty(), "the leaf gap is diagnosed");
    let ast = doc.ast();
    let items = ast.root().items(ast);
    let Some(ItemRef::Use(u)) = items.iter().find(|i| matches!(i, ItemRef::Use(_))) else {
        panic!("partial use expected, items: {items:?}")
    };
    let d = u.get(ast);
    assert_eq!(ast.name(d.path[0]), "p");
    assert!(d.names.is_empty());
    assert!(d.group.is_none());
    assert!(ast.is_incomplete(u.any()));
    // the cursor probe (the byte before the typing edge) lands on the
    // node — the consumer convention `node_at(pos - 1)`
    let pos = src.find("::").map(|i| i as u32 + 2).unwrap();
    let at = ast.node_at(pos - 1).expect("a real node at the cursor");
    assert!(
        ast.ancestors(at).any(|h| h.is::<UseNode>()),
        "the use is an ancestor of the cursor: {}",
        at.kind_name()
    );
}

#[test]
fn the_flat_and_braced_use_leaves_are_complete() {
    let parse_clean = |src: &str| {
        let (doc, diags) = parse(src, Mode::Impl);
        assert!(diags.is_empty(), "{src:?} -> {diags:?}");
        doc
    };
    let d = parse_clean("use p::A;\n");
    let ast = d.ast();
    for i in ast.root().items(ast) {
        if let ItemRef::Use(u) = *i {
            assert!(!ast.is_incomplete(u.any()));
            assert!(u.get(ast).group.is_none(), "the flat leaf has no group");
        }
    }
    let d = parse_clean("use p::{A, B};\n");
    let ast = d.ast();
    for i in ast.root().items(ast) {
        if let ItemRef::Use(u) = *i {
            let ud = u.get(ast);
            assert!(!ast.is_incomplete(u.any()));
            let g = ud.group.expect("the brace group's span");
            assert_eq!(d.text(g), "{A, B}");
        }
    }
}

#[test]
fn complete_kinds_answer_not_incomplete() {
    let (doc, _) = let_init_of("fn f(g: fn(i32) -> i32) -> i32 { let x = g(1); return x; }\n");
    let ast = doc.ast();
    // the module spine: nothing incomplete in a clean tree
    let h: AnyHandle = ast.root().any();
    let mut seen = 0u32;
    for a in ast.ancestors(h) {
        seen += 1;
        assert!(!ast.is_incomplete(a), "{} is incomplete", a.kind_name());
    }
    assert!(seen > 0);
}
