//! The ABI's wire types: ONE place that spells every envelope's field
//! names. The JS face (`js/index.d.ts`) declares the mirror of this
//! file; the crate's `#[cfg(test)]` gates and the extension's
//! `test/e2e-wasm.js` wire-shape block assert that the bytes on the
//! wire still carry exactly these keys.
//!
//! Only the envelopes whose shape is OURS live here. The query answers
//! (hover / completion / inlay / signature help / diagnostics / symbols
//! / semantic tokens) are `ls-types` values serialized as-is — an
//! external crate's field spelling, which no in-repo derive can prove;
//! the runtime gates above are what lock those.
//!
//! `ls-types` is the one external vocabulary the wire speaks: deriving
//! `Serialize` on a wrapper struct that HOLDS `ls_types::Diagnostic`s
//! is not a mirror of their fields — the values ride through whole.

use ls_types::{Diagnostic, DocumentSymbol, Range, SemanticTokens};
use serde::Serialize;

/// the error envelope — a malformed manifest / lock / archive, or a
/// degenerate request. Every face narrows on the `error` key.
#[derive(Serialize)]
pub struct Error {
    pub error: String,
}

impl Error {
    pub fn new(msg: impl Into<String>) -> Error {
        Error { error: msg.into() }
    }
}

/// `analyze`'s answer: the merged diagnostics, the flat semantic-token
/// stream, and the document-symbol tree.
#[derive(Serialize)]
pub struct Analysis {
    pub diags: Vec<Diagnostic>,
    pub tokens: SemanticTokens,
    pub symbols: Vec<DocumentSymbol>,
}

/// `set_lock`'s answer — the number of url→pin rows the ledger carried.
#[derive(Serialize)]
pub struct Pins {
    pub pins: usize,
}

/// `set_direct_deps`'s answer — the accumulated set's size.
#[derive(Serialize)]
pub struct Direct {
    pub direct: usize,
}

/// one `mod NAME;` declaration a stamped file carries — the host's
/// mount walk reads its next hop from these.
#[derive(Serialize)]
pub struct ModDecl {
    pub name: String,
    /// `"pub"` or `"mod"` — the visibility spell, not a Debug value
    pub vis: &'static str,
}

/// `add_def_mod`'s answer.
#[derive(Serialize)]
pub struct ModDecls {
    pub decls: Vec<ModDecl>,
}

/// one definition target — the LSP `Location` JSON (`{ uri, range }`).
#[derive(Serialize)]
pub struct Location {
    pub uri: String,
    pub range: Range,
}

/// where a dep row's bytes live. The tag spells the manifest's own
/// spelling (`kind: "path" | "url"`); the url row carries the pin the
/// lock supplied (`null` when unpinned — the honest refusal) and the
/// Rust-computed cache path (JS never hashes URLs).
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DepSource {
    Path {
        dir: String,
    },
    Url {
        url: String,
        sha256: Option<String>,
        cache_path: String,
    },
}

/// one dep row (`deps` / `dev-deps`).
#[derive(Serialize)]
pub struct DepRow {
    pub name: String,
    pub source: DepSource,
    pub optional: bool,
    pub lib: Option<String>,
}

/// one entry source: the path a pkg indexes, and the mode it indexes in.
#[derive(Serialize)]
pub struct EntrySource {
    pub path: String,
    /// `"Decl"` or `"Impl"`
    pub mode: &'static str,
}

/// the parsed `rut.jsonc` — the editor-shaped dep table.
#[derive(Serialize)]
pub struct DepTable {
    pub name: Option<String>,
    pub entries: Vec<EntrySource>,
    pub namespace: Option<String>,
    /// the compiler-materialized constants (`Math::PI`), name → value
    pub consts: Vec<(String, f64)>,
    pub deps: Vec<DepRow>,
    pub dev_deps: Vec<DepRow>,
}

