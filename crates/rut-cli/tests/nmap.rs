//! The `nmap` host experiment (the mapset-host plan, H2): the native
//! key table behind `Opaque` payload boxes, driven end to end the way
//! the H3 wrapper will drive it — rut code computes the hash, boxes the
//! key with `Opaque.new`, and calls the host surface; the payload lives
//! Rust-side in `OpaqueBox<NativeTable>` (RFC 0023/0026).
//!
//! Covered per the phase: insert / replace / find / miss / remove,
//! tombstone reuse, the load-factor law, grow with the relocation
//! iterator, the str/bytes key flavors, the closed-set trap (floats and
//! user records — the loud "use mapset" message), and the payload Drop
//! law at rc-0, including through a wrapper record's `Opaque` field.

use std::rc::Rc;

use rut_driver::{Module, Session};
use rut_vm::OpaqueRef;
use rut_vm::interp::Vm;

const PKG_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/nmap");

const SRC: &str = r#"
use core::{ Opaque };
use nmap::{ map_new, map_entry, map_find, map_remove, map_needs_grow, map_grow, map_take_reloc, map_cap, map_len };

// the wrapper-side hash vocabulary (mapset.rut verbatim): mix64 for the
// integer keys, FNV-1a 64 for str/bytes. nmap never recomputes a hash —
// it records what crosses in `h` — so these only need to be consistent
// per key, but using the real ones exercises collisions honestly. The
// `*i64` wrappers cast at the crossing: the host fn takes `h: i64` and
// the bits arrive unchanged.
fn mix64(bits: u64) -> u64 {
    return (14695981039346656037u64 ^ bits).wrapping_mul(1099511628211u64);
}

fn ihash(bits: u64) -> i64 {
    return mix64(bits) as i64;
}

fn fnv1a64(data: bytes) -> u64 {
    let mut h: u64 = 14695981039346656037u64;
    for (let b of data) {
        h = (h ^ b as u64).wrapping_mul(1099511628211u64);
    }
    return h;
}

fn bhash(data: bytes) -> i64 {
    return fnv1a64(data) as i64;
}

entry fn new_map(cap: i64) -> Opaque { return map_new(cap); }

entry fn put_i32(t: Opaque, k: i32) -> i32 {
    return map_entry(t, Opaque.new(k), ihash(k as u64));
}

entry fn find_i32(t: Opaque, k: i32) -> i32 {
    return map_find(t, Opaque.new(k), ihash(k as u64));
}

entry fn count(t: Opaque) -> i32 { return map_len(t); }

entry fn capacity(t: Opaque) -> i32 { return map_cap(t); }

// insert / replace / find / miss / remove — a fails accumulator the
// test asserts at zero. The grow check runs before the probe, exactly
// the wrapper's put shape.
entry fn basic_lifecycle() -> i64 {
    let t = map_new(8);
    let mut fails: i64 = 0;
    let mut i = 0;
    while (i < 10) {
        if (map_needs_grow(t)) {
            if (map_grow(t) < 4) { fails += 100000; }
        }
        let at = map_entry(t, Opaque.new(i), ihash(i as u64));
        if (at >= 0) { fails += 100; }   // fresh keys must insert
        i += 1;
    }
    if (map_len(t) != 10) { fails += 1; }
    // replace: the found slot back, no new entry
    let at = map_entry(t, Opaque.new(3), ihash(3));
    if (at < 0) { fails += 10; }
    if (map_len(t) != 10) { fails += 20; }
    // hit and miss
    if (map_find(t, Opaque.new(7), ihash(7)) < 0) { fails += 100; }
    if (map_find(t, Opaque.new(99), ihash(99)) >= 0) { fails += 200; }
    // remove: the freed slot back, then a miss, then a remove-miss
    let rm = map_remove(t, Opaque.new(7), ihash(7));
    if (rm < 0) { fails += 1000; }
    if (map_len(t) != 9) { fails += 2000; }
    if (map_find(t, Opaque.new(7), ihash(7)) >= 0) { fails += 4000; }
    if (map_remove(t, Opaque.new(7), ihash(7)) >= 0) { fails += 8000; }
    if (map_len(t) != 9) { fails += 16000; }
    return fails;
}

