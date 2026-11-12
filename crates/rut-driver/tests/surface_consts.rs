//! Surface consts (P1): a `.d.rut` surface's `let` rows fold into the
//! host body's const table — the literal-only law (D3: f64 typed or
//! inferred off a FLOAT LITERAL, `inf`/`-inf` included, negation
//! folded at the lowering; the surface computes nothing). One const,
//! one name: the rows bind BARE at the pkg's root, so a name spelled
//! twice — here or in a sibling module — is the loud refusal. (The
//! manifest's `consts` table is RETIRED: the directory grammar
//! refuses the key naming this spelling; the compat lane — the
//! zip/bundle/store boundary — discards the rows whole, so old
//! published bytes keep mounting while binding nothing.)

use std::collections::BTreeMap;

use rut_driver::{lower_decl_module, Mounted, MountedBody};

/// the PI family — calc_host's exact manifest numbers, now spelled on
/// the surface (typed, inferred, negated, and suffixed shapes all
/// fold)
const PI_SURFACE: &str = "\
pub host fn sqrt_f64(x: f64) -> f64;
pub let PI: f64 = 3.141592653589793;
pub let TAU = 6.283185307179586;
pub let SQRT_2: f64 = 1.4142135623730951;
pub let TWO: f64 = 2.0f64;
pub let NEG_E: f64 = -2.718281828459045;
";

const INF_SURFACE: &str = "\
pub let INFINITY: f64 = inf;
pub let NEG_INFINITY: f64 = -inf;
";

/// the host pkg lowered and named — the walk's mounted shape
fn host_pkg(surface: &str) -> Mounted {
    lower_decl_module(surface, &BTreeMap::new(), "host_pkg.d.rut")
        .expect("the surface lowers")
        .named("host_pkg")
}

/// app + host as one offered world, compiled with a clean graph
fn compile_world(host: Mounted, app_src: &str) -> rut_driver::Compiled {
    let world = vec![
        host,
        Mounted::source("app", app_src).manifest(rut_driver::Manifest::lib("app", &["host_pkg"])),
    ];
    let compiled = rut_driver::Walk {
        pkgs: world,
        root: String::new(),
    }
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
    .entrypoint("app")
    .compile()
    .expect("the world compiles");
    assert!(
        compiled.graph.diags.is_empty(),
        "{:?}",
        compiled
            .graph
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
    );
    compiled
}

/// boot the linked program and call `main` — the run-behavior probe.
/// `hosts` carries the world's bound rows (a consts-only host pkg
/// binds none — the empty registry)
fn run_main(compiled: rut_driver::Compiled, hosts: rut_vm::interp::HostRegistry) -> i64 {
    let prog = std::rc::Rc::new(rut_core::link::flatten(
        compiled.graph.program.expect("linked"),
    ));
    let mut vm = rut_vm::interp::Vm::builder()
        .program(prog)
        .limits(rut_vm::interp::Limits::default())
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(hosts)
        .build()
        .expect("boot");
    vm.call("main", ()).expect("run")
}

fn consts_of(m: &Mounted) -> &[(String, rut_core::types::TypeId, u64)] {
    let MountedBody::Host { consts, .. } = &m.body else {
        panic!("the host pkg carries a host body");
    };
    consts
}

#[test]
fn surface_consts_fold_mount_compile_and_run() {
    // the PI-family shape: the surface rows fold to TY_F64 bits, the
    // consumer's `pub use host_pkg::{ PI }` re-export binds them, and
    // the values assert AT RUNTIME
    let host = host_pkg(PI_SURFACE);
    let rows = consts_of(&host);
    let bits_of = |name: &str| -> u64 {
        rows.iter()
            .find(|(n, _, _)| n == name)
            .unwrap_or_else(|| panic!("the const `{name}` folded: {rows:?}"))
            .2
    };
    assert_eq!(rows.len(), 5, "{rows:?}");
    // the manifest's exact numbers (std's own consts are those bits)
    for (name, v) in [
        ("PI", std::f64::consts::PI),
        ("TAU", std::f64::consts::TAU),
        ("SQRT_2", std::f64::consts::SQRT_2),
        ("TWO", 2.0),
        ("NEG_E", -std::f64::consts::E),
    ] {
        assert_eq!(bits_of(name), v.to_bits(), "{name} folds to its exact bits");
        assert_eq!(
            rows.iter().find(|(n, _, _)| n == name).unwrap().1,
            rut_core::types::TY_F64,
            "{name} is f64 (the family is f64 end-to-end)"
        );
    }
    let compiled = compile_world(
        host,
        concat!(
            "pub use host_pkg::{ PI, TAU, SQRT_2 };\n",
            "entry fn main() -> i64 {\n",
            "    if (PI == 3.141592653589793f64 && TAU == 6.283185307179586f64\n",
            "        && SQRT_2 == 1.4142135623730951f64) {\n",
            "        return 42;\n",
            "    }\n",
            "    return 0;\n",
            "}\n",
        ),
    );
    // the pkg is the calc_host shape — host ROWS beside the consts —
    // so the boot binds the declared row
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_vm::register!(
        hosts,
        "host_pkg::sqrt_f64",
        (f64,) -> f64,
        |_vm: &mut rut_vm::interp::Vm, x: f64| x.sqrt()
    );
    assert_eq!(
        run_main(compiled, hosts),
        42,
        "the const values read at runtime"
    );
}

