//! Corpus gate — the runnable projects (`examples/**`) and the
//! playground classics (`demo/src/examples/`) classify cleanly:
//! sorted, non-overlapping, in-file tokens; every reserved word keyword;
//! every Impl file yields symbols; `.d.rut` parses in `Mode::Decl`.
//! Mirrors rut-parser's corpus test (RFC 0030 §7) from the LSP side.

use rut_lsp::line_index::LineIndex;
use rut_lsp::semantic::{classify, symbols, TokenType};
use rut_parser::{is_reserved_kw, Mode};

fn corpus() -> Vec<std::path::PathBuf> {
    let roots = [
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demo/src/examples"),
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

fn analyzed(src: &str, mode: Mode) -> (Vec<rut_lexer::token::Token>, rut_ast::ast::Ast, Vec<(rut_lexer::span::Span, TokenType)>) {
    let (toks, _) = rut_lexer::lexer::lex(src);
    let (ast, _) = rut_parser::parse(src, mode);
    let spans = classify(&toks, &ast);
    (toks, ast, spans)
}

#[test]
fn corpus_classifies() {
    let files = corpus();
    assert!(files.len() >= 15, "corpus shrank: {}", files.len());
    for path in &files {
        let raw = std::fs::read_to_string(path).unwrap();
        let src = rut_lexer::lexer::normalize(&raw);
        let mode = if path.to_string_lossy().ends_with(".d.rut") {
            Mode::Decl
        } else {
            Mode::Impl
        };
        let (_, _, spans) = analyzed(&src, mode);
        // tokens: non-empty, in-file, strictly increasing, non-overlapping
        let len = src.len() as u32;
        let mut prev_hi = 0;
        for (sp, _) in &spans {
            assert!(sp.lo < sp.hi, "{}: empty span at {}", path.display(), sp.lo);
            assert!(sp.hi <= len, "{}: span out of file", path.display());
            assert!(
                sp.lo >= prev_hi,
                "{}: overlap at {} (prev hi {prev_hi})",
                path.display(),
                sp.lo
            );
            prev_hi = sp.hi;
        }
        // line index round-trips every token start
        let index = LineIndex::new(&src);
        for (sp, _) in &spans {
            let (l, c) = index.position(&src, sp.lo);
            assert_eq!(index.byte(&src, l, c), sp.lo, "{}: round trip", path.display());
        }
        // symbols exist for every Impl file
        if mode == Mode::Impl {
            let (toks, ast, _) = analyzed(&src, mode);
            let syms = symbols(&toks, &ast);
            assert!(!syms.is_empty(), "{}: no document symbols", path.display());
            for s in &syms {
                assert!(
                    s.selection.lo >= s.range.lo && s.selection.hi <= s.range.hi,
                    "{}: symbol `{}` selection outside range",
                    path.display(),
                    s.name
                );
            }
        }
    }
}

#[test]
fn corpus_reserved_words_are_keywords() {
    for path in &corpus() {
        let raw = std::fs::read_to_string(path).unwrap();
        let src = rut_lexer::lexer::normalize(&raw);
        let mode = if path.to_string_lossy().ends_with(".d.rut") {
            Mode::Decl
        } else {
            Mode::Impl
        };
        let (toks, _, spans) = analyzed(&src, mode);
        for t in &toks {
            if let rut_lexer::token::Tok::Ident(s) = &t.tok {
                // true/false lex as Bool literals, never Ident tokens
                if is_reserved_kw(s) {
                    let hit = spans
                        .iter()
                        .any(|(sp, ty)| sp.lo == t.span.lo && *ty == TokenType::Keyword);
                    assert!(
                        hit,
                        "{}: reserved word `{}` at {} not keyword-classified",
                        path.display(),
                        s,
                        t.span.lo
                    );
                }
            }
        }
    }
}
