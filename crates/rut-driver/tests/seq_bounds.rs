//! P3 (mapset perf plan): the array element access fast path in
//! `seq_get`/`seq_set` — the `Array` arm comes first, ONE unsigned
//! compare folds the negative and overflow checks, then a direct
//! unchecked element read/write. Bounds behavior and trap text are
//! preserved (a negative index still prints the negative i64, window
//! traps keep their `view index …` text), every element kind decodes
//! identically through the unchecked accessors, windows keep their
//! write-through semantics, and threaded-vs-parked parity holds on a
//! hot index loop.

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

fn make_vm(src: &str) -> rut_vm::interp::Vm {
    make_vm_with_fuel(src, 50_000_000)
}

fn make_vm_with_fuel(src: &str, fuel: u64) -> rut_vm::interp::Vm {
    let out = compile(src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = rut_core::link::flatten(out.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(fuel),
        heap_limit_bytes: Some(64 * 1024 * 1024),
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

/// Compile, flatten, verify, and run a single-module `main` returning i32.
fn run_main(src: &str) -> i32 {
    make_vm(src).call::<_, i32>("main", ()).expect("run")
}

/// Run `main` with a tiny fuel budget, resuming in `chunk` steps — every
/// op lands on the park → resume → `step_one` boundary, so the match
/// interpreter executes the stretches the threaded engine would.
fn run_main_parked(src: &str, chunk: u64) -> i32 {
    let mut vm = make_vm_with_fuel(src, chunk);
    match vm.call::<_, i32>("main", ()) {
        Ok(v) => return v,
        Err(t) => assert_eq!(t.name(), "OutOfFuel", "{}", t.msg),
    }
    loop {
        vm.add_fuel(chunk);
        match vm.resume::<i32>() {
            Ok(v) => return v,
            Err(t) => assert_eq!(t.name(), "OutOfFuel", "{}", t.msg),
        }
    }
}

/// Run `main` expecting an index trap; the trap is the result.
fn run_main_trap(src: &str) -> rut_vm::Trap {
    match make_vm(src).call::<_, i32>("main", ()) {
        Ok(_) => panic!("expected a trap:\n{src}"),
        Err(t) => t,
    }
}

// ---- the std-enabled path (Vec windows live in pouch) ----

fn compile_std(src: &str) -> rut_core::binary::Program {
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std(&mut s);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    rut_driver::mount_dir(&mut s, &root.join("rut/pouch")).expect("mount pouch");
    let out = rut_driver::compile_module_in(&mut s, src, rut_parser::Mode::Impl, "test");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    rut_core::binary::decode(&out.binary.expect("binary")).expect("decode")
}

fn run_std(src: &str) -> i32 {
    let prog = compile_std(src);
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(50_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
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

fn run_std_trap(src: &str) -> rut_vm::Trap {
    let prog = compile_std(src);
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(50_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    match vm.call::<_, i32>("main", ()) {
        Ok(_) => panic!("expected a trap:\n{src}"),
        Err(t) => t,
    }
}

// ---- the fast path reads and writes every element kind identically ----

/// The unchecked accessors must decode/encode every `ArrKind` exactly as
/// the checked `get`/`set` did — checksum against a Rust reference.
#[test]
fn in_bounds_get_set_all_elem_kinds_match_reference() {
    let src = "\
pub fn main() -> i32 {
    let n = 400;
    let mut a: [i32] = [0; n];
    let mut b: [u8] = [0; n];
    let mut c: [u64] = [0; n];
    let mut f: [bool] = [false; n];
    for (let i = 0; i < n; i += 1) {
        a[i] = i * 3 + 1;
        b[i] = (i % 251) as u8;
        c[i] = (i as u64).wrapping_mul(11400714819323198485u64);
        f[i] = i % 2 == 0;
    }
    let mut h: u64 = 14695981039346656037u64;
    for (let i = 0; i < n; i += 1) {
        h = (h ^ (a[i] as u64)).wrapping_mul(1099511628211u64);
        h = (h ^ (b[i] as u64)).wrapping_mul(1099511628211u64);
        h = (h ^ c[i]).wrapping_mul(1099511628211u64);
        if (f[i]) { h = (h ^ 255u64).wrapping_mul(1099511628211u64); }
    }
    return h as i32;
}
";
    let n: i64 = 400;
    let mut h: u64 = 14695981039346656037;
    for i in 0..n {
        let a = (i * 3 + 1) as u64;
        let b = (i % 251) as u64;
        let c = (i as u64).wrapping_mul(11400714819323198485);
        h = (h ^ a).wrapping_mul(1099511628211);
        h = (h ^ b).wrapping_mul(1099511628211);
        h = (h ^ c).wrapping_mul(1099511628211);
        if i % 2 == 0 {
            h = (h ^ 255).wrapping_mul(1099511628211);
        }
    }
    assert_eq!(run_main(src), h as i32);
}

/// The Slots kind (ref elements) through the same path: store, read,
/// dereference — and the retain/release discipline stays balanced.
#[test]
fn pointer_array_get_set_matches_reference() {
    let src = "\
pub fn main() -> i32 {
    let n = 64;
    let mut p: [?i32] = [nil; n];
    for (let i = 0; i < n; i += 1) {
        p[i] = i * 10 + 3;
    }
    let mut s = 0;
    for (let i = 0; i < n; i += 1) {
        let q = p[i];
        if (q != nil) { s += q; }
    }
    p[5] = nil;
    let mut t = 0;
    for (let i = 0; i < n; i += 1) {
        let q = p[i];
        if (q == nil) { t += 1; }
    }
    return s * 10 + t;
}
";
    let s: i64 = (0..64).map(|i| i * 10 + 3).sum();
    assert_eq!(run_main(src), (s * 10 + 1) as i32);
}

// ---- bounds traps: one folded check, messages preserved ----

/// A negative index still traps with the NEGATIVE i64 printed (never a
/// wrapped usize) — get side.
#[test]
fn negative_get_traps_with_negative_i() {
    let src = "\
pub fn main() -> i32 {
    let a: [i32] = [7, 8, 9];
    let i = 0 - 3;
    return a[i];
}
";
    let t = run_main_trap(src);
    assert_eq!(t.kind, rut_vm::TrapKind::IndexOutOfBounds);
    assert_eq!(t.msg, "array index -3 out of bounds (len 3)", "{}", t.msg);
}

/// …and set side.
#[test]
fn negative_set_traps_with_negative_i() {
    let src = "\
pub fn main() -> i32 {
    let mut a: [i32] = [7, 8, 9];
    let i = 0 - 3;
    a[i] = 1;
    return 0;
}
";
    let t = run_main_trap(src);
    assert_eq!(t.kind, rut_vm::TrapKind::IndexOutOfBounds);
    assert_eq!(t.msg, "index -3 out of bounds (len 3)", "{}", t.msg);
}

/// `i == len` traps on both sides, and an i64-huge index (which would
/// wrap into range under any narrower check) traps too.
#[test]
fn overflow_get_set_trap() {
    let get = "\
pub fn main() -> i32 {
    let a: [i32] = [7, 8, 9];
    return a[3];
}
";
    let t = run_main_trap(get);
    assert_eq!(t.kind, rut_vm::TrapKind::IndexOutOfBounds);
    assert_eq!(t.msg, "array index 3 out of bounds (len 3)", "{}", t.msg);

    let set = "\
pub fn main() -> i32 {
    let mut a: [i32] = [7, 8, 9];
    a[3] = 1;
    return 0;
}
";
    let t = run_main_trap(set);
    assert_eq!(t.kind, rut_vm::TrapKind::IndexOutOfBounds);
    assert_eq!(t.msg, "index 3 out of bounds (len 3)", "{}", t.msg);

    let huge = "\
pub fn main() -> i32 {
    let a: [i32] = [7, 8, 9];
    let i = 2147483647;
    return a[i];
}
";
    let t = run_main_trap(huge);
    assert_eq!(t.kind, rut_vm::TrapKind::IndexOutOfBounds);
    assert_eq!(t.msg, "array index 2147483647 out of bounds (len 3)", "{}", t.msg);
}

