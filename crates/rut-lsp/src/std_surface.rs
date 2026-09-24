//! The toolchain's std surface — embedded source, indexed once and
//! consulted by hover/completion after the open document (one directory
//! per module: `rut/pouch/rut.toml` names `"pouch"`). Shared by both
//! faces: the stdio server (`server`, native) and the wasm shim
//! (`rut-lsp-wasm`) — one index, no drift.
//!
//! ALL TEN stdlib packages are embedded (the `rut.toml` `name` fields
//! — what a `use` path spells): the five `entry.type` declaration
//! surfaces (`core`, `calc`, `nmap_host`, `rt`, `bench_cross`) and the
//! five `entry.lib` sources (`pouch`, `nmapset`, `json`, `ink`,
//! `strbuild`) — the latter indexed in impl mode like the open
//! document, so their class methods complete. Before the `rut-lsp-align`
//! batch only core/calc/pouch were
//! here and a bare user project got no `HashMap`/`HashSet` completion
//! at all. (The hashmap-surface batch: the family rows resolve to the
//! val-column classes, so what a bare project needs is exactly the two
//! family names — the prim lane names left the public surface.)

use crate::hover::{self, DefIndex};

pub const CORE: &str = include_str!("../../../rut/core/core.d.rut");
pub const CALC: &str = include_str!("../../../rut/calc/calc.d.rut");
pub const NMAP_HOST: &str = include_str!("../../../rut/nmap_host/nmap.d.rut");
pub const RT: &str = include_str!("../../../rut/rt/rt.d.rut");
pub const BENCH_CROSS: &str = include_str!("../../../rut/bench-cross/bench_cross.d.rut");
pub const POUCH: &str = include_str!("../../../rut/pouch/pouch.rut");
pub const NMAPSET: &str = include_str!("../../../rut/nmapset/nmapset.rut");
pub const JSON: &str = include_str!("../../../rut/json/json.rut");
pub const INK: &str = include_str!("../../../rut/ink/ink.rut");
pub const STRBUILD: &str = include_str!("../../../rut/strbuild/strbuild.rut");

const CORE_LABEL: &str = "core";

/// the std surface as definition indexes, in lookup order. `src_path`
/// carries each package's TRUE repo-relative source path (the
/// `include_str!` origin) — the definition layer jumps there, so a
/// workspace that is the rut repo gets a real jump into `rut/pouch/
/// pouch.rut` instead of a synthetic label.
pub fn indexes() -> Vec<DefIndex> {
    [
        (CORE_LABEL, CORE, rut_parser::Mode::Decl, "rut/core/core.d.rut"),
        ("calc", CALC, rut_parser::Mode::Decl, "rut/calc/calc.d.rut"),
        ("nmap_host", NMAP_HOST, rut_parser::Mode::Decl, "rut/nmap_host/nmap.d.rut"),
        ("rt", RT, rut_parser::Mode::Decl, "rut/rt/rt.d.rut"),
        ("bench_cross", BENCH_CROSS, rut_parser::Mode::Decl, "rut/bench-cross/bench_cross.d.rut"),
        ("pouch", POUCH, rut_parser::Mode::Impl, "rut/pouch/pouch.rut"),
        ("nmapset", NMAPSET, rut_parser::Mode::Impl, "rut/nmapset/nmapset.rut"),
        ("json", JSON, rut_parser::Mode::Impl, "rut/json/json.rut"),
        ("ink", INK, rut_parser::Mode::Impl, "rut/ink/ink.rut"),
        ("strbuild", STRBUILD, rut_parser::Mode::Impl, "rut/strbuild/strbuild.rut"),
    ]
    .into_iter()
    .map(|(origin, src, mode, src_path)| {
        let src = rut_lexer::lexer::normalize(src);
        let (toks, _) = rut_lexer::lexer::lex(&src);
        let (ast, _) = rut_parser::parse(&src, mode);
        let mut idx = hover::index(&src, &ast, &toks);
        // the std surface is the PUBLIC face (the hashmap-surface
        // batch): nmapset's rows re-scope to the family — its internal
        // lane classes are `pub`-less in the source now, and a
        // pub-less type never rides the surface (completions, the
        // bare-project index, hovers from it). Other pkgs keep their
        // full index rows (their types are pub anyway).
        if origin == "nmapset" {
            idx.types.retain(|t| t.is_pub);
        }
        idx.origin = origin.to_string();
        idx.src_path = Some(src_path.to_string());
        idx
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_ten_packages_embed_and_index() {
        let idxs = indexes();
        assert_eq!(idxs.len(), 10, "origins: {:?}", idxs.iter().map(|i| i.origin.clone()).collect::<Vec<_>>());
        for i in &idxs {
            assert!(!i.types.is_empty() || !i.fns.is_empty(), "{}: empty index", i.origin);
        }
        let origins: Vec<&str> = idxs.iter().map(|i| i.origin.as_str()).collect();
        for want in ["core", "calc", "nmap_host", "rt", "bench_cross", "pouch", "nmapset", "json", "ink", "strbuild"] {
            assert!(origins.contains(&want), "missing pkg `{want}`: {origins:?}");
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
        // must find the family classes; the val-column rows resolve
        // through `HashMap` (the hashmap-surface batch: the prim lane
        // names are nmapset-internal now, and the surface offers the
        // family pair only)
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
        // the host surface's opaque-crossing decl too (nmap.d.rut)
        assert!(
            idxs.iter()
                .any(|i| i.fns.iter().any(|f| f.name == "map_entry")),
            "std surface lacks nmap_host's map_entry"
        );
    }
}
