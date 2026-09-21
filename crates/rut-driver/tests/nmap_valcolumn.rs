//! The `nmap` val column (nmapset-round3, phase 1): two raw crossings
//! — `map_val_set_u(t, slot, v: u64)` / `map_val_get_u(t, slot) ->
//! u64` — over the host table's cap-aligned u64 column. Presence is
//! BY-KEY (a val is valid iff its key slot is occupied; no nil tags),
//! and growth relocates vals WITH the keys host-side, so this path has
//! no relocation drain.
//!
//! Covered: raw-bit round-trips (the u64 sign range crossing
//! bit-exactly, i64 payloads through `as u64`), replace at a found
//! slot, the remove/re-insert staleness law (no val write on remove;
//! the reused DEAD slot takes the fresh val), vals surviving
//! sentinel-driven grows with NO drain, out-of-range slots trapping,
//! and THE CHECKSUM LAW — the bench workload's full wrapper-shaped op
//! sequence (build / replace / hits / misses / remove / re-scan /
//! re-add) through the column must equal the same sequence through
//! today's `nmapset.HashMap` `[?V]` sidecar wrapper, bit for bit.

use std::rc::Rc;

use rut_driver::{Module, Session};

const NMAP_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/nmap");
const NMAPSET_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/nmapset");

// mounted over rut/nmap alone: the raw crossings and the column laws
const COLUMN_SRC: &str = r#"
use nmap::{ map_new, map_grow, map_entry_i, map_find_i, map_remove_i, map_len,
            map_val_set_u, map_val_get_u };

fn is_grow_first(at: i32) -> bool { return at < -1610612736; }

// the wrapper's put shape, column-backed: entry -> grow (NO drain —
// the host moved the vals) -> retry -> store the val at the slot
fn col_put(t: opaque, k: i64, v: u64) -> bool {
    let mut at = map_entry_i(t, k);
    while (is_grow_first(at)) {
        map_grow(t);
        at = map_entry_i(t, k);
    }
    let mut slot: i32 = at;
    if (at < 0) { slot = -(at + 1); }
    map_val_set_u(t, slot, v);
    return at < 0;
}

// raw-bit round-trips: the u64 sign range, i64 payloads reinterpreted,
// replace on the found path, remove/re-insert staleness
entry fn val_lane() -> i64 {
    let t = map_new(8);
    let mut fails: i64 = 0;
    // five keys, vals straight over the sign boundary and back
    let mut i: i64 = 0;
    while (i < 5) {
        if (!col_put(t, i * 11, i as u64)) { fails += 1; }
        i += 1;
    }
    if (map_len(t) != 5) { fails += 2; }
    let big: u64 = 9223372036854775808u64;      // 2^63
    let max: u64 = 18446744073709551615u64;     // 2^64 - 1
    if (!col_put(t, 100, big)) { fails += 4; }
    if (!col_put(t, 101, max)) { fails += 8; }
    if (!col_put(t, 102, 18446744073709551601u64)) { fails += 16; } // 2^64 - 15
    // negative i64 payloads ride the raw lane sign-extended, exactly
    // as the wrapper will reinterpret them client-side
    if (!col_put(t, 103, (-7i64) as u64)) { fails += 32; }
    // every payload reads back bit-exact at its key's found slot
    let mut k: i64 = 0;
    while (k < 5) {
        let at = map_find_i(t, k * 11);
        if (at < 0) { fails += 64; }
        if (map_val_get_u(t, at) != k as u64) { fails += 128; }
        k += 1;
    }
    if (map_val_get_u(t, map_find_i(t, 100)) != big) { fails += 256; }
    if (map_val_get_u(t, map_find_i(t, 101)) != max) { fails += 512; }
    if (map_val_get_u(t, map_find_i(t, 103)) != 18446744073709551609u64) { fails += 1024; }
    // replace at the FOUND slot: no new entry, new bits
    let at = map_find_i(t, 100);
    if (col_put(t, 100, 5)) { fails += 2048; }
    if (map_len(t) != 9) { fails += 4096; }
    if (map_val_get_u(t, at) != 5) { fails += 8192; }
    // remove writes NOTHING (presence-by-key): the freed slot's stale
    // bits are unreachable, and the re-insert is a FRESH insert at the
    // reused DEAD slot (put answers true) that lands the FRESH val
    if (map_remove_i(t, 101) < 0) { fails += 16384; }
    if (!col_put(t, 101, 77)) { fails += 32768; }  // fresh again
    if (map_val_get_u(t, map_find_i(t, 101)) != 77) { fails += 65536; }
    return fails;
}

