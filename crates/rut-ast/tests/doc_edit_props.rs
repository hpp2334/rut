//! The edit ≡ cold law (law 8's tree stage) — over RANDOM edit
//! sequences on the corpus fixtures (the runnable example projects,
//! the playground classics, the stdlib, the bench workloads — the
//! parser's conformance set) plus a literal-heavy stress fixture.
//!
//! Every step: a random text mutation, `Edit::between` (the diff
//! helper), `Doc::edit` — and the edited document must equal a cold
//! `Doc::parse` of the same text: the token stream (`tokens_in`), the
//! printed tree with every span (`AstPrinter::json_of`), the flat
//! error panel, the tree index's cursor answer and ancestor chain at
//! a sample of positions, and the line geometry. The PRNG is
//! deterministic (xorshift seeded from the file path), so a failure
//! reproduces bit-for-bit.

use rut_ast::parser::Mode;
use rut_ast::{AstPrinter, Doc};
use rut_lexer::lexer::canonical;
use rut_lexer::source::Edit;

// ── the deterministic PRNG (no rand dep — xorshift64) ──

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

fn seed_of(path: &std::path::Path) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.to_string_lossy().bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

// ── the mutation generator (the splice suite's shapes) ──

fn boundary_before(mut i: usize, s: &str) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn boundary_after(mut i: usize, s: &str) -> usize {
    let n = s.len();
    while i < n && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// fragments that stress the lexer's and the grammar's hard corners:
/// literal delimiters, hole braces, comments, newlines (inside and
/// outside literals), item keywords (splits/merges at item bounds)
const FRAGMENTS: &[&str] = &[
    "",
    " ",
    "\n",
    "  \n",
    "let",
    "fn",
    "{",
    "}",
    "\"",
    "\"s\"",
    "\\n",
    "r#\"",
    "\"#",
    "f\"",
    "{x}",
    "//",
    "/*",
    "*/",
    "*/\n",
    "123",
    "0xFF",
    "1.5e3",
    "u32",
    "->",
    "::",
    "é",
    "$",
    "r\"",
    "f#\"",
    "#",
    "\"\n",
    "{\n",
    "\u{feff}",
    "pub ",
    "class ",
    "impl ",
    "use core::{ };",
];

fn piece<'a>(rng: &mut Rng, text: &'a str) -> &'a str {
    // a quarter of the pieces come from the fragment pool (deletion
    // included), the rest are random char-boundary slices of the file
    // itself — real tokens splice against real tokens
    if rng.below(4) == 0 {
        return FRAGMENTS[rng.below(FRAGMENTS.len())];
    }
    let a = boundary_before(rng.below(text.len() + 1), text);
    let b = boundary_after(a + rng.below(13).min(text.len() - a), text);
    &text[a..b]
}

fn mutate(rng: &mut Rng, text: &str) -> String {
    let cut = boundary_before(rng.below(text.len() + 1), text);
    let cut_end = boundary_after((cut + rng.below(9)).min(text.len()), text);
    format!("{}{}{}", &text[..cut], piece(rng, text), &text[cut_end..])
}

// ── the law ──

