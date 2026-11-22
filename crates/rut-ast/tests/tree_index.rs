//! TreeIndex + AnyHandle — the generic machinery's pins: `node_at`'s
//! deepest-match (operator bytes, error fences), the ancestor chain
//! through the closed-position kinds (BlockNode, WhenArmNode), the
//! parent rows, subtree windows (root equality, arm and block windows,
//! the dead row), and the canonical-comparable determinism obligation
//! (same tree → same bytes).

mod common;

use common::parse;
use rut_ast::parser::Mode;
use rut_ast::{
    AnyHandle, AstPrinter, BinaryNode, BlockNode, ErrorNode, ImplNode, LetStmtNode, MethodDeclNode,
    ModuleNode, NodeKind, TyPathNode, WhenArmNode, WhenStmtNode,
};

const SRC: &str = "\
let g = 1;

fn area(w: i32, h: i32) -> i32 {
    w * h
}

fn main() {
    let x = 2;
    when (x) {
        1 -> { let y = g; y }
        _ -> 0
    }
}

class Boxed {
    v: i32;
}

impl Boxed {
    fn hit(self, v: i32) -> i32 {
        v + 1
    }
}
";

/// the garbage item fenced at the top level (the fence's span carries
/// the diag rows) — node_at must land ON it
const BROKEN: &str = "let ok = 1;\noops here\n";

#[test]
fn node_at_finds_the_deepest_node() {
    let (doc, diags) = parse(SRC, Mode::Impl);
    let ast = doc.ast();
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");

    // an operator byte: only the binary (and its enclosing windows)
    // contain it — the deepest match is the BinaryNode itself
    let plus = SRC.find("v + 1").unwrap() as u32 + 2;
    let h = ast.node_at(plus).expect("the operator sits inside the fn");
    assert!(
        h.is::<BinaryNode>(),
        "deepest at `+` is the binary, got {}",
        h.kind_name()
    );

    // an operand byte: the leaf under it is deeper than any wrapper
    let w = SRC.find("w * h").unwrap() as u32;
    let h = ast.node_at(w).expect("the operand sits inside area");
    assert!(
        !h.is::<BinaryNode>(),
        "the operand's leaf wins, not the binary"
    );

    // a node's exact END is outside it: the byte after `let x = 2;`
    // belongs to the block, never the let
    let stmt = SRC.find("let x = 2;").unwrap() as u32 + "let x = 2;".len() as u32;
    let h = ast.node_at(stmt).expect("the block owns the byte after ;");
    assert!(
        h.downcast::<LetStmtNode>().is_none(),
        "the let ends at its own hi — got {}",
        h.kind_name()
    );

    // dead space past the last item answers None
    assert_eq!(ast.node_at(u32::MAX), None);
}

#[test]
fn node_at_lands_on_error_fences() {
    let (doc, diags) = parse(BROKEN, Mode::Impl);
    let ast = doc.ast();
    assert_eq!(diags.len(), 2, "the fence owns its region's diags");
    let pos = BROKEN.find("oops").unwrap() as u32;
    let h = ast.node_at(pos).expect("the fence covers the garbage");
    assert!(
        h.is::<ErrorNode>(),
        "the fenced region is an ErrorNode, got {}",
        h.kind_name()
    );
    // the fence is a root-level item: its parent IS the module
    assert_eq!(ast.parent(h).map(|p| p.is::<ModuleNode>()), Some(true));
}

#[test]
fn ancestors_walk_the_closed_positions() {
    let (doc, _) = parse(SRC, Mode::Impl);
    let ast = doc.ast();
    // inside the when arm's body block — the chain crosses BlockNode
    // and WhenArmNode (positions that hold exactly one kind and never
    // an Error)
    let pos = SRC.find("let y = g;").unwrap() as u32;
    let h = ast.node_at(pos).expect("the arm body owns the stmt");
    let chain: Vec<AnyHandle> = ast.ancestors(h).collect();
    let kinds: Vec<&'static str> = chain.iter().map(|a| a.kind_name()).collect();
    assert!(
        chain.iter().any(|&a| a.is::<WhenArmNode>()),
        "the when arm is in the chain: {kinds:?}"
    );
    assert!(
        chain.iter().any(|&a| a.is::<BlockNode>()),
        "the arm's block is in the chain: {kinds:?}"
    );
    assert!(
        chain.iter().any(|&a| a.is::<WhenStmtNode>()),
        "the when stmt is in the chain: {kinds:?}"
    );
    // the chain is the spine: arm → stmt → … → module, self first
    assert_eq!(kinds.last(), Some(&"Module"), "the root closes: {kinds:?}");
    assert_eq!(chain[0], h, "the walk starts at the node itself");
}

