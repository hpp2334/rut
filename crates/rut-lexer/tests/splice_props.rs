//! The splice ≡ cold law (law 8's token stage) — over RANDOM edit
//! sequences on the corpus fixtures (the runnable example projects,
//! the playground classics, the stdlib, the bench workloads — the
//! parser's conformance set) plus a literal-heavy stress fixture.
//!
//! Every step: a random text mutation, `Edit::between` (the diff
//! helper's own law pinned as a side effect), `Source::edit`, then
//! `splice_tokens` — and the spliced token stream and line table must
//! equal a cold `lex_normalized` over the edited text, exactly. The
//! PRNG is deterministic (law 5: the seed is a hash of the file's
//! BYTES, never its path, and every checkout runs the same fixed 64
//! sequences per file), so a failure reproduces bit-for-bit, on
//! every path the tree lives at.

use rut_lexer::lexer::{canonical, lex_normalized};
use rut_lexer::source::{Edit, Source};
use rut_lexer::splice::{splice_tokens, TokenChange};

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

/// law 5: FNV-1a over the file's bytes — the seed never sees the path
fn seed_of(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

// ── the mutation generator ──

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

/// fragments that stress the lexer's hard corners: literal delimiters,
/// hole braces, comments, newlines (inside and outside literals)
const FRAGMENTS: &[&str] = &[
    "", " ", "\n", "  \n", "let", "fn", "{", "}", "\"", "\"s\"", "\\n", "r#\"", "\"#", "f\"",
    "{x}", "//", "/*", "*/", "*/\n", "123", "0xFF", "1.5e3", "u32", "->", "::", "é", "$", "r\"",
    "f#\"", "#", "\"\n", "{\n", "\u{feff}",
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

fn assert_splice_cold(
    file: &str,
    seq: usize,
    text0: &str,
    seed: u64,
    steps: usize,
    suffix_survivor: &mut usize,
) {
    let norm = canonical(text0).into_owned();
    let (toks0, _, _, lines0) = lex_normalized(&norm, false);
    let mut src = Source::new(norm, lines0);
    let mut toks = toks0;
    let mut rng = Rng::new(seed ^ (seq as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    for step in 0..steps {
        let old = src.text().to_string();
        let new = mutate(&mut rng, &old);
        let e = Edit::between(&old, &new);
        // the diff helper's own law: the edit reproduces `new` exactly
        let mut check = old.clone();
        check.replace_range(e.start as usize..e.old_end as usize, &e.text);
        assert_eq!(
            check, new,
            "{file} seq {seq} step {step}: Edit::between lost the text"
        );
        src.edit(&e);
        let change: TokenChange = splice_tokens(&mut toks, &src, &e);
        // ≡ cold: tokens (kinds, lexemes, spans, holes) and line table
        let (cold, _, _, cold_lines) = lex_normalized(src.text(), false);
        let at = toks.iter().zip(cold.iter()).position(|(a, b)| a != b);
        assert!(
            toks == cold,
            "{file} seq {seq} step {step}: spliced tokens != cold lex\n\
             edit {}..{} (+{} bytes) over {} bytes; {} spliced vs {} cold; \
             first divergence at token {:?}",
            e.start,
            e.old_end,
            e.text.len(),
            src.text().len(),
            toks.len(),
            cold.len(),
            at,
        );
        assert_eq!(
            src.line_starts(),
            cold_lines.as_slice(),
            "{file} seq {seq} step {step}: spliced line table != cold recount"
        );
        // the window lane must be doing the work most of the time: a
        // surviving suffix means the splice kept the old stream's tail
        // instead of re-lexing everything
        if change.end < toks.len() {
            *suffix_survivor += 1;
        }
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

#[test]
fn spliced_tokens_equal_cold_lex_over_the_corpus() {
    // law 5: equal bytes under two names give equal seeds
    assert_eq!(seed_of(b"let a = 1;\n"), seed_of(b"let a = 1;\n"));
    let files = corpus();
    assert!(!files.is_empty(), "the corpus roots must exist");
    let mut suffix_survivors = 0usize;
    let mut steps_total = 0usize;
    for f in &files {
        let Ok(raw) = std::fs::read_to_string(f) else {
            continue;
        };
        if raw.len() > 128 * 1024 {
            continue;
        }
        let seed = seed_of(raw.as_bytes());
        let steps = if raw.len() > 16 * 1024 { 3 } else { 8 };
        for seq in 0..64 {
            let file = f.file_name().unwrap().to_string_lossy().into_owned();
            assert_splice_cold(&file, seq, &raw, seed, steps, &mut suffix_survivors);
            steps_total += steps;
        }
    }
    assert!(
        steps_total > 500,
        "the corpus must exercise a real number of edit steps ({steps_total})"
    );
    assert!(
        suffix_survivors * 2 > steps_total,
        "the window lane must carry most edits without a full relex \
         ({suffix_survivors}/{steps_total})"
    );
}

#[test]
fn spliced_tokens_equal_cold_lex_on_the_literal_stress_fixture() {
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
    let mut suffix_survivors = 0usize;
    for seq in 0..4 {
        assert_splice_cold(
            "STRESS",
            seq,
            STRESS,
            0x5eed_5eed_5eed,
            40,
            &mut suffix_survivors,
        );
    }
    assert!(suffix_survivors > 0, "the window lane must run here too");
}

#[test]
fn a_bom_led_fixture_splices_equal_cold() {
    // the BOM skip is position-0 context: an edit at the very start
    // relexes through it
    const BOM: &str = "\u{feff}let s = f\"{a} + {b}\";\nlet r = r#\"\nraw\n\"#;\n";
    let mut suffix_survivors = 0usize;
    assert_splice_cold("BOM", 0, BOM, 0xb0b0_b0b0, 40, &mut suffix_survivors);
}