// ---- windows: the ArrView arms are unchanged ----

/// Window reads and write-through writes keep working; window bounds are
/// vs the window (with the `view index …` text), negative included.
#[test]
fn window_get_set_write_through_unchanged() {
    let src = "\
use pouch::{ Vec };
pub fn main() -> i32 {
    let mut v: Vec<i32> = Vec.new();
    v.push(1); v.push(2); v.push(3);
    let mut w = v.slice(1, 3);
    let x = w[0] + w[1];
    w[0] = 40;
    return x * 1000 + v[1] * 10 + w[1];
}
";
    // x = 5, write-through makes v[1] = 40, w[1] = 3 → 5000 + 400 + 3
    assert_eq!(run_std(src), 5403);
}

#[test]
fn window_bounds_traps_unchanged() {
    let src = "\
use pouch::{ Vec };
pub fn main() -> i32 {
    let mut v: Vec<i32> = Vec.new();
    v.push(1); v.push(2);
    let w = v.slice(0, 2);
    return w[5];
}
";
    let t = run_std_trap(src);
    assert_eq!(t.kind, rut_vm::TrapKind::IndexOutOfBounds);
    assert_eq!(t.msg, "view index 5 out of bounds (len 2)", "{}", t.msg);

    let neg = src.replace("w[5]", "w[0 - 1]");
    let t = run_std_trap(&neg);
    assert_eq!(t.kind, rut_vm::TrapKind::IndexOutOfBounds);
    assert_eq!(t.msg, "view index -1 out of bounds (len 2)", "{}", t.msg);
}

// ---- the fused field paths and parity ----

/// `Vec`-shaped hot loops go through the fused `ArrGetF`/`ArrSetF`
/// (field-array) path, which funnels into the same fast `seq_get`/
/// `seq_set` — a sieve checksums identically to the reference count.
#[test]
fn sieve_shape_over_field_arrays_checksums() {
    let src = "\
use pouch::{ Vec };
pub fn main() -> i32 {
    let limit: i32 = 10_000;
    let mut marks = Vec<u8>.filled(0, limit + 1);
    let mut count = 0;
    for (let i = 2; i <= limit; i += 1) {
        if (marks[i] == 0) {
            count += 1;
            if (i <= limit / i) {
                let mut m = i * i;
                while (m <= limit) {
                    marks[m] = 1;
                    m += i;
                }
            }
        }
    }
    return count;
}
";
    // π(10000) = 1229
    assert_eq!(run_std(src), 1229);
}

/// A hot index loop run through the threaded engine matches the parked
/// run (every op through the park → resume → step boundary).
#[test]
fn hot_index_loop_threaded_matches_parked() {
    let src = "\
pub fn main() -> i32 {
    let n = 3000;
    let mut a: [i32] = [0; n];
    for (let i = 0; i < n; i += 1) { a[i] = i; }
    let mut acc = 0;
    for (let k = 0; k < 60; k += 1) {
        for (let i = 0; i < n; i += 1) {
            a[i] = a[i] + 1;
            acc += a[i];
        }
    }
    return acc + a[n - 1];
}
";
    let threaded = run_main(src);
    let parked = run_main_parked(src, 137);
    assert_eq!(threaded, parked);
}
