//! The package value: its body kinds.
//!
//! A `Mounted` is pure data — a named package. The walk (rut-native)
//! yields them; [`crate::run::RutRun`] offers them; the compile-time
//! mount table (the crate-internal `Session`) holds them. Walker-found
//! metadata (the pkg's own manifest — dep tables included) is pub data
//! on it; the walk's own bookkeeping (where the pkg's files live) never
//! rides — it died with the walk's move to rut-native.

use std::collections::BTreeMap;

use crate::bundle::{Entry, Manifest};
use crate::mods::ModSource;

/// What a mounted pkg's body IS. The graph dispatches on this:
/// a source body compiles (and may splice), a compiled body pushes as
/// decoded, a host body synthesizes its placeholder program.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
// the payloads ARE the enum: decoded bundles move as values
// through the graph — boxing would churn every construction/match
// for a size win the graph never sees
pub enum MountedBody {
    /// a `.rut` body — compile it. `is_decl` marks a declaration-mode
    /// pkg (a `.d.rut` surface parsed as its own unit):
    /// nothing to compile or run, but `rut dump` shows the AST.
    Source { text: String, is_decl: bool },
    /// a decoded `.rutc` program (a v7 compiled bundle's payload): the
    /// body already exists — the graph assigns it a fresh scope,
    /// rebases its packed ids, and pushes it. The loader's decode gate
    /// (version + surface verification) has passed.
    Compiled(rut_core::binary::Program),
    /// a native/host module: no rut body — bodyless
    /// functions the embedder binds at run time, exported constants,
    /// and the builtin rows (`core`'s prelude, `calc`'s `Math`). The
    /// graph synthesizes a placeholder program from these rows.
    Host {
        /// the declared rows — [`HostRow`] carries the mod path each
        /// row lives under (`""` = the pkg root's own surface),
        /// `is_async` marks a `host async fn` (the row family expands
        /// at the mount)
        host_funcs: Vec<HostRow>,
        /// exported constants: `(name, type, raw bits)` — `calc::PI`
        consts: Vec<(String, rut_core::types::TypeId, u64)>,
        /// Builtin containers published by name (`core` only), each
        /// with its ambient bit — `true` (`prelude builtin`) binds in
        /// every unit with no `use`, `false` (`pub builtin`) resolves
        /// only through `use`
        native_types: Vec<(String, rut_core::binary::NativeTy, bool)>,
        /// Builtin traits published by name (`core` only), same
        /// ambient-bit law as [`MountedBody::Host`]'s `native_types`
        /// Compiler-lowered builtin function names (`core` only) — no
        /// bodies; rut-lir lowers them. Each row carries its ambient
        /// bit (same law)
        native_fns: Vec<(String, bool)>,
        /// Builtin-impl methods (`core` only): the
        /// integer primitives' numeric methods — `(receiver prim, name,
        /// lowering id)`. Bodyless and hostless — rut-lir expands the
        /// method call inline, ambient on the primitive.
        native_impls: Vec<(rut_core::types::TypeId, String, rut_core::ops::Intrinsic)>,
        /// the `.d.rut` root surface the rows lowered from — `Some`
        /// for a loaded `type = "host"` pkg (the rows' only carrier;
        /// the dep-side decl tables of phase 2 read the tree off it),
        /// `None` for hand-built row sets (`core`, `Mounted::host`)
        root_surface: Option<String>,
    },
}

/// One declared host row: the mod path it lives under (`""` = the
/// pkg root's own surface, `"net"`, `"net/tls"`), its own name, the
/// signature, and the async flag (`is_async` marks a `host async fn`
/// — the row family expands at the mount).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostRow {
    /// the module path relative to the pkg root — `""` for a row on
    /// the root surface (flat pkgs), `"net"` / `"net/tls"` under a
    /// `mod` tree
    pub path: String,
    /// the row's own name (`connect`) — never path-qualified
    pub name: String,
    pub params: Vec<rut_core::types::TypeId>,
    pub ret: rut_core::types::TypeId,
    pub is_async: bool,
}

impl HostRow {
    /// A root-surface row (`path = ""`) — the flat spelling every
    /// pre-`mod` surface carries.
    pub fn root(
        name: &str,
        params: Vec<rut_core::types::TypeId>,
        ret: rut_core::types::TypeId,
        is_async: bool,
    ) -> HostRow {
        HostRow {
            path: String::new(),
            name: name.to_string(),
            params,
            ret,
            is_async,
        }
    }