// the bounds law: an out-of-range caller slot traps (the Rust harness
// asserts the trap message; this entry never returns)
entry fn val_out_of_range() -> nil {
    let t = map_new(8);
    map_val_get_u(t, 9999);
}

// vals survive sentinel-driven grows with NO drain: keys 0..n over a
// cap-4 table, every val exact after the dust settles
entry fn val_grow_sweep(n: i64) -> i64 {
    let t = map_new(4);
    let mut fails: i64 = 0;
    let mut i: i64 = 0;
    while (i < n) {
        if (!col_put(t, i, (i * 7 + 1) as u64)) { fails += 1; }
        i += 1;
    }
    // churn the tombstone / DEAD-slot-reuse paths mid-size
    let mut j: i64 = 0;
    while (j < n) {
        if (map_remove_i(t, j) < 0) { fails += 2; }
        j += 3;
    }
    j = 0;
    while (j < n) {
        if (!col_put(t, j, (j * 7 + 1) as u64)) { fails += 4; }
        j += 3;
    }
    // every surviving key's val is exact at its CURRENT slot
    let mut k: i64 = 0;
    while (k < n) {
        let at = map_find_i(t, k);
        if (at < 0) { fails += 8; }
        if (map_val_get_u(t, at) != (k * 7 + 1) as u64) { fails += 16; }
        k += 1;
    }
    return fails + map_cap(t) as i64;
}
"#;

// mounted over rut/nmap + rut/nmapset: THE CHECKSUM LAW — the bench
// workload's op sequence (nmapset-int/main.rut's `churn`, keyed i64)
// through today's `[?V]` sidecar wrapper vs the same sequence through
// the raw crossings + the column. Identical checksum or the column is
// broken.
const LAW_SRC: &str = r#"
use nmap::{ map_new, map_grow, map_entry_i, map_find_i, map_remove_i, map_len,
            map_val_set_u, map_val_get_u };
use nmapset::{ HashMap };

fn is_grow_first(at: i32) -> bool { return at < -1610612736; }

fn key(i: i64) -> i64 { return i.wrapping_mul(-1640531535); }

// the column path: map_entry_i + map_val_set_u instead of put
fn col_churn(n: i64) -> i64 {
    let t = map_new(4);
    let mut added: i64 = 0;
    let mut i: i64 = 0;
    while (i < n) {
        let mut at = map_entry_i(t, key(i));
        while (is_grow_first(at)) {
            map_grow(t);
            at = map_entry_i(t, key(i));
        }
        if (at < 0) { added += 1; }
        let mut slot: i32 = at;
        if (at < 0) { slot = -(at + 1); }
        map_val_set_u(t, slot, i as u64);
        i += 1;
    }
    let mut replaced: i64 = 0;
    i = 0;
    while (i < n) {
        let mut at = map_entry_i(t, key(i));
        while (is_grow_first(at)) {
            map_grow(t);
            at = map_entry_i(t, key(i));
        }
        if (at >= 0) { replaced += 1; }
        let mut slot: i32 = at;
        if (at < 0) { slot = -(at + 1); }
        map_val_set_u(t, slot, (i + 7) as u64);
        i += 1;
    }
    let mut sum: i64 = 0;
    let mut hits: i64 = 0;
    i = 0;
    while (i < n) {
        let at = map_find_i(t, key(i));
        if (at >= 0) {
            hits += 1;
            sum = sum.wrapping_add(map_val_get_u(t, at) as i64);
        }
        i += 1;
    }
    let mut misses: i64 = 0;
    i = 0;
    while (i < n) {
        if (map_find_i(t, key(i + n)) < 0) { misses += 1; }
        i += 1;
    }
    let mut removed: i64 = 0;
    i = 0;
    while (i < n) {
        if (map_remove_i(t, key(i)) >= 0) { removed += 1; }
        i += 2;
    }
    let mut present: i64 = 0;
    i = 0;
    while (i < n) {
        if (map_find_i(t, key(i)) >= 0) { present += 1; }
        i += 1;
    }
    let mut added2: i64 = 0;
    i = 0;
    while (i < n) {
        let mut at = map_entry_i(t, key(i));
        while (is_grow_first(at)) {
            map_grow(t);
            at = map_entry_i(t, key(i));
        }
        if (at < 0) { added2 += 1; }
        let mut slot: i32 = at;
        if (at < 0) { slot = -(at + 1); }
        map_val_set_u(t, slot, (0i64 - i) as u64);
        i += 2;
    }
    let mut c: i64 = sum;
    c = c.wrapping_add(added.wrapping_mul(31));
    c = c.wrapping_add(replaced.wrapping_mul(37));
    c = c.wrapping_add(hits.wrapping_mul(41));
    c = c.wrapping_add(misses.wrapping_mul(43));
    c = c.wrapping_add(removed.wrapping_mul(47));
    c = c.wrapping_add(present.wrapping_mul(53));
    c = c.wrapping_add(added2.wrapping_mul(59));
    c = c.wrapping_add((map_len(t) as i64).wrapping_mul(61));
    return c;
}

