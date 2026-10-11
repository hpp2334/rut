//! the `run` subcommand: load (cache-first), compile through the host
//! chain, verify, drive.

use crate::cache;

pub(crate) fn run(path: &str, fuel: Option<u64>, symbols: Option<String>, entry: Option<String>) {
    if path.ends_with(".d.rut") {
        eprintln!("run: {path} is a declaration file (a `.d.rut` surface) — nothing to run");
        std::process::exit(2);
    }
    // the door is an EXPLICIT ALLOWLIST: a module directory (`rut.jsonc`)
    // or a packed `.rutbundle` — every program is a manifest'd dir, so
    // there is no loose-file shape to fall back to
    let p = std::path::Path::new(path);
    if !(p.is_dir() || p.extension().map_or(false, |e| e == "rutbundle")) {
        eprintln!(
            "run: {path} is not a runnable unit — give the directory a `rut.jsonc` \
             (`{{\"name\": \"…\", \"entry\": {{\"lib\": \"./<file>.rut\"}}}}` — JSONC: \
             `//` comments and trailing commas are legal), or run a packed `.rutbundle`"
        );
        std::process::exit(2);
    }
    // the symbol table is opt-in against a compiled bundle: the
    // source/dir lanes compile fresh and need no map — a
    // mismatched input is a usage error, never a silent ignore
    if symbols.is_some()
        && !(p.is_file() && p.extension().map_or(false, |e| e == "rutbundle"))
    {
        eprintln!(
            "run: --symbols restores a packed artifact's symbol table — give a compiled `.rutbundle` \
             (a source or directory input compiles fresh and needs no map)"
        );
        std::process::exit(2);
    }
    // the program plus the host rows snapshot the installs answer to —
    // a module directory (`rut.jsonc`) or a `.rutbundle` — walk the
    // graph (url deps ride the cache-first fetcher), offer every pkg,
    // compile through the chain
    let loaded = match cache::load_with_cache(p) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            cache::evict_poisoned_cache(p, &e);
            std::process::exit(2);
        }
    };
    let root = loaded.root.clone();
    // a decl host bundle: nothing to run — its root is a declaration
    // surface the embedding Rust binds, never a program
    if matches!(
        loaded.pkg(&root).map(|m| &m.body),
        Some(rut_driver::PkgBody::Host { .. })
    ) {
        eprintln!(
            "run: {path} packs the host pkg `{root}` — a host bundle carries a \
             declaration surface, nothing to run; bind its rows from the embedder \
             (`.host_pkg(..)` over the mounted rows)"
        );
        std::process::exit(2);
    }
    let mut chain = rut_driver::RutRun::new().pkgs(&loaded);
    // the symbol table restores BEFORE the graph compiles, so
    // linking and the VM both see the real names and positions
    if let Some(sym) = &symbols {
        let bytes = match std::fs::read(sym) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("run: cannot read {sym}: {e}");
                std::process::exit(2);
            }
        };
        let map = match rut_core::strip::SymbolMap::from_bytes(&bytes) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("run: {sym}: {e}");
                std::process::exit(1);
            }
        };
        for section in &map.sections {
            if loaded.pkg(&section.spec).is_none() {
                eprintln!("run: warning — the symbol table names `{}`, which this bundle does not carry", section.spec);
            }
        }
        chain = chain.symbols(&map);
    }
    // the CLI is a host: it offers calc from the toolchain tree (the
    // `Math` surface; the core prelude auto-rides in .compile()) and
    // installs the matching pkgs — the blanket-install lane (the
    // asymmetry makes it legal): math always, the logger to stdout
    // when a program uses ink, the rest merging inert unless the
    // program mounts their pkg
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
        .entrypoint(&root)
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
    let prog = match compiled.graph.program.clone() {
        Some(p) => p,
        None => {
            eprintln!("no program emitted");
            std::process::exit(1);
        }
    };
    if let Err(e) = rut_vm::verify::verify(&prog) {
        eprintln!("verify: {e}");
        std::process::exit(1);
    }
    // the invocation designation: the entry-fn set of the root module.
    // Exactly one `entry fn` runs the program; several require
    // `--entry <name>`; none is nothing to run. `main` is an ordinary
    // NAME — the compiler has no main convention to fall back to.
    let entries: Vec<String> = prog
        .exports
        .iter()
        .map(|(n, _)| prog.name_of(*n).to_string())
        .collect();
    let entry: String = match entry {
        Some(name) => {
            if !entries.iter().any(|e| *e == name) {
                match entries.as_slice() {
                    [] => eprintln!("run: no `entry fn {name}` — this program declares no entry fns at all"),
                    _ => eprintln!("run: no `entry fn {name}` — this program's entry fns: {}", entries.join(", ")),
                }
                std::process::exit(1);
            }
            name
        }
        None => match entries.as_slice() {
            [one] => one.clone(),
            [] => {
                eprintln!("no `entry fn` — nothing to run (declare one: `entry fn main() {{ .. }}`)");
                std::process::exit(1);
            }
            many => {
                eprintln!(
                    "{} entry fns ({}) — name one with `--entry <fn>`",
                    many.len(),
                    many.join(", ")
                );
                std::process::exit(1);
            }
        },
    };
    let hooks = rut_vm::interp::HostHooks::default();
    let limits = rut_vm::interp::Limits {
        fuel,
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    // the installs rode the chain (`.host_pkg(..)`); the compiled run
    // carries the joined registry — the builder door takes it whole
    let mut vm = match rut_vm::interp::Vm::builder()
        .compiled(compiled)
        .limits(limits)
        .hooks(hooks)
        .build()
    {
        Ok(vm) => vm,
        Err(e) => {
            eprintln!("boot: {}", e.msg);
            std::process::exit(1);
        }
    };
    // (the host bindings were installed against the rows snapshot at
    // `.compile()`; the registry came back on `Compiled`)
    match vm.call::<_, ()>(&entry, ()) {
        Ok(_) => {}
        Err(t) => {
            eprintln!("trap: {} — {}", t.name(), t.msg);
            std::process::exit(1);
        }
    }
    // the async driving loop: drain the ready
    // queue, advance the virtual clock to the next sleep deadline,
    // repeat — idle when no frames and no timers remain. Capped, so a
    // program that never idles fails loudly instead of hanging.
    for _ in 0..1_000_000 {
        if let Err(t) = vm.run_ready() {
            eprintln!("trap: {} — {}", t.name(), t.msg);
            std::process::exit(1);
        }
        match vm.next_deadline() {
            Some(d) => vm.set_now(d),
            None => {
                if vm.pending_tasks() == 0 {
                    break;
                }
            }
        }
    }
}
