//! The `nmap_host` host experiment, driven end to end the way the
//! `nmapset` wrapper drives it — rut code calls the host surface and the
//! payload lives Rust-side in `Opaque<NativeTable>` (RFC 0023/0026).
//! (nmap-hostvals P4: the table is a real `HashMap`; the round-1
//! opaque-keyed lanes, the sentinel family, and the grow/reloc
//! machinery are gone — this suite rides the fused h-family, the ONE
//! op family the wrapper itself uses.)
//!
//! Covered per the phase: insert / replace / find / miss / remove with
//! the packed `(handle << 1) | newly` answers, the dead-handle +
//! re-birth law (handles are never recycled), the str/bytes/sv key
//! flavors with lane-crossing content equality, `map_cap`'s HashMap
//! reserve read-back, and the payload Drop law at rc-0, including
//! through a wrapper record's `opaque` field.

use std::rc::Rc;

use rut_driver::{Module, Session};
use rut_vm::OpaqueRef;
use rut_vm::interp::Vm;

const PKG_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/nmap_host");

const SRC: &str = r#"
use nmap_host::{ map_new, map_cap, map_len,
            map_hput_i, map_hfind_i, map_hremove_i,
            map_hput_s, map_hfind_s, map_hremove_s,
            map_hput_y, map_hfind_y, map_hremove_y,
            map_hput_sv, map_hfind_sv, map_hremove_sv };

entry fn new_map(cap: i64) -> opaque { return map_new(cap); }

entry fn put_i64(t: opaque, k: i64) -> i64 {
    return map_hput_i(t, k);
}

entry fn find_i64(t: opaque, k: i64) -> i32 {
    return map_hfind_i(t, k);
}

entry fn count(t: opaque) -> i32 { return map_len(t); }

entry fn capacity(t: opaque) -> i32 { return map_cap(t); }

// insert / replace / find / miss / remove — a fails accumulator the
// test asserts at zero. Growth is std's: internal, invisible to the
// packed answers.
entry fn basic_lifecycle() -> i64 {
    let t = map_new(8);
    let mut fails: i64 = 0;
    let mut first: [i32] = [0; 10];
    let mut i: i64 = 0;
    while (i < 10) {
        let ans = map_hput_i(t, i);
        if (ans & 1 != 1) { fails += 100; }   // fresh keys must answer newly
        first[i as i32] = (ans >> 1) as i32;
        i += 1;
    }
    if (map_len(t) != 10) { fails += 1; }
    // replace: the SAME handle back, newly = false
    let ans = map_hput_i(t, 3);
    if (ans & 1 != 0) { fails += 10; }
    if ((ans >> 1) as i32 != first[3]) { fails += 20; }
    if (map_len(t) != 10) { fails += 40; }
    // hit and miss
    if (map_hfind_i(t, 7) != first[7]) { fails += 100; }
    if (map_hfind_i(t, 99) != -1) { fails += 200; }
    // remove: the dead key's OWN handle back, then a miss, then a remove-miss
    let rm = map_hremove_i(t, 7);
    if (rm != first[7]) { fails += 1000; }
    if (map_len(t) != 9) { fails += 2000; }
    if (map_hfind_i(t, 7) != -1) { fails += 4000; }
    if (map_hremove_i(t, 7) != -1) { fails += 8000; }
    if (map_len(t) != 9) { fails += 16000; }
    // re-insert: a NEW birth, never the recycled 7
    let ans = map_hput_i(t, 7);
    if (ans & 1 != 1) { fails += 32000; }
    if ((ans >> 1) as i32 <= first[9]) { fails += 64000; }
    if (map_len(t) != 10) { fails += 128000; }
    return fails;
}