// the load-factor law with mapset's constants, observed on a table the
// test never grows: (11+0+1)*10 = 120 >= 16*7 = 112 flips first
entry fn load_law() -> i64 {
    let t = map_new(16);
    if (map_needs_grow(t)) { return -1; }
    let mut i = 0;
    while (!map_needs_grow(t)) {
        map_entry(t, Opaque.new(i), ihash(i as u64));
        i += 1;
    }
    if (map_len(t) != 11) { return -2; }
    if (map_cap(t) != 16) { return -3; }
    return 0;
}

// a tombstoned key re-lands on its own dead slot — the DEAD-reuse law
entry fn tombstone_reuse() -> i64 {
    let t = map_new(8);
    // three sparse keys: no grow ((3+0+1)*10 = 40 < 56)
    if (map_entry(t, Opaque.new(11), ihash(11)) >= 0) { return -1; }
    if (map_entry(t, Opaque.new(22), ihash(22)) >= 0) { return -1; }
    if (map_entry(t, Opaque.new(33), ihash(33)) >= 0) { return -1; }
    let before = map_find(t, Opaque.new(11), ihash(11));
    if (before < 0) { return -2; }
    if (map_remove(t, Opaque.new(11), ihash(11)) < 0) { return -3; }
    if (map_len(t) != 2) { return -4; }
    let again = map_entry(t, Opaque.new(11), ihash(11));
    if (again >= 0) { return -5; }
    if (-(again + 1) != before) { return -6; }   // same slot: DEAD reuse
    // the untouched keys still find their slots
    if (map_find(t, Opaque.new(22), ihash(22)) < 0) { return -7; }
    if (map_find(t, Opaque.new(33), ihash(33)) < 0) { return -8; }
    return 0;
}

// the wrapper's put + grow loop, exact shape: grow past the load
// factor, drain the relocation iterator, move the parallel values
// old->new, then probe-insert. Every value must land on its key.
entry fn grow_relocate(n: i32) -> i64 {
    let t = map_new(4);
    let mut vals: [i64] = [0; map_cap(t)];
    let mut i = 0;
    while (i < n) {
        if (map_needs_grow(t)) {
            let new_cap = map_grow(t);
            if (new_cap != map_cap(t)) { return -1000000; }
            let mut next: [i64] = [0; new_cap];
            while (true) {
                let packed = map_take_reloc(t);
                if (packed < 0) { break; }
                let old = (packed >> 32) as i32;
                let new = (packed & 4294967295i64) as i32;
                next[new] = vals[old];
            }
            vals = next;
        }
        let k = i * 37 - 1000;   // spans negative keys too
        let at = map_entry(t, Opaque.new(k), ihash(k as u64));
        let mut slot: i32 = at;
        if (at < 0) { slot = -(at + 1); }
        vals[slot] = k as i64 * 7 + 1;
        i += 1;
    }
    if (map_len(t) != n) { return -2000000; }
    let mut acc: i64 = 0;
    let mut j = 0;
    while (j < n) {
        let k = j * 37 - 1000;
        let at = map_find(t, Opaque.new(k), ihash(k as u64));
        if (at < 0) { return -3000000 - j as i64; }
        acc = acc + (vals[at] - (k as i64 * 7 + 1));
        j += 1;
    }
    if (acc != 0) { return acc; }
    return map_cap(t) as i64;
}

entry fn str_keys() -> i64 {
    let t = map_new(8);
    let mut fails: i64 = 0;
    let a: str = "alpha";
    let b: str = "beta";
    let c: str = "gamma";
    if (map_entry(t, Opaque.new(a), bhash(a.encode())) >= 0) { fails += 1; }
    if (map_entry(t, Opaque.new(b), bhash(b.encode())) >= 0) { fails += 1; }
    if (map_entry(t, Opaque.new(c), bhash(c.encode())) >= 0) { fails += 1; }
    if (map_len(t) != 3) { fails += 2; }
    // content equality: a fresh instance of the same text finds the slot
    let again: str = "beta";
    if (map_find(t, Opaque.new(again), bhash(again.encode())) < 0) { fails += 4; }
    let miss: str = "delta";
    if (map_find(t, Opaque.new(miss), bhash(miss.encode())) >= 0) { fails += 8; }
    // replace: the found slot back, no new entry
    if (map_entry(t, Opaque.new(a), bhash(a.encode())) < 0) { fails += 16; }
    if (map_len(t) != 3) { fails += 32; }
    if (map_remove(t, Opaque.new(c), bhash(c.encode())) < 0) { fails += 64; }
    if (map_len(t) != 2) { fails += 128; }
    return fails;
}

