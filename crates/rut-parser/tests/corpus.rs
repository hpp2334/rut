//! Corpus conformance — RFC 0030 §7: the runnable example projects
//! (`examples/**`) and the playground classics (`demo/src/examples/`)
//! parse with zero diags (declaration mode for `*.d.rut`); together they
//! are the parser's conformance suite. Also: depth budgets fire as one
//! clean Diag (C3).

use rut_ast::ast::*;
use rut_parser::{parse, Mode};

fn corpus() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let roots = [
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demo/src/examples"),
    ];
    let mut stack = roots.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().map(|x| x == "rut").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn corpus_parses_clean() {
    let files = corpus();
    assert!(files.len() >= 15, "expected the full corpus, found {}", files.len());
    let mut failures = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap();
        let decl = f.file_name().unwrap().to_str().unwrap().ends_with(".d.rut");
        let (_, diags) = parse(&src, if decl { Mode::Decl } else { Mode::Impl });
        if !diags.is_empty() {
            failures.push(format!(
                "{}: {} diag(s): first = {}",
                f.display(),
                diags.len(),
                diags[0].msg
            ));
        }
    }
    assert!(failures.is_empty(), "parser regressions:\n{}", failures.join("\n"));
}

#[test]
fn depth_budget_is_a_diag_not_a_crash() {
    // 100k-deep parens: one clean "nesting too deep", never a host crash (C3)
    let src = format!("fn f() -> nil {{ let x = {}1{}; }}", "(".repeat(100_000), ")".repeat(100_000));
    let (_, diags) = parse(&src, Mode::Impl);
    assert!(diags.iter().any(|d| d.msg.contains("nesting too deep")), "want a nesting diag, got: {diags:?}");
}

#[test]
fn reserved_words_explain_themselves() {
    for (word, want) in [
        ("switch", "when"),
        ("match", "when"),
        ("null", "nil"),
        ("var", "let"),
        ("instanceof", "is"),
    ] {
        let src = format!("fn f() -> nil {{ let x = {word}; }}");
        let (_, diags) = parse(&src, Mode::Impl);
        assert!(
            diags.iter().any(|d| d.msg.contains(want)),
            "`{word}` diag should mention the null-pointer literal; old text mentioned `{want}`: {diags:?}"
        );
    }
}

#[test]
fn fstring_holes_allow_string_arguments() {
    // corpus-driven (closures-generics.rut L32): hole termination is
    // brace-based, so string literals lex fine inside holes — RFC 0007 §2's
    // "bind it first" stays a style note, not a lex error
    let src = "fn f() -> nil { let name = Option.some(\"x\"); print(f\"n={name.unwrap_or(\"?\")}\"); }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn tuples_parse_and_destructure() {
    // RFC 0007: tuples are records with numeric fields
    let src = "fn f() -> nil { let x = (1, 2); let (a, b) = x; let n = x.0; }";
    let (_, diags) = parse(&src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // a one-element paren is still a grouping, not a tuple
    let src = "fn f() -> nil { let x = (1); }";
    let (_, diags) = parse(&src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn pointers_and_nil_parse() {
    // RFC 0005 §9 + RFC 0044: `?T` types, the `nil` literal, the
    // `on_drop` decl
    let src = "fn f() -> nil { let p: ?i32 = 7; if (p != nil) { } }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // anonymous closures: `fn (params) { .. }` — no arrow
    let src = "fn f() -> nil { let g = fn (a: i32) -> bool { return a > 0; }; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn nil_is_the_empty_type_and_value() {
    // v1.2: `unit` is gone; `nil` names the empty type and its value
    let src = "fn f() -> nil { let u: nil = nil; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn empty_parens_are_rejected_with_nil_hint() {
    // v1.2: neither `()` the value nor `()` the type survives; both
    // diagnostics must point at `nil`
    let (_, diags) = parse("fn f() -> nil { let x = (); }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`nil`")),
        "`()` value diag should point at `nil`: {diags:?}"
    );
    let (_, diags) = parse("fn f() -> () { }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`nil`")),
        "`()` type diag should point at `nil`: {diags:?}"
    );
    let (_, diags) = parse("fn f() -> nil { let x: () = nil; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`nil`")),
        "`()` annotation diag should point at `nil`: {diags:?}"
    );
}

