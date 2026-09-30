//! Corpus conformance for the formatter — the round-trip
//! invariant realized over the four corpus trees (the parser's own
//! enumerator): every corpus file formats, the formatted text REPARSES
//! to a clean tree, and formatting is IDEMPOTENT
//! (`fmt(fmt(x)) == fmt(x)`). The corpus is the conformance suite, not
//! byte-frozen: the files keep their hand-written layout; only the
//! three laws above are asserted.

use rut_parser::{parse, Mode};

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

fn mode_of(f: &std::path::Path) -> Mode {
    if f.file_name().unwrap().to_str().unwrap().ends_with(".d.rut") {
        Mode::Decl
    } else {
        Mode::Impl
    }
}

#[test]
fn corpus_formats_reparse_clean_and_are_idempotent() {
    let files = corpus();
    assert!(files.len() >= 50, "expected the full corpus, found {}", files.len());
    let mut failures = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap();
        let mode = mode_of(f);
        // the parser's own gate first — the formatter's input law
        let (_, diags) = parse(&src, mode);
        if !diags.is_empty() {
            failures.push(format!("{}: does not parse at base ({} diags)", f.display(), diags.len()));
            continue;
        }
        let out = match rut_fmt::format(&src, mode, &rut_fmt::Style::default()) {
            Ok(o) => o,
            Err(ds) => {
                failures.push(format!("{}: fmt refused: {}", f.display(), ds.first().cloned().unwrap_or_default()));
                continue;
            }
        };
        let (_, diags2) = parse(&out, mode);
        if !diags2.is_empty() {
            failures.push(format!(
                "{}: FORMATTED text does not reparse: {}",
                f.display(),
                diags2[0].msg
            ));
            continue;
        }
        let out2 = match rut_fmt::format(&out, mode, &rut_fmt::Style::default()) {
            Ok(o) => o,
            Err(ds) => {
                failures.push(format!("{}: fmt(fmt) refused: {}", f.display(), ds.first().cloned().unwrap_or_default()));
                continue;
            }
        };
        if out2 != out {
            failures.push(format!("{}: NOT idempotent (fmt(fmt(x)) != fmt(x))", f.display()));
        }
    }
    assert!(failures.is_empty(), "formatter corpus failures ({}):\n{}", failures.len(), failures.join("\n"));
}

