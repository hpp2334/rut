//! the host chain: the std pkg offers + installs, the symbol mount,
//! the compile, the verify.

use crate::run::symbols;

/// Compile the loaded graph through the host chain. The CLI is a
/// host: it offers calc from the toolchain tree (the `Math` surface;
/// the core prelude auto-rides in .compile()) and installs the
/// matching pkgs — the blanket-install lane (the asymmetry makes it
/// legal): math always, the logger to stdout when a program uses ink,
/// the rest merging inert unless the program mounts their pkg.
pub(crate) fn compile(loaded: &rut_driver::Loaded, symbols: Option<&str>) -> rut_driver::Compiled {
    let mut chain = rut_driver::RutRun::new().pkgs(loaded);
    // the symbol table restores BEFORE the graph compiles, so
    // linking and the VM both see the real names and positions
    if let Some(map) = symbols::restore(loaded, symbols) {
        chain = chain.symbols(&map);
    }
    let compiled = match chain
        .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
        .host_pkg(rut_std::math::pkg())
        // the nmap experiment's native key table (the mapset-host plan) — a
        // program only reaches it when it declares `use nmap_host::{...}` or a
        // pkg that does (`nmapset`)
        .host_pkg(rut_std::nmap::pkg())
        // the crossing-tax benchmark's nops (the crossing-fastpath plan,
        // phase 0) — reached only by a program that declares
        // `use bench_cross::{...}` (the bench row)
        .host_pkg(rut_std::bench_cross::pkg())
        // the async host set: launch/abort/sleep bodies for the
        // `async_host` rows — reached only by a program that mounts the
        // async packages (a `use futures::` pulls the tree pkg)
        .host_pkg(rut_std::async_host::pkg())
        // the string builder's bodies (the host strbuild pkg): reached only
        // by a program that mounts the strbuild pkg (a `use strbuild::` /
        // `use json::` pulls it — json's writer rides the builder)
        .host_pkg(rut_std::strbuild::pkg())
        // the std HTTP lane (the rut/http plan): get + the Response
        // readbacks — reached only by a program that mounts the http
        // packages (a `use http::` / `use http_host::` pulls the tree
        // pkgs; reqwest is the CLI's, native-only)
        .host_pkg(rut_std::http::pkg())
        .host_pkg(rut_std::logger::pkg(|s| println!("{s}")))
        .entrypoint(&loaded.root)
        .compile()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    if !compiled.graph.diags.is_empty() {
        for d in &compiled.graph.diags {
            eprintln!("{}", d.msg);
        }
        std::process::exit(1);
    }
    if compiled.graph.program.is_none() {
        eprintln!("no program emitted");
        std::process::exit(1);
    }
    if let Err(e) = rut_vm::verify::verify(compiled.graph.program.as_ref().unwrap()) {
        eprintln!("verify: {e}");
        std::process::exit(1);
    }
    compiled
}
