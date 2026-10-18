//! the `rut` binary — run / new / install / fetch / pack / fmt / dump.

mod cache;
mod dump;
mod fetch;
mod fmt;
mod install;
mod new;
mod pack;
mod run;

use clap::Parser;

/// the rut toolchain's temporary-run lane: run, scaffold, install,
/// format, pack, fetch, inspect.
#[derive(Parser)]
#[command(name = "rut", version, subcommand_required = true, arg_required_else_help = true)]
enum Cli {
    /// compile and run a package directory or a packed archive (`.rutbundle`)
    Run(run::RunArgs),
    /// scaffold a hello-world module directory (the std set as deps)
    New(new::NewArgs),
    /// resolve the url deps and pin them in rut.lock
    Install(install::InstallArgs),
    /// the source formatter — canonical house layout, rewrite in place
    Fmt(fmt::FmtArgs),
    /// pack a package directory into a deterministic `.rutbundle` archive
    Pack(pack::PackArgs),
    /// pre-warm the bundle cache for offline runs
    Fetch(fetch::FetchArgs),
    /// the AST + IR readbacks for one file
    Dump(dump::DumpArgs),
}

fn main() {
    match Cli::parse() {
        Cli::Run(a) => run::run(&a),
        Cli::New(a) => new::new(&a),
        Cli::Install(a) => install::install(&a),
        Cli::Fmt(a) => fmt::fmt(&a),
        Cli::Pack(a) => pack::pack(&a),
        Cli::Fetch(a) => fetch::fetch_cmd(&a),
        Cli::Dump(a) => dump::dump(&a),
    }
}