entry fn bytes_keys() -> i64 {
    let t = map_new(8);
    let mut fails: i64 = 0;
    let b1 = bytes.from([1, 2, 3]);
    let b2 = bytes.from([9, 9]);
    if (map_entry(t, Opaque.new(b1), bhash(b1)) >= 0) { fails += 1; }
    if (map_entry(t, Opaque.new(b2), bhash(b2)) >= 0) { fails += 1; }
    if (map_len(t) != 2) { fails += 2; }
    // octet equality on a fresh copy
    let again = bytes.from([1, 2, 3]);
    if (map_find(t, Opaque.new(again), bhash(again)) < 0) { fails += 4; }
    let miss = bytes.from([4, 5]);
    if (map_find(t, Opaque.new(miss), bhash(miss)) >= 0) { fails += 8; }
    if (map_remove(t, Opaque.new(b2), bhash(b2)) < 0) { fails += 16; }
    if (map_len(t) != 1) { fails += 32; }
    return fails;
}

struct Pt { x: i32; y: i32 }

// outside the closed key set — the HOST traps, loud, pointing at mapset
entry fn float_key(t: Opaque, k: f64) -> nil {
    map_entry(t, Opaque.new(k), ihash(0));
}

// the f32 twin (no overloading — the `_32` suffix carries the width)
entry fn float_key_32(t: Opaque, k: f32) -> nil {
    map_entry(t, Opaque.new(k), ihash(0));
}

// a user-defined key type: the honest experiment scope's loud edge
entry fn record_key(t: Opaque) -> nil {
    let p = Pt { x: 3, y: 4 };
    map_find(t, Opaque.new(p), ihash(0));
}

// the wrapper-record shape: each iteration mints a table behind an
// Opaque field and lets both die — the payload must Drop through the
// record's field release (RFC 0016 §3)
class Bag {
    t: Opaque;
}
impl Bag {
    fn adopt(t: Opaque) -> Self { return Self { t: t }; }
}

entry fn churn_tables(n: i64) -> nil {
    let mut i: i64 = 0;
    while (i < n) {
        let b = Bag.adopt(map_new(16));
        map_entry(b.t, Opaque.new(i as i32), ihash(i as u64));
        i += 1;
    }
}
"#;

fn vm_with_nmap() -> Vm {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    // `nmap` — this test's host pkg, declared in tests/data/nmap
    rut_driver::mount_dir(&mut session, std::path::Path::new(PKG_DIR)).expect("mount nmap");
    let expected = session.expected_host_fns();
    session
        .register_module(
            "app",
            Module { spec: "app".into(), source: Some(SRC.into()), ..Default::default() },
        )
        .unwrap();
    let g = rut_driver::compile_graph(&session, "app");
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let prog = g.program.expect("compile");
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(20_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    // bindings BEFORE the Vm (RFC 0025): install + contract + boot
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_std::nmap::install_std_nmap(&mut hosts);
    hosts.verify_against(&expected); // tests/data/nmap/nmap.d.rut ↔ the bodies
    rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts)
        .unwrap()
}

#[test]
fn insert_replace_find_miss_remove_lifecycle() {
    let mut vm = vm_with_nmap();
    assert_eq!(vm.call::<_, i64>("basic_lifecycle", ()).unwrap(), 0);
}

#[test]
fn the_load_factor_law_is_mapsets_constants() {
    let mut vm = vm_with_nmap();
    assert_eq!(vm.call::<_, i64>("load_law", ()).unwrap(), 0);
}

