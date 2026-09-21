//! The `nmapset` prim-val maps (nmapset-round3, phase 2): the
//! `PrimMapI64` / `PrimMapU64` / `PrimMapF64` classes over the host
//! table's native val column — keys AND values both Rust-side, no
//! `[?V]` sidecar, no relocation drain (the host moves vals with the
//! keys inside `map_grow`).
//!
//! Covered: the PARITY law — the identical op sequence over i32 keys /
//! i64 vals through `PrimMapI64` and through today's
//! `HashMap<i32, i64>` sidecar wrapper answers the SAME checksum
//! (n = 2000 -> 2598000, the nmapset-int churn shape), the prim `get`
//! semantics (fresh opt per hit — a held value keeps the pre-replace
//! bits, a fresh get reads the replacement, identically on both
//! classes), a multi-grow sweep under collisions THROUGH the wrapper,
//! u64 raw-bit round-trips (2^63..2^64-1) and f64 bit round-trips
//! incl. the -0.0 sign, K admission (the union diagnostic, unchanged),
//! and the coexistence of the new classes with `HashMap`/`HashSet` in
//! one program. The bool val lane is deferred (`?bool` cannot serve
//! the `get -> ?V` nil law in today's checker — the class family's
//! header comment records the repro).

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

fn diags_of(src: &str) -> Vec<String> {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    rut_driver::mount_dir(&mut session, std::path::Path::new(NMAPSET_DIR))
        .expect("mount pkg");
    session
        .register_module(
            "app",
            Module { spec: "app".into(), source: Some(src.into()), ..Default::default() },
        )
        .unwrap();
    rut_driver::compile_graph(&session, "app")
        .diags
        .iter()
        .map(|d| d.msg.clone())
        .collect()
}

// ---- the parity law -------------------------------------------------

/// The nmapset-int churn shape at n = 2000, keyed i32 with i64 vals,
/// through BOTH val storages: `PrimMapI64` (the column) and
/// `HashMap<i32, i64>` (the `[?V]` sidecar). Identical checksum — the
/// pinned 2598000 the phase-1 column law also answers — plus the same
/// per-key has/get answers after the churn (order where observable).
const PARITY_SRC: &str = r#"
use nmapset::{ HashMap, PrimMapI64 };

fn key(i: i32) -> i32 { return i.wrapping_mul(-1640531535); }

fn churn_col(n: i32) -> i64 {
    let mut m: PrimMapI64<i32> = PrimMapI64.new();
    let mut added: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.put(key(i), i as i64)) { added += 1; }
    }
    let mut replaced: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.put(key(i), i as i64 + 7) == false) { replaced += 1; }
    }
    let mut sum: i64 = 0;
    let mut hits: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        let p = m.get(key(i));
        if (p != nil) {
            hits += 1;
            sum = sum.wrapping_add(p);
        }
    }
    let mut misses: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.get(key(i + n)) == nil) { misses += 1; }
    }
    let mut removed: i64 = 0;
    for (let i = 0; i < n; i += 2) {
        if (m.remove(key(i))) { removed += 1; }
    }
    let mut present: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.has(key(i))) { present += 1; }
    }
    let mut added2: i64 = 0;
    for (let i = 0; i < n; i += 2) {
        if (m.put(key(i), 0i64 - i as i64)) { added2 += 1; }
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

fn churn_sidecar(n: i32) -> i64 {
    let mut m: HashMap<i32, i64> = HashMap.new();
    let mut added: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.put(key(i), i as i64)) { added += 1; }
    }
    let mut replaced: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.put(key(i), i as i64 + 7) == false) { replaced += 1; }
    }
    let mut sum: i64 = 0;
    let mut hits: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        let p = m.get(key(i));
        if (p != nil) {
            hits += 1;
            sum = sum.wrapping_add(p);
        }
    }
    let mut misses: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.get(key(i + n)) == nil) { misses += 1; }
    }
    let mut removed: i64 = 0;
    for (let i = 0; i < n; i += 2) {
        if (m.remove(key(i))) { removed += 1; }
    }
    let mut present: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.has(key(i))) { present += 1; }
    }
    let mut added2: i64 = 0;
    for (let i = 0; i < n; i += 2) {
        if (m.put(key(i), 0i64 - i as i64)) { added2 += 1; }
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
    let col = churn_col(2000);
    let side = churn_sidecar(2000);
    if (col != side) { return 0; }   // the caller pins the checksum itself
    return col;
}
"#;

