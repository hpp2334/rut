//! Corpus gate — every runnable project's rut source, classified and
//! analyzed through the `analyze` envelope: sorted, non-overlapping,
//! in-file tokens; every reserved word keyword-classified; every Impl
//! file yields symbols with selection ⊆ range; and the honest gate —
//! every corpus file parses with ZERO diagnostics through the same
//! pipeline the LSP serves. Mirrors the parser's corpus test
//! (rut-ast) from the LSP side.

mod common;

use common::*;
use serde_json::Value;

/// the four corpus trees (the lsp-align survey §6): examples, the
/// playground classics, the stdlib itself, and the bench workloads
fn corpus() -> Vec<std::path::PathBuf> {
    let roots = [
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demo/src/examples"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rut"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benches/workloads"),
    ];
    let mut out = Vec::new();
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "rut") {
                out.push(p);
            }
        }
    }
    for r in &roots {
        walk(r, &mut out);
    }
    out.sort();
    out
}

/// analyze one corpus file at a uri spelling its mode (`.d.rut` →
/// Decl — the filename convention the server itself rides)
fn analyze_corpus_file(s: &mut S, path: &std::path::Path, raw: &str) -> (String, Value) {
    let src = raw.replace("\r\n", "\n");
    let uri = format!("file:///corpus/{}", path.display());
    let a = s.analyze(&uri, &src);
    (src, a)
}

#[test]
fn corpus_classifies() {
    let files = corpus();
    assert!(files.len() >= 50, "corpus shrank: {}", files.len());
    let mut s = S::boot();
    for path in &files {
        let raw = std::fs::read_to_string(path).unwrap();
        let (src, a) = analyze_corpus_file(&mut s, path, &raw);
        // tokens: non-empty, in-file, strictly increasing, non-overlapping
        let toks = decode_tokens(&src, &a);
        let mut prev_hi = 0usize;
        for t in &toks {
            assert!(t.lo < t.hi, "{}: empty token at {}", path.display(), t.lo);
            assert!(t.hi <= src.len(), "{}: token out of file", path.display());
            assert!(
                t.lo >= prev_hi,
                "{}: overlap at {} (prev hi {prev_hi})",
                path.display(),
                t.lo
            );
            prev_hi = t.hi;
        }
        // symbols exist for every Impl file, selection ⊆ range
        if !path.to_string_lossy().ends_with(".d.rut") {
            let syms = a["symbols"].as_array().unwrap();
            assert!(!syms.is_empty(), "{}: no document symbols", path.display());
            for x in syms {
                let sel0 = pos_pair(&x["selectionRange"]["start"]);
                let sel1 = pos_pair(&x["selectionRange"]["end"]);
                let rng0 = pos_pair(&x["range"]["start"]);
                let rng1 = pos_pair(&x["range"]["end"]);
                assert!(
                    sel0 >= rng0 && sel1 <= rng1,
                    "{}: symbol `{}` selection outside range",
                    path.display(),
                    x["name"].as_str().unwrap_or("?")
                );
            }
        }
    }
}

#[test]
fn corpus_parses_clean_lsp() {
    // the honest gate: every corpus file parses with ZERO diagnostics
    // through the same pipeline the LSP serves (classification
    // invariants alone could pass over a corpus full of red squiggles)
    let files = corpus();
    assert!(files.len() >= 50, "corpus shrank: {}", files.len());
    let mut s = S::boot();
    let mut failures = Vec::new();
    for path in &files {
        let raw = std::fs::read_to_string(path).unwrap();
        let (_, a) = analyze_corpus_file(&mut s, path, &raw);
        let diags = a["diags"].as_array().unwrap();
        if !diags.is_empty() {
            failures.push(format!(
                "{}: {} diag(s): first = {}",
                path.display(),
                diags.len(),
                diags[0]["message"].as_str().unwrap_or_default()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "LSP corpus regressions:\n{}",
        failures.join("\n")
    );
}

#[test]
fn corpus_reserved_words_are_keywords() {
    let mut s = S::boot();
    for path in &corpus() {
        let raw = std::fs::read_to_string(path).unwrap();
        let (src, a) = analyze_corpus_file(&mut s, path, &raw);
        let toks = decode_tokens(&src, &a);
        for t in &toks {
            // true/false lex as Bool literals, never Ident tokens
            if rut_ast::parser::is_reserved_kw(&t.text) {
                assert_eq!(
                    t.ty,
                    t::KEYWORD,
                    "{}: reserved word `{}` at {}:{} not keyword-classified",
                    path.display(),
                    t.text,
                    t.line,
                    t.ch
                );
            }
        }
    }
}
