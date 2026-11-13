//! The hash-leveled multi-line literals through the parser: the lexed
//! values land in the AST unchanged (`Lit::RawStr`, the `FStr` parts),
//! `when` literal patterns accept the raw form, and level 0 keeps its
//! single-line diagnostics.

mod common;

use common::{first_fn_body, parse};
use rut_ast::parser::Mode;
use rut_ast::{Ast, ExprRef, FPartAst, Lit, StmtRef};

/// the statement's initializer expression of `fn f() { let x = INIT; }`
fn init_of(src: &str) -> (Ast, ExprRef) {
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    match first_fn_body(&ast).get(&ast).stmts[0] {
        StmtRef::Let(h) => {
            let init = h.get(&ast).init;
            (ast, init)
        }
        ref s => panic!("let stmt, got {s:?}"),
    }
}

#[test]
fn multiline_raw_lands_in_the_ast_verbatim() {
    let src = "fn f() {\n    let s = r#\"a\nb\"c\"#;\n}";
    let (ast, e) = init_of(src);
    let ExprRef::Lit(lh) = e else {
        panic!("raw literal, got {e:?}");
    };
    let Lit::RawStr(s) = &lh.get(&ast).lit else {
        panic!("raw literal, got {:?}", lh.get(&ast).lit);
    };
    assert_eq!(s, "a\nb\"c");
}

#[test]
fn multiline_fstr_parts_keep_the_newlines() {
    let src = "fn f() {\n    let t = f#\"row {n}\nend\"#;\n}";
    let (ast, e) = init_of(src);
    let ExprRef::FStr(fh) = e else {
        panic!("f-string, got {e:?}");
    };
    let parts = &fh.get(&ast).parts;
    assert_eq!(parts.len(), 3);
    let FPartAst::Lit(a) = &parts[0] else {
        panic!("first chunk, got {:?}", parts[0]);
    };
    assert_eq!(a, "row ");
    let FPartAst::Lit(c) = &parts[2] else {
        panic!("last chunk, got {:?}", parts[2]);
    };
    assert_eq!(c, "\nend");
    let FPartAst::Hole(_) = &parts[1] else {
        panic!("middle hole, got {:?}", parts[1]);
    };
}

#[test]
fn raw_string_pattern_matches_in_when() {
    let src = concat!(
        "fn pick(k: str) -> str {\n",
        "    return when (k) {\n",
        "        r##\"the \"# key\"## -> \"hash\",\n",
        "        else -> \"plain\",\n",
        "    };\n",
        "}"
    );
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
}

#[test]
fn level0_still_diagnoses_newlines() {
    // the frozen law: a newline inside any level-0 body is a lexer diag
    for src in [
        "fn f() {\n    let a = r\"x\ny\";\n}",
        "fn f() {\n    let a = \"x\ny\";\n}",
    ] {
        let (_, diags) = parse(src, Mode::Impl);
        assert!(
            diags.iter().any(|d| d.msg.contains("unterminated")),
            "{src:?}: expected an unterminated-literal diag"
        );
    }
}
