//! the `rut` binary — run / pack / fetch.

mod fetch;

use fetch::CacheFetch;
use std::future::Future;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(cmd) = args.get(1).map(|s| s.as_str()) else {
        usage();
        return;
    };
    match cmd {
        "run" => {
            let Some(path) = args.get(2) else {
                eprintln!("run: missing <dir | mod.rutbundle>");
                std::process::exit(2);
            };
            // fuel is opt-in and LOUD: absent → uncapped; a missing or
            // unparsable value is a usage error — no silent default
            let fuel: Option<u64> = match args.iter().position(|a| a == "--fuel") {
                None => None,
                Some(i) => match args.get(i + 1) {
                    Some(v) => match v.parse() {
                        Ok(n) => Some(n),
                        Err(_) => {
                            eprintln!("run: --fuel needs a number of ops (got `{v}`)");
                            std::process::exit(2);
                        }
                    },
                    None => {
                        eprintln!("run: --fuel needs a number of ops (got nothing)");
                        std::process::exit(2);
                    }
                },
            };
            // the symbol table is opt-in against a compiled bundle: the
            // source/dir lanes compile fresh and need no map — a
            // mismatched input is a usage error, never a silent ignore
            let symbols = match args.iter().position(|a| a == "--symbols") {
                None => None,
                Some(i) => match args.get(i + 1) {
                    Some(v) => Some(v.clone()),
                    None => {
                        eprintln!("run: --symbols needs a path to a `.rutsym` symbol table (got nothing)");
                        std::process::exit(2);
                    }
                },
            };
            run(path, fuel, symbols);
        }
        "fmt" => {
            let Some(path) = args.get(2) else {
                eprintln!("fmt: missing <file.rut | dir>");
                std::process::exit(2);
            };
            let check = args.iter().skip(2).any(|a| a == "--check");
            fmt(path, check);
        }
        "pack" => {
            let Some(dir) = args.get(2) else {
                eprintln!("pack: missing <dir>");
                std::process::exit(2);
            };
            pack(dir, arg_flag(&args, "-o").as_deref(), args.iter().any(|a| a == "--strip"));
        }
        "fetch" => {
            let Some(dir) = args.get(2) else {
                eprintln!("fetch: missing <dir>");
                std::process::exit(2);
            };
            fetch_cmd(dir);
        }
        "dump" => {
            let Some(path) = args.get(2) else {
                eprintln!("dump: missing <file.rut>");
                std::process::exit(2);
            };
            dump(path);
        }
        _ => usage(),
    }
}

fn arg_flag(args: &[String], name: &str) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == name {
            return it.next().cloned();
        }
    }
    None
}

fn usage() {
    eprintln!(
        "rut — run <dir | mod.rutbundle> [--fuel N] [--symbols <file.rutsym>] | fmt <file.rut | dir> [--check] | pack <dir> [-o out.rutbundle] [--strip] | fetch <dir> | dump <file.rut>"
    );
}

/// The block_on seam for the `_with` lanes: one tokio runtime, one
/// thread's worth of waiting — the CLI's own boundary, where a
/// `!Send`-tolerant contract meets a plain synchronous binary.
fn block_on<F: Future>(fut: F) -> Result<F::Output, String> {
    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("tokio runtime: {e}"))?;
    Ok(rt.block_on(fut))
}

/// The eviction law: a driver pin error names dep + url; the cache
/// entry for that url is poisoned by definition (the pin is checked at
/// the mount door on every load, so a bad entry reloads bad forever) —
/// delete it, and the next run re-fetches. Idempotent: any other error
/// evicts nothing.
fn evict_poisoned_cache(err: &str) {
    let Some(rest) = err.split("sha256 pin mismatch for ").nth(1) else {
        return;
    };
    let Some(url) = rest.split(": ").next() else {
        return;
    };
    let Ok(fetch) = CacheFetch::new() else {
        return;
    };
    let path = fetch.cache_path(url);
    if std::fs::remove_file(&path).is_ok() {
        eprintln!(
            "evicted the poisoned cache entry for {url} ({}) — it re-fetches on the next run",
            path.display()
        );
    }
}

