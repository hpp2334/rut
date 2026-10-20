//! the `pack` subcommand: a deterministic `.rutbundle` (+ the optional
//! sidecar symbol table), riding the cache-only remote — pack never
//! downloads (the install-first law: `rut install` fills the cache).

use crate::cache;

/// the `pack` arg surface.
#[derive(clap::Args)]
pub(crate) struct PackArgs {
    /// the package directory to pack
    dir: String,
    /// write the archive here (default: beside the directory)
    #[arg(short = 'o', value_name = "out.rutbundle")]
    out: Option<String>,
    /// strip private symbols into the sibling `.rutsym` sidecar
    #[arg(long)]
    strip: bool,
}

/// `rut pack <dir> [-o out.rutbundle] [--strip]` — pack a package
/// directory into a deterministic `.rutbundle` **archive** (the sole
/// publication unit): a **compiled** root for a lib pkg (the root and
/// every linkable dep ride as `.rutc` objects, source-binding deps as
/// source file sets), a **decl** root for a `type = "host"` pkg (the
/// surface rides as source, single-package). `--strip` writes the
/// sidecar symbol table at the sibling path `<out-without-ext>.rutsym`
/// — the PRIVATE half, never an entry inside the archive (refused on a
/// host root: no programs, no sidecar).
pub(crate) fn pack(args: &PackArgs) {
    let dir = args.dir.as_str();
    let out = args.out.as_deref();
    let strip = args.strip;
    let p = std::path::Path::new(dir);
    let remote = cache::offline_remote_for(p);
    if let Err(e) = cache::ensure_deps_cached(p, "pack", &remote) {
        eprintln!("{e}");
        std::process::exit(2);
    }
    let (bytes, symtab) = match cache::block_on(rut_native::pack_dir_opts_with(
        p,
        &rut_driver::PackOpts { strip },
        &remote,
    )) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("pack: {e}");
            cache::evict_poisoned_cache(p, &e.to_string());
            std::process::exit(2);
        }
    };
    let out = out
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| rut_native::default_out_path(p));
    if let Err(e) = std::fs::write(&out, &bytes) {
        eprintln!("pack: cannot write {}: {e}", out.display());
        std::process::exit(1);
    }
    print!(
        "packed {} -> {} ({} bytes)",
        p.display(),
        out.display(),
        bytes.len()
    );
    if let Some(sym) = symtab {
        let sym_path = out.with_extension("rutsym");
        if let Err(e) = std::fs::write(&sym_path, &sym) {
            eprintln!("pack: cannot write {}: {e}", sym_path.display());
            std::process::exit(1);
        }
        print!(
            " + symbol table {} ({} bytes, keep PRIVATE)",
            sym_path.display(),
            sym.len()
        );
    }
    println!();
}