#[test]
fn primmap_column_parity_with_the_hashmap_sidecar() {
    // the pinned literal: the nmapset-int churn shape at n = 2000 (the
    // phase-1 column law's number — sum 2013000 + the counters' prime
    // weights), identical through both classes
    assert_eq!(
        run_main(PARITY_SRC),
        2598000,
        "PrimMapI64 column vs HashMap sidecar: same ops, same checksum"
    );
}

/// The prim `get` semantics, mirrored against the sidecar in ONE
/// program: a hit returns a FRESH opt holding the value's bits — a
/// held `p1` keeps the pre-replace value (a prim copy, round-2
/// semantics), a fresh get reads the replacement, remove nils the
/// lookup without touching the held copy, and the re-insert lands the
/// fresh value at the reused slot. Both classes answer identically.
const GET_SEMANTICS_SRC: &str = r#"
use nmapset::{ HashMap, PrimMapI64 };

fn law_col() -> i64 {
    let mut m: PrimMapI64<i32> = PrimMapI64.new();
    let mut fails: i64 = 0;
    if (!m.put(1, 10)) { fails += 1; }
    let p1 = m.get(1);
    if (p1 != 10i64) { fails += 2; }
    if (m.put(1, 20) != false) { fails += 4; }
    if (p1 != 10i64) { fails += 8; }          // the held copy keeps the old bits
    if (m.get(1) != 20i64) { fails += 16; }   // a fresh get reads the replacement
    if (m.remove(1) != true) { fails += 32; }
    if (m.get(1) != nil) { fails += 64; }
    if (p1 != 10i64) { fails += 128; }        // the held copy survives the remove
    if (m.put(1, 30) != true) { fails += 256; }
    if (m.get(1) != 30i64) { fails += 512; }
    return fails;
}

fn law_sidecar() -> i64 {
    let mut m: HashMap<i32, i64> = HashMap.new();
    let mut fails: i64 = 0;
    if (!m.put(1, 10)) { fails += 1; }
    let p1 = m.get(1);
    if (p1 != 10i64) { fails += 2; }
    if (m.put(1, 20) != false) { fails += 4; }
    if (p1 != 10i64) { fails += 8; }
    if (m.get(1) != 20i64) { fails += 16; }
    if (m.remove(1) != true) { fails += 32; }
    if (m.get(1) != nil) { fails += 64; }
    if (p1 != 10i64) { fails += 128; }
    if (m.put(1, 30) != true) { fails += 256; }
    if (m.get(1) != 30i64) { fails += 512; }
    return fails;
}

pub fn main() -> i64 {
    let a = law_col();
    let b = law_sidecar();
    if (a != 0 || b != 0) { return 1000 + a * 10 + b; }
    return 0;
}
"#;

#[test]
fn prim_get_semantics_match_the_sidecar_both_directions() {
    assert_eq!(run_main(GET_SEMANTICS_SRC), 0, "fresh opt per hit, held copies keep pre-replace bits — identically on both classes");
}

// ---- growth under collisions, through the wrapper -------------------

/// Multi-grow sweep from cap 4 with COLLIDING keys (step 37 over i32
/// keeps landing on shared probes at every small cap), remove /
/// re-insert churn mid-size, and every val exact at its key's slot
/// after the dust settles — the wrapper must never drop a val across
/// the sentinel grow (the drain is gone; the host moves the column).
const GROW_SRC: &str = r#"
use nmapset::{ PrimMapI64 };