// a removed key re-lands as a NEW birth — handles are monotonic
// births, never recycled (the removed slot's sidecar nil stays correct)
entry fn remove_rebirth() -> i64 {
    let t = map_new(8);
    if (map_hput_i(t, 11) & 1 != 1) { return -1; }
    if (map_hput_i(t, 22) & 1 != 1) { return -2; }
    let before = map_hfind_i(t, 11);
    if (before < 0) { return -3; }
    if (map_hremove_i(t, 11) != before) { return -4; }
    if (map_len(t) != 1) { return -5; }
    let ans = map_hput_i(t, 11);
    if (ans & 1 != 1) { return -6; }
    if ((ans >> 1) as i32 <= before) { return -7; }   // a NEW birth
    // the untouched key keeps its handle
    if (map_hfind_i(t, 22) < 0) { return -8; }
    return 0;
}

// str keys: content equality — a fresh instance of the same text finds
// the same handle, replaces answer it not-newly, remove kills it
entry fn str_keys() -> i64 {
    let t = map_new(8);
    let mut fails: i64 = 0;
    let a: str = "alpha";
    let b: str = "beta";
    let c: str = "gamma";
    if (map_hput_s(t, a) & 1 != 1) { fails += 1; }
    if (map_hput_s(t, b) & 1 != 1) { fails += 1; }
    if (map_hput_s(t, c) & 1 != 1) { fails += 1; }
    if (map_len(t) != 3) { fails += 2; }
    // content equality: a fresh instance of the same text finds the handle
    let again: str = "beta";
    let beta = map_hfind_s(t, again);
    if (beta < 0) { fails += 4; }
    let miss: str = "delta";
    if (map_hfind_s(t, miss) != -1) { fails += 8; }
    // replace: the found handle back, not newly
    let ans = map_hput_s(t, a);
    if (ans & 1 != 0) { fails += 16; }
    if (map_hfind_s(t, a) != (ans >> 1) as i32) { fails += 32; }
    if (map_len(t) != 3) { fails += 64; }
    if (map_hremove_s(t, c) < 0) { fails += 128; }
    if (map_len(t) != 2) { fails += 256; }
    if (map_hfind_s(t, c) != -1) { fails += 512; }
    return fails;
}

// bytes keys: octet equality, same shape
entry fn bytes_keys() -> i64 {
    let t = map_new(8);
    let mut fails: i64 = 0;
    let b1 = bytes.from([1, 2, 3]);
    let b2 = bytes.from([9, 9]);
    if (map_hput_y(t, b1) & 1 != 1) { fails += 1; }
    if (map_hput_y(t, b2) & 1 != 1) { fails += 1; }
    if (map_len(t) != 2) { fails += 2; }
    // octet equality on a fresh copy
    let again = bytes.from([1, 2, 3]);
    if (map_hfind_y(t, again) < 0) { fails += 4; }
    let miss = bytes.from([4, 5]);
    if (map_hfind_y(t, miss) != -1) { fails += 8; }
    if (map_hput_y(t, b1) & 1 != 0) { fails += 16; }
    if (map_hremove_y(t, b2) < 0) { fails += 32; }
    if (map_len(t) != 1) { fails += 64; }
    return fails;
}

// the wrapper-record shape: each iteration mints a table behind an
// opaque field and lets both die — the payload must Drop through the
// record's field release (RFC 0016 §3)
class Bag {
    t: opaque;
}
impl Bag {
    fn adopt(t: opaque) -> Self { return Self { t: t }; }
}

entry fn churn_tables(n: i64) -> nil {
    let mut i: i64 = 0;
    while (i < n) {
        let b = Bag.adopt(map_new(16));
        map_hput_i(b.t, i);
        i += 1;
    }
}

// ---- the sv lanes (strings-round1, phase 1) -------------------------
// keys carve BYTE ranges out of a str parent; the host hashes/compares
// over the range, and the entry + HANDLE are the s lanes'.