#[test]
fn dead_slots_are_reused() {
    let mut vm = vm_with_nmap();
    assert_eq!(vm.call::<_, i64>("tombstone_reuse", ()).unwrap(), 0);
}

#[test]
fn grow_relocates_values_through_the_iterator() {
    let mut vm = vm_with_nmap();
    // no insert: no grow, the fresh cap-4 table answers
    assert_eq!(vm.call::<_, i64>("grow_relocate", (0i32,)).unwrap(), 4);
    // 1000 keys over negative and positive space: the value array must
    // survive every relocation — and the final cap is 2048 (1024's load
    // law fires first: 1024*7 = 7168 < 1000*10)
    assert_eq!(vm.call::<_, i64>("grow_relocate", (1000i32,)).unwrap(), 2048);
}

#[test]
fn str_and_bytes_keys_work_content_equal_and_remove() {
    let mut vm = vm_with_nmap();
    assert_eq!(vm.call::<_, i64>("str_keys", ()).unwrap(), 0);
    assert_eq!(vm.call::<_, i64>("bytes_keys", ()).unwrap(), 0);
}

#[test]
fn unsupported_key_types_trap_loudly_and_leave_the_table_intact() {
    let mut vm = vm_with_nmap();
    let t: OpaqueRef = vm.call("new_map", (8i64,)).unwrap();
    vm.call::<_, i32>("put_i32", (t.clone(), 5i32)).unwrap();

    // a float key: the closed-set trap names the type and points at mapset
    let err = vm.call::<_, ()>("float_key", (t.clone(), 1.5f64)).unwrap_err();
    assert!(
        err.msg.contains("nmap") && err.msg.contains("not natively supported") && err.msg.contains("mapset"),
        "{}",
        err.msg
    );
    assert!(err.msg.contains("f64"), "the trap names the offending type: {}", err.msg);
    // the f32 twin is named just as precisely
    let err = vm.call::<_, ()>("float_key_32", (t.clone(), 1.5f32)).unwrap_err();
    assert!(err.msg.contains("f32"), "{}", err.msg);

    // a user-defined key type: same trap
    let err = vm.call::<_, ()>("record_key", (t.clone(),)).unwrap_err();
    assert!(
        err.msg.contains("not natively supported") && err.msg.contains("mapset"),
        "{}",
        err.msg
    );

    // the traps stored nothing and the table still works
    assert_eq!(vm.call::<_, i32>("count", (t.clone(),)).unwrap(), 1);
    assert!(vm.call::<_, i32>("find_i32", (t.clone(), 5i32)).unwrap() >= 0);
    assert!(vm.call::<_, i32>("put_i32", (t.clone(), 6i32)).unwrap() < 0);
    assert_eq!(vm.call::<_, i32>("count", (t.clone(),)).unwrap(), 2);
}

#[test]
fn tables_release_at_rc0_including_through_wrapper_records() {
    let mut vm = vm_with_nmap();
    let base = vm.heap_usage();

    // direct handles: the payload lives while the host holds the handle
    // (each box charges its shallow payload — size_of::<NativeTable>())
    let held: Vec<OpaqueRef> =
        (0..4000).map(|_| vm.call::<_, OpaqueRef>("new_map", (16i64,)).unwrap()).collect();
    let held_usage = vm.heap_usage();
    assert!(
        held_usage > base + 4000 * 64,
        "host boxes must charge their payload: {base} -> {held_usage}"
    );

    // rc-0 (RFC 0016 §3): the last handle drop frees the cells and the
    // payloads — NativeTable is pure Rust data, nothing leaks
    drop(held);
    let after = vm.heap_usage();
    assert!(after <= base + 4096, "tables must free at rc-0: base {base}, after {after}");

    // the wrapper-record path: each iteration's Bag record releases its
    // Opaque field at rc-0, which drops the box and its payload
    vm.call::<_, ()>("churn_tables", (2000i64,)).unwrap();
    let after_churn = vm.heap_usage();
    assert!(
        after_churn <= base + 4096,
        "wrapper records must release table boxes at rc-0: base {base}, after {after_churn}"
    );
}
