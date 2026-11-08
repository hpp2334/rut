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

use rut_lexer::diag::Diag;
use rut_parser::Mode;

/// Format one rut source.
///
/// `src` may carry `\r\n` — the facade's normalization law rides first
/// (`parse_with_recover` returns the NORMALIZED text as `Parsed::src`,
/// and every span indexes it), so the comment reattachment and the
/// printer see the same offsets.
///
/// Errors are the frontend's own diagnostics (lex + parse), spans into
/// the normalized text: a source with diags is refused, never guessed
/// at — `is_clean` is the gate, `ast.errors()` the full report (the
/// recovering parse collects every diag, not just the first). The
/// caller renders them (`rut fmt` labels the group with the real path).
pub fn format(src: &str, mode: Mode, style: &Style) -> Result<String, Vec<Diag>> {
    let p = rut_parser::parse_with_recover(src, mode, rut_parser::ParseOpts::default());
    if !p.ast.is_clean() {
        return Err(p.ast.errors().to_vec());
    }
    // the leaf layer on demand: tokens + comments in one linear pass
    // over the NORMALIZED text (`p.src`) every span indexes
    let (toks, _, comments) = rut_lexer::lexer::lex_normalized(&p.src, true);
    let runs = comments::runs_from_comments(&comments, &toks, &p.src);
    let printer = printer::P::new(&p.ast, &p.src, &toks, style, runs);
    Ok(printer.print_module())
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
        panic!(
            "rut-fmt: refused to format: {}",
            ds.iter()
                .map(|d| d.msg.as_str())
                .collect::<Vec<_>>()
                .join(" | ")
        )
    })
}
