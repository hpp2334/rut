use rut_lsp::hover::DefIndex;
use rut_lsp::std_surface::*;

#[test]
fn all_thirteen_packages_embed_and_index() {
    let idxs = indexes();
    // myhost's tree yields three indexes (root + net + net/tls);
    // the named ORIGINS stay thirteen
    let mut origins: Vec<String> = idxs.iter().map(|i| i.origin.clone()).collect();
    origins.sort();
    origins.dedup();
    assert_eq!(origins.len(), 13, "origins: {origins:?}");
    for i in &idxs {
        assert!(
            !i.types.is_empty() || !i.fns.is_empty(),
            "{}: empty index",
            i.origin
        );
    }
    for want in [
        "core",
        "calc",
        "nmap_host",
        "ink_host",
        "bench_cross",
        "strbuild_host",
        "pouch",
        "nmapset",
        "json",
        "ink",
        "strbuild",
        "myhost",
    ] {
        assert!(
            origins.contains(&want.to_string()),
            "missing pkg `{want}`: {origins:?}"
        );
    }
}

#[test]
fn the_host_tree_indexes_root_and_children() {
    // the phase-2 shape: the host surface's entries are the root
    // `.d.rut` PLUS each child Decl file, per module with the mod
    // path stamped — the walk reads the (module, mod_path) pair
    let idxs = indexes();
    let mine: Vec<&DefIndex> = idxs.iter().filter(|i| i.origin == "myhost").collect();
    assert_eq!(
        mine.len(),
        3,
        "{:?}",
        mine.iter().map(|i| &i.mod_path).collect::<Vec<_>>()
    );
    assert_eq!(mine[0].mod_path, None, "the root file answers the root");
    assert_eq!(
        mine[0].src_path.as_deref(),
        Some("rut/myhost/src/myhost.d.rut")
    );
    assert_eq!(mine[1].mod_path.as_deref(), Some("net"));
    assert_eq!(
        mine[1].src_path.as_deref(),
        Some("rut/myhost/src/net/mod.d.rut")
    );
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
    let mut doc_idx = rut_lsp::hover::index(&normalized, &ast, &toks);
    let doc_uri = "file:///t/main.rut";
    doc_idx.origin = doc_uri.to_string();
    let idxs = indexes();
    let chain: Vec<&DefIndex> = std::iter::once(&doc_idx).chain(idxs.iter()).collect();
    let dm = rut_lsp::mods::DocMods::default();
    let ctx = rut_lsp::definition::Ctx {
        doc_uri,
        toks: &toks,
        ast: &ast,
        idxs: &chain,
        doc_mods: &dm,
    };
    // the leaf: the `connect` inside the braces
    let leaf = normalized.find("connect").expect("the leaf") as u32;
    let hits = rut_lsp::definition::definition(&ctx, leaf);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(
        hits[0].uri, "rut/myhost/src/net/mod.d.rut",
        "the leaf's file"
    );
    // the segment: `net` in the use path — the declaring edge
    let seg = normalized.find("net").expect("the segment") as u32;
    let hits = rut_lsp::definition::definition(&ctx, seg);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(
        hits[0].uri, "rut/myhost/src/myhost.d.rut",
        "the edge's file"
    );
}

#[test]
fn hover_over_a_qualified_host_use_renders_the_row() {
    // the hover face rides the same chain: the leaf renders the
    // row's signature from the child file
    let src = "use myhost::net::{ connect };\n";
    let normalized = rut_lexer::lexer::normalize(src);
    let (toks, _) = rut_lexer::lexer::lex(&normalized);
    let (ast, _) = rut_parser::parse(&normalized, rut_parser::Mode::Impl);
    let mut doc_idx = rut_lsp::hover::index(&normalized, &ast, &toks);
    let doc_uri = "file:///t/main.rut";
    doc_idx.origin = doc_uri.to_string();
    let idxs = indexes();
    let chain: Vec<&DefIndex> = std::iter::once(&doc_idx).chain(idxs.iter()).collect();
    let leaf = normalized.find("connect").expect("the leaf") as u32;
    let h = rut_lsp::hover::hover(
        &chain,
        &toks,
        &ast,
        &rut_lsp::mods::DocMods::default(),
        leaf,
    )
    .expect("the leaf hovers");
    assert!(h.markdown.contains("connect"), "{}", h.markdown);
    assert!(
        h.markdown.contains("myhost"),
        "the hover names the exporting pkg: {}",
        h.markdown
    );
}

#[test]
fn every_surface_index_names_its_module() {
    // the named-module law: each std index carries the SAME constant
    // as `module` — the use-path completion reads it like any dep's
    let idxs = indexes();
    for i in &idxs {
        assert_eq!(
            i.module.as_deref(),
            Some(i.origin.as_str()),
            "{}: unnamed",
            i.origin
        );
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
    for absent in [
        "clear",
        "reserve",
        "capacity",
        "append_char",
        "finish",
        "push",
    ] {
        assert!(
            !fns.contains(&absent),
            "StringBuilder must NOT carry `{absent}`: {fns:?}"
        );
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
    let has = |name: &str| idxs.iter().any(|i| i.types.iter().any(|t| t.name == name));
    for want in ["HashMap", "HashSet"] {
        assert!(has(want), "std surface lacks `{want}`");
    }
    for absent in ["PrimMapI64", "PrimMapU64", "PrimMapF64"] {
        assert!(
            !idxs
                .iter()
                .any(|i| i.types.iter().any(|t| t.name == absent)),
            "std surface still offers `{absent}` — the prim lane names left the public surface"
        );
    }
    // the row decls' negative pin: nmapset's index carries NO
    // alias-form entries at all (the three `pub type HashMap<K, ..>`
    // rows were its only aliases — repealed with the row form)
    let nmapset = idxs
        .iter()
        .find(|i| i.origin == "nmapset")
        .expect("nmapset in the std surface");
    let alias_rows: Vec<&str> = nmapset
        .types
        .iter()
        .filter(|t| t.form == rut_lsp::hover::TyForm::Alias)
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
        !idxs
            .iter()
            .any(|i| i.fns.iter().any(|f| f.name == "map_entry")),
        "std surface still offers nmap_host's map_entry — the probing core's lanes are deleted"
    );
}