#[test]
fn inf_consts_fold_to_the_right_bits_and_run() {
    // `inf` is the infinity float literal (the ident fold) and `-inf`
    // its negation, folded AT THE LOWERING — the rows carry the exact
    // non-finite bits, and the consumer distinguishes the pair at
    // runtime (IEEE: inf compares above every finite, -inf below)
    let host = host_pkg(INF_SURFACE);
    let rows = consts_of(&host);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(
        rows[0],
        (
            "INFINITY".into(),
            rut_core::types::TY_F64,
            f64::INFINITY.to_bits()
        )
    );
    assert_eq!(
        rows[1],
        (
            "NEG_INFINITY".into(),
            rut_core::types::TY_F64,
            f64::NEG_INFINITY.to_bits()
        )
    );
    let compiled = compile_world(
        host,
        concat!(
            "pub use host_pkg::{ INFINITY, NEG_INFINITY };\n",
            "entry fn main() -> i64 {\n",
            "    if (INFINITY > 1.7976931348623157e308f64\n",
            "        && NEG_INFINITY < -1.7976931348623157e308f64\n",
            "        && NEG_INFINITY == -INFINITY) {\n",
            "        return 7;\n",
            "    }\n",
            "    return 0;\n",
            "}\n",
        ),
    );
    assert_eq!(
        run_main(compiled, rut_vm::interp::HostRegistry::new()),
        7,
        "the non-finite pair reads at runtime"
    );
}

#[test]
fn non_literal_and_non_f64_lets_refuse_loudly() {
    // D3's law, every refusal shape: a non-literal initializer (the
    // untyped spelling too), a computed initializer, and every non-f64
    // type — each error names the law and the offender
    for (src, lead) in [
        ("pub let X: f64 = compute();", "let X"),
        ("pub let X = compute();", "let X"),
        ("pub let X: f64 = 1.0 + 2.0;", "let X"),
        ("pub let X: f32 = 1.0;", "let X: f32"),
        ("pub let X = 2.5f32;", "let X"),
        ("pub let X: i64 = 3;", "let X: i64"),
        ("pub let X: str = \"hi\";", "let X: str"),
        ("pub let X = 3;", "let X"),
        ("pub let X = \"hi\";", "let X"),
    ] {
        let err = lower_decl_module(src, &BTreeMap::new(), "t.d.rut")
            .err()
            .unwrap_or_else(|| panic!("{src} must refuse"));
        assert!(
            err.contains("a surface const is a FLOAT LITERAL (f64, `inf`/`-inf` allowed)"),
            "{src}: the error names the law: {err}"
        );
        assert!(
            err.contains("the surface computes nothing"),
            "{src}: the error names the no-compute clause: {err}"
        );
        assert!(
            err.contains(lead) && err.contains("t.d.rut"),
            "{src}: the error names the offender and the file: {err}"
        );
    }
}

#[test]
fn inf_literals_in_impl_code_compile_and_run() {
    // the ident fold is not decl-only: a tiny `entry fn main` USING
    // `inf`/`-inf` (annotated and bare — the literal carries f64
    // always, no f32-default question opens) compiles and runs
    let src = concat!(
        "entry fn main() -> i64 {\n",
        "    let a: f64 = inf;\n",
        "    let b = -inf;\n",
        "    if (a > 1.7976931348623157e308f64 && b < -1.7976931348623157e308f64) {\n",
        "        return 5;\n",
        "    }\n",
        "    return 0;\n",
        "}\n",
    );
    let out = rut_driver::compile_module(src, rut_driver::Mode::Impl, "main");
    assert!(
        out.diags.is_empty(),
        "{:?}",
        out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>()
    );
    let flat = rut_core::link::flatten(
        rut_core::binary::decode(&out.binary.expect("linked")).expect("decode"),
    );
    rut_vm::verify::verify(&flat).expect("verify");
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(rut_vm::interp::Limits::default())
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("boot");
    let got: i64 = vm.call("main", ()).expect("run");
    assert_eq!(got, 5, "-inf computes below every finite at runtime");
}

#[test]
fn one_const_one_name_across_the_whole_pkg() {
    // the rows bind BARE at the pkg's root, so a name folds ONCE: a
    // second `let` of an already-folded name — in the same module or
    // a sibling — is the loud refusal
    let err = lower_decl_module(
        "pub let PI: f64 = 3.141592653589793;\npub let PI: f64 = 3.14;\n",
        &BTreeMap::new(),
        "h.d.rut",
    )
    .expect_err("the duplicate refuses");
    assert!(
        err.contains("PI") && err.contains("one const, one name"),
        "the refusal names the const and the law: {err}"
    );
    // across sibling modules: the root and a mounted child both
    // spelling `PI` refuse the same way
    let mut children = BTreeMap::new();
    children.insert(
        "net".to_string(),
        rut_driver::ModSource {
            path: "net".to_string(),
            vis: rut_ast::Vis::Pub,
            text: "pub let PI: f64 = 3.141592653589793;\n".to_string(),
            file: "mod.d.rut".to_string(),
        },
    );
    let err = lower_decl_module(
        "pub mod net;\npub let PI: f64 = 3.141592653589793;\n",
        &children,
        "h.d.rut",
    )
    .expect_err("the sibling duplicate refuses");
    assert!(
        err.contains("net/mod.d.rut") && err.contains("one const, one name"),
        "the refusal names the second site and the law: {err}"
    );
}