pub fn main() -> i64 {
    let mut m: PrimMapI64<i32> = PrimMapI64.with_capacity(4);
    let mut fails: i64 = 0;
    let n = 500;
    // 500 keys from cap 4: the load law (count + 1)·10 >= cap·7 fires
    // at counts 2/5/11/22/44/89/179/358 — eight sentinel grows — and
    // the wrapper runs NO drain (there is nothing left to drain)
    for (let i = 0; i < n; i += 1) {
        if (!m.put(i * 37, i as i64 * 7 + 1)) { fails += 1; }
    }
    // churn the tombstone / DEAD-slot-reuse paths
    for (let i = 0; i < n; i += 3) {
        if (!m.remove(i * 37)) { fails += 2; }
    }
    for (let i = 0; i < n; i += 3) {
        if (!m.put(i * 37, i as i64 * 7 + 1)) { fails += 4; }
    }
    // every surviving key's val is exact
    for (let i = 0; i < n; i += 1) {
        let p = m.get(i * 37);
        if (p == nil) { fails += 8; }
        if (p != i as i64 * 7 + 1) { fails += 16; }
    }
    if (m.len() != n) { fails += 32; }
    return fails;
}
"#;

#[test]
fn primmap_vals_survive_multi_grow_under_collisions() {
    assert_eq!(run_main(GROW_SRC), 0, "500 colliding keys from cap 4: eight sentinel grows, remove/re-add churn, every val exact");
}

// ---- the u64 / f64 lanes at the wrapper -----------------------------

/// Raw bits through `PrimMapU64`: the 2^63..2^64-1 sign range,
/// replace at the found slot (put answers false, new bits visible),
/// the remove/re-insert staleness law (the reused DEAD slot takes the
/// fresh val), negative i64-ish payloads as raw patterns.
const U64_SRC: &str = r#"
use nmapset::{ PrimMapU64 };

pub fn main() -> i64 {
    let mut m: PrimMapU64<i32> = PrimMapU64.new();
    let mut fails: i64 = 0;
    let big: u64 = 9223372036854775808u64;      // 2^63
    let max: u64 = 18446744073709551615u64;     // 2^64 - 1
    if (!m.put(1, big)) { fails += 1; }
    if (!m.put(2, max)) { fails += 2; }
    if (!m.put(3, 18446744073709551601u64)) { fails += 4; }
    if (m.get(1) != big) { fails += 8; }
    if (m.get(2) != max) { fails += 16; }
    if (m.get(3) != 18446744073709551601u64) { fails += 32; }
    // miss + has + len
    if (m.get(99) != nil) { fails += 64; }
    if (!m.has(2) || m.has(99)) { fails += 128; }
    if (m.len() != 3) { fails += 256; }
    // replace at the found slot: no new entry, new bits
    if (m.put(2, 5u64) != false) { fails += 512; }
    if (m.len() != 3) { fails += 1024; }
    if (m.get(2) != 5u64) { fails += 2048; }
    // remove writes nothing; the re-insert lands the FRESH val
    if (!m.remove(2)) { fails += 4096; }
    if (m.has(2)) { fails += 8192; }
    if (!m.put(2, 77u64)) { fails += 16384; }
    if (m.get(2) != 77u64) { fails += 32768; }
    return fails;
}
"#;

#[test]
fn primmap_u64_lane_round_trips_raw_bits() {
    assert_eq!(run_main(U64_SRC), 0);
}

/// Float bits through `PrimMapF64`: exact values, the -0.0 sign bit
/// (1/-0.0 < 0 while 1/0.0 > 0 — value equality can't see it, bits
/// can), replace, remove/re-add, mixed keys over a grow.
const F64_SRC: &str = r#"
use nmapset::{ PrimMapF64 };

