//! The toolchain's std surface — embedded source, indexed once and
//! consulted by hover/completion after the open document (one directory
//! per module: `rut/pouch/rut.jsonc` names `"pouch"`). Shared by both
//! faces: the stdio server (`server`, native) and the wasm shim
//! (`rut-lsp-wasm`) — one index, no drift.
//!
//! All TWELVE stdlib packages are embedded (the `rut.jsonc` `name`
//! fields — what a `use` path spells): the six `entry.type` declaration
//! surfaces (`core`, `calc`, `nmap_host`, `ink_host`, `bench_cross`,
//! `strbuild_host`), the five root-module sources (`pouch`,
//! `nmapset`, `json`, `ink`, `strbuild`) — the latter indexed in impl
//! mode like the open document, so their class methods complete — and
//! the `myhost` fixture, the host-surface MOD TREE shape: its entries
//! are the root `.d.rut` PLUS each child Decl file (`net/mod.d.rut`,
//! `net/tls/mod.d.rut`), indexed per module with their mod paths
//! stamped, so a qualified host use (`use myhost::net::{ connect }`)
//! walks the tree exactly like a lib dep's. A flat host pkg carries no
//! children — the root file alone, as every std host pkg is today.
//! Before the `rut-lsp-align`
//! batch only core/calc/pouch were
//! here and a bare user project got no `HashMap`/`HashSet` completion
//! at all. (The hashmap-surface batch re-scoped this surface to the
//! family pair; the type-name-law batch repeals the alias-row form —
//! nmapset's index carries the two classes and NO `type` rows.)

use crate::hover::{self, DefIndex};

pub const CORE: &str = include_str!("../../../rut/core/core.d.rut");
pub const CALC: &str = include_str!("../../../rut/calc/calc.d.rut");
pub const NMAP_HOST: &str = include_str!("../../../rut/nmap_host/nmap.d.rut");
pub const INK_HOST: &str = include_str!("../../../rut/ink_host/ink_host.d.rut");
pub const BENCH_CROSS: &str = include_str!("../../../rut/bench-cross/bench_cross.d.rut");
pub const STRBUILD_HOST: &str = include_str!("../../../rut/strbuild_host/strbuild_host.d.rut");
pub const POUCH: &str = include_str!("../../../rut/pouch/mod.rut");
pub const NMAPSET: &str = include_str!("../../../rut/nmapset/mod.rut");
pub const JSON: &str = include_str!("../../../rut/json/mod.rut");
pub const INK: &str = include_str!("../../../rut/ink/mod.rut");
pub const STRBUILD: &str = include_str!("../../../rut/strbuild/mod.rut");
pub const MYHOST: &str = include_str!("../../../rut/myhost/myhost.d.rut");
pub const MYHOST_NET: &str = include_str!("../../../rut/myhost/net/mod.d.rut");
pub const MYHOST_NET_TLS: &str = include_str!("../../../rut/myhost/net/tls/mod.d.rut");

/// The embedded std pkgs' manifest namespaces — the qualified-access
/// heads the surface grammar cannot say (`"namespace": "Math"` lives in
/// `rut/calc/rut.jsonc`, not in `calc.d.rut`). The embedded surface
/// mirrors the manifest truth so a stamped document's
/// `use calc::{ Math }` + `Math.sqrt(x)` binds and type-checks in the
/// check lane's world assembly (`check::groups_from_defs` reads the
/// head off here) instead of false-flagging the use line.
pub const NAMESPACES: &[(&str, &str)] = &[("calc", "Math")];

const CORE_LABEL: &str = "core";

/// One embedded file of a std pkg: the mod path it indexes at (`""` =
/// the pkg root — the root surface's own row), the Decl-mode source,
/// and the TRUE repo-relative path (the `include_str!` origin — the
/// definition layer jumps there).
pub struct StdFile {
    pub mod_path: &'static str,
    pub src: &'static str,
    pub src_path: &'static str,
}