/// one extracted source file: where it lands, its text, the mode it
/// indexes in, and the mod path it mounts at (`null` for a flat file).
#[derive(Serialize)]
pub struct BundleFile {
    pub path: String,
    pub src: String,
    pub mode: &'static str,
    pub mod_path: Option<String>,
}

/// `add_bundle`'s answer — the module, its qualified-access head, its
/// constants, and the entry paths the call indexed.
#[derive(Serialize)]
pub struct BundleIndex {
    pub module: String,
    pub namespace: Option<String>,
    pub consts: Vec<(String, f64)>,
    pub files: Vec<String>,
}

/// `extract_bundle`'s answer — the editable surface, never an index
/// write (the host owns the disk).
#[derive(Serialize)]
pub struct ExtractBundle {
    pub module: String,
    pub namespace: Option<String>,
    pub consts: Vec<(String, f64)>,
    pub files: Vec<BundleFile>,
}

/// one archive-embedded `deps/<identity>/` entry (a source zip's
/// ride-along group), laid under the extracted tree by the host.
#[derive(Serialize)]
pub struct Embedded {
    pub path: String,
    pub src: String,
}

/// `extract_srczip`'s answer — the bundle surface plus the unpack law's
/// ingredients: the archive's own manifest text and the embedded groups.
#[derive(Serialize)]
pub struct ExtractSrczip {
    pub module: String,
    pub namespace: Option<String>,
    pub consts: Vec<(String, f64)>,
    pub files: Vec<BundleFile>,
    pub manifest: String,
    pub embedded: Vec<Embedded>,
}

/// the entry-source mode as the envelope spells it (the variant name —
/// the manifest's entry key decided it, `entry.type` → Decl).
pub fn mode_str(m: rut_parser::Mode) -> &'static str {
    match m {
        rut_parser::Mode::Decl => "Decl",
        rut_parser::Mode::Impl => "Impl",
    }
}

impl From<&rut_lsp::deps::DepSource> for DepSource {
    fn from(s: &rut_lsp::deps::DepSource) -> DepSource {
        match s {
            rut_lsp::deps::DepSource::Path { dir } => DepSource::Path { dir: dir.clone() },
            rut_lsp::deps::DepSource::Url {
                url,
                sha256,
                cache_path,
            } => DepSource::Url {
                url: url.clone(),
                sha256: sha256.clone(),
                cache_path: cache_path.clone(),
            },
        }
    }
}

impl From<&rut_lsp::deps::DepRow> for DepRow {
    fn from(r: &rut_lsp::deps::DepRow) -> DepRow {
        DepRow {
            name: r.name.clone(),
            source: DepSource::from(&r.source),
            optional: r.optional,
            lib: r.lib.clone(),
        }
    }
}

impl From<&rut_lsp::deps::DepTable> for DepTable {
    fn from(t: &rut_lsp::deps::DepTable) -> DepTable {
        DepTable {
            name: t.name.clone(),
            entries: t
                .entries
                .iter()
                .map(|e| EntrySource {
                    path: e.path.clone(),
                    mode: mode_str(e.mode),
                })
                .collect(),
            namespace: t.namespace.clone(),
            consts: t.consts.clone(),
            deps: t.deps.iter().map(DepRow::from).collect(),
            dev_deps: t.dev_deps.iter().map(DepRow::from).collect(),
        }
    }
}

impl BundleFile {
    /// one `bundle_sources` file at the host's mount lane: the mod path
    /// rides `bundle_file_mod_path` (the rows-lane root spells `""`, a
    /// mounted child its `<path>`; a flat file stays `null`), so the
    /// host never re-derives the mount rule in TypeScript.
    pub fn from_source(f: &rut_lsp::deps::BundleFile) -> BundleFile {
        BundleFile {
            path: f.path.clone(),
            src: f.src.clone(),
            mode: mode_str(f.mode),
            mod_path: rut_lsp::deps::bundle_file_mod_path(&f.path),
        }
    }
}