    /// The scope-relative registration spelling: `name` at the root,
    /// `path::name` under a module (each mod path segment a `::`
    /// component — `net/tls` + `handshake` spells `net::tls::handshake`)
    /// — what [`rut_vm::HostPkg`]'s builder accepts and the mount
    /// table's per-scope keys carry (the pkg scope prefixes at the
    /// install).
    pub fn rel_name(&self) -> String {
        let mut out = String::new();
        for seg in self.path.split('/') {
            if !out.is_empty() {
                out.push_str("::");
            }
            out.push_str(seg);
        }
        if !out.is_empty() {
            out.push_str("::");
        }
        out.push_str(&self.name);
        out
    }

    /// The FULL registration name — `pkg::path::name` (`myhost::net::connect`).
    /// The one source for the registration string: the synthesized
    /// surface's host ids and the expected-fns table both spell it
    /// through here, and the flat rows are the `path = ""` case
    /// (`myhost::name`) — byte-identical to the old shape.
    pub fn full_name(&self, pkg: &str) -> String {
        format!("{pkg}::{}", self.rel_name())
    }
}

impl Default for MountedBody {
    /// An empty source body — the manifest-only pkg (a pkg whose
    /// entries ride on other fields) parses to an empty unit.
    fn default() -> MountedBody {
        MountedBody::Source {
            text: String::new(),
            is_decl: false,
        }
    }
}

/// One package: the bare name it answers to, its entry files, its body
/// ([`MountedBody`]), and the walker-found metadata. Pure data — offer it
/// to a run with [`crate::run::RutRun::pkg`].
#[derive(Clone, Debug, Default)]
pub struct Mounted {
    /// the exact package name — bare `[a-zA-Z0-9_]+`
    pub spec: String,
    pub entry: Entry,
    /// the body: `.rut` source, a decoded `.rutc`, or the native rows
    pub body: MountedBody,
    /// The mounted bundle's own scope ledger — the pack-time module set
    /// (this root plus its groups), already namespaced into this
    /// session's numbering. The compiled walk's dep list: v21's
    /// split-pack root carries no foreign references (its bodies ride as
    /// source), so the binary's foreign scan sees none of these. Empty
    /// for source, host, and decl-root mounts.
    pub bundle_scopes: Vec<(rut_core::id::ScopeId, String)>,
    /// The pkg's mounted file modules (the `mod` tree's children),
    /// keyed by mod path (`"layout"`, `"layout/grid"`) — the loader
    /// mounts them by DECLARATION (`mod NAME;` resolves against
    /// `NAME/mod.rut` beside the declaring file), never a directory
    /// listing. The root file itself is the body's text, never a row
    /// here. Empty for flat packages: every manifest-spelled source
    /// set stays byte-stable, and a flat pkg's compile is untouched.
    pub mods: BTreeMap<String, ModSource>,
    /// The pkg's own parsed manifest — the STRUCTURAL form every
    /// producer stamps: the walk's `rut.jsonc` read, a bundle group's
    /// in-archive manifest, a host's in-memory build
    /// ([`Mounted::manifest`]). Json text is only a loader input; this is
    /// the one mechanism — the dep tables (`deps`, `dev-deps`) are read
    /// off it, never off pkg fields (the old flat `deps` field folded
    /// in here). For a walked pkg the WHOLE
    /// manifest rides, dev rows included, so a program root's
    /// self-build allowlist reads its own `dev-deps` back later.
    pub manifest: Manifest,
    /// The first-mount edge: WHO pulled this pkg into the world — the
    /// parent spec whose dep row declared it (a dir walk), the archive
    /// root whose group it rides (a bundle), or `None` (a hand offer,
    /// or the walk's own root). Pure diagnostic data: the uses
    /// diagnostic's later "pulled by `a`" clause reads it.
    pub mounted_by: Option<String>,
}

impl Mounted {
    /// The same pkg under another name — `lower_decl_module`'s surface
    /// with the registration scope spelled at the offer.
    pub fn named(mut self, spec: &str) -> Mounted {
        self.spec = spec.to_string();
        self
    }

