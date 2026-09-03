//! Corpus conformance — RFC 0030 §7: `examples/**/*.rut` parse with zero
//! diags (declaration mode for `*.d.rut`); the corpus IS the parser's
//! conformance suite. Also: depth budgets fire as one clean Diag (C3).

use rut_parser::{parse, Mode};

fn corpus() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")];
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
    assert!(files.len() >= 35, "expected the full corpus, found {}", files.len());
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
    let src = format!("fn f(): unit {{ let x = {}1{}; }}", "(".repeat(100_000), ")".repeat(100_000));
    let (_, diags) = parse(&src, Mode::Impl);
    assert!(diags.iter().any(|d| d.msg.contains("nesting too deep")), "want a nesting diag, got: {diags:?}");
}

#[test]
fn reserved_words_explain_themselves() {
    for (word, want) in [
        ("switch", "when"),
        ("match", "when"),
        ("null", "Option"),
        ("var", "let"),
        ("instanceof", "is"),
    ] {
        let src = format!("fn f(): unit {{ let x = {word}; }}");
        let (_, diags) = parse(&src, Mode::Impl);
        assert!(
            diags.iter().any(|d| d.msg.contains(want)),
            "`{word}` diag should mention `{want}`: {diags:?}"
        );
    }
}

#[test]
fn fstring_holes_allow_string_arguments() {
    // corpus-driven (closures-generics.rut L32): hole termination is
    // brace-based, so string literals lex fine inside holes — RFC 0007 §2's
    // "bind it first" stays a style note, not a lex error
    let src = "fn f(): unit { let name = Option.some(\"x\"); print(f\"n={name.unwrap_or(\"?\")}\"); }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn tuples_error_at_the_comma() {
    let src = "fn f(): unit { let x = (1, 2); }";
    let (_, diags) = parse(&src, Mode::Impl);
    assert!(diags.iter().any(|d| d.msg.contains("no tuples")), "{diags:?}");
}
