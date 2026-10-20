//! the `run` subcommand: cache-first for a package directory (the
//! whole-program compile is the reference; its archive caches), direct
//! load for a packed archive — either way compile through the host
//! chain, verify, drive.

mod chain;
mod door;
mod drive;
mod entry;
mod symbols;

use crate::cache;

/// the `run` arg surface.
#[derive(clap::Args)]
pub(crate) struct RunArgs {
    /// the package directory (`rut.jsonc`) or packed archive (`.rutbundle`) to
    /// run — omitted, the working directory (`.`)
    #[arg(default_value = ".")]
    path: String,
    /// name the `entry fn` to run — with several, the CLI refuses to guess
    #[arg(long, value_name = "FN")]
    entry: Option<String>,
    /// cap the op budget per turn (absent = uncapped)
    #[arg(long, value_name = "N")]
    fuel: Option<u64>,
    /// restore a stripped archive's private symbol table (compiled `.rutbundle` only)
    #[arg(long, value_name = "FILE")]
    symbols: Option<String>,
    /// force the miss path — recompile the whole program from source
    /// even on a cache hit; the fresh archive still refreshes the
    /// `.rut/cache` entry (alias: --repack)
    #[arg(long, visible_alias = "repack")]
    no_cache: bool,
}

pub(crate) fn run(args: &RunArgs) {
    let path = args.path.as_str();
    let fuel = args.fuel;
    let symbols = args.symbols.clone();
    let entry = args.entry.clone();
    let p = door::admit(path, symbols.as_deref());
    if p.extension().map_or(false, |e| e == "rutbundle") {
        // a packed archive: decode once, restore on the handle, offer
        let bytes = match std::fs::read(p) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("run: cannot read {path}: {e}");
                std::process::exit(2);
            }
        };
        run_archive(bytes, symbols.as_deref(), entry, fuel, path);
        return;
    }
    // a package directory: cache-first. A hit mounts the cached
    // archive exactly like a direct `.rutbundle`; a miss is the
    // whole-program compile — the deterministic reference — whose
    // archive then emits into the cache. `None` here covers the
    // uncachable shapes too: an unkeyable closure (no manifest — the
    // walk diagnoses) and a `<uncached>` key (never written — cache.rs)
    let cache_entry = if args.no_cache {
        None
    } else {
        match cache::run_cache_path(p) {
            Ok(Some(cached)) if cached.is_file() => Some(cached),
            _ => None,
        }
    };
    if let Some(cached) = cache_entry {
        match std::fs::read(&cached).map_err(|e| e.to_string()).and_then(|b| {
            rut_driver::Bundle::parse(&b)
                .map_err(|e| e.to_string())
                .map(|a| (b, a))
        }) {
            Ok((_, archive)) => {
                let label = cached.display().to_string();
                // defense-in-depth: a host archive never lands in the
                // cache, but the hit lane holds the decoded handle — it
                // refuses exactly like the direct `.rutbundle` lane
                door::refuse_host_archive(&archive, &label);
                run_archive_bytes(archive, entry, fuel, &label);
                return;
            }
            Err(_) => {
                // a corrupt or half-written entry: fall through to the
                // miss path; the fresh compile rewrites it below
            }
        }
    }
    // the miss path — today's run: the install-first gate (run never
    // networks — a missing url dep is one aggregated refusal), then
    // the walk (url deps ride the cache-first fetcher — offline here:
    // every dep is already cached), compile through the chain
    let remote = cache::offline_remote_for(p);
    if let Err(e) = cache::ensure_deps_cached(p, "run", &remote) {
        eprintln!("{e}");
        std::process::exit(2);
    }
    let loaded = match cache::load_offline(p) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            cache::evict_poisoned_cache(p, &e);
            std::process::exit(2);
        }
    };
    door::refuse_host_walk(&loaded, path);
    let compiled = chain::compile(&loaded);
    let prog = compiled
        .graph
        .program
        .clone()
        .expect("chain::compile refuses a graph with no program");
    let entry_name = entry::designate(entry, &prog);
    drive::run_entry(compiled, &entry_name, fuel);
    // the run succeeded: emit the archive via the pack lane and write
    // the cache entry (also under --no-cache — the entry mirrors what
    // the reference engine just produced). Best-effort; a host-rooted
    // dir never gets here (the door refused above). The pack rides the
    // cache-only remote — the closure was just loaded offline, so
    // every url dep is cached and the wire never fires.
    let remote = cache::offline_remote_for(p);
    match cache::block_on(rut_native::pack_dir_opts_with(
        p,
        &rut_driver::PackOpts::default(),
        &remote,
    )) {
        Ok((bytes, _)) => {
            // `None` = the closure cannot be cached honestly (unkeyable,
            // or a `<uncached>` url key — dead by construction); the
            // entry is skipped rather than written where no run can hit it
            if let Ok(Some(cache_path)) = cache::run_cache_path(p) {
                cache::store_run_cache(&cache_path, &bytes);
            }
        }
        // the program ran fine; the pack lane's refusal (a url-dep
        // closure it cannot re-emit) only costs the cache entry
        Err(_) => {}
    }
}

