//! Corpus conformance for the formatter — the round-trip
//! invariant realized over the four corpus trees (the parser's own
//! enumerator): every corpus file formats, the formatted text REPARSES
//! to a clean tree, and formatting is IDEMPOTENT
//! (`fmt(fmt(x)) == fmt(x)`). The corpus is the conformance suite, not
//! byte-frozen: the files keep their hand-written layout; only the
//! three laws above are asserted.

mod common;

use common::span_free;
use rut_ast::parser::{parse_with_recover, Mode};
use rut_ast::ItemRef;

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

/// The leaf law — a `*` on a use line is refused (the parser's
/// `STAR_MSG`); the ordinary leaves keep their spellings, and a clean
/// leaf-less group round-trips to a tree that is still leaf-less.
#[test]
fn use_leaves_round_trip_and_a_star_is_refused() {
    // the refusal: the same sentence, one diag, from the parse the
    // formatter refuses to guess at
    let err = rut_fmt::format("use pkg::*;\n", Mode::Impl, &rut_fmt::Style::default())
        .expect_err("a star is refused");
    assert_eq!(err.len(), 1, "exactly one diag: {err:?}");
    assert_eq!(
        err[0].msg,
        "there is no `*` in a `use` — a `use` names its leaves (`use pkg::{ A, B }`)"
    );
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
    // the ordinary leaves keep their spellings
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
    // the empty group: a leaf-less use at a walked depth prints a group
    // that reparses to the same leaf-less tree (the round-trip is the
    // assertion — not the spacing)
    let out = p("use pkg::a::{ };\n");
    let q = parse_with_recover(out.clone(), Mode::Impl);
    let ast = q.ast();
    let items = ast.root().items(ast);
    let Some(ItemRef::Use(h)) = items.iter().find(|i| matches!(i, ItemRef::Use(_))) else {
        panic!("the printed text reparses to a use: {out:?} / {items:?}")
    };
    let u = h.get(ast);
    let segs: Vec<&str> = u.path.iter().map(|s| ast.name(s.ident)).collect();
    assert_eq!(segs, vec!["pkg", "a"], "the path survives the round trip");
    assert!(u.names.is_empty(), "no leaf reappears: {out:?}");
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
