//! Book code-block gate — every ```rut fenced block under docs/src whose
//! content contains `pub fn main` compiles AND runs to completion, driven
//! the same way `rut run` drives a loose file (the CLI is a full host:
//! mount_std plus the tree packages the block's `use` lines name, peers
//! assembled, math/logger/nmap/async host fns bound — the same set the
//! wasm host mounts, which is what the book's ▶ Run buttons ride).
//!
//! The gate is anti-rot: a book block that claims to be a whole program
//! must stay a whole program. It passes on any block the language
//! surface accepts today; blocks that are broken FOR DOCUMENTED REASONS
//! (the tutorial walking the reader through building a package, excerpts
//! that omit their setup) live in SKIP below with the reason — phase 3
//! (converting the book to runnable blocks) clears that list.

use std::cell::RefCell;
use std::rc::Rc;

/// Blocks that cannot run today, each with the reason it is left out.
/// Format: (path relative to the repo root, rut-block index (1-based),
/// reason). Clearing entries is the book lane's job, never the gate's.
const SKIP: &[(&str, u32, &str)] = &[
    // the chapter's walk-through imports the reader-built `greet`
    // package (constructed earlier in the same page) — never a mounted
    // pkg, so this block is a fragment of a larger project
    (
        "docs/src/tutorial/modules.md",
        4,
        "imports `greet`, the package the reader builds earlier in the chapter",
    ),
    // the landing page's sieve uses `Logger` without importing ink
    // (`use pouch::{Vec}` is the only use line) — a book typo to fix in
    // the docs lane
    (
        "docs/src/README.md",
        1,
        "uses `Logger` without `use ink::{Logger}`",
    ),
];

fn docs_src() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/src")
}

/// Every fenced block whose info string starts with `rut`, in document
/// order (the index the SKIP table and the per-block lines cite).
fn fenced_rut_blocks(src: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut lines = src.lines();
    while let Some(line) = lines.next() {
        let Some(info) = line.trim_start().strip_prefix("```") else {
            continue;
        };
        let is_rut = info.trim().starts_with("rut");
        let mut body = Vec::new();
        for l in lines.by_ref() {
            if l.trim_start().starts_with("```") {
                break;
            }
            body.push(l);
        }
        if is_rut {
            blocks.push(body.join("\n"));
        }
    }
    blocks
}

fn collect_markdown(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            collect_markdown(&p, out);
        } else if p.extension().is_some_and(|e| e == "md") {
            out.push(p);
        }
    }
}

#[test]
fn book_blocks_compile_and_run() {
    let mut files = Vec::new();
    collect_markdown(&docs_src(), &mut files);
    assert!(
        files.len() >= 50,
        "expected the book corpus under {}, found {} pages",
        docs_src().display(),
        files.len()
    );

    let mut seen_runnable = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        let rel = format!(
            "docs/src/{}",
            path.strip_prefix(docs_src())
                .unwrap()
                .to_string_lossy()
                .replace("\\", "/")
        );
        let src = std::fs::read_to_string(path).unwrap();
        for (n, body) in fenced_rut_blocks(&src).into_iter().enumerate() {
            let n = (n + 1) as u32;
            if !body.contains("pub fn main") {
                continue; // fragments and surfaces are not the gate's business
            }
            seen_runnable += 1;
            if let Some((_, _, why)) = SKIP.iter().find(|(p, i, _)| *p == rel && *i == n) {
                println!("{rel}#{n}: skipped — {why}");
                continue;
            }

            // the CLI's loose-file host: mount what the block names
            let mut s = rut_driver::Session::new();
            rut_driver::mount_std(&mut s);
            let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
            for (name, dir) in [
                ("rt", "rut/rt"),
                ("ink", "rut/ink"),
                ("pouch", "rut/pouch"),
                ("nmapset", "rut/nmapset"),
                ("json", "rut/json"),
                ("strbuild", "rut/strbuild"),
                ("async_engine", "rut/async_engine"),
                ("async_host", "rut/async_host"),
                ("http_host", "rut/http_host"),
                ("http", "rut/http"),
            ] {
                if body.contains(&format!("use {name}::")) {
                    rut_driver::mount_dir(&mut s, &tree.join(dir))
                        .unwrap_or_else(|e| panic!("{rel}#{n}: mount {name}: {e}"));
                }
            }
            rut_driver::assemble_peers(&mut s)
                .unwrap_or_else(|e| panic!("{rel}#{n}: assemble peers: {e}"));

            let out = rut_driver::compile_module_in(&mut s, &body, rut_parser::Mode::Impl, "main");
            if !out.diags.is_empty() {
                failures.push(format!(
                    "{rel}#{n}: diags:\n{}",
                    rut_lexer::diag::render_diags(&body, &out.diags)
                ));
                continue;
            }
            let Some(binary) = out.binary else {
                failures.push(format!("{rel}#{n}: no binary emitted"));
                continue;
            };
            let prog = match rut_core::binary::decode(&binary) {
                Ok(p) => p,
                Err(e) => {
                    failures.push(format!("{rel}#{n}: decode: {e}"));
                    continue;
                }
            };
            if let Err(e) = rut_vm::verify::verify(&prog) {
                failures.push(format!("{rel}#{n}: verify: {e}"));
                continue;
            }

            // the budgets the playground test pins (5M fuel / 4 MiB heap)
            let lines: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
            let sink = lines.clone();
            let limits = rut_vm::interp::Limits {
                fuel: Some(5_000_000),
                heap_limit_bytes: Some(4 * 1024 * 1024),
                interrupt_every: 1024,
            };
            // the bindings BEFORE the Vm (RFC 0025): the same set the wasm
            // host's rut_run installs — the buttons' answers are the truth
            let mut hosts = rut_vm::interp::HostRegistry::new();
            rut_std::logger::install_std_log(&mut hosts, move |msg| {
                sink.borrow_mut().push(msg.to_string())
            });
            rut_std::math::install_std_math(&mut hosts);
            rut_std::nmap::install_std_nmap(&mut hosts);
            rut_std::async_host::install_std_async(&mut hosts);
            let mut vm = match rut_vm::interp::Vm::new(
                Rc::new(prog),
                &limits,
                rut_vm::interp::HostHooks::default(),
                hosts,
            ) {
                Ok(vm) => vm,
                Err(t) => {
                    failures.push(format!("{rel}#{n}: boot: {}", t.msg));
                    continue;
                }
            };
            if let Err(t) = vm.call::<_, ()>("main", ()) {
                failures.push(format!("{rel}#{n}: trap: {} — {}", t.name(), t.msg));
                continue;
            }
            // the async driving loop (the wasm host's own): drain ready
            // tasks and advance the virtual clock — idle without launches
            if !vm.is_running() {
                for _ in 0..1_000_000 {
                    if vm.run_ready().is_err() || vm.next_deadline().is_none() {
                        if vm.pending_tasks() == 0 {
                            break;
                        }
                    }
                }
            }
            println!("{rel}#{n}: ok ({} lines)", lines.borrow().len());
        }
    }
    assert!(
        seen_runnable >= 10,
        "expected the runnable-book corpus, found {seen_runnable} qualifying blocks"
    );
    assert!(
        failures.is_empty(),
        "{} of the book's runnable blocks failed:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
