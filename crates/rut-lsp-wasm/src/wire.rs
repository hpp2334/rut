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

/// one url dep the read-only walk refused to mount — the
/// informational hint row a host surfaces (the recipe itself —
/// `rut install` — already rode the notify lane, once per reason).
/// `sha256` is `null` for an unpinned row: the honest refusal, never
/// a guessed pin.
#[derive(Serialize)]
pub struct MissingDep {
    pub url: String,
    pub sha256: Option<String>,
    pub cache_path: String,
}

/// `resolve_deps`'s answer — the walk's refused url rows. Mounted
/// rows carry no row: the defs store they mounted into IS the answer,
/// exactly the world a host-indexed dep leaves.
#[derive(Serialize)]
pub struct WalkOut {
    pub missing: Vec<MissingDep>,
}

impl From<&crate::walk::WalkOut> for WalkOut {
    fn from(out: &crate::walk::WalkOut) -> WalkOut {
        WalkOut {
            missing: out
                .missing
                .iter()
                .map(|m| MissingDep {
                    url: m.url.clone(),
                    sha256: m.sha256.clone(),
                    cache_path: m.cache_path.clone(),
                })
                .collect(),
        }
    }
}

/// the entry-source mode as the envelope spells it (the variant name —
/// the manifest's entry key decided it, `entry.type` → Decl).
pub fn mode_str(m: rut_semantic::Mode) -> &'static str {
    match m {
        rut_semantic::Mode::Decl => "Decl",
        rut_semantic::Mode::Impl => "Impl",
    }
}

impl From<&crate::deps::DepSource> for DepSource {
    fn from(s: &crate::deps::DepSource) -> DepSource {
        match s {
            crate::deps::DepSource::Path { dir } => DepSource::Path { dir: dir.clone() },
            crate::deps::DepSource::Url {
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

impl From<&crate::deps::DepRow> for DepRow {
    fn from(r: &crate::deps::DepRow) -> DepRow {
        DepRow {
            name: r.name.clone(),
            source: DepSource::from(&r.source),
            optional: r.optional,
            lib: r.lib.clone(),
        }
    }
}

impl From<&crate::deps::DepTable> for DepTable {
    fn from(t: &crate::deps::DepTable) -> DepTable {
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
