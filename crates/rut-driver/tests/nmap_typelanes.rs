//! The `nmap` typed lanes (the mapset-host plan, phase 2): 15
//! `map_{entry,find,remove}_{i,u,b,s,y}` crossings where the key crosses
//! DIRECTLY — no `opaque(..)` mint, no wrapper-hash `h` — with the hash
//! computed host-side by the shared `hash_payload` (mapset's mix64 /
//! FNV-1a constants, bit-for-bit) and the grow-check FUSED into
//! `map_entry_*` as the `i32::MIN` sentinel.
//!
//! Covered: the lanes agree with the Opaque lane on the SAME table ops
//! (same slot outcomes, incl. mixed int widths through `KeyPayload::Bits`
//! collapse), explicit widening distinctness (-1i8 vs 255u8 vs 255i64),
//! the bool lane, str/bytes content equality, the u64 raw-bit crossing
//! (2^63 carries its sign bit), and the sentinel fires EXACTLY at the
//! load boundary — plus a grow + drain + retry driven through it.

use std::rc::Rc;

use rut_driver::{Module, Session};

const NMAP_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/nmap");

const SRC: &str = r#"
use nmap::{ map_new, map_entry, map_find, map_remove, map_len, map_needs_grow, map_grow,
            map_take_reloc, map_cap,
            map_entry_i, map_entry_u, map_entry_b, map_entry_s, map_entry_y,
            map_find_i, map_find_u, map_find_b, map_find_s, map_find_y,
            map_remove_i, map_remove_u, map_remove_b, map_remove_s, map_remove_y };

// the wrapper-side hash vocabulary (mapset.rut verbatim) — used ONLY by
// the Opaque-lane ops here; the typed lanes compute host-side and must
// agree with these bits
fn mix64(bits: u64) -> u64 {
    return (14695981039346656037u64 ^ bits).wrapping_mul(1099511628211u64);
}

fn ihash(bits: u64) -> i64 { return mix64(bits) as i64; }

fn fnv1a64(data: bytes) -> u64 {
    let mut h: u64 = 14695981039346656037u64;
    for (let b of data) {
        h = (h ^ b as u64).wrapping_mul(1099511628211u64);
    }
    return h;
}

fn bhash(data: bytes) -> i64 { return fnv1a64(data) as i64; }

// the sentinel threshold partition — every insert answer is -(slot + 1)
// >= -2^30 - 1, so anything below -1.5 * 2^30 IS the grow-first answer
fn is_grow_first(at: i32) -> bool { return at < -1610612736; }

// the i/u/b lanes on ONE table: Opaque-lane puts (wrapper hash) and
// typed-lane ops interleaved — the lanes must agree slot-for-slot
entry fn lanes_agree() -> i64 {
    let t = map_new(16);
    let mut fails: i64 = 0;
    // puts through the Opaque lane (wrapper hash); typed finds answer
    // the same slots from the host-computed hash
    let mut i: i64 = 0;
    while (i < 6) {
        if (map_entry(t, opaque(i), ihash(i as u64)) >= 0) { fails += 1; }
        i += 1;
    }
    if (map_len(t) != 6) { fails += 10; }
    // typed ops on that same table
    if (map_entry_i(t, 2) < 0) { fails += 100; }        // found slot: replace
    if (map_len(t) != 6) { fails += 1000; }
    if (map_find_i(t, 3) < 0) { fails += 10000; }
    if (map_find_i(t, 77) >= 0) { fails += 100000; }    // a miss
    // a negative key crosses sign-extended: the Opaque put hashed bits
    // 0xFFFF..FFFF (mix64 of -1i64 as u64); the typed find on raw -1
    // must land the same slot
    let at_o = map_entry(t, opaque(-1), ihash(18446744073709551615u64));
    if (at_o >= 0) { fails += 1000000; }
    let fi = map_find_i(t, -1);
    if (fi == -1) { fails += 10000000; }
    if (fi != -(at_o + 1)) { fails += 100000000; }      // lanes agree exactly
    // the i/u lanes share the Bits kind: 255u8 and 255i64 are one key
    let au = map_entry_u(t, 255u64);
    if (au >= 0) { fails += 1000000000; }
    if (map_find_i(t, 255) != -(au + 1)) { fails += 10000000000i64; }
    // remove through the typed lane what the Opaque lane stored
    if (map_remove_i(t, 3) < 0) { fails += 100000000000i64; }
    if (map_len(t) != 7) { fails += 1000000000000i64; }
    if (map_find_i(t, 3) >= 0) { fails += 10000000000000i64; }
    return fails;
}