// the SAME sequence through today's sidecar wrapper
fn wrap_churn(n: i64) -> i64 {
    let mut m: HashMap<i64, i64> = HashMap.new();
    let mut added: i64 = 0;
    let mut i: i64 = 0;
    while (i < n) {
        if (m.put(key(i), i)) { added += 1; }
        i += 1;
    }
    let mut replaced: i64 = 0;
    i = 0;
    while (i < n) {
        if (m.put(key(i), i + 7) == false) { replaced += 1; }
        i += 1;
    }
    let mut sum: i64 = 0;
    let mut hits: i64 = 0;
    i = 0;
    while (i < n) {
        let p = m.get(key(i));
        if (p != nil) {
            hits += 1;
            sum = sum.wrapping_add(p);
        }
        i += 1;
    }
    let mut misses: i64 = 0;
    i = 0;
    while (i < n) {
        if (m.get(key(i + n)) == nil) { misses += 1; }
        i += 1;
    }
    let mut removed: i64 = 0;
    i = 0;
    while (i < n) {
        if (m.remove(key(i))) { removed += 1; }
        i += 2;
    }
    let mut present: i64 = 0;
    i = 0;
    while (i < n) {
        if (m.has(key(i))) { present += 1; }
        i += 1;
    }
    let mut added2: i64 = 0;
    i = 0;
    while (i < n) {
        if (m.put(key(i), 0i64 - i)) { added2 += 1; }
        i += 2;
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

entry fn checksum_law(n: i64) -> i64 {
    let w = wrap_churn(n);
    let c = col_churn(n);
    if (w != c) { return 0; }   // the caller pins the checksum itself
    return c;
}
"#;

fn vm_with(dirs: &[&str], src: &str) -> Vm {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    for d in dirs {
        rut_driver::mount_dir(&mut session, std::path::Path::new(d))
            .expect("mount pkg");
    }
    let expected = session.expected_host_fns();
    session
        .register_module(
            "app",
            Module {
                spec: "app".into(),
                source: Some(src.into()),
                ..Default::default()
            },
        )
        .unwrap();
    let g = rut_driver::compile_graph(&session, "app");
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("\n")
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

use rut_vm::interp::Vm;

fn vm_column() -> Vm {
    vm_with(&[NMAP_DIR], COLUMN_SRC)
}

fn vm_law() -> Vm {
    vm_with(&[NMAP_DIR, NMAPSET_DIR], LAW_SRC)
}

#[test]
fn val_bits_round_trip_through_the_column() {
    let mut vm = vm_column();
    assert_eq!(
        vm.call::<_, i64>("val_lane", ()).unwrap(),
        0,
        "raw u64/i64 payloads, replace, remove/re-insert staleness"
    );
}

#[test]
fn vals_survive_grows_without_any_drain() {
    let mut vm = vm_column();
    // 500 keys from cap 4: the load law (count + 1)·10 >= cap·7 fires
    // at counts 2/5/11/22/44/89/179/358 across the doublings, so the
    // final cap is 1024 — tombstone churn included; the return carries
    // fails + the final cap
    assert_eq!(vm.call::<_, i64>("val_grow_sweep", (500i64,)).unwrap(), 1024);
}

#[test]
fn out_of_range_val_slots_trap() {
    let mut vm = vm_column();
    let err = vm
        .call::<_, ()>("val_out_of_range", ())
        .unwrap_err();
    assert!(
        err.msg.contains("nmap") && err.msg.contains("out of range") && err.msg.contains("cap"),
        "{}",
        err.msg
    );
}

#[test]
fn the_column_matches_the_sidecar_wrapper_bit_for_bit() {
    let mut vm = vm_law();
    // the pinned literal: nmapset-int's churn shape at n = 2000 must
    // produce the SAME checksum through both val storages (the entry
    // answers 0 on any disagreement — sum 2013000 + the counters'
    // prime weights, hand-reconcilable from the churn above)
    assert_eq!(
        vm.call::<_, i64>("checksum_law", (2000i64,)).unwrap(),
        2598000,
        "column vs sidecar: same ops, same checksum"
    );
}