/// The fetched load lane for `run`/`fetch`: a module dir or a
/// `.rutbundle`, with url deps served by the cache-first fetcher.
fn load_with_cache(path: &std::path::Path) -> Result<(rut_driver::Session, String), String> {
    let fetch = CacheFetch::new()?;
    block_on(rut_driver::load_path_session_with(path, &fetch))?
}

fn load(path: &str) -> String {
    // one file is one module unit — there is no include form (
    // use paths are inter-module), so loading is a plain read
    match rut_driver::load_module_source(std::path::Path::new(path)) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(2);
        }
    }
}

/// `.d.rut` parses in declaration mode — a surface, not a
/// runnable module.
fn mode_of(path: &str) -> rut_parser::Mode {
    if path.ends_with(".d.rut") {
        rut_parser::Mode::Decl
    } else {
        rut_parser::Mode::Impl
    }
}

fn run(path: &str, fuel: Option<u64>, symbols: Option<String>) {
    if path.ends_with(".d.rut") {
        eprintln!("run: {path} is a declaration file (a `.d.rut` surface) — nothing to run");
        std::process::exit(2);
    }
    // the door is an EXPLICIT ALLOWLIST: a module directory (`rut.toml`)
    // or a packed `.rutbundle` — every program is a manifest'd dir, so
    // there is no loose-file shape to fall back to
    let p = std::path::Path::new(path);
    if !(p.is_dir() || p.extension().map_or(false, |e| e == "rutbundle")) {
        eprintln!(
            "run: {path} is not a runnable unit — give the directory a `rut.toml` \
             (`name = \"…\"` + `entry.lib = \"./<file>.rut\"`), or run a packed `.rutbundle`"
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
    // the program plus the mount snapshot the host installs against —
    // the ctx is OWNED (the session may die here; the installs below
    // answer to the snapshot). A module directory (`rut.toml`) or a
    // `.rutbundle` — load the graph (url deps ride the cache-first
    // fetcher), mount std, compile, link
    let (mut session, root) = match load_with_cache(p) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            evict_poisoned_cache(&e);
            std::process::exit(2);
        }
    };
    // a v6 host bundle: nothing to run — its root is a declaration
    // surface the embedding Rust binds, never a program
    if matches!(
        session.resolve(&root).map(|m| &m.body),
        Ok(rut_driver::ModuleBody::Host { .. })
    ) {
        eprintln!(
            "run: {path} packs the host pkg `{root}` — a host bundle carries a \
             declaration surface, nothing to run; bind its rows from the embedder \
             (`install_host_pkg` over the mounted rows)"
        );
        std::process::exit(2);
    }
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
        for spec in rut_driver::apply_symbols_to_session(&mut session, &map) {
            eprintln!("run: warning — the symbol table names `{spec}`, which this bundle does not carry");
        }
    }
    rut_driver::mount_std(&mut session);
    let g = rut_driver::compile_graph(&session, &root);
    if !g.diags.is_empty() {
        for d in &g.diags {
            eprintln!("{}", d.msg);
        }
        std::process::exit(1);
    }
    let ctx = session.host_pkg_context();
    let prog = match g.program {
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
    let hooks = rut_vm::interp::HostHooks::default();
    let limits = rut_vm::interp::Limits {
        fuel,
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    // the CLI is a host: it mounts core+calc (mount_std) and installs
    // the matching pkgs — the blanket-install lane (the asymmetry makes
    // it legal): math always, the logger to stdout when a program uses
    // ink, the rest merging inert unless the program mounts their pkg
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::math::pkg());
    hosts.install_host_pkg(&ctx, rut_std::logger::pkg(|s| println!("{s}")));
    // the nmap experiment's native key table (the mapset-host plan) — a
    // program only reaches it when it declares `use nmap_host::{...}` or a
    // pkg that does (`nmapset`)
    hosts.install_host_pkg(&ctx, rut_std::nmap::pkg());
    // the crossing-tax benchmark's nops (the crossing-fastpath plan,
    // phase 0) — reached only by a program that declares
    // `use bench_cross::{...}` (the bench row)
    hosts.install_host_pkg(&ctx, rut_std::bench_cross::pkg());
    // the async host set: launch/abort/sleep bodies for the
    // `async_engine` rows — reached only by a program that mounts the
    // async packages (a `use async_host::` pulls the tree pkg)
    hosts.install_host_pkg(&ctx, rut_std::async_host::pkg());
    // the string builder's bodies (the host strbuild pkg): reached only
    // by a program that mounts the strbuild pkg (a `use strbuild::` /
    // `use json::` pulls it — json's writer rides the builder)
    hosts.install_host_pkg(&ctx, rut_std::strbuild::pkg());
    // the std HTTP lane (the rut/http plan): get + the Response
    // readbacks — reached only by a program that mounts the http
    // packages (a `use http::` / `use http_host::` pulls the tree
    // pkgs; reqwest is the CLI's, native-only)
    hosts.install_host_pkg(&ctx, rut_std::http::pkg());
    let mut vm = match rut_vm::interp::Vm::new(std::rc::Rc::new(prog), &limits, hooks, hosts) {
        Ok(vm) => vm,
        Err(t) => {
            eprintln!("boot: {}", t.msg);
            std::process::exit(1);
        }
    };
    // (bindings were installed into the registry before `Vm::new` above)
    match vm.call::<_, ()>("main", ()) {
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

/// `rut pack <dir> [-o out.rutbundle] [--strip]` — pack a module
/// directory into a deterministic `.rutbundle`: a **v5 compiled** root
/// for a lib pkg (linkable pkgs ride as `.rutc` binaries, splice-needed
/// deps as source), a **v6 decl** root for a `type = "host"` pkg (the
/// surface rides as source, single-package). `--strip` writes the
/// sidecar symbol table at the sibling path `<out-without-ext>.rutsym`
/// — the PRIVATE half, never an entry inside the bundle (refused on a
/// host root: no programs, no sidecar).
fn pack(dir: &str, out: Option<&str>, strip: bool) {
    let p = std::path::Path::new(dir);
    let fetch = match CacheFetch::new() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("pack: {e}");
            std::process::exit(2);
        }
    };
    let (bytes, symtab) = match block_on(rut_driver::pack_dir_opts_with(
        p,
        &rut_driver::PackOpts { strip },
        &fetch,
    )) {
        Ok(Ok(b)) => b,
        Ok(Err(e)) => {
            eprintln!("pack: {e}");
            evict_poisoned_cache(&e);
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("pack: {e}");
            std::process::exit(2);
        }
    };
    let out = out
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| rut_bundle::default_out_path(p));
    if let Err(e) = std::fs::write(&out, &bytes) {
        eprintln!("pack: cannot write {}: {e}", out.display());
        std::process::exit(1);
    }
    print!("packed {} -> {} ({} bytes)", p.display(), out.display(), bytes.len());
    if let Some(sym) = symtab {
        let sym_path = out.with_extension("rutsym");
        if let Err(e) = std::fs::write(&sym_path, &sym) {
            eprintln!("pack: cannot write {}: {e}", sym_path.display());
            std::process::exit(1);
        }
        print!(" + symbol table {} ({} bytes, keep PRIVATE)", sym_path.display(), sym.len());
    }
    println!();
}

