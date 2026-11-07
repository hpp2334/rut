//! Corpus conformance — the runnable example projects
//! (`examples/**`), the playground classics (`demo/src/examples/`),
//! the stdlib (`rut/**`), and the bench workloads: every `*.rut` in
//! the repo parses with zero diags (declaration mode for `*.d.rut`).
//! Together they are the parser's conformance suite.

mod common;

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
fn corpus_parses_clean() {
    let files = corpus();
    assert!(!files.is_empty(), "the corpus roots must exist");
    let mut failures = Vec::new();
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else {
            continue;
        };
        let mode = if f.to_string_lossy().ends_with(".d.rut") {
            rut_parser::Mode::Decl
        } else {
            rut_parser::Mode::Impl
        };
        let (ast, diags) = common::parse(&src, mode);
        if !diags.is_empty() {
            let first = &diags[0];
            failures.push(format!("{}:{}: {}", f.display(), first.span.lo, first.msg));
            continue;
        }
        // the tree itself must be sound: every item slot resolves
        let _ = ast.root().items(&ast).len();
    }
    assert!(
        failures.is_empty(),
        "{} of {} corpus files failed:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}
