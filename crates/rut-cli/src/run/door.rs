//! the run door: the input-shape refusals — a `.d.rut` surface, the
//! runnable-unit allowlist, the `--symbols` pairing, the host bundle.

/// Admit the input shape or refuse (exit 2); returns the path the rest
/// of the run uses.
pub(crate) fn admit<'a>(path: &'a str, symbols: Option<&str>) -> &'a std::path::Path {
    if path.ends_with(".d.rut") {
        eprintln!("run: {path} is a declaration file (a `.d.rut` surface) — nothing to run");
        std::process::exit(2);
    }
    // the door is an EXPLICIT ALLOWLIST: a package directory (`rut.jsonc`)
    // or a packed archive (`.rutbundle`) — every program is a manifest'd
    // dir, so there is no loose-file shape to fall back to
    let p = std::path::Path::new(path);
    if !(p.is_dir() || p.extension().map_or(false, |e| e == "rutbundle")) {
        eprintln!(
            "run: {path} is not a runnable unit — give the directory a `rut.jsonc` \
             (`{{\"name\": \"…\", \"entry\": {{\"lib\": \"./<file>.rut\"}}}}` — JSONC: \
             `//` comments and trailing commas are legal), or run a packed `.rutbundle`"
        );
        std::process::exit(2);
    }
    // the symbol table is opt-in against a packed archive: the
    // source/dir lanes compile fresh and need no map — a
    // mismatched input is a usage error, never a silent ignore
    if symbols.is_some() && !(p.is_file() && p.extension().map_or(false, |e| e == "rutbundle")) {
        eprintln!(
            "run: --symbols restores a packed archive's symbol table — give a compiled \
             `.rutbundle` (a source or directory input compiles fresh and needs no map)"
        );
        std::process::exit(2);
    }
    p
}

/// A decl host bundle: nothing to run — its root is a declaration
/// surface the embedding Rust binds, never a program.
pub(crate) fn refuse_host_walk(walked: &rut_driver::Walk, path: &str) {
    if matches!(
        walked.pkg(&walked.root).map(|m| &m.body),
        Some(rut_driver::MountedBody::Host { .. })
    ) {
        eprintln!(
            "run: {path} packs the host pkg `{}` — a host bundle carries a \
             declaration surface, nothing to run; bind its rows from the embedder \
             (`pkg(Offer::Host(..))` over the mounted rows)",
            walked.root
        );
        std::process::exit(2);
    }
}

/// The archive-shape spelling of the host refusal, for the lanes that
/// hold the decoded handle (a cached entry, a direct `.rutbundle`):
/// a host archive's root is its `mod.d.rut` surface — nothing to run.
pub(crate) fn refuse_host_archive(archive: &rut_driver::Bundle, path: &str) {
    if archive.is_host() {
        eprintln!(
            "run: {path} is a host bundle — it carries a declaration surface, \
             nothing to run; bind its rows from the embedder \
             (`pkg(Offer::Host(..))` over the mounted rows)"
        );
        std::process::exit(2);
    }
}
