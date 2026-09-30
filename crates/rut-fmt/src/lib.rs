//! rut-fmt — the rut source formatter. The
//! language's own meaning-preserving pretty printer.
//!
//! The formatter is an **AST reprint** (the RFC's `pretty(ast)`), not a
//! token-stream pass: every item, statement, expression and type
//! reprints from the typed arena in canonical house layout, and the
//! source's **comments** — which the AST deliberately drops (§7's law:
//! "formatting fidelity is the formatter's job") — are recovered by the
//! between-token gap scan ([`comments`]) and re-attached verbatim.
//!
//! The two fidelity losses the RFC's AST carries are reconstructed:
//! source parens (the AST is the precedence-reduced grouping — absent
//! parens are re-emitted exactly where the parse's grouping would be
//! flattened; the op-level table lives in [`printer`]) and literal
//! re-derivation (floats from their f64 bits, strings re-quoted from
//! the decoded content, raw strings re-quoted canonically).
//!
//! What it does NOT do: it never changes program meaning — every test
//! run proves the formatted output parses clean, reparses to the same
//! tree (`fmt(fmt(x)) == fmt(x)`) and RUNS the same (VM-verified
//! probes, `tests/semantic_equiv.rs`).

pub mod comments;
pub mod printer;
pub mod style;

pub use comments::CommentRun;
pub use style::Style;

use rut_parser::Mode;

/// Format one rut source.
///
/// `src` may carry `\r\n` — the lexer's normalization law rides first
/// (`rut_lexer::lexer::normalize`), and spans index the NORMALIZED text,
/// so the comment scan and the printer see the same offsets.
///
/// Errors are the frontend's rendered diagnostics (lex + parse): a
/// source with diags is refused, never guessed at — the formatter's
/// input is a parsed-clean module.
pub fn format(src: &str, mode: Mode, style: &Style) -> Result<String, Vec<String>> {
    let normalized = rut_lexer::lexer::normalize(src);
    let (toks, diags) = rut_lexer::lexer::lex_mode(&normalized);
    if !diags.is_empty() {
        return Err(diags.iter().map(|d| d.msg.clone()).collect());
    }
    let (ast, diags) = rut_parser::parse(&normalized, mode);
    if !diags.is_empty() {
        return Err(diags.iter().map(|d| d.msg.clone()).collect());
    }
    let runs = comments::scan_comment_runs(&normalized, &toks);
    let p = printer::P::new(&ast, &normalized, &toks, style, runs);
    Ok(p.print_module())
}

/// Format a source, preserving the caller's trailing newline convention:
/// the printer always ends the output with exactly one `\n` unless the
/// input was wholly empty.
pub fn format_or_passthrough(src: &str, mode: Mode, style: &Style) -> String {
    if src.trim().is_empty() {
        return src.to_string();
    }
    format(src, mode, style).unwrap_or_else(|ds| {
        // the refusal path — the formatted text is the ORIGINAL (the
        // caller decides what to do with the diags); where the CLI
        // still needs a String this keeps the file untouched
        panic!("rut-fmt: refused to format: {}", ds.join(" | "))
    })
}
