//! The hole gate — law 2: a hole is a single-line zone. For every
//! construct the printer could tear across lines, in an f-string hole
//! at every hash level, the four assertions of the paren matrix
//! (`tests/paren_matrix.rs`): the input parses clean, the OUTPUT
//! parses clean, the span-free tree is unchanged, and `fmt(out) ==
//! out`. The second assertion IS the one-line law — a layout newline
//! inside a hole is a LEX error (`unterminated format-string hole`),
//! at every hash level.

mod common;

use common::span_free;
use rut_ast::parser::{parse_with_recover, Mode};
use rut_ast::AstPrinter;
use rut_fmt::{format, Style};

/// one representative construct per shape the printer could break
/// across lines inside a hole: the block-carrying atoms (`when` with
/// its always-broken arm list, `async` with its always-broken block),
/// the nesting (an f-string whose inner hole holds a `when`), and the
/// wide shapes (the try-flat sites whose broken spelling emits
/// newlines — a 45-argument call, a struct literal, a binary chain)
fn constructs() -> Vec<(&'static str, String)> {
    let call_args = (1..=45)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let binary_chain = (1..=30)
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(" + ");
    vec![
        ("when", "when (n) { 1 -> 2, else -> 3 }".to_string()),
        (
            "when-block-arms",
            "when (n) { 1 -> { a + 1 }, else -> { b + 2 } }".to_string(),
        ),
        ("async", "async { a = 1; a }".to_string()),
        (
            "nested-fstr",
            "f\"in{when (n) { 1 -> 2, else -> 3 }}\"".to_string(),
        ),
        ("call-45", format!("g({call_args})")),
        (
            "struct-wide",
            "P { a: 1, b: 2, c: 3, d: 4, e: 5, f: 6, g: 7, h: 8, i: 9, j: 10, k: 11, l: 12, m: 13, n: 14, o: 15, p: 16 }".to_string(),
        ),
        ("binary-wide", binary_chain),
    ]
}

/// the cell's source: the construct in an f-string hole at the hash
/// level
fn spell(level: usize, hole: &str) -> String {
    let h = "#".repeat(level);
    format!("let s = f{h}\"{{{hole}}}\"{h};\n")
}

/// the first differing line of two dumps, for a readable failure
fn first_diff(want: &str, got: &str) -> String {
    let w: Vec<&str> = want.lines().collect();
    let g: Vec<&str> = got.lines().collect();
    for i in 0..w.len().max(g.len()) {
        let a = w.get(i).copied().unwrap_or("<eof>");
        let b = g.get(i).copied().unwrap_or("<eof>");
        if a != b {
            return format!("line {i}: `{a}` != `{b}`");
        }
    }
    String::new()
}

#[test]
fn holes_print_on_one_line_at_every_level() {
    let mode = Mode::Impl;
    let mut failures: Vec<String> = Vec::new();
    let mut cells = 0;
    for (name, hole) in constructs() {
        for level in 0..=2 {
            cells += 1;
            let cell = format!("{name} x level-{level}");
            let src = spell(level, &hole);
            let in_doc = parse_with_recover(src.clone(), mode);
            if !in_doc.ast().is_clean() {
                failures.push(format!(
                    "{cell}: the INPUT does not parse clean: {}",
                    in_doc.ast().errors()[0].msg
                ));
                continue;
            }
            let out = match format(&src, mode, &Style::default()) {
                Ok(o) => o,
                Err(ds) => {
                    failures.push(format!("{cell}: fmt refused: {}", ds[0].msg));
                    continue;
                }
            };
            let out_doc = parse_with_recover(out.clone(), mode);
            if !out_doc.ast().is_clean() {
                failures.push(format!(
                    "{cell}: the OUTPUT does not parse clean (the hole was \
                     torn across lines?): {}",
                    out_doc.ast().errors()[0].msg
                ));
                continue;
            }
            let want = span_free(&AstPrinter::text_of(
                in_doc.ast(),
                in_doc.text(in_doc.span()),
            ));
            let got = span_free(&AstPrinter::text_of(
                out_doc.ast(),
                out_doc.text(out_doc.span()),
            ));
            if got != want {
                failures.push(format!(
                    "{cell}: the tree changed under formatting ({})",
                    first_diff(&want, &got)
                ));
                continue;
            }
            let again = match format(&out, mode, &Style::default()) {
                Ok(a) => a,
                Err(ds) => {
                    failures.push(format!("{cell}: fmt(fmt) refused: {}", ds[0].msg));
                    continue;
                }
            };
            if again != out {
                failures.push(format!("{cell}: NOT idempotent"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "the hole gate failed on {} of {cells} cells:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