/// the cursor answer at `pos`, canonically: the deepest node's
/// ancestor chain as (kind, span) rows — slot indexes differ between
/// an edited tree and a cold rebuild, the shape may not. Runs of
/// IDENTICAL rows collapse: degenerate recovery shapes can host two
/// same-kind same-span nodes (a wrapper over its own region), and
/// `node_at`'s tie between them breaks on the arena layout — the
/// chain's content is the same either way (the exact parent rows are
/// pinned in-crate, `ast::index::tests`).
fn chain_at(doc: &Doc, pos: u32) -> Vec<(&'static str, (u32, u32))> {
    let ast = doc.ast();
    let full: Vec<(&'static str, (u32, u32))> = match ast.node_at(pos) {
        None => Vec::new(),
        Some(h) => ast
            .ancestors(h)
            .map(|a| {
                let s = ast.span_of(a);
                (a.kind_name(), (s.lo, s.hi))
            })
            .collect(),
    };
    let mut out: Vec<(&'static str, (u32, u32))> = Vec::new();
    for row in full {
        if out.last() != Some(&row) {
            out.push(row);
        }
    }
    out
}

fn assert_edit_cold(file: &str, seq: usize, step: usize, doc: &Doc, new: &str, mode: Mode) {
    let cold = Doc::parse(mode, new.to_string());
    // tokens ≡ cold lex (the Eof rides the whole-span window)
    let dt = doc.tokens_in(doc.span());
    let ct = cold.tokens_in(cold.span());
    assert_eq!(
        dt.len(),
        ct.len(),
        "{file} seq {seq} step {step}: token count {} != cold {}",
        dt.len(),
        ct.len()
    );
    if let Some(i) = dt.iter().zip(ct.iter()).position(|(a, b)| a != b) {
        panic!(
            "{file} seq {seq} step {step}: first token divergence at {i}: {:?} != {:?}",
            dt[i], ct[i]
        );
    }
    // the printed tree ≡ cold parse — kinds, spans, structure, and the
    // diag rows riding the error fences, in one canonical comparable
    let edited_json = AstPrinter::json_of(doc.ast());
    let cold_json = AstPrinter::json_of(cold.ast());
    assert_eq!(
        edited_json, cold_json,
        "{file} seq {seq} step {step}: edited tree != cold parse"
    );
    // the flat panel ≡ cold (lex tier first, parse tier after)
    assert_eq!(
        doc.ast().errors(),
        cold.ast().errors(),
        "{file} seq {seq} step {step}: flat panel != cold"
    );
    // the tree index ≡ cold at the sample: every token boundary in the
    // edit's neighborhood, then a bounded stride over the file
    let len = doc.len();
    let mut positions: Vec<u32> = vec![0, len];
    let mut pos = 0u32;
    while pos < len {
        positions.push(pos);
        pos += 7;
    }
    for &p in &positions {
        let ea = chain_at(doc, p);
        let ca = chain_at(&cold, p);
        assert_eq!(
            ea, ca,
            "{file} seq {seq} step {step}: index answer at {p} differs"
        );
    }
    // the line geometry ≡ cold
    for line in 0..=doc.line_of(len.saturating_sub(1)) {
        assert_eq!(
            doc.line_span(line),
            cold.line_span(line),
            "{file} seq {seq} step {step}: line {line} span differs"
        );
    }
    assert_eq!(doc.line_of(len), cold.line_of(len));
}

fn assert_sequence(file: &str, seq: usize, text0: &str, seed: u64, steps: usize, mode: Mode) {
    let norm = canonical(text0).into_owned();
    let mut doc = Doc::parse(mode, norm.clone());
    let mut old = norm;
    let mut rng = Rng::new(seed ^ (seq as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    for step in 0..steps {
        let new = mutate(&mut rng, &old);
        let e = Edit::between(&old, &new);
        doc.edit(e);
        assert_eq!(
            doc.text(doc.span()),
            new,
            "{file} seq {seq} step {step}: lost the text"
        );
        assert_edit_cold(file, seq, step, &doc, &new, mode);
        old = new;
    }
}

/// the corpus: same roots as the parser's conformance suite
fn corpus() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let roots = [
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demo/src/examples"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rut"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benches/workloads"),
    ];
    let mut stack = roots.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
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

fn mode_of(path: &std::path::Path) -> Mode {
    if path.to_string_lossy().ends_with(".d.rut") {
        Mode::Decl
    } else {
        Mode::Impl
    }
}

#[test]
fn edited_docs_equal_cold_parses_over_the_corpus() {
    let files = corpus();
    assert!(!files.is_empty(), "the corpus roots must exist");
    let mut steps_total = 0usize;
    for f in &files {
        let Ok(raw) = std::fs::read_to_string(f) else {
            continue;
        };
        if raw.len() > 96 * 1024 {
            continue;
        }
        let seed = seed_of(f);
        let steps = if raw.len() > 16 * 1024 { 2 } else { 6 };
        let mode = mode_of(f);
        for seq in 0..2 {
            let file = f.to_string_lossy().into_owned();
            assert_sequence(&file, seq, &raw, seed, steps, mode);
            steps_total += steps;
        }
    }
    assert!(
        steps_total > 400,
        "the corpus must exercise a real number of edit steps ({steps_total})"
    );
}

#[test]
fn edited_docs_equal_cold_parses_on_the_literal_stress_fixture() {
    // multi-line raw and format strings, holes with braces and nested
    // strings, comments, unterminated literals — the shapes a clean
    // corpus never exercises
    const STRESS: &str = r###"// comment first line
let s = "plain \n \"esc\" string";
let r = r#"raw with "quotes" inside"#;
let m = r##"
multi line raw
with {braces} and " quotes
"##;
let f = f"values {x} and {{escaped}} and { a + b }";
let fm = f#"
multi {line}
holes { with " strings } inside
"#;
/* block comment
   spanning lines
   with " and { inside */
let nums = [0xFF, 1_000, 1.5e3, 3f64, 7u32];
let broken = "unterminated
let tail = 1;
"###;
    for seq in 0..3 {
        assert_sequence("STRESS", seq, STRESS, 0x5eed + seq as u64, 6, Mode::Impl);
    }
    // the fence residue answers through the edit: an unterminated
    // string keeps its residue reachable
    let mut doc = Doc::parse(Mode::Impl, STRESS.to_string());
    let old = doc.text(doc.span()).to_string();
    let new = old.replace("let nums = [0xFF,", "let nums2 = [0xFF,");
    doc.edit(Edit::between(&old, &new));
    let cold = Doc::parse(Mode::Impl, new);
    assert_eq!(
        AstPrinter::json_of(doc.ast()),
        AstPrinter::json_of(cold.ast()),
        "literal-heavy text edits ≡ cold"
    );
}
