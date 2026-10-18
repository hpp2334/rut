//! the host chain: the walk's offer, the compile, the verify.

/// Compile the walked world through the host chain. The CLI is a
/// host: it installs the matching pkgs blanket-side (the logger to
/// stdout when a program uses ink, math for a mounted calc, the rest
/// merging inert unless the program mounts their pkg) — but it offers
/// NOTHING ambient: the walk mounts exactly the graph the root's
/// manifest declares (`deps` + `dev-deps` at the program root), and a
/// `use` head with no declared row is the closure check's loud
/// unknown-pkg error. `core` is the one unconditional head (the
/// prelude auto-rides in .compile()).
///
/// Every walked pkg offers through the ONE verb, `pkg(..)` — first
/// offer wins, the walk's law. Symbol restore is not here: it is a
/// property of the decoded artifact (`bundle.restore_symbols(..)`),
/// applied before the offer, not a run step.
pub(crate) fn compile(loaded: &rut_driver::Walk) -> rut_driver::Compiled {
    let mut chain = loaded.pkgs.iter().fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()));
    let compiled = match chain
        .pkg(rut_std::math::pkg())
        // the nmap experiment's native key table (the mapset-host plan) — a
        // program only reaches it when it declares `use nmap_host::{...}` or a
        // pkg that does (`nmapset`)
        .pkg(rut_std::nmap::pkg())
        // the crossing-tax benchmark's nops (the crossing-fastpath plan,
        // phase 0) — reached only by a program that declares
        // `use bench_cross::{...}` (the bench row)
        .pkg(rut_std::bench_cross::pkg())
        // the async host set: launch/abort/sleep bodies for the
        // `async_host` rows — reached only by a program that mounts the
        // async packages (a `use futures::` pulls the tree pkg)
        .pkg(rut_std::async_host::pkg())
        // the string builder's bodies (the host strbuild pkg): reached only
        // by a program that mounts the strbuild pkg (a `use strbuild::` /
        // `use json::` pulls it — json's writer rides the builder)
        .pkg(rut_std::strbuild::pkg())
        // the std HTTP lane (the rut/http plan): get + the Response
        // readbacks — reached only by a program that mounts the http
        // packages (a `use http::` / `use http_host::` pulls the tree
        // pkgs; reqwest is the CLI's, native-only)
        .pkg(rut_std::http::pkg())
        .pkg(rut_std::logger::pkg(|s| println!("{s}")))
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
        eprint!(
            "{}",
            rut_lexer::diag::render_diags_grouped(&compiled.graph.diags, &loaded_sources(loaded), None)
        );
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

/// The diag renderer's file → text map over a loaded world: a pkg's
/// body text keyed by its spec, each mounted mod child keyed by
/// `spec/path` — the exact labels the graph compiler stamps on
/// attributed diags. Walked pkgs carry no filesystem paths, so the
/// module NAME is the label there is.
fn loaded_sources(
    loaded: &rut_driver::Walk,
) -> std::collections::BTreeMap<String, &str> {
    let mut out = std::collections::BTreeMap::new();
    for pkg in &loaded.pkgs {
        if let rut_driver::MountedBody::Source { text, .. } = &pkg.body {
            out.insert(pkg.spec.clone(), text.as_str());
        }
        for m in pkg.mods.values() {
            out.insert(format!("{}/{}", pkg.spec, m.path), m.text.as_str());
        }
    }
    out
}