/// The archive lane: a decoded `.rutbundle` — direct (`rut run
/// <bundle>`) or a cache hit — runs exactly the same way. Symbol
/// restore rides the handle, before any offer.
fn run_archive(
    bytes: Vec<u8>,
    symbols: Option<&str>,
    entry: Option<String>,
    fuel: Option<u64>,
    label: &str,
) {
    let mut archive = match rut_driver::Bundle::parse(&bytes) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("run: {label}: {e}");
            std::process::exit(2);
        }
    };
    door::refuse_host_archive(&archive, label);
    if let Some(sym) = symbols {
        let map = symbols::decode(sym);
        symbols::warn_absent(&archive, &map);
        archive.restore_symbols(&map);
    }
    run_archive_bytes(archive, entry, fuel, label);
}

/// Compile the decoded archive through the host chain and drive it.
fn run_archive_bytes(
    archive: rut_driver::Bundle,
    entry: Option<String>,
    fuel: Option<u64>,
    label: &str,
) {
    // the walked world backs the diag renderer's file labels (source
    // groups carry text; compiled groups carry none)
    let walked = match archive.walk() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("run: {label}: {e}");
            std::process::exit(2);
        }
    };
    let compiled = {
        let mut chain = walked
            .pkgs
            .iter()
            .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()));
        for pkg in [
            rut_std::math::pkg(),
            rut_std::nmap::pkg(),
            rut_std::bench_cross::pkg(),
            rut_std::async_host::pkg(),
            rut_std::strbuild::pkg(),
            rut_std::http::pkg(),
        ] {
            chain = chain.pkg(pkg);
        }
        chain = chain.pkg(rut_std::logger::pkg(|s| println!("{s}")));
        match chain.entrypoint(&walked.root).compile() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    };
    if !compiled.graph.diags.is_empty() {
        eprint!(
            "{}",
            rut_lexer::diag::render_diags_grouped(&compiled.graph.diags, &walked_sources(&walked), None)
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
    let prog = compiled
        .graph
        .program
        .clone()
        .expect("a verified run carries its program");
    let entry_name = entry::designate(entry, &prog);
    drive::run_entry(compiled, &entry_name, fuel);
}

/// The diag renderer's file → text map over a walked archive world —
/// the archive twin of `chain::loaded_sources`.
fn walked_sources(
    walked: &rut_driver::Walk,
) -> std::collections::BTreeMap<String, &str> {
    let mut out = std::collections::BTreeMap::new();
    for pkg in &walked.pkgs {
        if let rut_driver::MountedBody::Source { text, .. } = &pkg.body {
            out.insert(pkg.spec.clone(), text.as_str());
        }
        for m in pkg.mods.values() {
            out.insert(format!("{}/{}", pkg.spec, m.path), m.text.as_str());
        }
    }
    out
}
