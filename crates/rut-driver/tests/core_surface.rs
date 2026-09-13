//! The std:core lockstep gate (RFC 0028): `rut/std-core/core.d.rut` is the
//! declarative form of the compiler's prelude surface
//! (`rut_core::binary::Surface::core`), and the two must agree — same
//! builtin containers, same interfaces, same compiler-lowered functions.
//! The file's own contract says the surface is kept TRUE to the
//! implementation; this test makes it enforced, not aspirational.

use rut_ast::ast::{ItemKind, Linkage};
use rut_parser::{parse, Mode};

const CORE_DECL: &str = include_str!("../../../rut/std-core/core.d.rut");

/// The names core.d.rut declares: (host fns, host classes, interfaces).
fn declared_names() -> (Vec<String>, Vec<String>, Vec<String>) {
    let (ast, diags) = parse(CORE_DECL, Mode::Decl);
    assert!(diags.is_empty(), "core.d.rut must parse cleanly: {diags:?}");
    let mut fns = Vec::new();
    let mut classes = Vec::new();
    let mut ifaces = Vec::new();
    for it in ast.module_items(ast.root).to_vec() {
        match ast.item(it) {
            ItemKind::SurfaceFn { name, linkage, .. } => {
                assert_eq!(*linkage, Linkage::Host, "prelude fns are host fns");
                fns.push(ast.name(*name).to_string());
            }
            ItemKind::SurfaceClass { name, .. } => classes.push(ast.name(*name).to_string()),
            ItemKind::Trait { name, .. } => ifaces.push(ast.name(*name).to_string()),
            _ => {}
        }
    }
    (fns, classes, ifaces)
}

#[test]
fn core_decl_matches_the_compilers_surface() {
    let surface = rut_core::binary::Surface::core();
    let (fns, classes, ifaces) = declared_names();

    // containers: the decl's host classes are exactly the native types
    let mut decl_types = classes;
    decl_types.sort();
    let mut surf_types: Vec<String> =
        surface.native_types.iter().map(|(n, _)| n.clone()).collect();
    surf_types.sort();
    assert_eq!(decl_types, surf_types, "core.d.rut host classes == Surface::core native_types");

    // interfaces: the decl's interfaces are exactly the native ifaces
    let mut decl_ifaces = ifaces;
    decl_ifaces.sort();
    let mut surf_ifaces: Vec<String> =
        surface.native_ifaces.iter().map(|(n, _)| n.clone()).collect();
    surf_ifaces.sort();
    assert_eq!(decl_ifaces, surf_ifaces, "core.d.rut interfaces == Surface::core native_ifaces");

    // functions: the decl's host fns are exactly the native fns
    let mut decl_fns = fns;
    decl_fns.sort();
    let mut surf_fns = surface.native_fns.clone();
    surf_fns.sort();
    assert_eq!(decl_fns, surf_fns, "core.d.rut host fns == Surface::core native_fns");
}

#[test]
fn the_prelude_is_imported_never_ambient() {
    // `Option` with no import: the diagnostic names the fix. With the
    // import, the same module compiles (RFC 0028).
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module(
        "app:main",
        rut_driver::Module {
            source: Some(
                "fn main() -> i32 { let x = Option.some(1); return x.unwrap_or(0); }\n".into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    let out = rut_driver::compile_graph(&s, "app:main");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("`Option` is not in scope")),
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
                 fn main() -> i32 { let x = Option.some(1); return x.unwrap_or(0); }\n"
                    .into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    let out = rut_driver::compile_graph(&s, "app:main");
    assert!(out.diags.is_empty(), "diags: {:?}", out.diags);
    assert!(out.program.is_some());
}