/// the std surface as definition indexes, in lookup order. A host pkg
/// with a mod tree yields one index PER FILE (root + children), all
/// named `module` — the (module, mod_path) pair keys the walk, the
/// same shape a directory dep's mount indexes with.
pub fn indexes() -> Vec<DefIndex> {
    [
        (CORE_LABEL, CORE, rut_parser::Mode::Decl, "rut/core/core.d.rut", &[] as &[StdFile]),
        ("calc", CALC, rut_parser::Mode::Decl, "rut/calc/calc.d.rut", &[]),
        ("nmap_host", NMAP_HOST, rut_parser::Mode::Decl, "rut/nmap_host/nmap.d.rut", &[]),
        ("ink_host", INK_HOST, rut_parser::Mode::Decl, "rut/ink_host/ink_host.d.rut", &[]),
        ("bench_cross", BENCH_CROSS, rut_parser::Mode::Decl, "rut/bench-cross/bench_cross.d.rut", &[]),
        ("strbuild_host", STRBUILD_HOST, rut_parser::Mode::Decl, "rut/strbuild_host/strbuild_host.d.rut", &[]),
        ("pouch", POUCH, rut_parser::Mode::Impl, "rut/pouch/mod.rut", &[]),
        ("nmapset", NMAPSET, rut_parser::Mode::Impl, "rut/nmapset/mod.rut", &[]),
        ("json", JSON, rut_parser::Mode::Impl, "rut/json/mod.rut", &[]),
        ("ink", INK, rut_parser::Mode::Impl, "rut/ink/mod.rut", &[]),
        ("strbuild", STRBUILD, rut_parser::Mode::Impl, "rut/strbuild/mod.rut", &[]),
        // the host-surface mod tree: root + each child Decl file
        (
            "myhost",
            MYHOST,
            rut_parser::Mode::Decl,
            "rut/myhost/myhost.d.rut",
            &[
                StdFile { mod_path: "net", src: MYHOST_NET, src_path: "rut/myhost/net/mod.d.rut" },
                StdFile {
                    mod_path: "net/tls",
                    src: MYHOST_NET_TLS,
                    src_path: "rut/myhost/net/tls/mod.d.rut",
                },
            ],
        ),
    ]
    .into_iter()
    .flat_map(|(origin, src, mode, src_path, children)| {
        let mut out = Vec::with_capacity(1 + children.len());
        // the root file's index (mod_path None answers the root)
        out.push(std_index(origin, src, mode, src_path, None));
        // each child Decl file, its mod path stamped
        for f in children {
            out.push(std_index(origin, f.src, rut_parser::Mode::Decl, f.src_path, Some(f.mod_path)));
        }
        out
    })
    .collect()
}

