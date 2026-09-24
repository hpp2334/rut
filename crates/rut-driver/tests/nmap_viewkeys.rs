//! The `nmapset` range-keyed methods (strings-round1, phase 2): the sv
//! crossings as a `HashMap` surface — `put_range` / `get_range` /
//! `has_range` / `remove_range`, the key crossing as a borrowed
//! `(parent, off, len)` BYTE range.
//!
//! SPELLING (the hashmap-surface batch): the range-keyed methods live
//! on the GENERIC class, and the family rows re-seat `HashMap<str,
//! i64>` to the val-column class — which has no range surface. This
//! suite spells the no-row val `HashMap<str, i32>` (the nmapset-str
//! shape this suite parity-matches anyway) so the range machinery
//! stays testable; every pinned literal below is unchanged (the folded
//! integers are the same). The range-surface-on-the-column question is
//! a new-surface menu item, not a rename.
//!
//! Covered: the PARITY law through the wrapper — the nmapset-str churn
//! shape spelled both ways over one parent (range methods vs
//! slice-then-put) answers the SAME pinned checksum; lane
//! interchangeability both directions on ONE table (a range key and
//! the equal-content slice key are THE SAME key, replace included); a
//! slice VIEW as the parent (flattens to the root — the RFC 0042
//! read); the grow + `vals` drain through `put_range` (multi-grow
//! sweep with remove / re-add churn, every val exact); the empty range
//! key == the `""` key; and the host's UTF-8 boundary / past-end traps
//! surfacing the house `Invalid` shape through the wrapper (the raw
//! lanes' store-nothing-on-trap law is phase 1's suite, still green).

use std::rc::Rc;

use rut_driver::{Module, Session};
use rut_vm::interp::Vm;

const NMAPSET_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/nmapset");

fn vm_nmapset(src: &str) -> Vm {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    rut_driver::mount_dir(&mut session, std::path::Path::new(NMAPSET_DIR))
        .expect("mount pkg");
    let expected = session.expected_host_fns();
    session
        .register_module(
            "app",
            Module { spec: "app".into(), source: Some(src.into()), ..Default::default() },
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
        fuel: Some(200_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_std::nmap::install_std_nmap(&mut hosts);
    hosts.verify_against(&expected); // the mounted .d.rut ↔ the bodies
    rut_vm::interp::Vm::new(
        Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        hosts,
    )
    .unwrap()
}

fn run_main(src: &str) -> i64 {
    vm_nmapset(src).call::<_, i64>("main", ()).expect("run")
}

// ---- the parity law --------------------------------------------------

/// The nmapset-str churn shape at n = 2000, keys carved as fixed-width
/// (6-char) injective decimal slots of ONE parent, spelled BOTH ways:
/// the range methods over `(parent, i*6, 6)` and slice-then-put over
/// `parent.slice(i*6, i*6+6)` (ASCII, so codepoint == byte). Same
/// keys, same slots, same answers — one pinned checksum.
const PARITY_SRC: &str = r#"
use nmapset::{ HashMap };

// one parent, 2n fixed-width slots: slot(i) = 6 chars at off i*6,
// injective decimal content (100000+i). Keys live in slots 0..n, the
// never-inserted miss keys in slots n..2n.
fn build_parent(n: i32) -> str {
    let mut p = "";
    for (let i = 0; i < 2 * n; i += 1) {
        let base = 100000 + i;
        p = f"{p}{base}";
    }
    return p;
}

// the nmapset-str six-phase churn, RANGE-spelled
fn churn_range(p: str, n: i32) -> i64 {
    let mut m: HashMap<str, i32> = HashMap.new();
    let mut added: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.put_range(p, i * 6, 6, i)) { added += 1; }
    }
    let mut replaced: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.put_range(p, i * 6, 6, i + 3) == false) { replaced += 1; }
    }
    let mut sum: i64 = 0;
    let mut hits: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        let q = m.get_range(p, i * 6, 6);
        if (q != nil) {
            hits += 1;
            sum = sum.wrapping_add(q as i64);
        }
    }
    let mut misses: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.get_range(p, (n + i) * 6, 6) == nil) { misses += 1; }
    }
    let mut removed: i64 = 0;
    for (let i = 0; i < n; i += 3) {
        if (m.remove_range(p, i * 6, 6)) { removed += 1; }
    }
    let mut present: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.has_range(p, i * 6, 6)) { present += 1; }
    }
    let mut added2: i64 = 0;
    for (let i = 0; i < n; i += 3) {
        if (m.put_range(p, i * 6, 6, 0 - i)) { added2 += 1; }
    }
    let mut c: i64 = sum;
    c = c.wrapping_add(added.wrapping_mul(31));
    c = c.wrapping_add(replaced.wrapping_mul(37));
    c = c.wrapping_add(hits.wrapping_mul(41));
    c = c.wrapping_add(misses.wrapping_mul(43));
    c = c.wrapping_add(removed.wrapping_mul(47));
    c = c.wrapping_add(present.wrapping_mul(53));
    c = c.wrapping_add(added2.wrapping_mul(59));
    c = c.wrapping_add((m.len() as i64).wrapping_mul(61));
    return c;
}

