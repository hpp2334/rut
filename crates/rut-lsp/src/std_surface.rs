//! The toolchain's std surface — embedded source, indexed once and
//! consulted by hover/completion after the open document (one directory
//! per module: `rut/pouch/rut.jsonc` names `"pouch"`). Shared by both
//! faces: the stdio server (`server`, native) and the wasm shim
//! (`rut-lsp-wasm`) — one index, no drift.
//!
//! All THIRTEEN stdlib packages are embedded (the `rut.jsonc` `name`
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

pub const CORE: &str = include_str!("../../../rut/core/src/core.d.rut");
pub const CALC_HOST: &str = include_str!("../../../rut/calc_host/src/calc.d.rut");
pub const CALC: &str = include_str!("../../../rut/calc/src/mod.rut");
pub const NMAP_HOST: &str = include_str!("../../../rut/nmap_host/src/nmap.d.rut");
pub const INK_HOST: &str = include_str!("../../../rut/ink_host/src/ink_host.d.rut");
pub const BENCH_CROSS: &str = include_str!("../../../rut/bench-cross/src/bench_cross.d.rut");
pub const STRBUILD_HOST: &str = include_str!("../../../rut/strbuild_host/src/strbuild_host.d.rut");
pub const POUCH: &str = include_str!("../../../rut/pouch/src/mod.rut");
pub const NMAPSET: &str = include_str!("../../../rut/nmapset/src/mod.rut");
pub const JSON: &str = include_str!("../../../rut/json/src/mod.rut");
pub const INK: &str = include_str!("../../../rut/ink/src/mod.rut");
pub const STRBUILD: &str = include_str!("../../../rut/strbuild/src/mod.rut");
pub const MYHOST: &str = include_str!("../../../rut/myhost/src/myhost.d.rut");
pub const MYHOST_NET: &str = include_str!("../../../rut/myhost/src/net/mod.d.rut");
pub const MYHOST_NET_TLS: &str = include_str!("../../../rut/myhost/src/net/tls/mod.d.rut");

/// The embedded std pkgs' manifest namespaces — the qualified-access
/// heads the surface grammar cannot say (`"namespace": "Math"` lives in
/// `rut/calc_host/rut.jsonc`, not in `calc.d.rut`). The embedded surface
/// mirrors the manifest truth so a stamped document's
/// `use calc_host::{ Math }` + `Math.sqrt(x)` binds and type-checks in
/// the check lane's world assembly (`check::groups_from_defs` reads the
/// head off here) instead of false-flagging the use line.
pub const NAMESPACES: &[(&str, &str)] = &[("calc_host", "Math")];

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
        (
            CORE_LABEL,
            CORE,
            rut_parser::Mode::Decl,
            "rut/core/src/core.d.rut",
            &[] as &[StdFile],
        ),
        (
            "calc_host",
            CALC_HOST,
            rut_parser::Mode::Decl,
            "rut/calc_host/src/calc.d.rut",
            &[],
        ),
        (
            "calc",
            CALC,
            rut_parser::Mode::Impl,
            "rut/calc/src/mod.rut",
            &[],
        ),
        (
            "nmap_host",
            NMAP_HOST,
            rut_parser::Mode::Decl,
            "rut/nmap_host/src/nmap.d.rut",
            &[],
        ),
        (
            "ink_host",
            INK_HOST,
            rut_parser::Mode::Decl,
            "rut/ink_host/src/ink_host.d.rut",
            &[],
        ),
        (
            "bench_cross",
            BENCH_CROSS,
            rut_parser::Mode::Decl,
            "rut/bench-cross/src/bench_cross.d.rut",
            &[],
        ),
        (
            "strbuild_host",
            STRBUILD_HOST,
            rut_parser::Mode::Decl,
            "rut/strbuild_host/src/strbuild_host.d.rut",
            &[],
        ),
        (
            "pouch",
            POUCH,
            rut_parser::Mode::Impl,
            "rut/pouch/src/mod.rut",
            &[],
        ),
        (
            "nmapset",
            NMAPSET,
            rut_parser::Mode::Impl,
            "rut/nmapset/src/mod.rut",
            &[],
        ),
        (
            "json",
            JSON,
            rut_parser::Mode::Impl,
            "rut/json/src/mod.rut",
            &[],
        ),
        (
            "ink",
            INK,
            rut_parser::Mode::Impl,
            "rut/ink/src/mod.rut",
            &[],
        ),
        (
            "strbuild",
            STRBUILD,
            rut_parser::Mode::Impl,
            "rut/strbuild/src/mod.rut",
            &[],
        ),
        // the host-surface mod tree: root + each child Decl file
        (
            "myhost",
            MYHOST,
            rut_parser::Mode::Decl,
            "rut/myhost/src/myhost.d.rut",
            &[
                StdFile {
                    mod_path: "net",
                    src: MYHOST_NET,
                    src_path: "rut/myhost/src/net/mod.d.rut",
                },
                StdFile {
                    mod_path: "net/tls",
                    src: MYHOST_NET_TLS,
                    src_path: "rut/myhost/src/net/tls/mod.d.rut",
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
            out.push(std_index(
                origin,
                f.src,
                rut_parser::Mode::Decl,
                f.src_path,
                Some(f.mod_path),
            ));
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
