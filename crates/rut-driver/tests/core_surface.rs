//! The core lockstep gate (RFC 0028): `rut/core/core.d.rut` is the
//! declarative form of the compiler's prelude surface
//! (`rut_core::binary::Surface::core`), and the two must agree — same
//! engine builtin types, same engine-woven traits, same
//! compiler-lowered functions. The whole prelude is `builtin` (the
//! engine implements it, RFC 0025 revised): core declares NO `host`
//! surface — that is exclusively the embedder's. The file's own contract
//! says the surface is kept TRUE to the implementation; this test makes
//! it enforced, not aspirational.

use rut_ast::ast::{ItemKind, Linkage};
use rut_parser::{parse, Mode};

const CORE_DECL: &str = include_str!("../../../rut/core/core.d.rut");

/// The names core.d.rut declares: (builtin fns, builtin types, builtin
/// traits, plain traits) + the `builtin impl` method table
/// (prim → method names).
fn declared_names() -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>, Vec<(String, Vec<String>)>) {
    let (ast, diags) = parse(CORE_DECL, Mode::Decl);
    assert!(diags.is_empty(), "core.d.rut must parse cleanly: {diags:?}");
    let mut builtin_fns = Vec::new();
    let mut builtin_types = Vec::new();
    let mut builtin_traits = Vec::new();
    let mut plain_traits = Vec::new();
    let mut builtin_impls = Vec::new();
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
            ItemKind::BuiltinTrait { name, .. } => builtin_traits.push(ast.name(*name).to_string()),
            ItemKind::Trait { name, .. } => plain_traits.push(ast.name(*name).to_string()),
            ItemKind::BuiltinImpl { prim, methods, .. } => {
                let names = methods
                    .iter()
                    .map(|&m| ast.name(ast.method_decl(m).name).to_string())
                    .collect();
                builtin_impls.push((ast.name(*prim).to_string(), names));
            }
            // `host fn`/`host struct` are the embedder's surface — a
            // toolchain decl file may not spell them
            other => panic!("core declares an embedder surface item: {other:?}"),
        }
    }
    (builtin_fns, builtin_types, builtin_traits, plain_traits, builtin_impls)
}

#[test]
fn core_decl_matches_the_compilers_surface() {
    let surface = rut_core::binary::Surface::core();
    let (builtin_fns, builtin_types, builtin_traits, plain_traits, builtin_impls) = declared_names();

    // builtin types: the decl's `builtin` decls are exactly the native types
    let mut decl_types = builtin_types;
    decl_types.sort();
    let mut surf_types: Vec<String> =
        surface.native_types.iter().map(|(n, _)| surface.names.name(*n).to_string()).collect();
    surf_types.sort();
    assert_eq!(decl_types, surf_types, "core.d.rut builtin decls == Surface::core native_types");

    // traits: the decl's builtin traits are exactly the native
    // traits — and nothing in the prelude is a plain library trait
    let mut decl_traits = builtin_traits;
    decl_traits.sort();
    let mut surf_traits: Vec<String> =
        surface.native_traits.iter().map(|(n, _)| surface.names.name(*n).to_string()).collect();
    surf_traits.sort();
    assert_eq!(decl_traits, surf_traits, "core.d.rut builtin traits == Surface::core native_traits");
    assert!(
        plain_traits.is_empty(),
        "every engine-woven trait is `builtin trait` (plain `trait` is the library form — Hashable lives in pouch)"
    );

    // functions: the decl's builtin fns are exactly the compiler-lowered
    // native fns — all of them (own/downcast/assert/panic/str/bytes)
    let mut decl_fns = builtin_fns;
    decl_fns.sort();
    let mut surf_fns: Vec<String> =
        surface.native_fns.iter().map(|n| surface.names.name(*n).to_string()).collect();
    surf_fns.sort();
    assert_eq!(decl_fns, surf_fns, "core.d.rut builtin fns == Surface::core native_fns");

    // builtin impls: the decl's `builtin impl <prim>` blocks are exactly
    // the numeric-method table — same prims, same method names per prim
    // (RFC 0032 §1.1 R2)
    let prim_name = |t: rut_core::types::TypeId| -> String {
        use rut_core::types::*;
        match t {
            TY_I8 => "i8", TY_I16 => "i16", TY_I32 => "i32", TY_I64 => "i64",
            TY_U8 => "u8", TY_U16 => "u16", TY_U32 => "u32", TY_U64 => "u64",
            _ => panic!("non-int prim in native_impls: {t:?}"),
        }
        .to_string()
    };
    let mut decl_impls: Vec<(String, Vec<String>)> = builtin_impls;
    for (_, ms) in &mut decl_impls {
        ms.sort();
    }
    decl_impls.sort();
    let mut by_prim: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (t, n, _) in &surface.native_impls {
        by_prim
            .entry(prim_name(*t))
            .or_default()
            .push(surface.names.name(*n).to_string());
    }
    let mut surf_impls: Vec<(String, Vec<String>)> = by_prim
        .into_iter()
        .map(|(p, mut ms)| {
            ms.sort();
            (p, ms)
        })
        .collect();
    assert_eq!(
        decl_impls, surf_impls,
        "core.d.rut builtin impl blocks == Surface::core native_impls"
    );

    // consts: `NAN` is core's one const — f64, name-explicit
    let surf_consts: Vec<String> =
        surface.consts.iter().map(|c| surface.names.name(c.name).to_string()).collect();
    assert_eq!(surf_consts, vec!["NAN"], "core's consts are exactly [NAN]");
    assert_eq!(surface.consts[0].bits, f64::NAN.to_bits());
}

#[test]
fn the_prelude_is_used_never_ambient() {
    // `Option` is a v1.1 removal: use-sites diagnose with the removal and
    // its replacement — with or without the use statement (RFC 0028 v1.1).
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module(
        "app_main",
        rut_driver::Module {
            source: Some(
                "fn main() -> i32 { let x = Option.some(1); return 0; }\n".into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("`Option` was removed")),
        "diags: {:?}",
        out.diags
    );

    let mut s = rut_driver::Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module(
        "app_main",
        rut_driver::Module {
            source: Some(
                "use core::{ Option };\n\
                 fn main() -> i32 { let x = Option.some(1); return 0; }\n"
                    .into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("`Option` was removed")),
        "diags: {:?}",
        out.diags
    );
}