    /// Attach the parsed structural manifest — the builder-style door
    /// for hosts building worlds in memory
    /// (`Mounted::source("app", src).manifest(Manifest::lib("app", &["ink"]))`).
    /// The walk's producers stamp the same field; a hand offer without
    /// one carries `Manifest::default()`. Parsing json text is never
    /// this method's business — the host calls
    /// [`crate::bundle::parse_manifest`] itself.
    pub fn manifest(mut self, m: Manifest) -> Mounted {
        self.manifest = m;
        self
    }

    /// A source pkg — the ordinary `.rut` body. `Mounted::source("app", src)`.
    pub fn source(name: &str, text: impl Into<String>) -> Mounted {
        Mounted {
            spec: name.to_string(),
            body: MountedBody::Source {
                text: text.into(),
                is_decl: false,
            },
            ..Default::default()
        }
    }

    /// A declaration-mode pkg — a `.d.rut` surface parsed as its own
    /// unit (the surface-only dev state).
    pub fn decl(name: &str, text: impl Into<String>) -> Mounted {
        Mounted {
            spec: name.to_string(),
            body: MountedBody::Source {
                text: text.into(),
                is_decl: true,
            },
            ..Default::default()
        }
    }

    /// A host-ABI pkg: declared rows, no rut body — the embedding Rust
    /// binds them at run time. Async rows carry `is_async = true` and
    /// expand into their five-row family at the mount.
    pub fn host(name: &str, rows: Vec<HostRow>) -> Mounted {
        Mounted {
            spec: name.to_string(),
            body: MountedBody::Host {
                host_funcs: rows,
                consts: vec![],
                native_types: vec![],
                native_fns: vec![],
                native_impls: vec![],
                root_surface: None,
            },
            ..Default::default()
        }
    }

    /// A decoded `.rutc` program as a pkg's body (a compiled bundle's
    /// payload, already gated).
    pub fn compiled(name: &str, program: rut_core::binary::Program) -> Mounted {
        Mounted {
            spec: name.to_string(),
            body: MountedBody::Compiled(program),
            ..Default::default()
        }
    }

    /// The pure container parse: a packed `.rutbundle`'s bytes → the
    /// walked yield ([`crate::run::Walk`]) — gates, groups, and the
    /// scope ledger. No filesystem, no network: the same contract a
    /// url dep rides. The legacy one-shot spelling — decode ONCE and
    /// reuse the handle instead ([`crate::Bundle::parse`], then offer
    /// it to any number of runs).
    pub fn from_bundle(bytes: &[u8]) -> Result<crate::run::Walk, crate::run::RunError> {
        crate::run::Bundle::parse(bytes)?.walk()
    }

    /// The pure source-zip parse: a packed `.zip`'s bytes → the
    /// walked yield — the in-memory twin of the native lane's url-dep
    /// mount ([`crate::bundle::read_srczip`] → the manifest → the
    /// entry module → the declaration-driven mod tree → the embedded
    /// dep groups, each identity-verified against the archive's own
    /// `deps/<name>@<fp16>/` spelling). No filesystem, no network:
    /// the mirror lanes' door onto the SAME bytes a primed cache
    /// serves — the [`Mounted::from_bundle`] of the source-zip era
    /// (which it supersedes for `--mode source` artifacts).
    pub fn from_srczip(bytes: &[u8]) -> Result<crate::run::Walk, crate::run::RunError> {
        let entries = crate::bundle::read_srczip(bytes)
            .map_err(|e| crate::run::RunError::law(format!("srczip: {e}")))?;
        let mut pkgs = Vec::new();
        srczip_pkg(&entries, "", None, None, &mut pkgs).map_err(crate::run::RunError::law)?;
        let root = pkgs
            .first()
            .map(|p| crate::bundle::identity::mount_name(&p.spec).to_string())
            .unwrap_or_default();
        Ok(crate::run::Walk { pkgs, root })
    }
}

/// one zip entry's text, decoded — `None` when the name is absent
/// (a missing module is the caller's loud error to name)
fn zip_text(entries: &[(String, Vec<u8>)], rel: &str) -> Option<String> {
    entries
        .iter()
        .find(|(n, _)| n == rel)
        .map(|(_, b)| String::from_utf8_lossy(b).into_owned())
}

