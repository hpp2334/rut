//! P1 (mapset perf plan): boxless static trait dispatch + trait-impl/
//! free-fn inlining. Prim-target impl methods compile in TWO ABI
//! variants — the slot ABI (vtable rows; scalars cross boxed, prologue
//! unboxes) and the concrete ABI (bare-receiver static calls; params
//! cross raw). Bare concrete receivers bind the concrete variant: no
//! `box` is minted, and a tiny body inlines at the site (no call op at
//! all). Free fns inline under the same budget.

use rut_core::ops::Op;
use rut_core::types::TY_I32;
use rut_parser::Mode;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    let collection = rut_core::binary::Surface::default();
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core()), (3, collection)],
    )
}

/// Compile, flatten (RFC 0035 §1), verify, and run a single-module
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

const TINY: &str = "\
trait T { fn m(self) -> i32; }\n\
impl T for i32 { fn m(self) -> i32 { return self + 100; } }\n";

/// (a) an origin-pinned bare-prim trait call: no `box` op mints, and a
/// tiny body inlines — no `callm` either. The dump spells it: main is
/// just the const + the arithmetic.
#[test]
fn bare_prim_trait_call_is_boxless_and_inlines() {
    let out = compile(&format!(
        "{TINY}\
         fn main() -> i32 {{\n\
         \x20   let x = 5;\n\
         \x20   return x.m();\n\
         }}\n"
    ));
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    let main = main_of(&p);
    // op-level: no box, no dispatch op of any kind
    assert!(
        main.code.iter().all(|op| !matches!(
            op,
            Op::Box { .. } | Op::Call { .. } | Op::CallM { .. } | Op::CallI { .. }
        )),
        "bare-prim call must inline without a box:\n{}",
        rut_driver::ir_dump_of(&p.funcs, &p.interner)
    );
    // dump-level: same answer in the text (guarded — `unbox` must not
    // count as a box hit)
    let ir = rut_driver::ir_dump_of(&p.funcs, &p.interner);
    let main_text = ir.split("fn #").find(|s| s.starts_with("1 main")).unwrap_or("");
    assert!(!main_text.contains(" box "), "no box op in main:\n{ir}");
    assert!(!main_text.contains("callm"), "no callm in main:\n{ir}");
}

