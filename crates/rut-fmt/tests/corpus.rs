//! Corpus conformance for the formatter — the round-trip
//! invariant realized over the four corpus trees (the parser's own
//! enumerator): every corpus file formats, the formatted text REPARSES
//! to a clean tree, and formatting is IDEMPOTENT
//! (`fmt(fmt(x)) == fmt(x)`). The corpus is the conformance suite, not
//! byte-frozen: the files keep their hand-written layout; only the
//! three laws above are asserted.

use rut_ast::parser::{parse_with_recover, Mode};

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
    assert!(
        files.len() >= 50,
        "expected the full corpus, found {}",
        files.len()
    );
    let mut failures = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap();
        let mode = mode_of(f);
        // the parser's own gate first — the formatter's input law
        let base = parse_with_recover(src.clone(), mode);
        if !base.ast().is_clean() {
            failures.push(format!(
                "{}: does not parse at base ({} diags)",
                f.display(),
                base.ast().errors().len()
            ));
            continue;
        }
        let out = match rut_fmt::format(&src, mode, &rut_fmt::Style::default()) {
            Ok(o) => o,
            Err(ds) => {
                failures.push(format!(
                    "{}: fmt refused: {}",
                    f.display(),
                    ds.first().map(|d| d.msg.clone()).unwrap_or_default()
                ));
                continue;
            }
        };
        let reparsed = parse_with_recover(out.clone(), mode);
        if !reparsed.ast().is_clean() {
            failures.push(format!(
                "{}: FORMATTED text does not reparse: {}",
                f.display(),
                reparsed.ast().errors()[0].msg
            ));
            continue;
        }
        let out2 = match rut_fmt::format(&out, mode, &rut_fmt::Style::default()) {
            Ok(o) => o,
            Err(ds) => {
                failures.push(format!(
                    "{}: fmt(fmt) refused: {}",
                    f.display(),
                    ds.first().map(|d| d.msg.clone()).unwrap_or_default()
                ));
                continue;
            }
        };
        if out2 != out {
            failures.push(format!(
                "{}: NOT idempotent (fmt(fmt(x)) != fmt(x))",
                f.display()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "formatter corpus failures ({}):\n{}",
        failures.len(),
        failures.join("\n")
    );
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
                let mut it = rest.split_inclusive(')');
                it.next().is_some_and(|head| {
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

#[test]
fn corpus_dump_trees_match_through_format() {
    // the round-trip invariant itself: dump(parse(x)) == dump(parse(fmt(x)))
    // ignoring spans — the printed source parses to the SAME tree.
    // Literals need NO normalization: fmt reprints them VERBATIM as
    // span×src slices, so the dump's source-sliced spellings match
    // byte-for-byte — that byte-stability IS the point of the test.
    let files = corpus();
    let mut failures = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap();
        let mode = mode_of(f);
        let p1 = parse_with_recover(src.clone(), mode);
        let want = span_free(&rut_ast::AstPrinter::text_of(p1.ast(), p1.text(p1.span())));
        let Ok(out) = rut_fmt::format(&src, mode, &rut_fmt::Style::default()) else {
            continue; // covered by the other test's message
        };
        let p2 = parse_with_recover(out.clone(), mode);
        let got = span_free(&rut_ast::AstPrinter::text_of(p2.ast(), p2.text(p2.span())));
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
    assert!(
        failures.is_empty(),
        "round-trip failures ({}):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// the structural-interfaces fork: an `interface` and its satisfying
/// inherent `impl` round-trip — the printed source reparses clean and
/// formatting is idempotent, and the printer spells the new forms
/// (`interface Name { .. }`, `impl T { .. }` — no `for` clause).
#[test]
fn interface_and_inherent_impl_round_trip() {
    let src = "\
pub interface Shape {
    fn area(self) -> f64;
}

interface Wrap<T> {
    fn unwrap_or(self, d: T) -> T;
}

class Circle {
    r: f64;
}

impl Circle {
    pub fn area(self) -> f64 { return self.r * 3.0; }
}
";
    let p = parse_with_recover(src.to_string(), Mode::Impl);
    assert!(
        p.ast().is_clean(),
        "the fixture must parse clean: {:?}",
        p.ast().errors()
    );
    let out = rut_fmt::format(src, Mode::Impl, &rut_fmt::Style::default()).expect("fmt accepts it");
    assert!(
        out.contains("pub interface Shape {"),
        "vis prefix as spelled: {out}"
    );
    assert!(out.contains("interface Wrap<T> {"), "generics print: {out}");
    assert!(
        out.contains("impl Circle {"),
        "inherent impl prints without a for-clause: {out}"
    );
    assert!(
        !out.contains(" for "),
        "no impl-for spelling anywhere: {out}"
    );
    let q = parse_with_recover(out.clone(), Mode::Impl);
    assert!(
        q.ast().is_clean(),
        "the formatted text must reparse clean: {:?}",
        q.ast().errors()
    );
    let out2 =
        rut_fmt::format(&out, Mode::Impl, &rut_fmt::Style::default()).expect("fmt(fmt) accepts it");
    assert_eq!(out2, out, "formatting must be idempotent");
}

/// the glob leaf — the star round-trips in both spellings: the flat
/// `use pkg::*;` prints its `::*`, the brace star prints as one more
/// list member (`{ A, * }` — the tree records the star's span, not its
/// spelled slot), each reparses to the same tree, and formatting is
/// idempotent.
#[test]
fn glob_use_round_trips_with_its_star() {
    let p = |src: &str| {
        let parsed = parse_with_recover(src.to_string(), Mode::Impl);
        assert!(
            parsed.ast().is_clean(),
            "the fixture must parse clean: {:?}",
            parsed.ast().errors()
        );
        let out =
            rut_fmt::format(src, Mode::Impl, &rut_fmt::Style::default()).expect("fmt accepts it");
        let q = parse_with_recover(out.clone(), Mode::Impl);
        assert!(
            q.ast().is_clean(),
            "the formatted text must reparse clean: {:?}",
            q.ast().errors()
        );
        let out2 = rut_fmt::format(&out, Mode::Impl, &rut_fmt::Style::default())
            .expect("fmt(fmt) accepts it");
        assert_eq!(out2, out, "formatting must be idempotent: {src:?}");
        out
    };
    assert_eq!(p("use pkg::*;\n"), "use pkg::*;\n", "the flat star prints");
    assert_eq!(
        p("use pkg::a::b::*;\n"),
        "use pkg::a::b::*;\n",
        "the nested walk prints whole, star at the leaf"
    );
    assert_eq!(
        p("use pkg::{ A, * };\n"),
        "use pkg::{ A, * };\n",
        "the brace star prints as a list member"
    );
    assert_eq!(
        p("use pkg::{ *, A };\n"),
        "use pkg::{ A, * };\n",
        "the spelled slot is not the tree — the star canonicalizes last"
    );
    // the pub re-export spelling over a glob
    assert_eq!(
        p("pub use pkg::*;\n"),
        "pub use pkg::*;\n",
        "the vis prefix prints before the glob"
    );
    // the ordinary leaves keep their spellings (the guard against the
    // star leaking into them)
    assert_eq!(
        p("use pkg::{ helper, Widget };\n"),
        "use pkg::{ helper, Widget };\n",
        "the plain brace list is unchanged"
    );
    assert_eq!(
        p("use pkg::A;\n"),
        "use pkg::{ A };\n",
        "the flat single-name leaf still normalizes to braces"
    );
}

/// `pub use` — the re-export: the printer spells the vis prefix, the
/// printed text reparses clean (with the vis back on the node), and
/// formatting is idempotent. A private `use` prints bare, unchanged.
#[test]
fn pub_use_round_trips_with_its_prefix() {
    let src = "\
pub use util::{ helper, Widget };
pub(pkg) use util::{ scoped };
use util::{ hidden };
";
    let p = parse_with_recover(src.to_string(), Mode::Impl);
    assert!(
        p.ast().is_clean(),
        "the fixture must parse clean: {:?}",
        p.ast().errors()
    );
    let out = rut_fmt::format(src, Mode::Impl, &rut_fmt::Style::default()).expect("fmt accepts it");
    assert!(
        out.contains("pub use util::{ helper, Widget };"),
        "the pub prefix prints: {out}"
    );
    assert!(
        out.contains("pub(pkg) use util::{ scoped };"),
        "the scoped prefix prints: {out}"
    );
    let bare = out
        .lines()
        .find(|l| l.starts_with("use "))
        .unwrap_or_else(|| panic!("the private use prints bare: {out}"));
    assert_eq!(bare, "use util::{ hidden };", "{out}");
    let q = parse_with_recover(out.clone(), Mode::Impl);
    assert!(
        q.ast().is_clean(),
        "the formatted text must reparse clean: {:?}",
        q.ast().errors()
    );
    let out2 =
        rut_fmt::format(&out, Mode::Impl, &rut_fmt::Style::default()).expect("fmt(fmt) accepts it");
    assert_eq!(out2, out, "formatting must be idempotent");
}
