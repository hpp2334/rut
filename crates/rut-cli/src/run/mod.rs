//! the `run` subcommand: load (cache-first), compile through the host
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
    /// the module directory (`rut.jsonc`) or packed `.rutbundle` to run
    path: String,
    /// name the `entry fn` to run — with several, the CLI refuses to guess
    #[arg(long, value_name = "FN")]
    entry: Option<String>,
    /// cap the op budget per turn (absent = uncapped)
    #[arg(long, value_name = "N")]
    fuel: Option<u64>,
    /// restore a stripped artifact's private symbol table (compiled `.rutbundle` only)
    #[arg(long, value_name = "FILE")]
    symbols: Option<String>,
}

pub(crate) fn run(args: &RunArgs) {
    let path = args.path.as_str();
    let fuel = args.fuel;
    let symbols = args.symbols.clone();
    let entry = args.entry.clone();
    let p = door::admit(path, symbols.as_deref());
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
    door::refuse_host_bundle(&loaded, path);
    let compiled = chain::compile(&loaded, symbols.as_deref());
    let prog = compiled
        .graph
        .program
        .clone()
        .expect("chain::compile refuses a graph with no program");
    let entry_name = entry::designate(entry, &prog);
    drive::run_entry(compiled, &entry_name, fuel);
}
