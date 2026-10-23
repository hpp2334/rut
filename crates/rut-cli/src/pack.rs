//! the `pack` subcommand: a package directory → one deterministic
//! artifact. `--mode source` (the default) packs the SOURCE ZIP —
//! `rut.jsonc` + `src/` + the whole closure embedded under `deps/`;
//! `--mode bin` packs the `.rutbundle` — the compiled wire (the root
//! and every linkable dep as `.rutc`, host surfaces as source), with
//! the optional sidecar symbol table. Both ride the cache-only remote
//! — pack never downloads (the install-first law: `rut install` fills
//! the cache).

use crate::cache;

/// what to pack the directory into.
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum PackMode {
    /// the source zip — `rut.jsonc` + `src/` + the closure embedded
    /// under `deps/<name>@<fp16>/` (the default)
    Source,
    /// the compiled rutbundle — the root as IR, groups at
    /// `deps/<name>@<fp16>/`
    Bin,
}

/// the `pack` arg surface.
#[derive(clap::Args)]
pub(crate) struct PackArgs {
    /// the package directory to pack
    dir: String,
    /// write the archive here (default: beside the directory —
    /// `<dir>.zip` for source, `<dir>.rutbundle` for bin)
    #[arg(short = 'o', value_name = "out")]
    out: Option<String>,
    /// the artifact kind: `source` (the default) or `bin`
    #[arg(long, value_enum, default_value_t = PackMode::Source)]
    mode: PackMode,
    /// strip private symbols into the sibling `.rutsym` sidecar —
    /// a bin-mode lane (a source zip carries source, not symbols)
    #[arg(long)]
    strip: bool,
}

/// `rut pack <dir> [-o out] [--mode source|bin] [--strip]`.
pub(crate) fn pack(args: &PackArgs) {
    let dir = args.dir.as_str();
    let out = args.out.as_deref();
    let mode = args.mode;
    let strip = args.strip;
    let p = std::path::Path::new(dir);
    if strip && mode != PackMode::Bin {
        eprintln!(
            "pack: --strip is a --mode bin lane — a source zip carries source, not symbols; \
             drop the flag or pass --mode bin"
        );
        std::process::exit(2);
    }
    let remote = cache::offline_remote_for(p);
    if let Err(e) = cache::ensure_deps_cached(p, "pack", &remote) {
        eprintln!("{e}");
        std::process::exit(2);
    }
    let (bytes, symtab) = match mode {
        PackMode::Bin => match cache::block_on(rut_native::pack_dir_opts_with(
            p,
            &rut_driver::PackOpts { strip },
            &remote,
        )) {
            Ok(b) => b,
            Err(e) => pack_failed(p, e),
        },
        PackMode::Source => match cache::block_on(rut_native::pack_dir_zip_with(p, &remote)) {
            Ok(b) => (b, None),
            Err(e) => pack_failed(p, e),
        },
    };
    let out = out.map(std::path::PathBuf::from).unwrap_or_else(|| {
        let m = match mode {
            PackMode::Source => rut_native::PackMode::Source,
            PackMode::Bin => rut_native::PackMode::Bin,
        };
        rut_native::default_out_path(p, m)
    });
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

/// A pack failure: the message, the poisoned-cache eviction when the
/// pin law names one, exit 2.
fn pack_failed(p: &std::path::Path, e: rut_driver::pack::PackError) -> ! {
    eprintln!("pack: {e}");
    cache::evict_poisoned_cache(p, &e.to_string());
    std::process::exit(2);
}