/// `rut fetch <dir>` — pre-populate the bundle cache: load the
/// directory's graph with the cache-first fetcher and discard the
/// session. CI warms the cache while the network is up; the later
/// offline `run` hits only the cache. A `.rutbundle` input is refused:
/// bundles are closed — there is nothing to fetch.
fn fetch_cmd(dir: &str) {
    let p = std::path::Path::new(dir);
    if !p.is_dir() {
        eprintln!("fetch: {dir} is not a module directory (a `.rutbundle` is closed — nothing to fetch)");
        std::process::exit(2);
    }
    match load_with_cache(p) {
        Ok((_, root)) => println!("fetch: {dir} — all url deps are cached (root `{root}`)"),
        Err(e) => {
            eprintln!("{e}");
            evict_poisoned_cache(&e);
            std::process::exit(2);
        }
    }
}

fn dump(path: &str) {
    let src = load(path);
    let out = rut_driver::compile_module(&src, mode_of(path), "main");
    if !out.diags.is_empty() {
        print!("{}", rut_lexer::diag::render_diags(&src, &out.diags));
        std::process::exit(1);
    }
    println!("== AST =="); 
    print!("{}", out.ast_dump);
    println!("== IR ==");
    print!("{}", out.ir_dump);
}

/// `rut fmt <file.rut | dir> [--check]` — the source formatter: canonical house layout over the AST reprint, comments recovered
/// and reattached verbatim, style from the nearest ancestor `rut.toml`'s
/// `[style]` block. Default: rewrite in place. `--check`: write nothing,
/// exit 1 when anything would change.
fn fmt(path: &str, check: bool) {
    let p = std::path::Path::new(path);
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if p.is_dir() {
        let mut stack = vec![p.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let ep = e.path();
                if ep.is_dir() {
                    stack.push(ep);
                } else if ep.extension().map_or(false, |x| x == "rut") {
                    files.push(ep);
                }
            }
        }
        files.sort();
        if files.is_empty() {
            eprintln!("fmt: no .rut files under {path}");
            std::process::exit(2);
        }
    } else {
        files.push(p.to_path_buf());
    }

    let mut changed: Vec<String> = Vec::new();
    for f in &files {
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("fmt: cannot read {}: {e}", f.display());
                std::process::exit(2);
            }
        };
        let style = match style_for(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("fmt: {}: {e}", f.display());
                std::process::exit(1);
            }
        };
        let mode = if f.to_string_lossy().ends_with(".d.rut") {
            rut_parser::Mode::Decl
        } else {
            rut_parser::Mode::Impl
        };
        match rut_fmt::format(&src, mode, &style) {
            Ok(out) => {
                if out != src {
                    changed.push(f.display().to_string());
                    if !check {
                        if let Err(e) = std::fs::write(f, &out) {
                            eprintln!("fmt: cannot write {}: {e}", f.display());
                            std::process::exit(2);
                        }
                    }
                }
            }
            Err(diags) => {
                eprintln!("fmt: {} refuses — the source does not parse clean:", f.display());
                for d in &diags {
                    eprintln!("  {d}");
                }
                std::process::exit(1);
            }
        }
    }
    if check {
        if changed.is_empty() {
            println!("fmt: {} file(s) formatted", files.len());
        } else {
            for c in &changed {
                println!("unformatted: {c}");
            }
            std::process::exit(1);
        }
    } else if changed.is_empty() {
        println!("fmt: {} file(s) already formatted", files.len());
    } else {
        for c in &changed {
            println!("formatted: {c}");
        }
    }
}

/// the style for a file: the nearest ancestor `rut.toml`'s `[style]`
/// block, parsed and validated by the fmt crate; no manifest → defaults
fn style_for(f: &std::path::Path) -> Result<rut_fmt::Style, String> {
    let mut dir = f.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    loop {
        let manifest = dir.join("rut.toml");
        if manifest.is_file() {
            let text = std::fs::read_to_string(&manifest).map_err(|e| e.to_string())?;
            let m = rut_bundle::parse_manifest(&text).map_err(|e| e.to_string())?;
            return rut_fmt::style::from_manifest(&m.style);
        }
        if !dir.pop() {
            return Ok(rut_fmt::Style::default());
        }
    }
}