pub fn main() -> i64 {
    let mut m: PrimMapF64<i32> = PrimMapF64.new();
    let mut fails: i64 = 0;
    if (!m.put(1, 1.5f64)) { fails += 1; }
    if (!m.put(2, -2.25f64)) { fails += 2; }
    if (!m.put(3, 0.0f64)) { fails += 4; }
    if (!m.put(4, -0.0f64)) { fails += 8; }
    if (!m.put(5, 65536.0f64)) { fails += 16; }
    if (m.get(1) != 1.5f64) { fails += 32; }
    if (m.get(2) != -2.25f64) { fails += 64; }
    if (m.get(5) != 65536.0f64) { fails += 128; }
    // -0.0 arrived as -0.0: its reciprocal is -inf (0.0's is +inf)
    let nz: f64 = m.get(4);
    if (m.get(4) == nil) { fails += 256; }
    let zn: f64 = 1.0f64 / nz;
    if (!(zn < 0.0f64)) { fails += 512; }
    let z: f64 = m.get(3);
    let zp: f64 = 1.0f64 / z;
    if (!(zp > 0.0f64)) { fails += 1024; }
    // miss + replace + remove/re-add
    if (m.get(99) != nil) { fails += 2048; }
    if (m.put(1, 7.25f64) != false) { fails += 4096; }
    if (m.get(1) != 7.25f64) { fails += 8192; }
    if (!m.remove(1) || m.get(1) != nil) { fails += 16384; }
    if (!m.put(1, -7.25f64)) { fails += 32768; }
    if (m.get(1) != -7.25f64) { fails += 65536; }
    if (m.len() != 5) { fails += 131072; }
    return fails;
}
"#;

#[test]
fn primmap_f64_lane_round_trips_float_bits() {
    assert_eq!(run_main(F64_SRC), 0);
}

// ---- admission + coexistence ----------------------------------------

/// K admission is the SAME closed union as HashMap's: a user record
/// fails at compile time naming the offending type and the union.
#[test]
fn primmap_k_admission_names_the_union() {
    let ds = diags_of(
        "use nmapset::{ PrimMapI64 };\n\
         struct Pt { x: i32; y: i32 }\n\
         pub fn main() -> i32 {\n\
         \x20   let mut pm: PrimMapI64<Pt> = PrimMapI64.new();\n\
         \x20   return 0;\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`Pt` does not satisfy `K` requires `i8 | i16 | i32 | i64 | u8 | u16 | u32 | u64 | bool | str | bytes`")),
        "the diagnostic names the offending type and the union: {ds:?}"
    );
}

/// The new classes coexist with `HashMap` / `HashSet` in one program
/// (additive surface — nothing about the existing classes moved), and
/// a `str`-keyed PrimMap rides the s lane with i64 vals.
#[test]
fn primmap_coexists_with_hashmap_and_hashset() {
    let checksum = run_main(
        "use nmapset::{ HashMap, HashSet, PrimMapI64 };\n\
         pub fn main() -> i64 {\n\
         \x20   let mut pm: PrimMapI64<str> = PrimMapI64.new();\n\
         \x20   let mut acc: i64 = 0;\n\
         \x20   for (let i = 0; i < 24; i += 1) {\n\
         \x20       if (pm.put(f\"k{i}\", i as i64 * 3)) { acc += 1; }\n\
         \x20   }\n\
         \x20   acc += pm.get(\"k7\");\n\
         \x20   let mut hm: HashMap<i32, str> = HashMap.new();\n\
         \x20   hm.put(1, \"one\");\n\
         \x20   if (hm.get(1) == \"one\") { acc += 1; }\n\
         \x20   let mut hs: HashSet<i64> = HashSet.new();\n\
         \x20   hs.put(5i64);\n\
         \x20   if (hs.has(5i64)) { acc += 1; }\n\
         \x20   return acc + pm.len() as i64;\n\
         }\n",
    );
    assert_eq!(checksum, 24 + 21 + 1 + 1 + 24, "str lane adds, the k7 val 21, the ref-V and set checks, len");
}
