//! Playground classics gate — every `demo/src/examples/*/main.rut`
//! compiles AND runs clean through the full pipeline: the demo reads the
//! same sources raw, so what the page offers to run is exactly what the
//! native engine accepts and executes — never a hand-written guess.

use std::rc::Rc;

fn examples_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo/src/examples")
}

#[test]
fn classics_run_clean() {
    let dir = examples_dir();
    // the dir-shaped classics: one module dir (rut.jsonc + main.rut) per
    // name — the snippet identity is the DIRECTORY name
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|d| d.join("main.rut"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 12,
        "the classics corpus is missing — found {} files in {}",
        files.len(),
        dir.display()
    );

    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        let name = path
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let src = std::fs::read_to_string(path).unwrap();

        // the classics use the toolchain libs (`ink`+`ink_host`, `pouch`) —
        // third-party pkgs mounted from the tree (the driver doesn't
        // know them); `nmapset` is the map lane (survey D6: it pulls
        // `nmap_host` through its `[deps]`)
        let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        // the classics use the toolchain libs offered from the tree;
        // calc rides beside; the core prelude auto-offers
        let mut chain = rut_driver::RutRun::new()
            .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
            .pkg(rut_driver::Pkg::source("main", &src));
        for d in ["rut/ink", "rut/pouch", "rut/nmapset"] {
            let pkgs = rut_native::dir_pkgs(&tree.join(d))
                .unwrap_or_else(|e| panic!("mount {d}: {e}"));
            chain = chain.pkgs(&pkgs);
        }
        let out = match chain.entrypoint("main").compile() {
            Ok(c) => c,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        if !out.graph.diags.is_empty() {
            failures.push(format!(
                "{name}: diags:\n{}",
                rut_lexer::diag::render_diags(&src, &out.graph.diags)
            ));
            continue;
        }
        let Some(prog) = out.graph.program else {
            failures.push(format!("{name}: no binary emitted"));
            continue;
        };
        if let Err(e) = rut_vm::verify::verify(&prog) {
            failures.push(format!("{name}: verify: {e}"));
            continue;
        }
        let limits = rut_vm::interp::Limits {
            fuel: Some(5_000_000),
            heap_limit_bytes: Some(4 * 1024 * 1024),
            interrupt_every: 1024,
        };
        // the bindings BEFORE the Vm: the compiled program
        // carries calc's and ink_host's thunks (mount = declare =
        // install); the nmap pkg rides like the CLI's — reached only
        // by a program that declares the nmap lane. The log sink
        // discards: the run's truth here is the trap channel, not the
        // bytes.
        // the installs against the offered world's rows snapshot
        let mut world = Vec::new();
        for d in ["rut/ink", "rut/pouch", "rut/nmapset"] {
            world.extend(
                rut_native::dir_pkgs(&tree.join(d))
                    .unwrap_or_else(|e| panic!("mount {d}: {e}"))
                    .pkgs,
            );
        }
        world.push(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"));
        let ctx = rut_driver::host_pkg_ctx(&world);
        let mut hosts = rut_vm::interp::HostRegistry::new();
        hosts.install_host_pkg(&ctx, rut_std::logger::pkg(|_| {}));
        hosts.install_host_pkg(&ctx, rut_std::math::pkg());
        hosts.install_host_pkg(&ctx, rut_std::nmap::pkg());
        let mut vm = match rut_vm::interp:: Vm::builder().program(Rc::new(prog)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(hosts).build() {
            Ok(vm) => vm,
            Err(t) => {
                failures.push(format!("{name}: boot: {}", t.msg));
                continue;
            }
        };
        if let Err(t) = vm.call::<_, ()>("main", ()) {
            failures.push(format!("{name}: trap: {} — {}", t.name(), t.msg));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
