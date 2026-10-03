//! P1's survivors, restated for the structural fork: free-fn inlining
//! (bodies flatten into their callers under the budget), dispatch parity
//! across every call form an interface-typed value can take (bare
//! concrete receiver, slot-bound local, merged origins), and the
//! cross-module member call binding the exporter's compiled fn. The
//! prim-target dual-ABI machinery died with prim satisfaction (a
//! primitive carries no members — capability rides wrapper classes),
//! and method-inlining retired with it: methods stay real `CallM`s.

use rut_core::ops::Op;
use rut_parser::Mode;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    let collection = rut_core::binary::Surface::default();
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string()), (3, collection, "collection".to_string())],
    )
}

/// Compile, flatten, verify, and run a single-module
/// `main` returning i32.
fn run_main(src: &str) -> i32 {
    let out = compile(src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = rut_core::link::flatten(out.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(8 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

fn main_of(p: &rut_core::binary::Program) -> &rut_core::binary::FuncCode {
    p.funcs
        .iter()
        .find(|f| p.name_of(f.name) == "main")
        .expect("main compiled")
}

/// (b) a `mix64`-shaped free fn flattens into its caller: no `call` at
/// the site (one frame saved per hash).
#[test]
fn mix64_shaped_free_fn_flattens() {
    let out = compile(
        "fn mix64(bits: u64) -> u64 { return (14695981039346656037u64 ^ bits).wrapping_mul(1099511628211u64); }\n\
         entry fn main() -> u64 {\n\
         \x20   let mut h = 0u64;\n\
         \x20   for (let i = 0; i < 4; i += 1) {\n\
         \x20       h = mix64(h ^ i as u64);\n\
         \x20   }\n\
         \x20   return h;\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    let main = main_of(&p);
    assert!(
        main.code.iter().all(|op| !matches!(op, Op::Call { .. })),
        "free-fn call site must flatten:\n{}",
        rut_driver::ir_dump_of(&p.funcs, &p.interner)
    );
}

/// (c) checksum parity on an interface-heavy program: bare concrete
/// sites (static bind), wrapper sites, slot sites (`let wk: K = w`) and
/// merged origins (vtable) all answer identically. Expected checksum
/// derived from the branches: 1 + 10 + 100 + 10000 + 100000 + 1000000 +
/// 10000000 + 100000000 = 111110111 (the `h1 == h2` branch is
/// deliberately false — mix(5) != mix(9)).
#[test]
fn iface_heavy_checksum_parity() {
    let checksum = run_main(
        "interface K {\n\
         \x20   fn key(self) -> u64;\n\
         \x20   fn key_eq(self, other: Self) -> bool;\n\
         }\n\
         fn mix(bits: u64) -> u64 { return (bits ^ 14695981039346656037u64).wrapping_mul(1099511628211u64); }\n\
         class KI(i32);\n\
         impl KI {\n\
         \x20   pub fn key(self) -> u64 { return mix(self.inner as u64); }\n\
         \x20   pub fn key_eq(self, other: KI) -> bool { return self.inner == other.inner; }\n\
         }\n\
         class KU(u8);\n\
         impl KU {\n\
         \x20   pub fn key(self) -> u64 { return mix(self.inner as u64); }\n\
         \x20   pub fn key_eq(self, other: KU) -> bool { return self.inner == other.inner; }\n\
         }\n\
         struct Pt { x: i32 }\n\
         impl Pt {\n\
         \x20   pub fn key(self) -> u64 { return mix(self.x as u64); }\n\
         \x20   pub fn key_eq(self, other: Pt) -> bool { return self.x == other.x; }\n\
         }\n\
         fn pick(k: bool) -> K {\n\
         \x20   if (k) { return KI(5); }\n\
         \x20   return KU(9u8);\n\
         }\n\
         entry fn main() -> i32 {\n\
         \x20   let mut acc: i32 = 0;\n\
         \x20   let a = KI(5);\n\
         \x20   if (a.key_eq(KI(5))) { acc += 1; }\n\
         \x20   if (!a.key_eq(KI(6))) { acc += 10; }\n\
         \x20   let h1 = a.key();\n\
         \x20   let b = KU(9u8);\n\
         \x20   if (b.key_eq(KU(9u8))) { acc += 100; }\n\
         \x20   let h2 = b.key();\n\
         \x20   if (h1 == h2) { acc += 1000; }\n\
         \x20   let p = Pt { x: 3 };\n\
         \x20   let q = Pt { x: 3 };\n\
         \x20   if (p.key_eq(q)) { acc += 10000; }\n\
         \x20   if (!p.key_eq(Pt { x: 4 })) { acc += 100000; }\n\
         \x20   let wa: K = a;\n\
         \x20   if (wa.key_eq(a)) { acc += 1000000; }\n\
         \x20   if (wa.key() == h1) { acc += 10000000; }\n\
         \x20   let v: K = pick(true);\n\
         \x20   if (v.key_eq(KI(5))) { acc += 100000000; }\n\
         \x20   return acc;\n\
         }\n",
    );
    assert_eq!(checksum, 111_110_111);
}

/// (d) the slot path (`let hk: K = w; hk.m()`) still works: the
/// interface-typed local holds the ref-repr wrapper, the call binds the
/// concrete member statically (single origin) — no box is minted for a
/// ref-repr receiver, ever.
#[test]
fn slot_path_still_binds_statically() {
    let src = "interface T { fn m(self) -> i32; }\n\
               class WI(i32);\n\
               impl WI { pub fn m(self) -> i32 { return self.inner + 100; } }\n\
               entry fn main() -> i32 {\n\
               \x20   let w = WI(5);\n\
               \x20   let hk: T = w;\n\
               \x20   return hk.m();\n\
               }\n";
    let out = compile(src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    let main = main_of(&p);
    // a ref-repr receiver crosses as a reference — no box op
    assert!(
        main.code.iter().all(|op| !matches!(op, Op::Box { .. })),
        "ref-repr slot path never boxes:\n{}",
        rut_driver::ir_dump_of(&p.funcs, &p.interner)
    );
    assert_eq!(run_main(src), 105);
}

/// (e) the extern twin: a member call whose member lives in another
/// module binds the exporter's compiled id through the surface — one
/// `CallM`, linked, and correct.
#[test]
fn cross_module_member_call_binds_the_exporters_fn() {
    let dep_src = "interface T { fn m(self) -> i32; }\n\
                   class WI(i32);\n\
                   impl WI { pub fn m(self) -> i32 { return self.inner + 100; } }\n\
                   pub fn make() -> WI { return WI(5); }\n";
    let dep = rut_driver::compile_program(
        dep_src,
        Mode::Impl,
        "dep",
        1,
        &[],
    );
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    let dep = dep.program.expect("dep program");
    let surface = dep.surface.clone();

    let app = rut_driver::compile_program(
        "use dep::{WI, make};\n\
         entry fn main() -> i32 {\n\
         \x20   let w = make();\n\
         \x20   return w.m();\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface, "dep".to_string())],
    );
    assert!(app.diags.is_empty(), "{:?}", app.diags);
    let app = app.program.expect("app program");
    assert!(
        main_of(&app).code.iter().all(|op| !matches!(op, Op::Box { .. })),
        "cross-module member call is boxless:\n{}",
        rut_driver::ir_dump_of(&app.funcs, &app.interner)
    );

    let linked = rut_core::link::link(vec![dep, app]).expect("link");
    let main = linked
        .funcs
        .iter()
        .find(|f| linked.name_of(f.name) == "main")
        .expect("main in the linked program");
    let callee = main.code.iter().find_map(|op| match op {
        Op::CallM { func, .. } => Some(*func as usize),
        _ => None,
    }).expect("app main dispatches with callm");
    let fname = linked.name_of(linked.funcs[callee].name);
    assert!(fname.contains("m"), "the linked callee is the dep's member: {fname}");
    rut_vm::verify::verify(&linked).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(linked),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    assert_eq!(vm.call::<_, i32>("main", ()).expect("run"), 105);
}