#[test]
fn parent_rows_answer_the_owners() {
    let (doc, _) = parse(SRC, Mode::Impl);
    let ast = doc.ast();
    // the method's parent is its impl; the method body block's parent
    // is the method; the root has no parent
    let plus = SRC.find("v + 1").unwrap() as u32 + 2;
    let bin = ast.node_at(plus).unwrap();
    let method = ast
        .ancestors(bin)
        .find(|&a| a.is::<MethodDeclNode>())
        .expect("the binary sits inside hit()");
    let owner = ast.parent(method).expect("the method has an owner");
    assert!(
        owner.is::<ImplNode>(),
        "hit()'s owner is the impl, got {}",
        owner.kind_name()
    );
    let body = method
        .downcast::<MethodDeclNode>()
        .unwrap()
        .get(ast)
        .body
        .expect("hit() has a body")
        .any();
    let body_owner = ast.parent(body).expect("the block has an owner");
    assert_eq!(body_owner, method, "the closed position names the method");
    // and the impl's parent is the module
    let module = ast.parent(owner).expect("the impl has an owner");
    assert!(module.is::<ModuleNode>(), "the impl hangs off the module");
    let root = ast.root().any();
    assert_eq!(ast.parent(root), None, "the root has no parent");
}

#[test]
fn span_of_agrees_with_the_typed_span() {
    let (doc, _) = parse(SRC, Mode::Impl);
    let ast = doc.ast();
    let root = ast.root();
    let any = root.any();
    assert_eq!(ast.span_of(any), root.span(ast));
    // every ancestor address, checked against its typed span
    let pos = SRC.find("v + 1").unwrap() as u32 + 2;
    for a in ast.ancestors(ast.node_at(pos).unwrap()) {
        if let Some(m) = a.downcast::<MethodDeclNode>() {
            assert_eq!(ast.span_of(a), m.span(ast));
        }
        if let Some(b) = a.downcast::<BlockNode>() {
            assert_eq!(ast.span_of(a), b.span(ast));
        }
    }
}

#[test]
fn subtree_of_the_root_is_the_tree() {
    let (doc, _) = parse(SRC, Mode::Impl);
    let ast = doc.ast();
    let whole = AstPrinter::json_of(ast);
    let window = AstPrinter::json(&AstPrinter::subtree(ast, ast.root().any()));
    assert_eq!(window, whole, "the root window IS the whole tree");
}

