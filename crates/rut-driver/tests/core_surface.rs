//! The core lockstep gate: `rut/core/core.d.rut` is the
//! declarative form of the compiler's prelude surface
//! (`rut_core::binary::Surface::core`), and the two must agree — same
//! engine builtin types, same engine-woven traits, same
//! compiler-lowered functions. The whole prelude is `builtin` (the
//! engine implements it): core declares NO `host`
//! surface — that is exclusively the embedder's. The file's own contract
//! says the surface is kept TRUE to the implementation; this test makes
//! it enforced, not aspirational.

use rut_ast::ast::{ItemKind, Linkage};
use rut_parser::{parse, Mode};

const CORE_DECL: &str = include_str!("../../../rut/core/core.d.rut");

/// The names core.d.rut declares: (builtin fns, builtin types, builtin
/// traits, plain traits) + the `builtin impl` method table
/// (prim → method names). The fn/type/trait name lists carry each
/// decl's ambient bit (`true` — `prelude builtin`, `false` —
/// `pub builtin`) so the lockstep can check VISIBILITY AGREEMENT
/// against `Surface::core`'s rows.
fn declared_names() -> (
    Vec<(String, bool)>,
    Vec<(String, bool)>,
    Vec<(String, bool)>,
    Vec<String>,
    Vec<(String, Vec<String>)>,
) {
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
                assert!(
                    matches!(*linkage, Linkage::Builtin { .. }),
                    "the prelude's fns are all engine-lowered"
                );
                assert!(
                    generics.is_empty() || !matches!(linkage, Linkage::Host),
                    "crossing signatures are concrete"
                );
                let ambient = matches!(*linkage, Linkage::Builtin { ambient: true });
                builtin_fns.push((ast.name(*name).to_string(), ambient));
            }
            ItemKind::BuiltinTy { name, ambient, .. } => {
                let n = ast.name(*name).to_string();
                // `str`/`bytes` are language primitives — their
                // member contracts are doc surface, not registered natives
                if !rut_parser::is_primitive_ty(&n) {
                    builtin_types.push((n, *ambient));
                }
            }
            ItemKind::BuiltinPrimitive { name, ambient, .. } => {
                let n = ast.name(*name).to_string();
                // `str`/`bytes` are language primitives — their
                // member contracts are doc surface, not registered natives
                if rut_parser::is_primitive_ty(&n) {
                    continue;
                }
                builtin_types.push((n, *ambient));
            }
            ItemKind::BuiltinTrait { name, ambient, .. } => {
                builtin_traits.push((ast.name(*name).to_string(), *ambient))
            }
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

    // builtin types: the decl's `builtin` decls are exactly the native
    // types — name AND ambient bit (the decl spelling vs the row)
    let mut decl_types = builtin_types;
    decl_types.sort();
    let mut surf_types: Vec<(String, bool)> =
        surface.native_types.iter().map(|(n, _, a)| (surface.names.name(*n).to_string(), *a)).collect();
    surf_types.sort();
    assert_eq!(decl_types, surf_types, "core.d.rut builtin decls == Surface::core native_types");

    // traits: the decl's builtin traits are exactly the native
    // traits (name AND ambient bit) — and nothing in the prelude is a
    // plain library trait
    let mut decl_traits = builtin_traits;
    decl_traits.sort();
    let mut surf_traits: Vec<(String, bool)> =
        surface.native_traits.iter().map(|(n, _, a)| (surface.names.name(*n).to_string(), *a)).collect();
    surf_traits.sort();
    assert_eq!(decl_traits, surf_traits, "core.d.rut builtin traits == Surface::core native_traits");
    assert!(
        plain_traits.is_empty(),
        "every engine-woven trait is `builtin trait` (plain `trait` is the library form — Hashable lives in pouch)"
    );

    // functions: the decl's builtin fns are exactly the compiler-lowered
    // native fns (name AND ambient bit) — all of them
    // (own/downcast/panic/str/bytes — `assert` left the surface: it is
    // plain rut code over `panic` now, pinned by tests/ambient.rs; the
    // bare spelling is an ordinary unknown name)
    let mut decl_fns = builtin_fns;
    decl_fns.sort();
    let mut surf_fns: Vec<(String, bool)> =
        surface.native_fns.iter().map(|(n, a)| (surface.names.name(*n).to_string(), *a)).collect();
    surf_fns.sort();
    assert_eq!(decl_fns, surf_fns, "core.d.rut builtin fns == Surface::core native_fns");

    // builtin impls: the decl's `builtin impl <prim>` blocks are exactly
    // the numeric-method table — same prims, same method names per prim
    //
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
fn retired_names_are_ordinary_unknown_names() {
    // `Option` retired in v1.1: the name is an ordinary identifier now,
    // with or without the use statement — an unresolved use falls to
    // the plain unknown-name diagnostic, exactly like a never-existing
    // name. (The LIVE builtin names are ambient now —
    // builtin-surface — see tests/ambient.rs.)
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module(
        "app_main",
        rut_driver::Module {
            body: rut_driver::ModuleBody::Source {
                text: "entry fn main() -> i32 { let x = Option.some(1); return 0; }\n".into(),
                is_decl: false,
            },
            ..Default::default()
        },
    )
    .unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("unknown name `Option`")),
        "diags: {:?}",
        out.diags
    );

    let mut s = rut_driver::Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module(
        "app_main",
        rut_driver::Module {
            body: rut_driver::ModuleBody::Source {
                text: "use core::{ Option };\n\
                 entry fn main() -> i32 { let x = Option.some(1); return 0; }\n"
                    .into(),
                is_decl: false,
            },
            ..Default::default()
        },
    )
    .unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("unknown name `Option`")),
        "diags: {:?}",
        out.diags
    );
}