/// One std pkg file's definition index — the ordinary hover pass, the
/// origin stamped with the pkg's TRUE repo-relative source path, the
/// (module, mod_path) pair naming where it stands.
fn std_index(
    origin: &str,
    src: &str,
    mode: rut_parser::Mode,
    src_path: &str,
    mod_path: Option<&str>,
) -> DefIndex {
    let src = rut_lexer::lexer::normalize(src);
    let (toks, _) = rut_lexer::lexer::lex(&src);
    let (ast, _) = rut_parser::parse(&src, mode);
    let mut idx = hover::index(&src, &ast, &toks);
    // the std surface is the PUBLIC face (the hashmap-surface
    // batch): nmapset's internals stay off it — its lane classes
    // are `pub`-less in the source now, and a pub-less type never
    // rides the surface (completions, the bare-project index,
    // hovers from it). Other pkgs keep their full index rows
    // (their types are pub anyway).
    if origin == "nmapset" {
        idx.types.retain(|t| t.is_pub);
    }
    idx.origin = origin.to_string();
    idx.src_path = Some(src_path.to_string());
    // the manifest name rides EVERY surface index — the use-path
    // completion reads the same named set as any dep (std is not
    // special), and the use-graph matcher consults it first
    idx.module = Some(origin.to_string());
    idx.mod_path = mod_path.map(|p| p.to_string());
    // the mode rides too — `Decl` for the `.d.rut` surfaces, `Impl`
    // for the root-module bodies (the same split `analysis::mode_of`
    // speaks for open documents); the definition layer's
    // body-over-decl preference reads it
    idx.mode = Some(mode);
    // the std surface rides every chain as an ambient DEP lane: the
    // auto-import tier offers its names wherever the doc's manifest
    // stands (the editor's std is the dist's public face — the same
    // shape a direct dep's mount answers with). The direct-dep marker
    // gates the MOUNTED dep walks (transitive-only deps stay off);
    // this surface is not one of them.
    idx.direct_dep = true;
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_twelve_packages_embed_and_index() {
        let idxs = indexes();
        // myhost's tree yields three indexes (root + net + net/tls);
        // the named ORIGINS stay twelve
        let mut origins: Vec<String> = idxs.iter().map(|i| i.origin.clone()).collect();
        origins.sort();
        origins.dedup();
        assert_eq!(origins.len(), 12, "origins: {origins:?}");
        for i in &idxs {
            assert!(!i.types.is_empty() || !i.fns.is_empty(), "{}: empty index", i.origin);
        }
        for want in ["core", "calc", "nmap_host", "ink_host", "bench_cross", "strbuild_host", "pouch", "nmapset", "json", "ink", "strbuild", "myhost"] {
            assert!(origins.contains(&want.to_string()), "missing pkg `{want}`: {origins:?}");
        }
    }

    #[test]
    fn the_host_tree_indexes_root_and_children() {
        // the phase-2 shape: the host surface's entries are the root
        // `.d.rut` PLUS each child Decl file, per module with the mod
        // path stamped — the walk reads the (module, mod_path) pair
        let idxs = indexes();
        let mine: Vec<&DefIndex> = idxs.iter().filter(|i| i.origin == "myhost").collect();
        assert_eq!(mine.len(), 3, "{:?}", mine.iter().map(|i| &i.mod_path).collect::<Vec<_>>());
        assert_eq!(mine[0].mod_path, None, "the root file answers the root");
        assert_eq!(mine[0].src_path.as_deref(), Some("rut/myhost/myhost.d.rut"));
        assert_eq!(mine[1].mod_path.as_deref(), Some("net"));
        assert_eq!(mine[1].src_path.as_deref(), Some("rut/myhost/net/mod.d.rut"));
        assert_eq!(mine[2].mod_path.as_deref(), Some("net/tls"));
        // the root carries the `pub mod net;` edge and its flat row;
        // the children carry their own rows
        assert!(mine[0].mods.iter().any(|m| m.name == "net"));
        assert!(mine[0].fns.iter().any(|f| f.name == "legacy_ping"));
        assert!(mine[1].mods.iter().any(|m| m.name == "tls"));
        assert!(mine[1].fns.iter().any(|f| f.name == "connect"));
        assert!(mine[2].fns.iter().any(|f| f.name == "handshake"));
    }

    #[test]
    fn definition_over_a_qualified_host_use_jumps_into_the_tree() {
        // F12 on the leaf of `use myhost::net::{ connect }` lands in
        // `net/mod.d.rut` (the walked module's own file); F12 on the
        // `net` segment lands on the declaring `pub mod net;` edge
        let src = "use myhost::net::{ connect };\n\nentry fn main() -> nil {\n    let _ = connect(\"x\");\n}\n";
        let normalized = rut_lexer::lexer::normalize(src);
        let (toks, _) = rut_lexer::lexer::lex(&normalized);
        let (ast, _) = rut_parser::parse(&normalized, rut_parser::Mode::Impl);
        let mut doc_idx = crate::hover::index(&normalized, &ast, &toks);
        let doc_uri = "file:///t/main.rut";
        doc_idx.origin = doc_uri.to_string();
        let idxs = indexes();
        let chain: Vec<&DefIndex> = std::iter::once(&doc_idx).chain(idxs.iter()).collect();
        let dm = crate::mods::DocMods::default();
        let ctx = crate::definition::Ctx {
            doc_uri,
            toks: &toks,
            ast: &ast,
            idxs: &chain,
            doc_mods: &dm,
        };
        // the leaf: the `connect` inside the braces
        let leaf = normalized.find("connect").expect("the leaf") as u32;
        let hits = crate::definition::definition(&ctx, leaf);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].uri, "rut/myhost/net/mod.d.rut", "the leaf's file");
        // the segment: `net` in the use path — the declaring edge
        let seg = normalized.find("net").expect("the segment") as u32;
        let hits = crate::definition::definition(&ctx, seg);
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].uri, "rut/myhost/myhost.d.rut", "the edge's file");
    }

    #[test]
    fn hover_over_a_qualified_host_use_renders_the_row() {
        // the hover face rides the same chain: the leaf renders the
        // row's signature from the child file
        let src = "use myhost::net::{ connect };\n";
        let normalized = rut_lexer::lexer::normalize(src);
        let (toks, _) = rut_lexer::lexer::lex(&normalized);
        let (ast, _) = rut_parser::parse(&normalized, rut_parser::Mode::Impl);
        let mut doc_idx = crate::hover::index(&normalized, &ast, &toks);
        let doc_uri = "file:///t/main.rut";
        doc_idx.origin = doc_uri.to_string();
        let idxs = indexes();
        let chain: Vec<&DefIndex> = std::iter::once(&doc_idx).chain(idxs.iter()).collect();
        let leaf = normalized.find("connect").expect("the leaf") as u32;
        let h = crate::hover::hover(&chain, &toks, &ast, &crate::mods::DocMods::default(), leaf)
            .expect("the leaf hovers");
        assert!(h.markdown.contains("connect"), "{}", h.markdown);
        assert!(h.markdown.contains("myhost"), "the hover names the exporting pkg: {}", h.markdown);
    }

    #[test]
    fn every_surface_index_names_its_module() {
        // the named-module law: each std index carries the SAME constant
        // as `module` — the use-path completion reads it like any dep's
        let idxs = indexes();
        for i in &idxs {
            assert_eq!(i.module.as_deref(), Some(i.origin.as_str()), "{}: unnamed", i.origin);
        }
    }

    #[test]
    fn strbuild_class_methods_complete() {
        // the 10th pkg's class-method surface completes from the std
        // index alone: the six members exactly as censused
        // (docs/strbuild-survey.md §2) — nothing more
        let idxs = indexes();
        let sb = idxs
            .iter()
            .find(|i| i.origin == "strbuild")
            .expect("strbuild in the std surface");
        let has_type = sb.types.iter().any(|t| t.name == "StringBuilder");
        assert!(has_type, "std surface lacks the StringBuilder class");
        let fns: Vec<&str> = sb.fns.iter().map(|f| f.name.as_str()).collect();
        for want in ["new", "with_cap", "append", "append_code", "len", "build"] {
            assert!(fns.contains(&want), "StringBuilder lacks `{want}`: {fns:?}");
        }
        for absent in ["clear", "reserve", "capacity", "append_char", "finish", "push"] {
            assert!(!fns.contains(&absent), "StringBuilder must NOT carry `{absent}`: {fns:?}");
        }
    }

    #[test]
    fn nmapset_lane_classes_reachable_bare() {
        // M3 (lsp-align survey): what a bare user project (no workspace
        // index) gets from the std surface alone — `use nmapset::{{ HashMap }};`
        // must find the family classes. (The type-name-law batch: the
        // alias-row form is repealed — one name, one decl — so the
        // surface offers the class pair and NO `type` rows: the three
        // `pub type HashMap<K, ..>` index rows left with the row decls.)
        let idxs = indexes();
        let has = |name: &str| {
            idxs.iter()
                .any(|i| i.types.iter().any(|t| t.name == name))
        };
        for want in ["HashMap", "HashSet"] {
            assert!(has(want), "std surface lacks `{want}`");
        }
        for absent in ["PrimMapI64", "PrimMapU64", "PrimMapF64"] {
            assert!(
                !idxs.iter().any(|i| i.types.iter().any(|t| t.name == absent)),
                "std surface still offers `{absent}` — the prim lane names left the public surface"
            );
        }
        // the row decls' negative pin: nmapset's index carries NO
        // alias-form entries at all (the three `pub type HashMap<K, ..>`
        // rows were its only aliases — repealed with the row form)
        let nmapset = idxs.iter().find(|i| i.origin == "nmapset").expect("nmapset in the std surface");
        let alias_rows: Vec<&str> = nmapset
            .types
            .iter()
            .filter(|t| t.form == crate::hover::TyForm::Alias)
            .map(|t| t.name.as_str())
            .collect();
        assert!(
            alias_rows.is_empty(),
            "nmapset's index still carries alias rows {alias_rows:?} — the row form is repealed"
        );
        // the host surface's fused h-family decl too (nmap.d.rut) —
        // the P4 surface: the h-family is the ONE op family (the
        // round-1 `map_entry` opaque lane and the sentinel family left
        // with the probing core)
        assert!(
            idxs.iter()
                .any(|i| i.fns.iter().any(|f| f.name == "map_hput")),
            "std surface lacks nmap_host's map_hput"
        );
        // the round-1 lane's negative pin: the opaque-keyed crossing is
        // GONE from the surface (the P4 cull)
        assert!(
            !idxs.iter().any(|i| i.fns.iter().any(|f| f.name == "map_entry")),
            "std surface still offers nmap_host's map_entry — the probing core's lanes are deleted"
        );
    }
}