#[test]
fn bracket_array_and_async_parse() {
    // `[T]` is the array type spelling (RFC 0005 §9)
    let src = "fn f(xs: [i32]) -> i32 { return xs.len(); }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // `async fn` — the async spelling (RFC 0018 §2)
    let src = "async fn tick() -> nil { }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // the fn type with an omitted return is `nil` (RFC 0013 §1, v1.2)
    let src = "fn run(f: fn(i32)) -> nil { f(1); }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn repeat_and_address_of_parse() {
    // RFC 0005 §9: `[v; n]` — a VALUE and a count; no type-in-expression form
    let src = "fn f() -> nil { let a: [i32] = [0; 8]; let b: [?i32] = [nil; 8]; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // the count is a full expression
    let src = "fn f(n: i32) -> nil { let a: [f64] = [1.5; n * 2 + 1]; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // RFC 0044: bindings share — a `T` widens into `?T`, no `&` needed
    let src = "fn f() -> nil { let p: ?i32 = 7; let q: ?Point = Point { x: 1 }; let y: i32 = p; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // binary `&` and `*` still parse
    let src = "fn f(a: i32, b: i32) -> i32 { return a & b; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    let src = "fn f(a: i32, b: i32) -> i32 { return a * b; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn nullable_types_parse() {
    // RFC 0044 (user ruling): `?` is the ONE nullable spelling — prefix,
    // binding the following type TERM. `[?T]` is `[T | nil]` (nullable
    // ELEMENT — TyArray(TyOpt)), while `?[T]` is `[T] | nil` (nullable
    // ARRAY — TyOpt(TyArray)); `??T` chains, `?[?T]` nests the other way.
    let src = "fn f(a: [?i32], b: ?[i32], c: ??i32, d: ?[?i32]) -> ?i32 { return a[0]; }";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    let items = ast.module_items(ast.root);
    let f = items.iter().find(|i| matches!(ast.item(**i), ItemKind::Fn(_))).expect("fn");
    let ItemKind::Fn(d) = ast.item(*f) else { unreachable!() };
    let param_ty = |i: usize| ast.param(d.params[i]).clone();
    let MemberKind::Param(p0) = param_ty(0) else { panic!("param 0") };
    let MemberKind::Param(p1) = param_ty(1) else { panic!("param 1") };
    let MemberKind::Param(p2) = param_ty(2) else { panic!("param 2") };
    let MemberKind::Param(p3) = param_ty(3) else { panic!("param 3") };
    // `[?i32]` — the array OUTSIDE, the nullable on the ELEMENT
    let TypeKind::TyArray { elem } = ast.ty(p0.ty.unwrap()) else {
        panic!("`[?i32]` must be TyArray(TyOpt), got {:?}", ast.ty(p0.ty.unwrap()));
    };
    assert!(matches!(ast.ty(*elem), TypeKind::TyOpt { .. }), "the element carries the `?`");
    // `?[i32]` — the nullable OUTSIDE, the array inside
    let TypeKind::TyOpt { inner } = ast.ty(p1.ty.unwrap()) else {
        panic!("`?[i32]` must be TyOpt(TyArray), got {:?}", ast.ty(p1.ty.unwrap()));
    };
    assert!(matches!(ast.ty(*inner), TypeKind::TyArray { .. }), "the array is the payload");
    // `??i32` chains — Opt(Opt(path))
    let TypeKind::TyOpt { inner } = ast.ty(p2.ty.unwrap()) else {
        panic!("`??i32` must be TyOpt, got {:?}", ast.ty(p2.ty.unwrap()));
    };
    assert!(matches!(ast.ty(*inner), TypeKind::TyOpt { .. }), "the chain nests");
    // `?[?i32]` — both at once: Opt(Array(Opt))
    let TypeKind::TyOpt { inner } = ast.ty(p3.ty.unwrap()) else {
        panic!("`?[?i32]` must be TyOpt, got {:?}", ast.ty(p3.ty.unwrap()));
    };
    let TypeKind::TyArray { elem } = ast.ty(*inner) else {
        panic!("the payload is the array, got {:?}", ast.ty(*inner));
    };
    assert!(matches!(ast.ty(*elem), TypeKind::TyOpt { .. }), "the element is nullable");
    // the return keeps the same rule
    let TypeKind::TyOpt { .. } = ast.ty(d.ret.unwrap()) else {
        panic!("`-> ?i32` must be TyOpt");
    };
    // `?` composes with generics and fn types
    let src = "fn f(v: Vec<?Point>) -> fn(?i32) -> ?str { panic(\"\"); }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // each union member carries its own `?` — prefix, before the member
    let src = "type U = ?i32 | ?str;";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn the_postfix_nullable_spelling_is_removed() {
    // the user ruling: `T?` / `[T]?` no longer parse — each trailing `?`
    // diagnoses pointing at the prefix spelling, and the bare type is
    // the recovery result (the Diag fails the compile either way)
    for src in [
        "fn f(p: i32?) -> nil { }",
        "fn f(p: [i32]?) -> nil { }",
        "fn f() -> [i32]? { return nil; }",
        "type U = i32? | str?;",
    ] {
        let (_, diags) = parse(src, Mode::Impl);
        assert!(
            diags.iter().any(|d| d.msg.contains("the postfix spelling `T?` was removed") && d.msg.contains("`?T`")),
            "`{src}` must diagnose the removed postfix spelling: {diags:?}"
        );
    }
}

#[test]
fn removed_pointer_spellings_diagnose() {
    // the `*T` type spelling points at `?T`...
    let (_, diags) = parse("fn f(p: *i32) -> nil { }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`?T`")),
        "`*T` should diagnose `?T`: {diags:?}"
    );
    // ...and the `&x` / `*x` prefix forms point at direct passing
    let (_, diags) = parse("fn f() -> nil { let p: ?i32 = &7; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("share by reference")),
        "`&x` should diagnose the sharing law: {diags:?}"
    );
    let (_, diags) = parse("fn f() -> nil { let p: ?i32 = 7; let y: i32 = *p; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("share by reference")),
        "`*x` should diagnose the sharing law: {diags:?}"
    );
}

#[test]
fn the_array_name_is_gone_from_the_grammar() {
    // the `Array` name is removed — the type is `[T]`, construction is
    // `[v; n]` (RFC 0005 §9); `[]` stays the empty list literal
    let src = "fn f() -> nil { let a: [i32] = []; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}