#[test]
fn subtree_prints_the_closed_positions_and_the_fence() {
    let (doc, _) = parse(SRC, Mode::Impl);
    let ast = doc.ast();
    // the method body block — a closed position (Handle<BlockNode>)
    let pos = SRC.find("v + 1").unwrap() as u32;
    let body = ast
        .ancestors(ast.node_at(pos).unwrap())
        .find(|&a| a.is::<BlockNode>())
        .expect("the method body is in the chain");
    let json = AstPrinter::json(&AstPrinter::subtree(ast, body));
    let head = json.split(',').nth(1);
    assert_eq!(
        head,
        Some("\"kind\":\"Block\""),
        "the block window opens on the block row: {json}"
    );
    // the when arm — the other closed position (Handle<WhenArmNode>)
    let pos = SRC.find("let y = g;").unwrap() as u32;
    let arm = ast
        .ancestors(ast.node_at(pos).unwrap())
        .find(|&a| a.is::<WhenArmNode>())
        .expect("the arm is in the chain");
    let json = AstPrinter::json(&AstPrinter::subtree(ast, arm));
    let head = json.split(',').nth(1);
    assert_eq!(
        head,
        Some("\"kind\":\"WhenArm\""),
        "the arm window opens on the arm row: {json}"
    );
    // a type window (the hit() param's annotation)
    let ty_pos = SRC.find("self, v: i32").unwrap() as u32 + 9;
    let ty = ast.node_at(ty_pos).expect("the annotation covers i32");
    assert!(
        ty.is::<TyPathNode>(),
        "the param type is a TyPath, got {}",
        ty.kind_name()
    );
    let json = AstPrinter::json(&AstPrinter::subtree(ast, ty));
    assert!(
        json.contains("\"kind\":\"TyPath\""),
        "the ty window prints the TyPath row: {json}"
    );
    // a pattern window (the arm's `1 ->`)
    let pat = arm.downcast::<WhenArmNode>().unwrap().get(ast).pats[0].any();
    let json = AstPrinter::json(&AstPrinter::subtree(ast, pat));
    assert!(
        json.contains("\"kind\":\"PatLit\""),
        "the pat window prints the PatLit row: {json}"
    );
    // the error fence window (from the broken module)
    let (broken, _) = parse(BROKEN, Mode::Impl);
    let broken_ast = broken.ast();
    let fence = broken_ast
        .node_at(BROKEN.find("oops").unwrap() as u32)
        .unwrap();
    let json = AstPrinter::json(&AstPrinter::subtree(broken_ast, fence));
    assert!(
        json.contains("\"kind\":\"Error\"") && json.contains("\"diag\":\""),
        "the fence window carries its diags: {json}"
    );
}

#[test]
fn a_dead_slot_prints_the_dead_row() {
    let (doc, _) = parse(SRC, Mode::Impl);
    let ast = doc.ast();
    // a fabricated address into a dead (never-allocated) slot
    let dead = AnyHandle {
        kind: BlockNode::KIND,
        idx: u32::MAX,
    };
    let json = AstPrinter::json(&AstPrinter::subtree(ast, dead));
    assert_eq!(json, "{\"id\":0,\"kind\":\"dead\",\"span\":[0,0]}");
    // (a bogus KIND cannot be spelled anymore — the closed enum
    // refuses it at compile time; only the dead slot remains, and
    // it prints the same honest row)
    // the doors degrade, never lie: no parent, and span_of refuses
    assert_eq!(ast.parent(dead), None);
}

#[test]
#[should_panic(expected = "stale AnyHandle")]
fn span_of_refuses_a_dead_slot() {
    let (doc, _) = parse(SRC, Mode::Impl);
    let ast = doc.ast();
    let dead = AnyHandle {
        kind: BlockNode::KIND,
        idx: u32::MAX,
    };
    let _ = ast.span_of(dead);
}

#[test]
fn same_tree_same_bytes() {
    // the canonical-comparable obligation: two cold parses of the
    // same text produce byte-identical JSON, text, node_at answers,
    // and subtree windows — the edit≡cold tests will diff these bytes
    let (da_doc, da) = parse(SRC, Mode::Impl);
    let (db_doc, db) = parse(SRC, Mode::Impl);
    let (a, b) = (da_doc.ast(), db_doc.ast());
    assert_eq!(da.len(), db.len(), "clean parse, twice");
    assert_eq!(AstPrinter::json_of(a), AstPrinter::json_of(b));
    assert_eq!(AstPrinter::text_of(a, SRC), AstPrinter::text_of(b, SRC));

    let pos = SRC.find("let y = g;").unwrap() as u32;
    let ha = a.node_at(pos).unwrap();
    let hb = b.node_at(pos).unwrap();
    assert_eq!(ha.kind, hb.kind);
    assert_eq!(ha.idx, hb.idx);
    assert_eq!(
        AstPrinter::json(&AstPrinter::subtree(a, ha)),
        AstPrinter::json(&AstPrinter::subtree(b, hb)),
    );
    let chain_a: Vec<AnyHandle> = a.ancestors(ha).collect();
    let chain_b: Vec<AnyHandle> = b.ancestors(hb).collect();
    assert_eq!(chain_a, chain_b);
}