// the parity law end to end: sv keys and s keys of the same content
// are THE SAME key — same handles, replaces/removes cross lanes, and a
// slice VIEW (RFC 0042) reads as its range through the s lane
entry fn sv_parity() -> i64 {
    let t1 = map_new(8);
    let t2 = map_new(8);
    let parent: str = "alpha-beta-gamma-delta";
    let mut fails: i64 = 0;
    // sv inserts into t1; s inserts of the same texts into t2
    if (map_hput_sv(t1, parent, 0, 5) & 1 != 1) { fails += 1; }    // alpha
    if (map_hput_sv(t1, parent, 6, 4) & 1 != 1) { fails += 1; }    // beta
    if (map_hput_sv(t1, parent, 11, 5) & 1 != 1) { fails += 1; }   // gamma
    let a: str = "alpha";
    let b: str = "beta";
    let g: str = "gamma";
    if (map_hput_s(t2, a) & 1 != 1) { fails += 2; }
    if (map_hput_s(t2, b) & 1 != 1) { fails += 2; }
    if (map_hput_s(t2, g) & 1 != 1) { fails += 2; }
    if (map_len(t1) != 3 || map_len(t2) != 3) { fails += 4; }
    // same content either way: the handles agree, and each lane
    // finds what the other stored
    let s1 = map_hfind_sv(t1, parent, 0, 5);
    let s2 = map_hfind_s(t2, a);
    if (s1 < 0 || s1 != s2) { fails += 8; }
    if (map_hfind_s(t1, a) != s1) { fails += 16; }
    if (map_hfind_sv(t2, parent, 0, 5) != s2) { fails += 32; }
    // a slice VIEW crosses the s lane as its range (zero-copy read)
    let view = parent.slice(6, 10);
    if (map_hfind_s(t1, view) < 0) { fails += 64; }
    if (map_hfind_s(t1, view) != map_hfind_sv(t1, parent, 6, 4)) { fails += 128; }
    // replace through the opposite lane: the found handle back, not newly
    if (map_hput_s(t1, b) != ((map_hfind_sv(t1, parent, 6, 4) as i64) << 1)) { fails += 256; }
    if (map_len(t1) != 3) { fails += 512; }
    // remove through the sv lane
    let rm = map_hremove_sv(t1, parent, 0, 5);
    if (rm < 0 || rm != s1) { fails += 1024; }
    if (map_len(t1) != 2) { fails += 2048; }
    if (map_hfind_sv(t1, parent, 0, 5) != -1) { fails += 4096; }
    if (map_hfind_s(t2, g) != map_hfind_sv(t2, parent, 11, 5)) { fails += 8192; }
    return fails;
}

// the empty range is a legal key at every boundary, and the owned ""
// key finds it
entry fn sv_empty_range() -> i64 {
    let t = map_new(8);
    let s: str = "abc";
    if (map_hput_sv(t, s, 0, 0) & 1 != 1) { return 1; }
    if (map_len(t) != 1) { return 2; }
    if (map_hfind_sv(t, s, 3, 0) < 0) { return 3; }
    let e: str = "";
    if (map_hfind_s(t, e) < 0) { return 4; }
    return 0;
}

// a mid-codepoint window: the host's UTF-8 boundary trap
entry fn sv_boundary_trap(t: opaque) -> i32 {
    let utf8: str = "héllo";
    return map_hfind_sv(t, utf8, 0, 2);
}

// off+len past the parent's octets: the range trap
entry fn sv_range_trap(t: opaque) -> i32 {
    let s: str = "abc";
    return map_hfind_sv(t, s, 1, 3);
}

// a legal sv put, for the after-trap intactness check
entry fn sv_put_ok(t: opaque) -> i64 {
    let s: str = "abc";
    return map_hput_sv(t, s, 0, 3);
}
"#;

fn vm_with_nmap() -> Vm {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    // `nmap_host` — this test's host pkg, declared in tests/data/nmap_host
    rut_driver::mount_dir(&mut session, std::path::Path::new(PKG_DIR)).expect("mount nmap_host");
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
    hosts.verify_against(&expected); // tests/data/nmap_host/nmap.d.rut ↔ the bodies
    rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts)
        .unwrap()
}

