//! The toolchain's std surface — embedded source, indexed once and
//! consulted by hover/completion after the open document (one directory
//! per module: `rut/pouch/rut.toml` names `"pouch"`). Shared by both
//! faces: the stdio server (`server`, native) and the wasm shim
//! (`rut-lsp-wasm`) — one index, no drift.
//!
//! ALL EIGHT stdlib packages are embedded (the `rut.toml` `name` fields
//! — what a `use` path spells): the four `entry.type` declaration
//! surfaces (`core`, `calc`, `nmap_host`, `rt`, `bench_cross`) and the
//! four `entry.lib` sources (`pouch`, `nmapset`, `ink`) — the latter
//! indexed in impl mode like the open document, so their class methods
//! complete. Before the `rut-lsp-align` batch only core/calc/pouch were
//! here and a bare user project got no `HashMap`/`HashSet`/`PrimMapI64`
//! completion at all.

use crate::hover::{self, DefIndex};

pub const CORE: &str = include_str!("../../../rut/core/core.d.rut");
pub const CALC: &str = include_str!("../../../rut/calc/calc.d.rut");
pub const NMAP_HOST: &str = include_str!("../../../rut/nmap_host/nmap.d.rut");
pub const RT: &str = include_str!("../../../rut/rt/rt.d.rut");
pub const BENCH_CROSS: &str = include_str!("../../../rut/bench-cross/bench_cross.d.rut");
pub const POUCH: &str = include_str!("../../../rut/pouch/pouch.rut");
pub const NMAPSET: &str = include_str!("../../../rut/nmapset/nmapset.rut");
pub const INK: &str = include_str!("../../../rut/ink/ink.rut");

const CORE_LABEL: &str = "core";

/// the std surface as definition indexes, in lookup order
pub fn indexes() -> Vec<DefIndex> {
    [
        (CORE_LABEL, CORE, rut_parser::Mode::Decl),
        ("calc", CALC, rut_parser::Mode::Decl),
        ("nmap_host", NMAP_HOST, rut_parser::Mode::Decl),
        ("rt", RT, rut_parser::Mode::Decl),
        ("bench_cross", BENCH_CROSS, rut_parser::Mode::Decl),
        ("pouch", POUCH, rut_parser::Mode::Impl),
        ("nmapset", NMAPSET, rut_parser::Mode::Impl),
        ("ink", INK, rut_parser::Mode::Impl),
    ]
    .into_iter()
    .map(|(origin, src, mode)| {
        let src = rut_lexer::lexer::normalize(src);
        let (toks, _) = rut_lexer::lexer::lex(&src);
        let (ast, _) = rut_parser::parse(&src, mode);
        let mut idx = hover::index(&src, &ast, &toks);
        idx.origin = origin.to_string();
        idx
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_eight_packages_embed_and_index() {
        let idxs = indexes();
        assert_eq!(idxs.len(), 8, "origins: {:?}", idxs.iter().map(|i| i.origin.clone()).collect::<Vec<_>>());
        for i in &idxs {
            assert!(!i.types.is_empty() || !i.fns.is_empty(), "{}: empty index", i.origin);
        }
        let origins: Vec<&str> = idxs.iter().map(|i| i.origin.as_str()).collect();
        for want in ["core", "calc", "nmap_host", "rt", "bench_cross", "pouch", "nmapset", "ink"] {
            assert!(origins.contains(&want), "missing pkg `{want}`: {origins:?}");
        }
    }

    #[test]
    fn nmapset_lane_classes_reachable_bare() {
        // M3 (lsp-align survey): what a bare user project (no workspace
        // index) gets from the std surface alone — `use nmapset::{{ HashMap }};`
        // must find the wrapper classes and the PrimMap lanes
        let idxs = indexes();
        let has = |name: &str| {
            idxs.iter()
                .any(|i| i.types.iter().any(|t| t.name == name))
        };
        for want in ["HashMap", "HashSet", "PrimMapI64", "PrimMapU64", "PrimMapF64"] {
            assert!(has(want), "std surface lacks `{want}`");
        }
        // the host surface's opaque-crossing decl too (nmap.d.rut)
        assert!(
            idxs.iter()
                .any(|i| i.fns.iter().any(|f| f.name == "map_entry")),
            "std surface lacks nmap_host's map_entry"
        );
    }
}