/// blank the `[lo, hi)` spans out of a rendered dump — the formatted
/// text is legitimately a different LENGTH (whitespace collapses,
/// comments move), so the round-trip compares SHAPE, not spans
fn span_free(dump: &str) -> String {
    let mut out = String::with_capacity(dump.len());
    let mut chars = dump.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '[' {
            // a span opens: `[digits, digits)` — blank it
            let rest = &dump[i + 1..];
            let is_span = {
                let mut it = rest.split_inclusive(|ch: char| ch == ')');
                it.next().map_or(false, |head| {
                    let inner = head.trim_end_matches(')');
                    let mut parts = inner.splitn(2, ',');
                    match (parts.next(), parts.next()) {
                        (Some(a), Some(b)) => {
                            !a.is_empty()
                                && !b.is_empty()
                                && a.trim().chars().all(|c| c.is_ascii_digit())
                                && b.trim().chars().all(|c| c.is_ascii_digit())
                        }
                        _ => false,
                    }
                })
            };
            if is_span {
                out.push_str("[..)");
                for (_, c) in chars.by_ref() {
                    if c == ')' {
                        break;
                    }
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// canonicalize the `Lit` leaf spellings in a rendered dump — the dump
/// slices literal text from the SOURCE, and the formatter legitimately
/// re-spells literals (hex → decimal is the same u64; `0.10` → `0.1`
/// the same f64; `3.14159265f32` → `3.1415927f32` the same f32 bits;
/// `\\'` → `'` the same char). Values, not spellings, are the tree.
fn norm_lits(dump: &str) -> String {
    let mut out = String::with_capacity(dump.len());
    for line in dump.lines() {
        // the leaf spells `@ID Lit <text>` — canonicalize <text>
        if let Some(k) = line.find(" Lit ") {
            let (head, rest) = line.split_at(k + " Lit ".len());
            let text = rest.trim_end();
            if !text.is_empty() && !text.starts_with('@') {
                out.push_str(head);
                out.push_str(&norm_one_lit(text));
                out.push('\n');
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn norm_one_lit(text: &str) -> String {
    // split a trailing ` [..)` back off
    let (body, tail) = match text.find(" [..)") {
        Some(i) => (&text[..i], &text[i..]),
        None => (text, ""),
    };
    // string literal: collapse the optional backslash-apostrophe spelling
    if body.starts_with('"') || body.starts_with("f\"") {
        return format!("{}{}", body.replace("\\'", "'"), tail);
    }
    // a KNOWN numeric suffix at the end only (hex digits carry u/i/f!)
    const SUFS: [&str; 10] =
        ["u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "f32", "f64"];
    let mut digits = body;
    let mut sfx = "";
    for cand in SUFS {
        if let Some(d) = body.strip_suffix(cand) {
            digits = d;
            sfx = cand;
            break;
        }
    }
    // radix ints FIRST (hex digits legitimately contain e/f)
    for (p, r) in [("0x", 16u32), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(d) = digits.strip_prefix(p) {
            if let Ok(v) = u64::from_str_radix(d, r) {
                return format!("{}{}{}", v, sfx, tail);
            }
        }
    }
    // floats: canonicalize to the shortest digits (with a forced `.0`
    // when integral — a bare `9f32` would re-lex as a suffixed int)
    if digits.contains('.') || sfx == "f32" || sfx == "f64" {
        let canon = if sfx == "f32" {
            with_dot(format!("{}", digits.parse::<f32>().unwrap_or(0.0)))
        } else {
            with_dot(format!("{}", digits.parse::<f64>().unwrap_or(0.0)))
        };
        return format!("{}{}{}", canon, sfx, tail);
    }
    format!("{}{}{}", digits, sfx, tail)
}

fn with_dot(mut s: String) -> String {
    if !s.contains('.') && !s.contains('e') && !s.contains("inf") && !s.contains("NaN") {
        s.push_str(".0");
    }
    s
}

#[test]
fn corpus_dump_trees_match_through_format() {
    // the round-trip invariant itself: dump(parse(x)) == dump(parse(fmt(x)))
    // ignoring spans — the printed source parses to the SAME tree.
    let files = corpus();
    let mut failures = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap();
        let mode = mode_of(f);
        let (a1, _) = parse(&src, mode);
        let want = norm_lits(&span_free(&rut_ast::dump::render_text(&rut_ast::dump::to_dump_tree(&a1), &src)));
        let Ok(out) = rut_fmt::format(&src, mode, &rut_fmt::Style::default()) else {
            continue; // covered by the other test's message
        };
        let (a2, _) = parse(&out, mode);
        let got = norm_lits(&span_free(&rut_ast::dump::render_text(&rut_ast::dump::to_dump_tree(&a2), &out)));
        if got != want {
            // find the first differing line for a readable failure
            let w: Vec<&str> = want.lines().collect();
            let g: Vec<&str> = got.lines().collect();
            let mut first = String::new();
            for i in 0..w.len().max(g.len()) {
                let a = w.get(i).copied().unwrap_or("<eof>");
                let b = g.get(i).copied().unwrap_or("<eof>");
                if a != b {
                    first = format!("line {}: `{}` != `{}`", i, a, b);
                    break;
                }
            }
            failures.push(format!("{}: dump trees differ: {}", f.display(), first));
        }
    }
    assert!(failures.is_empty(), "round-trip failures ({}):\n{}", failures.len(), failures.join("\n"));
}
