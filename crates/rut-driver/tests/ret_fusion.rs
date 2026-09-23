//! The return-position destructure fusion (err-channel phase 1): a
//! `MakeRecord` handed off through single-def `mov`/`movref` copies and
//! read only by destructuring `GetF`s is consumed — the components pass
//! through their existing registers, the mint and the handoff die, and
//! each `GetF` becomes the move it always was semantically (`Mov` for a
//! prim field — the pair unboxes entirely; `MovRef` for a ref field — one
//! retain at the handoff, no record cell). Pins: destructured pairs
//! elide, escaping tuples still mint (the returned and stored classes —
//! the census's 16%), the boxed `?T` mint is untouched, the multi-writer
//! join slot declines, and every pin rides a value check (values
//! identical; only cells not minted).

use std::rc::Rc;

use rut_core::binary::Surface;
use rut_parser::Mode;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    let collection = Surface::default();
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, Surface::core(), "core".to_string()), (3, collection, "collection".to_string())],
    )
}

/// Compile, flatten (RFC 0035 §1), verify, and run a single-module
/// `main` returning i64.
fn run_main(src: &str) -> i64 {
    let out = compile(src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = rut_core::link::flatten(out.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(50_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    vm.call::<_, i64>("main", ()).expect("run")
}

/// The linked program's IR dump (lowering evidence).
fn ir_dump(src: &str) -> String {
    let out = compile(src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = out.program.expect("program");
    rut_driver::ir_dump_of(&prog.funcs, &prog.interner)
}

// ---- the elision: destructured pairs never mint ------------------------

/// The checkedadd row's exact shape — a helper fn returning
/// `a.checked_add(b)` (the callee/caller pair every `(T, err)` return
/// rides), destructured at the call site with both lanes load-bearing.
/// After the inliner splices the body, the mint → ret-slot `movref` →
/// 2×`getf` window is one op stream and the fusion consumes it: no
/// `makerecord` survives, and the checksum is the Rust model's, bit for
/// bit (pure lowering — values identical, cells not minted).
#[test]
fn destructured_checked_pair_elides() {
    let src = r#"
        fn pair(a: i64, b: i64) -> (i64, bool) {
            return a.checked_add(b);
        }
        pub fn main() -> i64 {
            let b: i64 = 9223372036854774807i64;
            let mut s: i64 = 0;
            let mut bad: i64 = 0;
            for (let i = 0; i < 4000; i += 1) {
                let a: i64 = ((i % 2048) - 1024) as i64;
                let (v, ok) = pair(a, b);
                s = s.wrapping_add(v);
                if (ok == false) {
                    bad = bad.wrapping_add(1);
                }
            }
            return s.wrapping_add(bad);
        }
    "#;
    let dump = ir_dump(src);
    assert!(
        !dump.contains("makerecord"),
        "the destructured (i64, bool) pair must not mint:\n{dump}"
    );
    // the value model, wrapping i64 exactly as the VM's slots carry it
    let (b, mut s, mut bad) = (9223372036854774807i64, 0i64, 0i64);
    for i in 0..4000i64 {
        let a = (i % 2048) - 1024;
        let v = a.wrapping_add(b);
        if a.checked_add(b).is_none() {
            bad = bad.wrapping_add(1);
        }
        s = s.wrapping_add(v);
    }
    assert_eq!(run_main(src), s.wrapping_add(bad));
}

/// The binding handoff — the pair bound to a name, then destructured —
/// is the same single-def chain one hop longer, and elides too.
#[test]
fn bound_pair_destructured_elides() {
    let src = r#"
        fn pair(a: i64, b: i64) -> (i64, bool) {
            return a.checked_add(b);
        }
        pub fn main() -> i64 {
            let t = pair(5, 3);
            let (v, ok) = t;
            if (ok == false) {
                return 0 - 1;
            }
            return v;
        }
    "#;
    let dump = ir_dump(src);
    assert!(
        !dump.contains("makerecord"),
        "the bound-then-destructured pair must not mint:\n{dump}"
    );
    assert_eq!(run_main(src), 8);
}

/// A ref-typed component costs ONE retain at the handoff (`MovRef`), no
/// record cell — the `(T, str)` economy of every parser return. Single
/// return (the two-return shape is the multi-writer join, pinned below);
/// both inlined call sites fuse; the strings' cells are shared, never
/// copied.
#[test]
fn ref_component_pair_elides_to_one_retain_per_read() {
    let src = r#"
        fn tag(n: i64) -> (i64, str) {
            let mut name: str = "neg";
            if (n > 0) {
                name = "pos";
            }
            return (n, name);
        }
        pub fn main() -> i64 {
            let (v, name) = tag(7);
            let (w, other) = tag(0i64 - 4);
            return v + w + name.len() as i64 + other.len() as i64;
        }
    "#;
    let dump = ir_dump(src);
    assert!(
        !dump.contains("makerecord"),
        "the (i64, str) pairs must not mint — the str handoff is a movref:\n{dump}"
    );
    assert!(dump.contains("movref"), "str components cross as refs:\n{dump}");
    assert_eq!(run_main(src), 7 + (0 - 4) + 3 + 3);
}

/// The 3+-field return family (the census's ~15%) fuses by the same walk.
#[test]
fn three_field_return_family_elides() {
    let src = r#"
        fn stats(a: i64, b: i64) -> (i64, i64, i64) {
            return (a.wrapping_add(b), a.wrapping_sub(b), a.wrapping_mul(b));
        }
        pub fn main() -> i64 {
            let (s, d, p) = stats(7, 3);
            return s + d + p;
        }
    "#;
    let dump = ir_dump(src);
    assert!(
        !dump.contains("makerecord"),
        "the destructured 3-field tuple must not mint:\n{dump}"
    );
    assert_eq!(run_main(src), 10 + 4 + 21);
}

// ---- the escapes: genuine records still mint ---------------------------

/// A pair genuinely RETURNED (a callee too big to inline) still mints —
/// the record crosses the real frame boundary (the census's
/// return-family `returned directly` class, 105 mints).
#[test]
fn returned_pair_from_a_non_inlined_callee_still_mints() {
    let src = r#"
        fn big(a: i64, b: i64) -> (i64, bool) {
            let t01 = a.wrapping_add(b);
            let t02 = t01.wrapping_add(b);
            let t03 = t02.wrapping_add(b);
            let t04 = t03.wrapping_add(b);
            let t05 = t04.wrapping_add(b);
            let t06 = t05.wrapping_add(b);
            let t07 = t06.wrapping_add(b);
            let t08 = t07.wrapping_add(b);
            let t09 = t08.wrapping_add(b);
            let t10 = t09.wrapping_add(b);
            let t11 = t10.wrapping_add(b);
            let t12 = t11.wrapping_add(b);
            let t13 = t12.wrapping_add(b);
            let t14 = t13.wrapping_add(b);
            let t15 = t14.wrapping_add(b);
            let t16 = t15.wrapping_add(b);
            let t17 = t16.wrapping_add(b);
            let t18 = t17.wrapping_add(b);
            let t19 = t18.wrapping_add(b);
            let t20 = t19.wrapping_add(b);
            let t21 = t20.wrapping_add(b);
            let t22 = t21.wrapping_add(b);
            let t23 = t22.wrapping_add(b);
            let t24 = t23.wrapping_add(b);
            let t25 = t24.wrapping_add(b);
            return a.checked_add(b);
        }
        pub fn main() -> i64 {
            let (v, ok) = big(2, 3);
            if (ok == false) {
                return 0 - 1;
            }
            return v;
        }
    "#;
    let dump = ir_dump(src);
    assert!(
        dump.contains("makerecord"),
        "the non-inlined callee's pair escapes through `ret` — it must mint:\n{dump}"
    );
    assert_eq!(run_main(src), 5);
}

/// A pair passed as a CALL ARGUMENT escapes — the identity-observing
/// disqualifier holds through the fusion (the recursive sink is never
/// inlined, so the argument record is real).
#[test]
fn pair_into_a_call_argument_still_mints() {
    let src = r#"
        fn eat(t: (i64, bool), n: i32) -> i64 {
            if (n <= 0) {
                let (v, ok) = t;
                if (ok == false) {
                    return 0 - 1;
                }
                return v;
            }
            return eat(t, n - 1);
        }
        pub fn main() -> i64 {
            return eat((41i64, true), 2);
        }
    "#;
    let dump = ir_dump(src);
    assert!(
        dump.contains("makerecord"),
        "the call-argument pair escapes — it must mint:\n{dump}"
    );
    assert_eq!(run_main(src), 41);
}

/// The multi-writer join slot declines the whole window (the risk note:
/// fusible chains must stay single-def — a two-return inlined callee
/// writes its result slot twice, so both mints survive).
#[test]
fn two_return_join_declines_and_still_mints() {
    let src = r#"
        fn pick(n: i64) -> (i64, bool) {
            if (n > 0) {
                return (1i64, true);
            }
            return (0i64 - 1i64, false);
        }
        pub fn main() -> i64 {
            let (v, ok) = pick(3);
            if (ok == false) {
                return 0 - 100;
            }
            return v;
        }
    "#;
    let dump = ir_dump(src);
    assert!(
        dump.contains("makerecord"),
        "the join slot has two writers — both mints must survive:\n{dump}"
    );
    assert_eq!(run_main(src), 1);
}

/// The boxed `?T` mint (the census's boxed-payload class) is untouched by
/// the fusion: every `T → ?T` coercion still boxes.
#[test]
fn boxed_opt_mint_unchanged() {
    let src = r#"
        pub fn main() -> i64 {
            let x: ?i64 = 5;
            let mut s: i64 = 0;
            if (x != nil) {
                s = s.wrapping_add(x);
            }
            let y: ?i64 = nil;
            if (y != nil) {
                s = s.wrapping_add(1000);
            }
            return s;
        }
    "#;
    let dump = ir_dump(src);
    // exactly one box: `x: ?i64 = 5` coerces through `makeopt`; `= nil`
    // needs none (the null slot IS nil — no cell for absence)
    assert_eq!(
        dump.matches("makeopt").count(),
        1,
        "the `T → ?T` coercion still boxes and the fusion touches no opt mint:\n{dump}"
    );
    assert_eq!(run_main(src), 5);
}
