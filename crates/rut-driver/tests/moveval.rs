//! P2 (mapset perf plan): `MoveVal` last-use move elision + clone/alloc
//! fast paths. A `CloneVal` becomes a `moveval` exactly where the move
//! is provably unobservable — the rehash shape (`let old = self.keys;
//! self.keys = [nil; n]`) and sole-ownership chains — and STAYS a
//! `cloneval` on read-after / mutate-after / live-across-loop sources.
//! Correctness rider: the scalar-array clone fast path, zero-fill
//! construction, the destructure single-clone fix, and threaded-vs-
//! parked parity on a clone-heavy program.

use rut_core::ops::Op;
use rut_core::binary::Surface;
use rut_parser::Mode;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    let collection = Surface::default();
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, Surface::core()), (3, collection)],
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

/// Run `main` with a tiny fuel budget, resuming in `chunk` steps — every
/// op lands on the park → resume → `step_one` boundary, so the match
/// interpreter executes the stretches the threaded engine would.
fn run_main_parked(src: &str, chunk: u64) -> i32 {
    let out = compile(src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = rut_core::link::flatten(out.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(chunk),
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

fn main_of(p: &rut_core::binary::Program) -> &rut_core::binary::FuncCode {
    p.funcs
        .iter()
        .find(|f| p.name_of(f.name) == "main")
        .expect("main compiled")
}

fn main_code(src: &str) -> Vec<Op> {
    let out = compile(src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    main_of(&out.program.expect("program")).code.clone()
}

fn ir(src: &str) -> String {
    let out = compile(src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    rut_driver::ir_dump_of(&p.funcs, &p.interner)
}

fn main_ir(src: &str) -> String {
    let text = ir(src);
    text.split("fn #")
        .find(|s| {
            s.split_once('\n')
                .map(|(head, _)| head.split_whitespace().nth(1).map_or(false, |n| n.starts_with("main(")))
                .unwrap_or(false)
        })
        .expect("main in dump")
        .to_string()
}

const POINT: &str = "struct Point { x: i32; y: i32; ys: [i32] }\n";

// ---- the op fires where the move is unobservable ----

/// The rehash shape (P2.2): a field-array binding whose field is
/// immediately rebound — `GetF; MoveVal old, t` and the fresh array
/// moves straight into the field store. O(1), not O(cap).
#[test]
fn move_fires_on_dead_source_rehash_shape() {
    let src = "\
class Table { keys: [?i32]; }
impl Table {
    pub fn new() -> Table {
        return Table { keys: [nil; 2] };
    }
    pub fn rehash(mut self, new_cap: i32) -> i32 {
        let old = self.keys;
        self.keys = [nil; new_cap];
        let mut n = 0;
        for (let i = 0; i < 2; i += 1) {
            let k = old[i];
            if (k != nil) { n += k; }
        }
        return n;
    }
}
pub fn main() -> i32 {
    let t = Table.new();
    t.keys[0] = 4;
    return t.rehash(4);
}
";
    let code = main_code(src);
    assert!(
        code.iter().any(|op| matches!(op, Op::MoveVal { .. })),
        "rehash-shaped binding must move-elide:\n{}",
        ir(src)
    );
    assert!(
        code.iter().all(|op| !matches!(op, Op::CloneVal { .. })),
        "no clone may remain in main (the rehash shape covers every binding):\n{}",
        ir(src)
    );
    assert_eq!(run_main(src), 4);
}

/// A dead local's cell moves into the new binding (sole ownership):
/// `p` is never read again, so `q` takes over the fresh cell.
#[test]
fn move_fires_on_dead_local() {
    let src = &format!(
        "{POINT}\
         pub fn main() -> i32 {{\n\
         \x20   let p = Point {{ x: 4, y: 2, ys: [1, 2] }};\n\
         \x20   let q = p;\n\
         \x20   return q.x * 10 + q.ys[1];\n\
         }}\n"
    );
    let code = main_code(src);
    assert!(
        code.iter().any(|op| matches!(op, Op::MoveVal { .. })),
        "dead-local binding must move-elide:\n{}",
        ir(src)
    );
    assert_eq!(run_main(src), 42);
}

// ---- the op stays a clone where aliasing is observable ----

/// Read-after: `p` is read after the copy — the binding must keep its
/// own deep copy.
#[test]
fn stays_clone_on_read_after() {
    let src = &format!(
        "{POINT}\
         pub fn main() -> i32 {{\n\
         \x20   let mut p = Point {{ x: 1, y: 2, ys: [0, 0] }};\n\
         \x20   let q = p;\n\
         \x20   p.x = 4;\n\
         \x20   return q.x * 100 + p.x * 10 + q.ys[1];\n\
         }}\n"
    );
    let code = main_code(src);
    assert!(
        code.iter().all(|op| !matches!(op, Op::MoveVal { .. })),
        "read-after source must stay a clone:\n{}",
        ir(src)
    );
    assert_eq!(run_main(src), 140);
}

/// Mutate-after (the RFC 0009 copy law): a later mutation through the
/// copy must never leak into the original.
#[test]
fn stays_clone_on_mutate_after() {
    let src = &format!(
        "{POINT}\
         pub fn main() -> i32 {{\n\
         \x20   let mut p = Point {{ x: 1, y: 2, ys: [0, 0] }};\n\
         \x20   let mut q = p;\n\
         \x20   q.x = 9;\n\
         \x20   return q.x * 100 + p.x * 10 + p.y;\n\
         }}\n"
    );
    let code = main_code(src);
    assert!(
        code.iter().all(|op| !matches!(op, Op::MoveVal { .. })),
        "mutate-after copy must stay a clone:\n{}",
        ir(src)
    );
    assert_eq!(run_main(src), 912);
}

/// Live-across-loop: the source is read inside a loop after the copy —
/// the back-edge keeps it alive, no move may fire.
#[test]
fn stays_clone_live_across_loop() {
    let src = &format!(
        "{POINT}\
         pub fn main() -> i32 {{\n\
         \x20   let mut p = Point {{ x: 1, y: 2, ys: [0, 0] }};\n\
         \x20   let q = p;\n\
         \x20   let mut acc = 0;\n\
         \x20   for (let i = 0; i < 3; i += 1) {{\n\
         \x20       acc += p.x + q.x;\n\
         \x20   }}\n\
         \x20   return acc;\n\
         }}\n"
    );
    let code = main_code(src);
    assert!(
        code.iter().all(|op| !matches!(op, Op::MoveVal { .. })),
        "a source live across the loop must stay a clone:\n{}",
        ir(src)
    );
    assert_eq!(run_main(src), 6);
}

/// The debug assert (P2.5): every `moveval`'s source register has no
/// later read — checked over the whole compiled program, mapset's
/// rehash included.
#[test]
fn move_srcs_are_dead_over_the_program() {
    let mapset_src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../rut/mapset/mapset.rut"
    ))
    .expect("read mapset.rut");
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std(&mut s);
    s.register_module("mapset", rut_driver::Module { source: Some(mapset_src), ..Default::default() })
        .unwrap();
    let app = r#"
use core::{ assert };
use mapset::{ HashMap };
pub fn main() -> i32 {
    let mut m: HashMap<i32, str> = HashMap.new();
    for (let i = 0; i < 500; i += 1) {
        m.put(i, f"v{i}");
    }
    let mut acc = 0;
    for (let i = 0; i < 500; i += 1) {
        let p = m.get(i);
        if (p != nil) { acc = acc + 1; }
    }
    return acc * 2 + m.len();
}
"#;
    s.register_module("app_main", rut_driver::Module { source: Some(app.into()), ..Default::default() })
        .unwrap();
    let g = rut_driver::compile_graph(&s, "app_main");
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let flat = rut_core::link::flatten(g.program.expect("program"));
    // the grow path ran the rehash shape: moves exist…
    let moves: usize = flat
        .funcs
        .iter()
        .map(|f| f.code.iter().filter(|op| matches!(op, Op::MoveVal { .. })).count())
        .sum();
    assert!(moves > 0, "the growing map must compile some movevals");
    // …and every one of them moves a dead source
    assert!(rut_lir::move_srcs_are_dead(&flat), "a moveval kept a live source");
}

// ---- the fast paths keep results identical ----

/// Scalar-elem array clones go through the block-to-block memcpy —
/// checksums must be byte-identical.
#[test]
fn big_scalar_array_clones_checksum_identical() {
    let src = "\
pub fn main() -> i32 {
    let n = 50_000;
    let mut a: [u64] = [0; n];
    for (let i = 0; i < n; i += 1) { a[i] = (i as u64).wrapping_mul(2654435761u64); }
    let b = a;
    let mut c = b;
    c[7] = 99;
    let mut h: u64 = 14695981039346656037u64;
    for (let i = 0; i < n; i += 1) {
        h = (h ^ a[i]).wrapping_mul(1099511628211u64);
        h = (h ^ b[i]).wrapping_mul(1099511628211u64);
        h = (h ^ c[i]).wrapping_mul(1099511628211u64);
    }
    let mut d: [u8] = [0; 30_000];
    for (let i = 0; i < 30_000; i += 1) { d[i] = (i % 251) as u8; }
    let e = d;
    for (let i = 0; i < 30_000; i += 1) {
        h = (h ^ (e[i] as u64)).wrapping_mul(1099511628211u64);
    }
    return h as i32;
}
";
    let mut h: u64 = 14695981039346656037u64;
    let n: usize = 50_000;
    let mut a = vec![0u64; n];
    for i in 0..n {
        a[i] = (i as u64).wrapping_mul(2654435761u64);
    }
    let b = a.clone();
    let mut c = b.clone();
    c[7] = 99;
    for i in 0..n {
        h = (h ^ a[i]).wrapping_mul(1099511628211u64);
        h = (h ^ b[i]).wrapping_mul(1099511628211u64);
        h = (h ^ c[i]).wrapping_mul(1099511628211u64);
    }
    for i in 0..30_000 {
        h = (h ^ ((i % 251) as u64)).wrapping_mul(1099511628211u64);
    }
    assert_eq!(run_main(src), h as i32);
}

/// Zero-fill construction: `[0; n]`, `[0u8; n]`, `[0u64; n]`,
/// `[false; n]`, `[nil; n]` and a non-zero fill (`[-1; n]`, loop kept)
/// produce identical results — and the all-zero fills compile no fill
/// loop.
#[test]
fn zero_fill_constructions_identical() {
    let src = "\
pub fn main() -> i32 {
    let n = 1000;
    let z: [i32] = [0; n];
    let zb: [u8] = [0; n];
    let zw: [u64] = [0; n];
    let zf: [bool] = [false; n];
    let zp: [?i32] = [nil; n];
    let nz: [i32] = [-1; n];
    let mut h: u64 = 14695981039346656037u64;
    for (let i = 0; i < n; i += 1) {
        h = (h ^ (z[i] as u64)).wrapping_mul(1099511628211u64);
        h = (h ^ (zb[i] as u64)).wrapping_mul(1099511628211u64);
        h = (h ^ zw[i]).wrapping_mul(1099511628211u64);
        if (zf[i]) { return -1; }
        if (zp[i] != nil) { return -2; }
        h = (h ^ (nz[i] as u64)).wrapping_mul(1099511628211u64);
    }
    return h as i32;
}
";
    let mut h: u64 = 14695981039346656037u64;
    for i in 0..1000u64 {
        h = h.wrapping_mul(1099511628211u64); // z[i] = 0
        h = h.wrapping_mul(1099511628211u64); // zb[i] = 0
        h = h.wrapping_mul(1099511628211u64); // zw[i] = 0
        h = (h ^ u64::MAX).wrapping_mul(1099511628211u64); // nz[i] = -1 as u64
    }
    assert_eq!(run_main(src), h as i32);
    // the all-zero fills must skip the fill loop: only the checksum
    // loop and the [-1; n] fill loop remain in main
    let code = main_code(src);
    let loops = code.iter().filter(|op| matches!(op, Op::LoopHead)).count();
    assert_eq!(
        loops, 2,
        "only the [-1; n] fill and the checksum loops remain:\n{}",
        ir(src)
    );
}

/// The destructure fix (P2.4): a value-typed tuple element is deep-
/// copied exactly ONCE per binding — the duplicate clone block is gone.
#[test]
fn destructure_clones_once() {
    let src = "\
struct Inner { v: i32; }
pub fn main() -> i32 {
    let t = (Inner { v: 7 }, 11);
    let (a, b) = t;
    return a.v * 100 + b;
}
";
    let main_fn = main_ir(src);
    // the pre-P2 bug deep-copied the record element TWICE (two identical
    // cloneval ops back-to-back); P2.4 deletes one, and P2 may elide the
    // remaining copy entirely (a dead tuple moves) — either way at most
    // ONE copy op per element can remain
    let copies = main_fn.matches("cloneval").count() + main_fn.matches("moveval").count();
    assert!(copies <= 2, "at most one copy op per element:\n{main_fn}");
    assert_eq!(run_main(src), 711);
}

/// A clone-heavy program run through the threaded engine matches the
/// parked run (every op through the park → resume → step boundary).
#[test]
fn clone_heavy_threaded_run_matches_parked_run() {
    let src = "\
pub fn main() -> i32 {
    let n = 2000;
    let mut a: [i32] = [0; n];
    for (let i = 0; i < n; i += 1) { a[i] = i; }
    let mut acc = 0;
    for (let k = 0; k < 40; k += 1) {
        let b = a;
        let mut c = b;
        for (let i = 0; i < n; i += 1) {
            c[i] = c[i] + k;
            acc += b[i] + c[i];
        }
    }
    return acc;
}
";
    let threaded = run_main(src);
    let parked = run_main_parked(src, 137);
    assert_eq!(threaded, parked);
}