// the identical churn, SLICE-spelled (the pre-phase-2 spelling)
fn churn_slice(p: str, n: i32) -> i64 {
    let mut m: HashMap<str, i32> = HashMap.new();
    let mut added: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.put(p.slice(i * 6, i * 6 + 6), i)) { added += 1; }
    }
    let mut replaced: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.put(p.slice(i * 6, i * 6 + 6), i + 3) == false) { replaced += 1; }
    }
    let mut sum: i64 = 0;
    let mut hits: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        let q = m.get(p.slice(i * 6, i * 6 + 6));
        if (q != nil) {
            hits += 1;
            sum = sum.wrapping_add(q as i64);
        }
    }
    let mut misses: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.get(p.slice((n + i) * 6, (n + i) * 6 + 6)) == nil) { misses += 1; }
    }
    let mut removed: i64 = 0;
    for (let i = 0; i < n; i += 3) {
        if (m.remove(p.slice(i * 6, i * 6 + 6))) { removed += 1; }
    }
    let mut present: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.has(p.slice(i * 6, i * 6 + 6))) { present += 1; }
    }
    let mut added2: i64 = 0;
    for (let i = 0; i < n; i += 3) {
        if (m.put(p.slice(i * 6, i * 6 + 6), 0 - i)) { added2 += 1; }
    }
    let mut c: i64 = sum;
    c = c.wrapping_add(added.wrapping_mul(31));
    c = c.wrapping_add(replaced.wrapping_mul(37));
    c = c.wrapping_add(hits.wrapping_mul(41));
    c = c.wrapping_add(misses.wrapping_mul(43));
    c = c.wrapping_add(removed.wrapping_mul(47));
    c = c.wrapping_add(present.wrapping_mul(53));
    c = c.wrapping_add(added2.wrapping_mul(59));
    c = c.wrapping_add((m.len() as i64).wrapping_mul(61));
    return c;
}

pub fn main() -> i64 {
    let n = 2000;
    let p = build_parent(n);
    let a = churn_range(p, n);
    let b = churn_slice(p, n);
    if (a != b) { return 0; }   // a divergence reads 0 and fails the pin
    return a;
}
"#;

#[test]
fn range_methods_match_slice_then_put_through_the_churn() {
    // the pinned value is whatever the two spellings must agree on; a
    // divergence returns 0 and trips this assert
    assert_eq!(
        run_main(PARITY_SRC),
        2572351,
        "range-spelled churn == slice-spelled churn, one pinned checksum"
    );
}

/// Lane interchangeability on ONE table, both directions: what a range
/// put stores, a slice get/remove finds; what a slice put stores, a
/// range get/remove finds; a replace via either spell replaces the
/// same entry (len never grows on equal content); and a slice VIEW as
/// the PARENT reads as its own range of the root.
const INTERLEAVE_SRC: &str = r#"
use nmapset::{ HashMap };

pub fn main() -> i64 {
    let seq = "ACGTTCAGGCATXZ";
    let mut m: HashMap<str, i32> = HashMap.new();
    let mut f: i64 = 0;
    // range inserts, slice reads
    if (!m.put_range(seq, 0, 3, 1)) { f += 1; }      // "ACG"
    if (!m.put_range(seq, 3, 3, 2)) { f += 2; }      // "TTC"
    if (m.get(seq.slice(0, 3)) != 1) { f += 4; }
    if (!m.has(seq.slice(3, 6))) { f += 8; }
    // slice put over a range key: replace, not a second entry
    if (m.put(seq.slice(0, 3), 10)) { f += 16; }
    if (m.len() != 2) { f += 32; }
    if (m.get_range(seq, 0, 3) != 10) { f += 64; }
    // a window key off the slot grid: range stores, slice finds
    if (!m.put_range(seq, 5, 4, 3)) { f += 128; }    // "CAGG"
    if (m.get(seq.slice(5, 9)) != 3) { f += 256; }
    if (!m.has_range(seq, 5, 4)) { f += 512; }
    // cross-lane removes, both directions
    if (!m.remove(seq.slice(0, 3))) { f += 1024; }
    if (m.get_range(seq, 0, 3) != nil) { f += 2048; }
    if (!m.remove_range(seq, 5, 4)) { f += 4096; }
    if (m.get(seq.slice(5, 9)) != nil) { f += 8192; }
    if (m.len() != 1) { f += 16384; }
    // a slice VIEW as the parent: flattened to the root, so the
    // window is read out of the ORIGINAL seq — win[3..6] == "TTC",
    // the surviving range key: this is a REPLACE (answers false)
    let win = seq.slice(0, 9);
    if (m.put_range(win, 3, 3, 99) != false) { f += 32768; }
    if (m.get_range(seq, 3, 3) != 99) { f += 65536; }
    if (m.get(seq.slice(3, 6)) != 99) { f += 131072; }
    if (m.len() != 1) { f += 262144; }
    return f;
}
"#;

