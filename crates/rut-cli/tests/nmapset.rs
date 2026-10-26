//! nmapset e2e — `for..of` over `HashMap`/`HashSet` (the `[iterable]`
//! lane) through the TREE lane: the consumer mounts `rut/nmapset` as
//! source (not the committed `dist/std` bundle — those bytes move only
//! at the std cut), `rut_std::nmap::pkg()` binds the crossings, and
//! every case runs compile → verify → interpret.
//!
//! Covered per the plan's laws: the keys walk (a map's element type IS
//! `K`, values ride `get`'s stored-cell aliasing law), birth order
//! (insertion order of live entries; a remove-then-reinsert moves the
//! key to the end), THE SNAPSHOT LAW (puts/removes inside the body
//! never touch the walk), `break` (emit false = stop pulling), the
//! fresh-copy law is unobservable at prim/str/bytes (content equality)
//! but the widths are NOT — an `i8` key reads `-5`, a `u64` key keeps
//! its high bit (the mint's `type_id<K>()` law), empty tables walk zero
//! times, the `mut` binding form compiles, and the aliasing law rides
//! iteration end to end (writes through `get(k)` inside the loop hit
//! the map's stored cell).

use rut_driver::Mounted;
use rut_vm::interp::{HostRegistry, Limits, Vm};
use std::path::PathBuf;
use std::rc::Rc;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Mount `rut/nmapset` from the tree (the dep closure pulls
/// `rut/nmap_host`'s decl surface through the manifest's relative path).
fn setup(src: &str) -> Vm {
    let base = root();
    let mut world = rut_native::dir_pkgs(&base.join("rut/nmapset"))
        .expect("mount nmapset")
        .pkgs;
    world
        .push(Mounted::source("app", src).manifest(rut_driver::Manifest::lib("app", &["nmapset"])));
    let ctx = rut_driver::host_pkg_ctx(&world);
    let compiled = rut_driver::Walk {
        pkgs: world,
        root: String::new(),
    }
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
    .entrypoint("app")
    .compile()
    .unwrap();
    assert!(
        compiled.graph.diags.is_empty(),
        "diags:\n{}",
        compiled
            .graph
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
    let prog = compiled.graph.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let mut hosts = HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::nmap::pkg());
    hosts.verify_against(&ctx.flatten());
    let limits = Limits {
        fuel: Some(8_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    Vm::builder()
        .program(Rc::new(flat))
        .limits(limits)
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(hosts)
        .build()
        .expect("vm")
}

const SRC: &str = r#"
use nmapset::{ HashMap, HashSet };

// the capture law (the for-of body IS the synthetic emit closure):
// scalars copy in, so every accumulator is a shared record cell — the
// impls.rs iterable test's `?Acc` spelling
struct W { n: i32 = 0; total: i32 = 0; order: [i32]; }
struct S { n: i32 = 0; first: str = ""; }
struct F { n: i32 = 0; sum: i64 = 0; neg: i8 = 0; hi: bool = false; got: i32 = 0; }

// the keys walk: birth order, values through get (the stored-cell law)
entry fn map_walk() -> i64 {
    let mut m = HashMap<str, i32>::new();
    m.put("ada", 36);
    m.put("linus", 55);
    m.put("grace", 45);
    let mut fails: i64 = 0;
    let mut w: ?W = W { order: [0, 0, 0] };
    for (let k of m) {
        w.total += m.get(k);
        if (k == "ada") { w.order[w.n] = 1; }
        else if (k == "linus") { w.order[w.n] = 2; }
        else { w.order[w.n] = 3; }
        w.n += 1;
    }
    if (w.n != 3) { fails += 1; }
    if (w.order[0] != 1 || w.order[1] != 2 || w.order[2] != 3) { fails += 2; }
    if (w.total != 136) { fails += 4; }
    return fails;
}

// break: emit false stops the walk; the map itself is untouched
entry fn map_break() -> i64 {
    let mut m = HashMap<i32, i32>::new();
    let mut i = 0;
    while (i < 100) { m.put(i, i); i += 1; }
    let mut w: ?W = W { order: [0, 0, 0] };
    for (let k of m) {
        w.n += 1;
        if (k == 2) { break; }
    }
    if (w.n != 3) { return 1; }
    if (m.len() != 100) { return 2; }
    return 0;
}

// THE SNAPSHOT LAW: the walk is the keys taken when the loop starts —
// removes and fresh inserts inside the body never touch it
entry fn snapshot_walk() -> i64 {
    let mut m = HashMap<i32, i32>::new();
    let mut i = 0;
    while (i < 5) { m.put(i, i * 10); i += 1; }
    let mut w: ?W = W { order: [0, 0, 0] };
    for (let k of m) {
        m.remove(k);
        m.put(100 + k, k);
        w.n += 1;
    }
    if (w.n != 5) { return 1; }
    if (m.len() != 5) { return 2; }
    return 0;
}

// a set walks its elements; the first birth answers first
entry fn set_walk() -> i64 {
    let mut s = HashSet<str>::new();
    s.put("x");
    s.put("y");
    s.put("x");
    let mut fails: i64 = 0;
    let mut w: ?S = S { };
    for (let e of s) {
        if (w.n == 0) { w.first = e; }
        w.n += 1;
    }
    if (w.n != 2) { fails += 1; }
    if (w.first != "x") { fails += 2; }
    return fails;
}

// the widths are the mint's law: an i8 key reads -5, a u64 key keeps
// its high bit, a bytes key compares by content
entry fn key_widths() -> i64 {
    let mut fails: i64 = 0;
    let mut m8 = HashMap<i8, i32>::new();
    m8.put(-5, 1);
    m8.put(7, 2);
    let mut w: ?F = F { };
    for (let k of m8) {
        if (k < 0) { w.neg = k; }
        w.n += 1;
    }
    if (w.n != 2 || w.neg != -5) { fails += 1; }
    let big: u64 = 18446744073709551615u64;
    let mut mu = HashMap<u64, bool>::new();
    mu.put(big, true);
    mu.put(0, false);
    for (let k of mu) {
        if (k == big) { w.hi = true; }
        w.sum += 1;
    }
    if (!w.hi || w.sum != 2) { fails += 2; }
    let mut mb = HashSet<bytes>::new();
    mb.put(bytes::from([1, 2, 3]));
    for (let k of mb) {
        if (k == bytes::from([1, 2, 3])) { w.got += 1; }
    }
    if (w.got != 1) { fails += 4; }
    return fails;
}

// empty tables walk zero times, both classes
entry fn empty_walk() -> i64 {
    let m = HashMap<str, i32>::new();
    let mut n = 0;
    for (let k of m) { n += 1; }
    if (n != 0) { return 1; }
    let s = HashSet<i32>::new();
    for (let x of s) { n += 1; }
    if (n != 0) { return 2; }
    return 0;
}

// remove-then-reinsert: a NEW birth walks last
entry fn reinsert_order() -> i64 {
    let mut m = HashMap<i32, i32>::new();
    m.put(1, 1);
    m.put(2, 2);
    m.put(3, 3);
    m.remove(2);
    m.put(2, 22);
    let mut fails: i64 = 0;
    let mut w: ?W = W { order: [0, 0, 0] };
    for (let k of m) {
        w.order[w.n] = k;
        w.n += 1;
    }
    if (w.n != 3 || w.order[0] != 1 || w.order[1] != 3 || w.order[2] != 2) { fails += 1; }
    if (m.get(2) != 22) { fails += 2; }
    return fails;
}

// the mut binding form: the body assigns the loop variable
entry fn mut_binding() -> i64 {
    let mut s = HashSet<i32>::new();
    s.put(5);
    let mut w: ?S = S { };
    for (let mut x of s) {
        x = 9;
        w.n += 1;
    }
    if (w.n != 1) { return 1; }
    return 0;
}

// the aliasing law rides iteration: a write through `get(k)` inside the
// loop lands in the map's STORED cell
struct P { v: i32 }
entry fn get_aliasing() -> i64 {
    let mut m = HashMap<str, P>::new();
    let mut p = P { v: 1 };
    m.put("k", p);
    for (let k of m) {
        let mut g = m.get(k);
        g.v = 42;
    }
    return m.get("k").v as i64;
}

// probes: is it the member or the desugar? (records share, prims copy —
// the closure counts through a record cell)
struct C { n: i32 }
entry fn probe_member_map() -> i64 {
    let mut m = HashMap<i32, i32>::new();
    m.put(7, 1);
    m.put(8, 2);
    let mut c = C { n: 0 };
    m.iterate(fn (k: i32) -> bool { c.n += 1; return true; });
    return c.n as i64;
}
entry fn probe_member_set() -> i64 {
    let mut s = HashSet<i32>::new();
    s.put(7);
    s.put(8);
    let mut c = C { n: 0 };
    s.iterate(fn (k: i32) -> bool { c.n += 1; return true; });
    return c.n as i64;
}
entry fn probe_forof_map_count() -> i64 {
    let mut m = HashMap<i32, i32>::new();
    m.put(7, 1);
    m.put(8, 2);
    let mut c = C { n: 0 };
    for (let k of m) { c.n += 1; }
    return c.n as i64;
}
"#;

#[test]
fn the_member_and_the_desugar_agree() {
    // the explicit member call and the for-of desugar walk the same
    // snapshot: the same count for the same table (and the capture law's
    // record-cell accumulator — scalars copy into the emit closure)
    let mut vm = setup(SRC);
    assert_eq!(vm.call::<_, i64>("probe_member_map", ()).unwrap(), 2);
    assert_eq!(vm.call::<_, i64>("probe_member_set", ()).unwrap(), 2);
    assert_eq!(vm.call::<_, i64>("probe_forof_map_count", ()).unwrap(), 2);
}

#[test]
fn for_of_walks_map_keys_in_birth_order() {
    let mut vm = setup(SRC);
    assert_eq!(vm.call::<_, i64>("map_walk", ()).unwrap(), 0);
}

#[test]
fn for_of_break_stops_the_walk_and_spares_the_map() {
    let mut vm = setup(SRC);
    assert_eq!(vm.call::<_, i64>("map_break", ()).unwrap(), 0);
}

#[test]
fn the_walk_is_the_snapshot_taken_at_loop_entry() {
    let mut vm = setup(SRC);
    assert_eq!(vm.call::<_, i64>("snapshot_walk", ()).unwrap(), 0);
}

#[test]
fn a_set_walks_its_elements() {
    let mut vm = setup(SRC);
    assert_eq!(vm.call::<_, i64>("set_walk", ()).unwrap(), 0);
}

#[test]
fn the_mint_law_reads_every_width_bit_for_bit() {
    let mut vm = setup(SRC);
    assert_eq!(vm.call::<_, i64>("key_widths", ()).unwrap(), 0);
}

#[test]
fn empty_tables_walk_zero_times() {
    let mut vm = setup(SRC);
    assert_eq!(vm.call::<_, i64>("empty_walk", ()).unwrap(), 0);
}

#[test]
fn reinsert_moves_the_key_to_the_walks_end() {
    let mut vm = setup(SRC);
    assert_eq!(vm.call::<_, i64>("reinsert_order", ()).unwrap(), 0);
}

#[test]
fn the_mut_binding_form_compiles() {
    let mut vm = setup(SRC);
    assert_eq!(vm.call::<_, i64>("mut_binding", ()).unwrap(), 0);
}

#[test]
fn get_inside_the_loop_answers_the_stored_cell() {
    let mut vm = setup(SRC);
    assert_eq!(vm.call::<_, i64>("get_aliasing", ()).unwrap(), 42);
}
