//! The std:core lockstep gate (RFC 0028): `rut/std-core/core.d.rut` is the
//! declarative form of the compiler's prelude surface
//! (`rut_core::binary::Surface::core`), and the two must agree — same
//! engine builtin types, same engine-woven interfaces, same
//! compiler-lowered functions. The whole prelude is `builtin` (the
//! engine implements it, RFC 0025 revised): std:core declares NO `host`
//! surface — that is exclusively the embedder's. The file's own contract
//! says the surface is kept TRUE to the implementation; this test makes
//! it enforced, not aspirational.

use rut_ast::ast::{ItemKind, Linkage};
use rut_parser::{parse, Mode};

const CORE_DECL: &str = include_str!("../../../rut/std-core/core.d.rut");

/// The names core.d.rut declares: (builtin fns, builtin types, builtin
/// interfaces, plain interfaces).
fn declared_names() -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let (ast, diags) = parse(CORE_DECL, Mode::Decl);
    assert!(diags.is_empty(), "core.d.rut must parse cleanly: {diags:?}");
    let mut builtin_fns = Vec::new();
    let mut builtin_types = Vec::new();
    let mut builtin_ifaces = Vec::new();
    let mut plain_ifaces = Vec::new();
    for it in ast.module_items(ast.root).to_vec() {
        match ast.item(it) {
            ItemKind::SurfaceFn { name, linkage, generics, .. } => {
                assert_eq!(*linkage, Linkage::Builtin, "the prelude's fns are all engine-lowered");
                assert!(
                    generics.is_empty() || !matches!(linkage, Linkage::Host),
                    "crossing signatures are concrete (RFC 0023 §1)"
                );
                builtin_fns.push(ast.name(*name).to_string());
            }
            ItemKind::BuiltinTy { name, .. } => {
                let n = ast.name(*name).to_string();
                // `str`/`bytes` are language primitives (RFC 0004) — their
                // member contracts are doc surface, not registered natives
                if !rut_parser::is_primitive_ty(&n) {
                    builtin_types.push(n);
                }
            }
            ItemKind::BuiltinIface { name, .. } => builtin_ifaces.push(ast.name(*name).to_string()),
            ItemKind::Trait { name, .. } => plain_ifaces.push(ast.name(*name).to_string()),
            // `host fn`/`host struct` are the embedder's surface — a
            // toolchain decl file may not spell them
            other => panic!("std:core declares an embedder surface item: {other:?}"),
        }
    }
    (builtin_fns, builtin_types, builtin_ifaces, plain_ifaces)
}

#[test]
fn core_decl_matches_the_compilers_surface() {
    let surface = rut_core::binary::Surface::core();
    let (builtin_fns, builtin_types, builtin_ifaces, plain_ifaces) = declared_names();

    // builtin types: the decl's `builtin` decls are exactly the native types
    let mut decl_types = builtin_types;
    decl_types.sort();
    let mut surf_types: Vec<String> =
        surface.native_types.iter().map(|(n, _)| surface.names.name(*n).to_string()).collect();
    surf_types.sort();
    assert_eq!(decl_types, surf_types, "core.d.rut builtin decls == Surface::core native_types");

    // interfaces: the decl's builtin interfaces are exactly the native
    // ifaces — and nothing in the prelude is a plain library interface
    let mut decl_ifaces = builtin_ifaces;
    decl_ifaces.sort();
    let mut surf_ifaces: Vec<String> =
        surface.native_ifaces.iter().map(|(n, _)| surface.names.name(*n).to_string()).collect();
    surf_ifaces.sort();
    assert_eq!(decl_ifaces, surf_ifaces, "core.d.rut builtin interfaces == Surface::core native_ifaces");
    assert!(
        plain_ifaces.is_empty(),
        "every engine-woven interface is `builtin interface` (plain `interface` is the library form — Hashable lives in std:collection)"
    );

    // functions: the decl's builtin fns are exactly the compiler-lowered
    // native fns — all of them (own/downcast/assert/panic/str/bytes)
    let mut decl_fns = builtin_fns;
    decl_fns.sort();
    let mut surf_fns: Vec<String> =
        surface.native_fns.iter().map(|n| surface.names.name(*n).to_string()).collect();
    surf_fns.sort();
    assert_eq!(decl_fns, surf_fns, "core.d.rut builtin fns == Surface::core native_fns");
}

#[test]
fn the_prelude_is_imported_never_ambient() {
    // `Option` is a v1.1 removal: use-sites diagnose with the removal and
    // its replacement — with or without the import (RFC 0028 v1.1).
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module(
        "app:main",
        rut_driver::Module {
            source: Some(
                "fn main() -> i32 { let x = Option.some(1); return 0; }\n".into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    let out = rut_driver::compile_graph(&s, "app:main");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("`Option` was removed")),
        "diags: {:?}",
        out.diags
    );

    let mut s = rut_driver::Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module(
        "app:main",
        rut_driver::Module {
            source: Some(
                "import { Option } from \"std:core\";\n\
                 fn main() -> i32 { let x = Option.some(1); return 0; }\n"
                    .into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    let out = rut_driver::compile_graph(&s, "app:main");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("`Option` was removed")),
        "diags: {:?}",
        out.diags
    );
}