// explicit widening distinctness: -1i8 (sign-extended = bits
// 0xFFFF..FFFF) vs 255u8 (zero-extended = bits 255) vs 255i64 — the
// plan's three keys; and the narrow widths of one sign collapse
entry fn widen_lanes() -> i64 {
    // -1i8 crosses as i64 -1: one key under the host's bits law
    let n8: i8 = -1;
    let n16: i16 = -1;
    let n32: i32 = -1;
    let t = map_new(8);
    let mut fails: i64 = 0;
    if (map_entry_i(t, n8 as i64) >= 0) { fails += 1; }
    if (map_len(t) != 1) { fails += 2; }
    if (map_find_i(t, n16 as i64) < 0) { fails += 4; }
    if (map_find_i(t, n32 as i64) < 0) { fails += 8; }
    if (map_find_i(t, -1) < 0) { fails += 16; }
    // 255u8 vs 255i64: equal bits collide (the Opaque lane's contract)
    let u8v: u8 = 255;
    let t2 = map_new(8);
    if (map_entry_u(t2, u8v as u64) >= 0) { fails += 32; }
    if (map_find_i(t2, 255) < 0) { fails += 64; }        // same key, i lane
    if (map_len(t2) != 1) { fails += 128; }
    // and the three plan keys separate: -1i8 is NOT 255, and 255i64/
    // 255u8/256 never collapse into -1's slot
    let t3 = map_new(8);
    if (map_entry_i(t3, n8 as i64) >= 0) { fails += 256; }
    if (map_find_u(t3, 255u64) >= 0) { fails += 512; }   // distinct from -1
    if (map_find_i(t3, 255) >= 0) { fails += 1024; }     // distinct too
    if (map_len(t3) != 1) { fails += 2048; }
    return fails;
}

// the bool lane: mix64(1) / mix64(0) — the wrapper bool hash's exact
// bits, and consistent with an Opaque-lane put of `true`
entry fn bool_lane() -> i64 {
    let t = map_new(8);
    let mut fails: i64 = 0;
    if (map_entry_b(t, true) >= 0) { fails += 1; }
    if (map_entry_b(t, false) >= 0) { fails += 2; }
    if (map_len(t) != 2) { fails += 4; }
    let tt = map_find_b(t, true);
    let ff = map_find_b(t, false);
    if (tt < 0 || ff < 0) { fails += 8; }
    // the Opaque lane with the wrapper's bool hash lands the same slot
    if (map_find(t, opaque(true), ihash(1)) != tt) { fails += 16; }
    if (map_find(t, opaque(false), ihash(0)) != ff) { fails += 32; }
    if (map_remove_b(t, true) < 0) { fails += 64; }
    if (map_find_b(t, true) >= 0) { fails += 128; }
    if (map_remove_b(t, true) >= 0) { fails += 256; }
    if (map_len(t) != 1) { fails += 512; }
    // bool and ints share the Bits kind — the Opaque lane's contract:
    // `opaque(1)` is a FRESH key (the linh's `true` was removed) and a
    // later typed `find_b(true)` lands the same slot again
    if (map_entry(t, opaque(1), ihash(1)) >= 0) { fails += 1024; }
    if (map_len(t) != 2) { fails += 2048; }
    if (map_find_b(t, true) < 0) { fails += 4096; }
    return fails;
}

// str/bytes typed lanes: content equality — a fresh copy of the same
// text finds the slot the Opaque lane stored; misses never share slots
entry fn str_bytes_lanes() -> i64 {
    let t = map_new(8);
    let mut fails: i64 = 0;
    let a: str = "alpha";
    // Opaque-lane put with the wrapper's FNV-1a over the octets
    if (map_entry(t, opaque(a), bhash(a.encode())) >= 0) { fails += 1; }
    if (map_len(t) != 1) { fails += 2; }
    // typed find on a fresh copy: the host hashes the borrowed octets
    // with the SAME constants — same slot
    let b: str = "beta";
    if (map_entry_s(t, b) >= 0) { fails += 4; }
    if (map_len(t) != 2) { fails += 8; }
    let again: str = "alpha";
    if (map_find_s(t, again) < 0) { fails += 16; }
    let miss: str = "delta";
    if (map_find_s(t, miss) >= 0) { fails += 32; }
    // replace through the typed lane: found slot back, no new entry
    if (map_entry_s(t, again) < 0) { fails += 64; }
    if (map_len(t) != 2) { fails += 128; }
    if (map_remove_s(t, again) < 0) { fails += 256; }
    if (map_find_s(t, again) >= 0) { fails += 512; }
    if (map_len(t) != 1) { fails += 1024; }

    // bytes — octet equality, same shape
    let t2 = map_new(8);
    let b1 = bytes.from([1, 2, 3]);
    if (map_entry_y(t2, b1) >= 0) { fails += 2048; }
    if (map_find_y(t2, bytes.from([1, 2, 3])) < 0) { fails += 4096; }
    if (map_find_y(t2, bytes.from([9, 9])) >= 0) { fails += 8192; }
    if (map_remove_y(t2, bytes.from([1, 2, 3])) < 0) { fails += 16384; }
    if (map_len(t2) != 0) { fails += 32768; }
    return fails;
}