/// The declaration-driven mod tree over the archive: every
/// `mod NAME;` in `root_text` resolves against the zip's two child
/// spellings (`<stem>/<child_file>` and `<stem>.rut`) — both present
/// is the loader's loud ambiguity, either alone mounts, neither the
/// loud miss. [`crate::mods::mount_mod_children`] with the archive
/// lookup; `root_rel` (prefix-joined) is the root's file identity
/// for the cycle guard.
fn srczip_mods(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    root_rel: &str,
    root_text: &str,
    mode: crate::Mode,
    child_file: &str,
) -> Result<BTreeMap<String, crate::mods::ModSource>, String> {
    let base = format!("{prefix}src/");
    let mut read = |parent: &str, child: &str| -> Result<crate::mods::ChildLookup, String> {
        let stem = if parent.is_empty() {
            format!("{base}{child}")
        } else {
            format!("{base}{parent}/{child}")
        };
        let dir_form = format!("{stem}/{child_file}");
        let file_form = format!("{stem}.rut");
        match (zip_text(entries, &dir_form), zip_text(entries, &file_form)) {
            (Some(_), Some(_)) => Ok(crate::mods::ChildLookup::Ambiguous),
            (Some(text), None) => Ok(crate::mods::ChildLookup::Found {
                text,
                canon: dir_form,
                file: child_file.to_string(),
            }),
            (None, Some(text)) => Ok(crate::mods::ChildLookup::Found {
                text,
                canon: file_form,
                file: format!("{child}.rut"),
            }),
            (None, None) => Ok(crate::mods::ChildLookup::Missing),
        }
    };
    crate::mods::mount_mod_children(
        root_text,
        &format!("{prefix}{root_rel}"),
        mode,
        child_file,
        &mut read,
    )
}

