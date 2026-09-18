//! Playground classics gate (RFC 0041 §3) — every `demo/src/examples/*.rut`
//! compiles AND runs through the full pipeline, and its output matches the
//! `.expected` sidecar next to it. The demo reads those same files raw, so
//! the playground's static-preview promises are exactly what the pipeline
//! produces — never a hand-written guess.

use std::cell::RefCell;
use std::rc::Rc;

fn examples_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo/src/examples")
}

#[test]
fn classics_run_and_match_their_expected_sidecars() {
    let dir = examples_dir();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "rut"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 12,
        "expected the classics corpus, found {} in {}",
        files.len(),
        dir.display()
    );

    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let src = std::fs::read_to_string(path).unwrap();
        let expected: Vec<String> = std::fs::read_to_string(path.with_extension("expected"))
            .unwrap_or_else(|e| panic!("{name}: cannot read sidecar: {e}"))
            .trim_end_matches('\n')
            .split('\n')
            .map(str::to_string)
            .collect();

        // the classics use the toolchain libs (`ink`+`rt`, `pouch`) —
        // third-party pkgs mounted from the tree (the driver doesn't
        // know them)
        let mut s = rut_driver::Session::new();
        rut_driver::mount_std(&mut s);
        let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        rut_driver::mount_dir(&mut s, &tree.join("rut/ink")).expect("mount ink (+rt)");
        rut_driver::mount_dir(&mut s, &tree.join("rut/pouch")).expect("mount pouch");
        let out = rut_driver::compile_module_in(&mut s, &src, rut_parser::Mode::Impl, "main");
        if !out.diags.is_empty() {
            failures.push(format!(
                "{name}: diags:\n{}",
                rut_lexer::diag::render_diags(&src, &out.diags)
            ));
            continue;
        }
        let Some(binary) = out.binary else {
            failures.push(format!("{name}: no binary emitted"));
            continue;
        };
        let prog = match rut_core::binary::decode(&binary) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!("{name}: decode: {e}"));
                continue;
            }
        };
        if let Err(e) = rut_vm::verify::verify(&prog) {
            failures.push(format!("{name}: verify: {e}"));
            continue;
        }
        let lines: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = lines.clone();
        let limits = rut_vm::interp::Limits {
            fuel: Some(5_000_000),
            heap_limit_bytes: Some(4 * 1024 * 1024),
            interrupt_every: 1024,
        };
        let mut vm = match rut_vm::interp::Vm::new(
            Rc::new(prog),
            &limits,
            rut_vm::interp::HostHooks::default(),
        ) {
            Ok(vm) => vm,
            Err(t) => {
                failures.push(format!("{name}: boot: {}", t.msg));
                continue;
            }
        };
        rut_std::logger::install_std_log(&mut vm, move |msg| {
            sink.borrow_mut().push(msg.to_string())
        });
        rut_std::math::install_std_math(&mut vm);
        if let Err(t) = vm.call("main", &[]) {
            failures.push(format!("{name}: trap: {} — {}", t.name(), t.msg));
            continue;
        }
        let got = lines.borrow().clone();
        if got != expected {
            failures.push(format!("{name}:\n  got:  {got:?}\n  want: {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