// the u64 lane: the raw-bit crossing — 2^63 and 2^64-1 cross WITH their
// sign bits (the u64 param reads the slot's bit pattern), and a
// 2^63-key put through the Opaque lane is found by the typed lane
entry fn u64_lane() -> i64 {
    let t = map_new(8);
    let mut fails: i64 = 0;
    let big: u64 = 9223372036854775808u64;
    let max: u64 = 18446744073709551615u64;
    let ab = map_entry_u(t, big);
    if (ab >= 0) { fails += 1; }
    let am = map_entry_u(t, max);
    if (am >= 0) { fails += 2; }
    if (map_len(t) != 2) { fails += 4; }
    // each finds its own slot — the two big-bit keys stayed distinct
    if (map_find_u(t, big) != -(ab + 1)) { fails += 8; }
    if (map_find_u(t, max) != -(am + 1)) { fails += 16; }
    if (map_find_u(t, 0u64) >= 0) { fails += 32; }
    if (map_find_u(t, 255u64) >= 0) { fails += 64; }
    // the Opaque lane agrees: put via opaque(big) with mix64(big) bits
    let t2 = map_new(8);
    let ao = map_entry(t2, opaque(big), ihash(big));
    if (ao >= 0) { fails += 128; }
    if (map_find_u(t2, big) != -(ao + 1)) { fails += 256; }
    if (map_remove_u(t2, big) < 0) { fails += 512; }
    if (map_len(t2) != 0) { fails += 1024; }
    return fails;
}

// the fused sentinel: cap-8 table, the law (count+1)*10 >= 56 flips at
// count 5 — the 5th insert goes through, the 6th call answers the
// sentinel and stores NOTHING. Then grow + drain + retry through the
// wrapper's shape lands the key.
entry fn sentinel_boundary() -> i64 {
    let t = map_new(8);
    let mut fails: i64 = 0;
    let mut vals: [i64] = [0; 8];
    let mut i: i64 = 0;
    while (i < 4) {
        let at = map_entry_i(t, i);
        if (at >= 0) { fails += 1; }
        if (at < 0) { vals[-(at + 1)] = i; }
        i += 1;
    }
    if (map_len(t) != 4) { fails += 2; }
    // (4+0+1)*10 = 50 < 56 — the 5th insert is still legal
    let a4 = map_entry_i(t, 4);
    if (a4 >= 0) { fails += 4; }
    if (is_grow_first(a4)) { fails += 8; }
    if (a4 < 0) { vals[-(a4 + 1)] = 4; }
    if (map_len(t) != 5) { fails += 16; }
    if (map_cap(t) != 8) { fails += 32; }
    // (5+0+1)*10 = 60 >= 56 — the 6th answers i32::MIN, stores nothing
    if (!is_grow_first(map_entry_i(t, 5))) { fails += 64; }
    if (map_len(t) != 5) { fails += 128; }
    if (map_cap(t) != 8) { fails += 256; }
    if (vals[-(a4 + 1)] != 4) { fails += 512; }
    // grow + drain + retry — the relocation keeps the parallel values
    let new_cap = map_grow(t);
    if (new_cap != 16) { fails += 1024; }
    let mut next: [i64] = [0; new_cap];
    while (true) {
        let packed = map_take_reloc(t);
        if (packed < 0) { break; }
        let old = (packed >> 32) as i32;
        let new = (packed & 4294967295i64) as i32;
        next[new] = vals[old];
    }
    vals = next;
    let mut j: i64 = 0;
    while (j < 5) {
        let at = map_find_i(t, j);
        if (at < 0) { fails += 2048; }
        if (vals[at] != j) { fails += 4096; }
        j += 1;
    }
    if (is_grow_first(map_entry_i(t, 5))) { fails += 8192; }
    if (map_len(t) != 6) { fails += 16384; }
    return fails;
}