#[test]
fn lane_interchange_both_directions_on_one_table() {
    assert_eq!(run_main(INTERLEAVE_SRC), 0, "range key == slice key of the same content, both directions; a view parent flattens to the root");
}

// ---- growth through the range path -----------------------------------

/// Multi-grow sweep from cap 4 through `put_range`: 500 injective
/// 4-char keys fire the load law's sentinel grows again and again,
/// remove / re-add churn exercises the tombstone + DEAD-reuse paths,
/// and every val is exact afterwards — the `[?V]` relocation drain
/// works through the sv lane exactly as through `put`. The plain
/// lanes still see every entry (same keys).
const GROW_SRC: &str = r#"
use nmapset::{ HashMap };

pub fn main() -> i64 {
    // one parent of 4-char injective decimal slots (1000 + i)
    let mut p = "";
    for (let i = 0; i < 600; i += 1) {
        let base = 1000 + i;
        p = f"{p}{base}";
    }
    let mut m: HashMap<str, i32> = HashMap.with_capacity(4);
    let mut fails: i64 = 0;
    let n = 500;
    for (let i = 0; i < n; i += 1) {
        if (!m.put_range(p, i * 4, 4, i * 7 + 1)) { fails += 1; }
    }
    if (m.len() != n) { fails += 64; }
    for (let i = 0; i < n; i += 3) {
        if (!m.remove_range(p, i * 4, 4)) { fails += 2; }
    }
    for (let i = 0; i < n; i += 3) {
        if (!m.put_range(p, i * 4, 4, i * 7 + 1)) { fails += 4; }
    }
    for (let i = 0; i < n; i += 1) {
        let q = m.get_range(p, i * 4, 4);
        if (q == nil) { fails += 8; }
        if (q != i * 7 + 1) { fails += 16; }
    }
    // the plain lanes see the same entries
    for (let i = 0; i < n; i += 97) {
        if (m.get(p.slice(i * 4, i * 4 + 4)) != i * 7 + 1) { fails += 32; }
    }
    return fails;
}
"#;

#[test]
fn vals_survive_multi_grow_through_put_range() {
    assert_eq!(run_main(GROW_SRC), 0, "500 keys from cap 4: sentinel grows + churn, every val exact through the range path");
}

// ---- the empty range key ---------------------------------------------

/// The empty window is legal at every boundary and IS the `""` key:
/// `put_range(s, len, 0, v)` then `get("")` finds it — and vice versa.
const EMPTY_SRC: &str = r#"
use nmapset::{ HashMap };

pub fn main() -> i64 {
    let s = "abc";
    let mut a: HashMap<str, i32> = HashMap.new();
    let mut b: HashMap<str, i32> = HashMap.new();
    let mut f: i64 = 0;
    if (!a.put_range(s, 3, 0, 7)) { f += 1; }
    if (a.get("") != 7) { f += 2; }
    if (!b.put("", 9)) { f += 4; }
    if (b.get_range(s, 0, 0) != 9) { f += 8; }
    if (b.get_range(s, 3, 0) != 9) { f += 16; }  // any boundary window of len 0
    if (a.len() != 1 || b.len() != 1) { f += 32; }
    return f;
}
"#;

#[test]
fn empty_range_key_is_the_empty_string_key() {
    assert_eq!(run_main(EMPTY_SRC), 0);
}

// ---- traps ------------------------------------------------------------

/// The host's window checks surface through the wrapper as the house
/// `Invalid` trap — a mid-codepoint END is loud, naming the offset.
#[test]
fn boundary_trap_surfaces_the_house_invalid_through_the_wrapper() {
    let mut vm = vm_nmapset(TRAP_SRC);
    let err = vm.call::<_, i64>("main", ()).unwrap_err();
    assert_eq!(err.kind, rut_vm::TrapKind::Invalid, "{}", err.msg);
    assert!(
        err.msg.contains("nmap") && err.msg.contains("not a UTF-8 boundary"),
        "{}",
        err.msg
    );
}

const TRAP_SRC: &str = r#"
use nmapset::{ HashMap };

pub fn main() -> i64 {
    let mut m: HashMap<str, i32> = HashMap.new();
    m.put("ok", 1);
    let utf8 = "héllo";
    m.get_range(utf8, 0, 2);   // byte 2 is mid-codepoint: trap
    return 0;
}
"#;

/// A past-end window on a mutating method is equally loud.
#[test]
fn past_end_trap_surfaces_through_remove_range() {
    let mut vm = vm_nmapset(PAST_END_SRC);
    let err = vm.call::<_, i64>("main", ()).unwrap_err();
    assert_eq!(err.kind, rut_vm::TrapKind::Invalid, "{}", err.msg);
    assert!(
        err.msg.contains("nmap") && err.msg.contains("out of range"),
        "{}",
        err.msg
    );
}

const PAST_END_SRC: &str = r#"
use nmapset::{ HashMap };

pub fn main() -> i64 {
    let mut m: HashMap<str, i32> = HashMap.new();
    m.put("ok", 1);
    let s = "abc";
    m.remove_range(s, 1, 3);   // 1 + 3 = 4 > 3 bytes: trap
    return 0;
}
"#;