/// Mount one package under `prefix` — the archive root (`""`), or an
/// embedded `deps/<name>@<fp16>/` group (`spec` then carries the
/// identity spelling, `mounted_by` the declaring pkg). The tree
/// mounts by DECLARATION over the entries
/// ([`crate::mods::mount_mod_children`] with a lookup resolving the
/// two child spellings against the archive), and the embedded groups
/// recurse with their identity VERIFIED — `fp16_of_tree` over the
/// group's own files must equal the directory's spelling (the
/// packer's half of the law).
fn srczip_pkg(
    entries: &[(String, Vec<u8>)],
    prefix: &str,
    spec: Option<String>,
    mounted_by: Option<String>,
    pkgs: &mut Vec<Mounted>,
) -> Result<(), String> {
    let manifest_text = zip_text(entries, &format!("{prefix}rut.jsonc"))
        .ok_or_else(|| format!("{prefix}: the archive carries no rut.jsonc"))?;
    // the zip boundary rides the compat lane: the manifest rode the
    // wire, where today's published bytes may still carry the retired
    // keys (the wire pair parses; the `consts` rows retain, stamping
    // at the mount below)
    let manifest = crate::bundle::parse_manifest_compat(&manifest_text)
        .map_err(|e| format!("{prefix}rut.jsonc: {e}"))?;
    let name = manifest
        .name
        .clone()
        .ok_or_else(|| format!("{prefix}rut.jsonc: no `name`"))?;
    // the entry mount, by pkg type — the dir lane's own dispatch: a
    // HOST pkg's surface is a `.d.rut` (a spelled `entry.type` wins,
    // `src/mod.d.rut` the default root), LOWERED to host rows — the
    // surface `pub let` rows fold into the const rows inside the
    // lowering, with the manifest's RETAINED `consts` rows (the compat
    // lane parsed this wire-carried manifest) stamping beside them; a
    // LIB pkg's body is `src/mod.rut` (a spelled decl surface on a lib
    // is the dir lane's law — refused here, named)
    let mut pkg = match manifest.pkg_type {
        crate::bundle::PkgType::Host => {
            let rel = match &manifest.entry.type_path {
                Some(tp) => tp.trim_start_matches("./").to_string(),
                None => "mod.d.rut".to_string(),
            };
            let root_rel = format!("src/{rel}");
            let root_text = zip_text(entries, &format!("{prefix}{root_rel}")).ok_or_else(|| {
                format!("{prefix}: no {root_rel} — the entry surface the manifest names")
            })?;
            let mods = srczip_mods(
                entries,
                prefix,
                &root_rel,
                &root_text,
                crate::Mode::Decl,
                "mod.d.rut",
            )?;
            let mut m = crate::lower_decl_module(&root_text, &mods, &format!("{prefix}{root_rel}"))
                .map_err(|e| format!("{prefix}{root_rel}: {e}"))?;
            m.entry = manifest.entry.clone();
            // the retained manifest rows ride BESIDE the surface `let`
            // rows (the merge law — a name spelled both ways refuses);
            // f64 rows → TY_F64 bits, exactly the compat walk's shape
            crate::decl::fold_manifest_consts(
                &mut m.body,
                &manifest.consts,
                &format!("{prefix}{root_rel}"),
            )?;
            m
        }
        crate::bundle::PkgType::Lib => {
            if manifest.entry.type_path.is_some() {
                return Err(format!(
                    "{prefix}: a lib pkg with a spelled decl surface is the dir \
                     lane's law — from_srczip carries body-only libs"
                ));
            }
            let root_rel = "src/mod.rut";
            let root_text = zip_text(entries, &format!("{prefix}{root_rel}")).ok_or_else(|| {
                format!("{prefix}: no {root_rel} — the entry module the manifest names")
            })?;
            let mods = srczip_mods(
                entries,
                prefix,
                root_rel,
                &root_text,
                crate::Mode::Impl,
                "mod.rut",
            )?;
            Mounted {
                spec: String::new(),
                entry: manifest.entry.clone(),
                body: MountedBody::Source {
                    text: root_text,
                    is_decl: false,
                },
                bundle_scopes: Vec::new(),
                mods,
                manifest: crate::bundle::Manifest::default(),
                mounted_by: None,
            }
        }
    };
    // the embedded groups the manifest's rewritten rows name —
    // collected before the manifest moves into the pkg
    let groups: Vec<(String, String)> = manifest
        .deps
        .iter()
        .filter_map(|(key, desc)| {
            desc.get("path")
                .filter(|p| p.starts_with("deps/"))
                .map(|p| (key.clone(), p.clone()))
        })
        .collect();
    let my_spec = spec.clone().unwrap_or_else(|| name.clone());
    pkg.spec = my_spec.clone();
    pkg.manifest = manifest;
    pkg.mounted_by = mounted_by;
    pkgs.push(pkg);
    for (key, path) in groups {
        // identity is the directory's own spelling — read, never
        // hand-typed — and the group's bytes must vouch for it
        let id = path.rsplit('/').next().unwrap_or_default();
        let Some((dname, fp)) = crate::bundle::identity::parse_identity(id) else {
            return Err(format!(
                "{prefix}: dep `{key}` names {path} — not an identity spelling"
            ));
        };
        let group_prefix = format!("{prefix}{path}/");
        let files: Vec<(String, Vec<u8>)> = entries
            .iter()
            .filter(|(n, _)| n.starts_with(&group_prefix))
            .map(|(n, b)| (n[group_prefix.len()..].to_string(), b.clone()))
            .collect();
        if files.is_empty() {
            return Err(format!(
                "{prefix}: dep `{key}` names {path} — the archive carries no such group"
            ));
        }
        let computed = crate::bundle::identity::fp16_of_tree(&files);
        if computed != fp {
            return Err(format!(
                "{prefix}: dep `{key}` — the identity {dname}@{fp} does not match the group's bytes (@{computed})"
            ));
        }
        srczip_pkg(
            entries,
            &group_prefix,
            Some(crate::bundle::identity::identity(dname, fp)),
            Some(my_spec.clone()),
            pkgs,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod srczip_tests {
    use super::*;

    fn entries_of(list: &[(&str, &str)]) -> Vec<(String, Vec<u8>)> {
        list.iter()
            .map(|(n, t)| (n.to_string(), t.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn from_srczip_mounts_the_real_std_nmapset() {
        // the frozen std-v16 envelope — committed, immutable, the
        // exact bytes a primed cache serves for the pinned url
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../dist/std-v16/nmapset.zip"
        );
        let bytes = std::fs::read(path).expect("the committed std-v16 envelope");
        let walk = Mounted::from_srczip(&bytes).expect("the real zip mounts");
        assert_eq!(walk.root, "nmapset");
        let specs: Vec<&str> = walk.pkgs.iter().map(|p| p.spec.as_str()).collect();
        assert_eq!(
            specs,
            vec!["nmapset", "nmap_host@9d688d1ca40944fd"],
            "the embedded group rides with its identity spec"
        );
        let root = &walk.pkgs[0];
        assert_eq!(root.manifest.name.as_deref(), Some("nmapset"));
        assert!(matches!(
            root.body,
            MountedBody::Source { is_decl: false, .. }
        ));
        let host = &walk.pkgs[1];
        assert_eq!(host.manifest.name.as_deref(), Some("nmap_host"));
        assert!(
            matches!(&host.body, MountedBody::Host { host_funcs, .. } if !host_funcs.is_empty()),
            "the host pkg's `.d.rut` surface lowers to host rows"
        );
        assert_eq!(host.mounted_by.as_deref(), Some("nmapset"));
    }

    #[test]
    fn from_srczip_mounts_a_flat_lib() {
        let entries = entries_of(&[
            ("rut.jsonc", r#"{"name": "flat"}"#),
            ("src/mod.rut", "fn f() -> i64 {\n    7\n}\n"),
        ]);
        let zip = crate::bundle::write_srczip(&entries).expect("zip writes");
        let walk = Mounted::from_srczip(&zip).expect("the flat lib mounts");
        assert_eq!(walk.pkgs.len(), 1);
        assert_eq!(walk.pkgs[0].spec, "flat");
        assert!(walk.pkgs[0].mods.is_empty());
    }

    #[test]
    fn from_srczip_mounts_children_by_declaration_with_vis() {
        let entries = entries_of(&[
            ("rut.jsonc", r#"{"name": "tree"}"#),
            (
                "src/mod.rut",
                "pub mod math;\nmod imp;\npub mod layout;\nfn f() -> i64 {\n    1\n}\n",
            ),
            ("src/math.rut", "fn g() -> i64 {\n    2\n}\n"),
            ("src/imp.rut", "fn h() -> i64 {\n    3\n}\n"),
            ("src/layout/mod.rut", "fn k() -> i64 {\n    4\n}\n"),
        ]);
        let zip = crate::bundle::write_srczip(&entries).expect("zip writes");
        let walk = Mounted::from_srczip(&zip).expect("the tree mounts");
        let tree = &walk.pkgs[0];
        assert_eq!(tree.mods.len(), 3);
        let math = tree.mods.get("math").expect("the single-file child");
        assert_eq!(math.vis, rut_ast::Vis::Pub);
        assert_eq!(math.file, "math.rut");
        let imp = tree.mods.get("imp").expect("the private child");
        assert_eq!(imp.vis, rut_ast::Vis::Self_);
        let layout = tree.mods.get("layout").expect("the directory-form child");
        assert_eq!(layout.file, "mod.rut");
    }

    #[test]
    fn from_srczip_refuses_a_tampered_identity() {
        // a root naming an embedded group whose directory spelling no
        // longer matches its bytes
        let entries: Vec<(String, Vec<u8>)> = vec![
            (
                "rut.jsonc".to_string(),
                br#"{"name": "root", "deps": {"x": {"path": "deps/x@0000000000000000"}}}"#.to_vec(),
            ),
            (
                "src/mod.rut".to_string(),
                b"fn f() -> i64 {\n    1\n}\n".to_vec(),
            ),
            (
                "deps/x@0000000000000000/rut.jsonc".to_string(),
                br#"{"name": "x"}"#.to_vec(),
            ),
            (
                "deps/x@0000000000000000/src/mod.rut".to_string(),
                b"fn g() -> i64 {\n    2\n}\n".to_vec(),
            ),
        ];
        let zip = crate::bundle::write_srczip(&entries).expect("zip writes");
        let err = Mounted::from_srczip(&zip).expect_err("the identity must not verify");
        assert!(
            err.to_string().contains("identity"),
            "the error names the identity law: {err}"
        );
    }

    #[test]
    fn from_srczip_names_the_child_spelling_ambiguity() {
        let entries = entries_of(&[
            ("rut.jsonc", r#"{"name": "amb"}"#),
            ("src/mod.rut", "mod m;\nfn f() -> i64 {\n    1\n}\n"),
            ("src/m/mod.rut", "fn g() -> i64 {\n    2\n}\n"),
            ("src/m.rut", "fn h() -> i64 {\n    3\n}\n"),
        ]);
        let zip = crate::bundle::write_srczip(&entries).expect("zip writes");
        assert!(
            Mounted::from_srczip(&zip).is_err(),
            "both child spellings is the loader's loud ambiguity"
        );
    }
}
