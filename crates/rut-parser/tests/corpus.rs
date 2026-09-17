//! Corpus conformance — RFC 0030 §7: the runnable example projects
//! (`examples/**`) and the playground classics (`demo/src/examples/`)
//! parse with zero diags (declaration mode for `*.d.rut`); together they
//! are the parser's conformance suite. Also: depth budgets fire as one
//! clean Diag (C3).

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
    // RFC 0005: `*T` types, the `nil` literal, `make_ptr`/`on_drop` decls
    let src = "fn f() -> nil { let p: *i32 = make_ptr(7); if (p != nil) { } }";
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
    // `[T]` is the accepted spelling of `Array<T>` (RFC 0005)
    let src = "fn f(xs: [i32]) -> i32 { return xs.len(); }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // `async fn` — the `suspend fn` spelling (RFC 0018 §2)
    let src = "async fn tick() -> nil { }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // the fn type with an omitted return is `nil` (RFC 0013 §1, v1.2)
    let src = "fn run(f: fn(i32)) -> nil { f(1); }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}