// the full fused put shape the phase-3 wrapper will be: one map_entry_i
// per put — the sentinel answers, the caller grows + drains, the SAME
// call retries. `n` keys over cap 4: every value survives every grow.
entry fn grow_retry_through_sentinel(n: i64) -> i64 {
    let t = map_new(4);
    let mut vals: [i64] = [0; 4];
    let mut i: i64 = 0;
    while (i < n) {
        let mut at = map_entry_i(t, i);
        if (is_grow_first(at)) {
            let new_cap = map_grow(t);
            let mut next: [i64] = [0; new_cap];
            while (true) {
                let packed = map_take_reloc(t);
                if (packed < 0) { break; }
                let old = (packed >> 32) as i32;
                let new = (packed & 4294967295i64) as i32;
                next[new] = vals[old];
            }
            vals = next;
            at = map_entry_i(t, i);
        }
        if (is_grow_first(at)) { return -999; }
        let mut slot: i32 = at;
        if (at < 0) { slot = -(at + 1); }
        vals[slot] = i * 7 + 1;
        i += 1;
    }
    if (map_len(t) != n as i32) { return -888; }
    let mut acc: i64 = 0;
    let mut j: i64 = 0;
    while (j < n) {
        let at2 = map_find_i(t, j);
        if (at2 < 0) { return -777; }
        acc = acc + (vals[at2] - (j * 7 + 1));
        j += 1;
    }
    if (acc != 0) { return acc; }
    return map_cap(t) as i64;
}
"#;

fn vm_with_nmap(src_extra: &str) -> Vm {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    // `nmap` — this test's host pkg, mounted under the same name the
    // committed rut/nmap package mounts (not a test copy)
    rut_driver::mount_dir(&mut session, std::path::Path::new(NMAP_DIR)).expect("mount nmap");
    let expected = session.expected_host_fns();
    session
        .register_module(
            "app",
            Module {
                spec: "app".into(),
                source: Some(src_extra.into()),
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
        fuel: Some(20_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    // bindings BEFORE the Vm (RFC 0025): install + contract + boot
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_std::nmap::install_std_nmap(&mut hosts);
    hosts.verify_against(&expected); // rut/nmap/nmap.d.rut ↔ the bodies
    rut_vm::interp::Vm::new(
        Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        hosts,
    )
    .unwrap()
}

use rut_vm::interp::Vm;

fn vm() -> Vm {
    vm_with_nmap(SRC)
}

#[test]
fn the_typed_lanes_agree_with_the_opaque_lane_on_the_same_ops() {
    let mut vm = vm();
    assert_eq!(vm.call::<_, i64>("lanes_agree", ()).unwrap(), 0);
}

#[test]
fn widening_distinctness_and_bits_collapse() {
    let mut vm = vm();
    assert_eq!(vm.call::<_, i64>("widen_lanes", ()).unwrap(), 0);
}

#[test]
fn the_bool_lane_is_consistent_with_the_wrappers_bool_hash() {
    let mut vm = vm();
    assert_eq!(vm.call::<_, i64>("bool_lane", ()).unwrap(), 0);
}

#[test]
fn str_and_bytes_typed_lanes_are_content_equal() {
    let mut vm = vm();
    assert_eq!(vm.call::<_, i64>("str_bytes_lanes", ()).unwrap(), 0);
}

#[test]
fn u64_keys_cross_as_raw_bits_through_the_typed_lane() {
    let mut vm = vm();
    assert_eq!(vm.call::<_, i64>("u64_lane", ()).unwrap(), 0);
}

#[test]
fn the_fused_sentinel_fires_exactly_at_the_load_boundary() {
    let mut vm = vm();
    assert_eq!(
        vm.call::<_, i64>("sentinel_boundary", ()).unwrap(),
        0,
        "sentinel boundary: the law flips at exactly count 5 on cap 8"
    );
}

#[test]
fn the_fused_put_shape_grows_and_retries_through_the_sentinel() {
    let mut vm = vm();
    // never grown: the fresh cap-4 table answers with 0 keys
    assert_eq!(
        vm.call::<_, i64>("grow_retry_through_sentinel", (0i64,))
            .unwrap(),
        4
    );
    // 50 keys over cap 4: every value lands on its key through the
    // grow+drain+retry sequence; the load law ends the run at cap 128
    // (fires at count 49 on cap 64: 500 >= 448)
    let cap = vm
        .call::<_, i64>("grow_retry_through_sentinel", (50i64,))
        .unwrap();
    assert_eq!(cap, 128, "cap doubles to 128 by the 50th key");
}

// the fixture mirror holds: the cli tests/data copy must declare the
// same surface (its bodies bind identically) — compile THAT surface
// against the same source set and run one scenario through it
#[test]
fn the_test_fixture_surface_matches_the_committed_pkg() {
    let fixture_dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/rut-cli/tests/data/nmap"
    );
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    rut_driver::mount_dir(&mut session, std::path::Path::new(fixture_dir)).expect("mount fixture");
    let expected = session.expected_host_fns();
    session
        .register_module(
            "app",
            Module {
                spec: "app".into(),
                source: Some(SRC.into()),
                ..Default::default()
            },
        )
        .unwrap();
    let g = rut_driver::compile_graph(&session, "app");
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_std::nmap::install_std_nmap(&mut hosts);
    hosts.verify_against(&expected);
}