/// (a2) a body over the inline budget still dispatches BOXLESS: the
/// call binds the CONCRETE variant (param type is the prim, no unbox
/// prologue), and the slot variant exists for the vtable row.
#[test]
fn fat_body_falls_back_to_the_concrete_variant_without_a_box() {
    let mut body = String::from("        let a0 = self;\n");
    for i in 1..=30 {
        body.push_str(&format!("        let a{i} = a{} + 1;\n", i - 1));
    }
    let src = format!(
        "trait T {{ fn m(self) -> i32; }}\n\
         impl T for i32 {{\n\
         \x20   fn m(self) -> i32 {{\n\
         {body}\
         \x20       return a30;\n\
         \x20   }}\n\
         }}\n\
         fn main() -> i32 {{\n\
         \x20   let x = 5;\n\
         \x20   return x.m();\n\
         }}\n"
    );
    let out = compile(&src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    let main = main_of(&p);
    assert!(
        main.code.iter().all(|op| !matches!(op, Op::Box { .. })),
        "bare-prim fallback must not box the receiver:\n{}",
        rut_driver::ir_dump_of(&p.funcs, &p.interner)
    );
    let callee = main.code.iter().find_map(|op| match op {
        Op::CallM { func, .. } => Some(*func as usize),
        _ => None,
    }).expect("the fat body dispatches with callm");
    // the callee is the CONCRETE variant: prim param, no unbox prologue
    let f = &p.funcs[callee];
    assert_eq!(f.params[0], TY_I32, "concrete-ABI param:\n{}", rut_driver::ir_dump_of(&p.funcs, &p.interner));
    assert!(
        f.code.iter().all(|op| !matches!(op, Op::Unbox { .. })),
        "concrete-ABI variant has no unbox prologue:\n{}",
        rut_driver::ir_dump_of(&p.funcs, &p.interner)
    );
    // and the slot variant exists alongside (vtable rows bind it)
    let slot_variant = p.funcs.iter().find(|f| {
        f.params.first()
            .map(|&t| matches!(p.types.kind(t), rut_core::types::TyKind::TraitObj { .. }))
            .unwrap_or(false)
    });
    assert!(
        slot_variant.is_some(),
        "the slot-ABI variant is compiled too:\n{}",
        rut_driver::ir_dump_of(&p.funcs, &p.interner)
    );
}

/// (b) a `mix64`-shaped free fn flattens into its caller: no `call` at
/// the site (one frame saved per hash).
#[test]
fn mix64_shaped_free_fn_flattens() {
    let out = compile(
        "fn mix64(bits: u64) -> u64 { return (14695981039346656037u64 ^ bits).wrapping_mul(1099511628211u64); }\n\
         fn main() -> u64 {\n\
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

/// (c) checksum parity on a trait-heavy program: bare-prim sites
/// (inlined), record sites, slot sites (`let hk: T = k`) and merged
/// origins (vtable) all answer identically. Expected checksum derived
/// from the branches: 1 + 10 + 100 + 10000 + 100000 + 1000000 +
/// 10000000 + 100000000 = 111110111 (the `h1 == h2` branch is
/// deliberately false — mix(5) != mix(9)).
#[test]
fn trait_heavy_checksum_parity() {
    let checksum = run_main(
        "trait H {\n\
         \x20   fn hash(self) -> u64;\n\
         \x20   fn hash_eq(self, other: Self) -> bool;\n\
         }\n\
         fn mix(bits: u64) -> u64 { return (bits ^ 14695981039346656037u64).wrapping_mul(1099511628211u64); }\n\
         impl H for i32 {\n\
         \x20   fn hash(self) -> u64 { return mix(self as u64); }\n\
         \x20   fn hash_eq(self, other: Self) -> bool { return self == other; }\n\
         }\n\
         impl H for u8 {\n\
         \x20   fn hash(self) -> u64 { return mix(self as u64); }\n\
         \x20   fn hash_eq(self, other: Self) -> bool { return self == other; }\n\
         }\n\
         struct Pt { x: i32 }\n\
         impl H for Pt {\n\
         \x20   fn hash(self) -> u64 { return mix(self.x as u64); }\n\
         \x20   fn hash_eq(self, other: Self) -> bool { return self.x == other.x; }\n\
         }\n\
         fn pick(k: bool) -> H {\n\
         \x20   if (k) { return 5; }\n\
         \x20   return 9u8;\n\
         }\n\
         pub fn main() -> i32 {\n\
         \x20   let mut acc: i32 = 0;\n\
         \x20   let a = 5;\n\
         \x20   if (a.hash_eq(5)) { acc += 1; }\n\
         \x20   if (!a.hash_eq(6)) { acc += 10; }\n\
         \x20   let h1 = a.hash();\n\
         \x20   let b = 9u8;\n\
         \x20   if (b.hash_eq(9u8)) { acc += 100; }\n\
         \x20   let h2 = b.hash();\n\
         \x20   if (h1 == h2) { acc += 1000; }\n\
         \x20   let p = Pt { x: 3 };\n\
         \x20   let q = Pt { x: 3 };\n\
         \x20   if (p.hash_eq(q)) { acc += 10000; }\n\
         \x20   if (!p.hash_eq(Pt { x: 4 })) { acc += 100000; }\n\
         \x20   let wa: H = a;\n\
         \x20   if (wa.hash_eq(a)) { acc += 1000000; }\n\
         \x20   if (wa.hash() == h1) { acc += 10000000; }\n\
         \x20   let v: H = pick(true);\n\
         \x20   if (v.hash_eq(5)) { acc += 100000000; }\n\
         \x20   return acc;\n\
         }\n",
    );
    assert_eq!(checksum, 111_110_111);
}

/// (d) the slot path (`let hk: T = k; hk.m()`) still works: the box is
/// materialized by the widening, the call dispatches the SLOT variant
/// (trait-object param + unbox prologue).
#[test]
fn slot_path_still_dispatches_the_slot_variant() {
    let src = format!(
        "{TINY}\
         pub fn main() -> i32 {{\n\
         \x20   let k = 5;\n\
         \x20   let hk: T = k;\n\
         \x20   return hk.m();\n\
         }}\n"
    );
    let out = compile(&src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    let main = main_of(&p);
    // the receiver crosses boxed into the slot variant
    assert!(
        main.code.iter().any(|op| matches!(op, Op::Box { .. })),
        "slot path widens the scalar:\n{}",
        rut_driver::ir_dump_of(&p.funcs, &p.interner)
    );
    let callee = main.code.iter().find_map(|op| match op {
        Op::CallM { func, .. } => Some(*func as usize),
        _ => None,
    }).expect("slot path emits callm");
    let f = &p.funcs[callee];
    assert_ne!(
        f.params.first(), Some(&TY_I32),
        "the bound variant must be the SLOT one:\n{}",
        rut_driver::ir_dump_of(&p.funcs, &p.interner)
    );
    assert_eq!(run_main(&src), 105);
}

/// (e) the extern twin: a bare-prim call whose impl lives in another
/// module binds the exporter's CONCRETE-ABI id through the surface —
/// boxless, linked, and correct.
#[test]
fn cross_module_bare_prim_call_binds_the_concrete_variant() {
    let dep = rut_driver::compile_program(
        TINY,
        Mode::Impl,
        "dep",
        1,
        &[],
    );
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    let dep = dep.program.expect("dep program");
    // the exporter compiled and published both variants
    assert_eq!(dep.surface.impls.len(), 1, "one impl registered");
    assert!(
        dep.surface.impls[0].methods_concrete.iter().any(|(n, _)| dep.name_of(*n) == "m"),
        "the concrete-ABI id is published: {:?}",
        dep.surface.impls[0]
    );
    let surface = dep.surface.clone();

    let app = rut_driver::compile_program(
        "use dep::{T};\n\
         pub fn main() -> i32 {\n\
         \x20   let w = 5;\n\
         \x20   return w.m();\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface)],
    );
    assert!(app.diags.is_empty(), "{:?}", app.diags);
    let app = app.program.expect("app program");
    assert!(
        main_of(&app).code.iter().all(|op| !matches!(op, Op::Box { .. })),
        "cross-module bare-prim call is boxless:\n{}",
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
    assert_eq!(
        linked.funcs[callee].params.first(), Some(&TY_I32),
        "the linked callee is the dep's concrete variant:\n{}",
        rut_driver::ir_dump_of(&linked.funcs, &linked.interner)
    );
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