/// `assert` left the core surface (the builtin's removal): it is plain
/// rut code the business writes over `panic`. The law that outlives the
/// removal table's deletion: a module's own `fn assert` resolves,
/// compiles, and works (a failing condition traps with the given
/// message through the user fn's `panic`), while the bare unresolved
/// spelling is an ordinary unknown function.
fn compile_with_core(src: &str) -> rut_driver::GraphOutput {
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module(
        "app_main",
        rut_driver::Module {
            body: rut_driver::ModuleBody::Source { text: src.into(), is_decl: false },
            ..Default::default()
        },
    )
    .unwrap();
    rut_driver::compile_graph(&s, "app_main")
}

fn vm_for(src: &str) -> rut_vm::interp::Vm {
    let out = compile_with_core(src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = out.program.expect("linked program");
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    rut_vm::interp::Vm::new(
        std::rc::Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm")
}

#[test]
fn own_assert_helper_still_resolves() {
    // the helper compiles and answers at run time: a failing condition
    // traps with the CALLER's message, kind `Panic` (the user fn's
    // abort — there is no `Assert` trap kind anymore)
    let err = vm_for(
        "fn assert(c: bool, m: str) { if (!c) { panic(m); } }\n\
         fn check(total: i32) { assert(total == 42, \"checksum failed\"); }\n\
         entry fn main() -> i32 { check(7); return 0; }\n",
    )
    .call::<_, i32>("main", ())
    .expect_err("the failing assert must trap");
    assert_eq!(err.kind, rut_vm::TrapKind::Panic);
    assert_eq!(err.msg, "checksum failed");

    // the passing side stays silent
    let v = vm_for(
        "fn assert(c: bool, m: str) { if (!c) { panic(m); } }\n\
         entry fn main() -> i32 { assert(1 == 1, \"never\"); return 5; }\n",
    )
    .call::<_, i32>("main", ())
    .expect("run");
    assert_eq!(v, 5);
}

/// The unresolved bare spelling — NO local helper — is an ordinary
/// unknown function, exactly like a never-existing name.
#[test]
fn unresolved_assert_is_an_ordinary_unknown_fn() {
    let out = compile_with_core("entry fn main() -> i32 { assert(1 == 1, \"fine\"); return 0; }\n");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("unknown function `assert`")),
        "the bare assert miss carries the plain unknown-fn diag: {:?}",
        out.diags
    );
    assert!(out.program.is_none(), "the unresolved spelling must not compile");
}

/// The numeric-conversion CALL spellings (`i32(x)` …) retired with the
/// `as` cast: the names are ordinary identifiers now, and the call
/// falls to the plain unknown-function diagnostic (pinned: `i32` is a
/// type, never a fn).
#[test]
fn numeric_conversion_call_is_an_ordinary_unknown_fn() {
    let out = compile_with_core("entry fn main() -> i32 { return i32(5); }\n");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("unknown function `i32`")),
        "i32(x) falls to the plain unknown-fn diag: {:?}",
        out.diags
    );
    assert!(out.program.is_none(), "the unresolved spelling must not compile");
}

/// Twin of the assert pin: a module's OWN
/// `StrBuf`-named class resolves, compiles, and works — retired names
/// are ordinary identifiers, so a user type may take the retired name.
#[test]
fn own_strbuf_named_type_still_resolves() {
    let out = compile_with_core(
        "class StrBuf {\n    n: i32;\n}\n\
         impl StrBuf {\n    pub fn new() -> Self { return Self { n: 0 }; }\n\
         \x20   pub fn push(mut self, k: i32) { self.n += k; }\n\
         \x20   pub fn len(self) -> i32 { return self.n; }\n}\n\
         entry fn main() -> i32 { let b = StrBuf.new(); b.push(4); return b.len(); }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags.iter().map(|d| &d.msg).collect::<Vec<_>>());
    let flat = rut_core::link::flatten(out.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    assert_eq!(vm.call::<_, i32>("main", ()).expect("run"), 4);
}

/// VISIBILITY AGREEMENT, spelled out per row: each decl's
/// `Linkage::Builtin { ambient }` matches the ambient bit on its
/// `Surface::core()` row (checked set-wise by the lockstep above); this
/// pin fixes the phase's intent — the disposal pair and the
/// engine-woven trio (`Iterable`/`Future`/`RunContext`) are the
/// import-gated spelling (`pub builtin`, ambient=false; the engine
/// weaves on the symbols regardless), the native types gate only
/// `DisposalContext`, and every fn row stays ambient (`prelude
/// builtin`). A future row flips only with a deliberate test update.
#[test]
fn engine_trait_rows_are_import_gated_everything_else_ambient() {
    let surface = rut_core::binary::Surface::core();
    for (name, _, ambient) in &surface.native_types {
        let expected = surface.names.name(*name) == "DisposalContext";
        assert_eq!(!*ambient, expected, "native type `{}`: ambient bit", surface.names.name(*name));
    }
    for (name, _, ambient) in &surface.native_traits {
        assert!(!*ambient, "native trait `{}` is import-gated (`pub builtin`)", surface.names.name(*name));
    }
    for (name, ambient) in &surface.native_fns {
        assert!(*ambient, "native fn `{}` stays ambient", surface.names.name(*name));
    }
}