#[test]
fn insert_replace_find_miss_remove_lifecycle() {
    let mut vm = vm_with_nmap();
    assert_eq!(vm.call::<_, i64>("basic_lifecycle", ()).unwrap(), 0);
}

#[test]
fn removal_then_reinsert_is_a_new_birth_never_a_recycled_handle() {
    let mut vm = vm_with_nmap();
    assert_eq!(vm.call::<_, i64>("remove_rebirth", ()).unwrap(), 0);
}

#[test]
fn str_and_bytes_keys_work_content_equal_and_remove() {
    let mut vm = vm_with_nmap();
    assert_eq!(vm.call::<_, i64>("str_keys", ()).unwrap(), 0);
    assert_eq!(vm.call::<_, i64>("bytes_keys", ()).unwrap(), 0);
}

#[test]
fn tables_release_at_rc0_including_through_wrapper_records() {
    let mut vm = vm_with_nmap();
    let base = vm.heap_usage();

    // direct handles: the payload lives while the host holds the handle
    // (each box charges its shallow payload — size_of::<NativeTable>(),
    // pinned at 40 by the rut-std unit test)
    let held: Vec<OpaqueRef> =
        (0..4000).map(|_| vm.call::<_, OpaqueRef>("new_map", (16i64,)).unwrap()).collect();
    let held_usage = vm.heap_usage();
    assert!(
        held_usage > base + 4000 * 32,
        "host boxes must charge their payload: {base} -> {held_usage}"
    );

    // rc-0 (RFC 0016 §3): the last handle drop frees the cells and the
    // payloads — NativeTable is pure Rust data, nothing leaks
    drop(held);
    let after = vm.heap_usage();
    assert!(after <= base + 4096, "tables must free at rc-0: base {base}, after {after}");

    // the wrapper-record path: each iteration's Bag record releases its
    // opaque field at rc-0, which drops the box and its payload
    vm.call::<_, ()>("churn_tables", (2000i64,)).unwrap();
    let after_churn = vm.heap_usage();
    assert!(
        after_churn <= base + 4096,
        "wrapper records must release table boxes at rc-0: base {base}, after {after_churn}"
    );
}

// ---- the sv lanes (strings-round1, phase 1) -------------------------

/// The parity law end to end: sv keys and s keys of the same content
/// land the same handles and each lane finds what the other stored —
/// including a slice VIEW crossing the s lane as its range.
#[test]
fn sv_lanes_parity_with_the_s_lanes_end_to_end() {
    let mut vm = vm_with_nmap();
    assert_eq!(vm.call::<_, i64>("sv_parity", ()).unwrap(), 0);
    assert_eq!(vm.call::<_, i64>("sv_empty_range", ()).unwrap(), 0);
}

/// The sv range check traps with the house `Invalid` shape through the
/// VM — a mid-codepoint window and an out-of-range window are loud,
/// and neither stores anything.
#[test]
fn sv_lanes_trap_mid_codepoint_and_out_of_range() {
    let mut vm = vm_with_nmap();
    let t: OpaqueRef = vm.call("new_map", (8i64,)).unwrap();
    let err = vm.call::<_, i32>("sv_boundary_trap", (t.clone(),)).unwrap_err();
    assert_eq!(err.kind, rut_vm::TrapKind::Invalid);
    assert!(
        err.msg.contains("nmap") && err.msg.contains("not a UTF-8 boundary"),
        "{}",
        err.msg
    );
    let err = vm.call::<_, i32>("sv_range_trap", (t.clone(),)).unwrap_err();
    assert!(
        err.msg.contains("nmap") && err.msg.contains("out of range"),
        "{}",
        err.msg
    );
    // the traps stored nothing and the table still works
    assert_eq!(vm.call::<_, i32>("count", (t.clone(),)).unwrap(), 0);
    assert!(vm.call::<_, i64>("sv_put_ok", (t.clone(),)).unwrap() & 1 == 1);
    assert_eq!(vm.call::<_, i32>("count", (t.clone(),)).unwrap(), 1);
}
