//! The experimental `RefMap` class (refval-round2, phase 1 — the
//! refval-exp round-1 surface revived on the ?T downcast): the nmapset
//! wrapper over the host table's EXPERIMENTAL opaque-val column —
//! `map_val_set_o(t, slot, v: opaque)` / `map_val_get_o(t, slot) ->
//! opaque`. The stored unit is the `opaque(v)` box put mints; `get`
//! re-hands the SAME box and recovers it with `opaque.downcast<V>` —
//! the ?T IS the box (the alias handoff, RFC 0014), so the one-cell
//! law holds BY CONSTRUCTION.
//!
//! Covered per plan §0.3/§0.4: the ONE-CELL ALIAS LAW (mutation
//! through a get result is the map's value — both directions, multiple
//! holders, and the replace law where held aliases keep the
//! pre-replace cell; the round-2 strengthening pins the full put/get/
//! remove lifecycle in ONE stream — aliases across replaces, removes,
//! and re-adds), the rc discipline with counts asserted at checkpoints
//! (release AT the overwrite, release AT the remove, full release at
//! table teardown with live cells incl. the arena-teardown smoke — the
//! table box outliving its entry into Rust — and the GET path
//! rc-neutral to the byte: re-handed cloned handles balance), the trap
//! matrix (out-of-range slots, foreign host boxes loud, get on a
//! never-stored slot, and the WIRING-DRIFT panic — a right-erasure
//! wrong-V box under a found key traps loud, never nil-quiet),
//! sentinel-driven growth with NO drain, and THE CHECKSUM LAW —
//! refvals' op sequence through RefMap answers the same checksum as
//! the same sequence through today's `HashMap` `[?V]` sidecar wrapper
//! (the law carries: identical counters, identical value stream, bit
//! for bit).

use std::rc::Rc;

use rut_driver::{Module, Session};
use rut_vm::interp::Vm;

const NMAP_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/nmap_host");
const NMAPSET_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/nmapset");

const SRC: &str = r#"
use nmap_host::{ map_new, map_len, map_grow, map_entry_i, map_find_i, map_remove_i, map_val_set_o, map_val_get_o };
use nmapset::{ HashMap, RefMap };

struct Pt { x: i32, y: i32 }
struct Other { z: i64 }   // right erasure, wrong V — the drift-trap val

fn key(i: i64) -> i64 { return i.wrapping_mul(-1640531535); }

// ---- the one-cell law (plan §0.3, pinned BEFORE the bench) ----------
// mutation through a get result writes through to the stored cell,
// observed via a SECOND get (one direction) and via a second
// simultaneous holder (the other); both holders and the map agree.

entry fn alias_law() -> i64 {
    let mut m: RefMap<i64, Pt> = RefMap.new();
    m.put(11, Pt { x: 5, y: 6 });
    let mut p = m.get(11);
    p.x += 1;
    let mut q = m.get(11);
    let mut fails: i64 = 0;
    if (q.x != 6) { fails += 1; }      // the next get sees the write
    if (p.x != 6) { fails += 2; }
    // the reverse direction: mutate through the SECOND holder
    q.y = 99;
    if (p.y != 99) { fails += 4; }
    // and a FRESH get (a third holder) observes both writes
    let r = m.get(11);
    if (r.x != 6 || r.y != 99) { fails += 8; }
    return fails;
}

// the replace law: a held alias keeps the PRE-replace cell, fresh gets
// see the new one, and the table never grew (a replace, not an insert)
entry fn overwrite_law() -> i64 {
    let mut m: RefMap<i64, Pt> = RefMap.new();
    m.put(3, Pt { x: 1, y: 1 });
    let held = m.get(3);
    if (m.put(3, Pt { x: 2, y: 2 })) { return 100; }  // false: replaced
    let mut fails: i64 = 0;
    if (held.x != 1) { fails += 1; }
    let fresh = m.get(3);
    if (fresh.x != 2) { fails += 2; }
    if (m.len() != 1) { fails += 4; }
    return fails;
}

// remove: the key goes, a held alias keeps the value alive (the mapset
// law), the re-add is a fresh put under a fresh cell
entry fn remove_law() -> i64 {
    let mut m: RefMap<i64, Pt> = RefMap.new();
    m.put(7, Pt { x: 41, y: 1 });
    let held = m.get(7);
    if (m.remove(7) == false) { return 100; }
    let mut fails: i64 = 0;
    if (m.has(7)) { fails += 1; }
    if (m.get(7) != nil) { fails += 2; }
    if (m.len() != 0) { fails += 4; }
    if (held.x != 41) { fails += 8; }   // the alias survived the remove
    // re-add: fresh put, fresh cell
    if (m.put(7, Pt { x: 42, y: 2 }) == false) { fails += 16; }
    let fresh = m.get(7);
    if (fresh.x != 42) { fails += 32; }
    return fails;
}

// ---- the ROUND-2 lifecycle pin (the stronger alias law the ?T
// recovery admits): aliases across the wrapper's OWN put/remove paths
// in one stream — a held alias survives a replace (staying mutable on
// its detached cell), the replacement alias IS the map's value, both
// survive a remove, and a re-add under the same key re-points fresh
// gets without touching either; three same-generation holders share
// one cell both directions.
entry fn alias_across_wrapper_ops() -> i64 {
    let mut m: RefMap<i64, Pt> = RefMap.new();
    m.put(1, Pt { x: 10, y: 10 });
    let mut a = m.get(1);              // alias to cell A
    m.put(1, Pt { x: 20, y: 20 });     // replace: future gets re-point at cell B
    let mut b = m.get(1);              // alias to cell B
    let mut fails: i64 = 0;
    if (a.x != 10 || b.x != 20) { fails += 1; }
    // mutation through the NEW alias is the map's value
    b.x = 25;
    if (m.get(1).x != 25) { fails += 2; }
    // ...while the OLD alias keeps mutating ITS (detached) cell,
    // invisible to the map
    a.x = 15;
    if (m.get(1).x != 25) { fails += 4; }
    // remove: BOTH aliases stay alive and mutable (the mapset law)
    if (m.remove(1) == false) { fails += 8; }
    if (a.y != 10 || b.y != 20) { fails += 16; }
    a.y = 11;
    b.y = 21;
    if (a.y != 11 || b.y != 21) { fails += 32; }
    // re-add under the same key: a fresh cell — fresh gets see it,
    // neither old alias moves
    if (m.put(1, Pt { x: 30, y: 30 }) == false) { fails += 64; }
    if (m.get(1).x != 30) { fails += 128; }
    if (a.x != 15 || b.x != 25) { fails += 256; }
    // three same-generation holders: one mutation, all three see it
    // (the alias handoff — every get re-hands the SAME box)
    let mut h1 = m.get(1);
    let mut h2 = m.get(1);
    let mut h3 = m.get(1);
    h1.y = 33;
    if (h2.y != 33 || h3.y != 33) { fails += 512; }
    if (m.get(1).y != 33) { fails += 1024; }
    return fails;
}

// vals survive sentinel-driven growth with NO drain (the column moves
// with the keys host-side): keys 0..n from a fresh map, every field
// exact after the dust settles
entry fn grow_sweep(n: i64) -> i64 {
    let mut m: RefMap<i64, Pt> = RefMap.new();
    let mut fails: i64 = 0;
    let mut i: i64 = 0;
    while (i < n) {
        if (m.put(key(i), Pt { x: i as i32, y: (i.wrapping_mul(2)) as i32 }) == false) { fails += 1; }
        i += 1;
    }
    if (m.len() != n as i32) { fails += 4; }
    i = 0;
    while (i < n) {
        let p = m.get(key(i));
        if (p.x != i as i32 || p.y != (i.wrapping_mul(2)) as i32) { fails += 8; }
        i += 1;
    }
    return fails;
}

// ---- the rc discipline, byte-accounted at checkpoints (plan §0.4) ----
// The stored unit is the `opaque(v)` box (32 B accounted: 24 header +
// 8 payload) over its record (24 + n*8 B): a big val is an 80 B pair,
// a small one 64 B. Releases are SYNCHRONOUS — the accounting refund
// lands inside the call that released — so the Rust harness reads the
// VM-heap usage between calls and pins the release counts byte-exact:
// at the overwrite (a skipped release would read +2880 — the mints
// alone — instead of the pinned -720), at the remove, and at table
// teardown with live cells.
struct Pt3 { x: i32, y: i32, z: i32 }   // the big val: its box pair is 80 B
struct Pt1 { x: i32 }                   // the small val: its box pair is 64 B

entry fn rc_build(n: i64) -> opaque {
    let t = map_new(8);
    let mut i: i64 = 0;
    while (i < n) {
        let mut at = map_entry_i(t, key(i));
        while (at <= -2147483647) {
            map_grow(t);
            at = map_entry_i(t, key(i));
        }
        let mut slot: i32 = at;
        if (at < 0) { slot = -(at + 1); }
        map_val_set_o(t, slot, opaque(Pt3 { x: i as i32, y: 7, z: 9 }));
        i += 1;
    }
    return t;
}

// overwrite every 2nd key under a SMALL fresh val: 45 mints (+45*64)
// and — the checkpoint — 45 big releases (-45*80)
entry fn rc_overwrite(t: opaque, n: i64) -> i64 {
    let mut i: i64 = 0;
    let mut replaced: i64 = 0;
    while (i < n) {
        let at = map_entry_i(t, key(i));
        if (at >= 0) {
            map_val_set_o(t, at, opaque(Pt1 { x: i as i32 }));
            replaced += 1;
        }
        i += 2;
    }
    return replaced;
}

// remove every 3rd key: 15 small vals (the overwritten ones) and 15
// big ones (never overwritten) release here
entry fn rc_remove(t: opaque, n: i64) -> i64 {
    let mut i: i64 = 0;
    let mut removed: i64 = 0;
    while (i < n) {
        if (map_remove_i(t, key(i)) >= 0) { removed += 1; }
        i += 3;
    }
    return removed;
}

// arena-teardown-with-live-cells: the table box leaves the entry as an
// opaque handle with cells still stored; the Vm (and then the arena)
// drops Rust-side. The release path is the owners'; the caller just
// proves it does not double-free or leak-trap.
entry fn make_map(n: i64) -> opaque {
    let mut m: RefMap<i64, Pt> = RefMap.new();
    let mut i: i64 = 0;
    while (i < n) {
        m.put(key(i), Pt { x: i as i32, y: 9 });
        i += 1;
    }
    return m.t;
}

// ---- the trap matrix -------------------------------------------------

entry fn set_out_of_range() -> nil {
    let mut m: RefMap<i64, Pt> = RefMap.new();
    m.put(1, Pt { x: 1, y: 2 });
    map_val_set_o(m.t, 9999, opaque(Pt { x: 0, y: 0 }));
}

entry fn get_out_of_range() -> nil {
    let mut m: RefMap<i64, Pt> = RefMap.new();
    m.put(1, Pt { x: 1, y: 2 });
    map_val_get_o(m.t, 9999);
}

// a FOREIGN opaque — the table box itself crossed as the val — is
// rejected at the store, loud (host payload boxes cannot be map vals)
entry fn foreign_box_val() -> nil {
    let mut m: RefMap<i64, Pt> = RefMap.new();
    m.put(1, Pt { x: 1, y: 2 });
    map_val_set_o(m.t, 0, m.t);
}

// a get on a slot with no stored box (present keys always have one —
// this is direct misuse of the raw crossing)
entry fn get_unstored_slot() -> nil {
    let m: RefMap<i64, Pt> = RefMap.new();
    map_val_get_o(m.t, 0);
}

// WIRING DRIFT (the round-2 trap the ?T recovery makes testable): a
// right-erasure WRONG-V rut box under a found key — stored through the
// raw crossing, which admits any rut box — must trap LOUD in `get`
// (the downcast answers nil and RefMap panics), never answer a silent
// nil through the nullable surface.
entry fn drift_loud_trap() -> nil {
    let mut m: RefMap<i64, Pt> = RefMap.new();
    m.put(5, Pt { x: 1, y: 2 });
    let at = map_find_i(m.t, 5);
    if (at < 0) { panic("drift: the key is missing"); }
    map_val_set_o(m.t, at, opaque(Other { z: 9 }));
    let p = m.get(5);   // traps: the stored value is not the map's V
    if (p == nil) { panic("drift: unreachable"); }
}

// the GET path is rc-NEUTRAL: re-handed cloned handles balance to the
// byte — 1000 gets over one stored box read +0 B (a leaked handle
// would read +32000, a double-release trips the debug asserts)
entry fn rc_get_loop(t: opaque, n: i64) -> i64 {
    let slot = map_find_i(t, key(0));
    if (slot < 0) { panic("rc loop: the key is missing"); }
    let mut i: i64 = 0;
    let mut sum: i64 = 0;
    while (i < n) {
        let b: opaque = map_val_get_o(t, slot);
        let v = opaque.downcast<Pt>(b);
        if (v == nil) { panic("rc loop: drift"); }
        sum = sum.wrapping_add(v.x as i64);
        i += 1;
    }
    return sum;
}

// ---- the checksum law ------------------------------------------------
// refvals' op sequence (build / overwrite churn / hit-heavy gets both
// fields / misses / READ-MODIFY-WRITE THROUGH THE ALIAS / fresh-get
// read-back / grow-heavy sweep / removal churn) through BOTH wrappers:
// identical counters, identical value stream, identical checksum.

fn refmap_churn(n: i32, gn: i32) -> i64 {
    let mut m: RefMap<i64, Pt> = RefMap.new();
    let mut added: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.put(key(i as i64), Pt { x: i, y: i.wrapping_add(1) })) { added += 1; }
    }
    let mut replaced: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.put(key(i as i64), Pt { x: i.wrapping_add(7), y: i.wrapping_add(8) }) == false) { replaced += 1; }
    }
    let mut sum_before: i64 = 0;
    let mut hits: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        let p = m.get(key(i as i64));
        if (p != nil) {
            hits += 1;
            sum_before = sum_before.wrapping_add((p.x + p.y) as i64);
        }
    }
    let mut misses: i64 = 0;
    for (let i = 0; i < n / 5; i += 1) {
        if (m.get(key((2 * n + i) as i64)) == nil) { misses += 1; }
    }
    let mut rmw: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        let mut p = m.get(key(i as i64));
        if (p != nil) {
            p.x += 1;
            rmw += 1;
        }
    }
    let mut sum_after: i64 = 0;
    let mut hits2: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        let p = m.get(key(i as i64));
        if (p != nil) {
            hits2 += 1;
            sum_after = sum_after.wrapping_add(p.x as i64);
        }
    }
    let mut g: RefMap<i64, Pt> = RefMap.new();
    let mut added_g: i64 = 0;
    for (let i = 0; i < gn; i += 1) {
        if (g.put(key(i as i64), Pt { x: i.wrapping_add(1), y: i.wrapping_mul(2) })) { added_g += 1; }
    }
    let mut replaced_g: i64 = 0;
    for (let i = 0; i < gn; i += 1) {
        if (g.put(key(i as i64), Pt { x: i.wrapping_add(3), y: i.wrapping_mul(2) }) == false) { replaced_g += 1; }
    }
    let mut hits_g: i64 = 0;
    let mut sum_g: i64 = 0;
    for (let i = 0; i < gn; i += 1) {
        let p = g.get(key(i as i64));
        if (p != nil) {
            hits_g += 1;
            sum_g = sum_g.wrapping_add(p.y as i64);
        }
    }
    let mut removed: i64 = 0;
    for (let i = 0; i < n; i += 2) {
        if (m.remove(key(i as i64))) { removed += 1; }
    }
    let mut present: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.has(key(i as i64))) { present += 1; }
    }
    let mut added2: i64 = 0;
    for (let i = 0; i < n; i += 2) {
        if (m.put(key(i as i64), Pt { x: i.wrapping_mul(3), y: i })) { added2 += 1; }
    }
    let mut c: i64 = sum_before;
    c = c.wrapping_add(hits.wrapping_mul(13));
    c = c.wrapping_add(misses.wrapping_mul(17));
    c = c.wrapping_add(replaced.wrapping_mul(11));
    c = c.wrapping_add(added.wrapping_mul(7));
    c = c.wrapping_add(rmw.wrapping_mul(19));
    c = c.wrapping_add(sum_after.wrapping_mul(2));
    c = c.wrapping_add(hits2.wrapping_mul(23));
    c = c.wrapping_add(added_g.wrapping_mul(29));
    c = c.wrapping_add(replaced_g.wrapping_mul(31));
    c = c.wrapping_add(hits_g.wrapping_mul(37));
    c = c.wrapping_add(sum_g.wrapping_mul(3));
    c = c.wrapping_add(removed.wrapping_mul(41));
    c = c.wrapping_add(present.wrapping_mul(43));
    c = c.wrapping_add(added2.wrapping_mul(47));
    c = c.wrapping_add((m.len() as i64).wrapping_mul(53));
    c = c.wrapping_add((g.len() as i64).wrapping_mul(59));
    return c;
}

// the SAME sequence through today's sidecar wrapper — refvals'
// HashMap<i64, Pt> churn, verbatim
fn hashmap_churn(n: i32, gn: i32) -> i64 {
    let mut m: HashMap<i64, Pt> = HashMap.new();
    let mut added: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.put(key(i as i64), Pt { x: i, y: i.wrapping_add(1) })) { added += 1; }
    }
    let mut replaced: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.put(key(i as i64), Pt { x: i.wrapping_add(7), y: i.wrapping_add(8) }) == false) { replaced += 1; }
    }
    let mut sum_before: i64 = 0;
    let mut hits: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        let p = m.get(key(i as i64));
        if (p != nil) {
            hits += 1;
            sum_before = sum_before.wrapping_add((p.x + p.y) as i64);
        }
    }
    let mut misses: i64 = 0;
    for (let i = 0; i < n / 5; i += 1) {
        if (m.get(key((2 * n + i) as i64)) == nil) { misses += 1; }
    }
    let mut rmw: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        let mut p = m.get(key(i as i64));
        if (p != nil) {
            p.x += 1;
            rmw += 1;
        }
    }
    let mut sum_after: i64 = 0;
    let mut hits2: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        let p = m.get(key(i as i64));
        if (p != nil) {
            hits2 += 1;
            sum_after = sum_after.wrapping_add(p.x as i64);
        }
    }
    let mut g: HashMap<i64, Pt> = HashMap.new();
    let mut added_g: i64 = 0;
    for (let i = 0; i < gn; i += 1) {
        if (g.put(key(i as i64), Pt { x: i.wrapping_add(1), y: i.wrapping_mul(2) })) { added_g += 1; }
    }
    let mut replaced_g: i64 = 0;
    for (let i = 0; i < gn; i += 1) {
        if (g.put(key(i as i64), Pt { x: i.wrapping_add(3), y: i.wrapping_mul(2) }) == false) { replaced_g += 1; }
    }
    let mut hits_g: i64 = 0;
    let mut sum_g: i64 = 0;
    for (let i = 0; i < gn; i += 1) {
        let p = g.get(key(i as i64));
        if (p != nil) {
            hits_g += 1;
            sum_g = sum_g.wrapping_add(p.y as i64);
        }
    }
    let mut removed: i64 = 0;
    for (let i = 0; i < n; i += 2) {
        if (m.remove(key(i as i64))) { removed += 1; }
    }
    let mut present: i64 = 0;
    for (let i = 0; i < n; i += 1) {
        if (m.has(key(i as i64))) { present += 1; }
    }
    let mut added2: i64 = 0;
    for (let i = 0; i < n; i += 2) {
        if (m.put(key(i as i64), Pt { x: i.wrapping_mul(3), y: i })) { added2 += 1; }
    }
    let mut c: i64 = sum_before;
    c = c.wrapping_add(hits.wrapping_mul(13));
    c = c.wrapping_add(misses.wrapping_mul(17));
    c = c.wrapping_add(replaced.wrapping_mul(11));
    c = c.wrapping_add(added.wrapping_mul(7));
    c = c.wrapping_add(rmw.wrapping_mul(19));
    c = c.wrapping_add(sum_after.wrapping_mul(2));
    c = c.wrapping_add(hits2.wrapping_mul(23));
    c = c.wrapping_add(added_g.wrapping_mul(29));
    c = c.wrapping_add(replaced_g.wrapping_mul(31));
    c = c.wrapping_add(hits_g.wrapping_mul(37));
    c = c.wrapping_add(sum_g.wrapping_mul(3));
    c = c.wrapping_add(removed.wrapping_mul(41));
    c = c.wrapping_add(present.wrapping_mul(43));
    c = c.wrapping_add(added2.wrapping_mul(47));
    c = c.wrapping_add((m.len() as i64).wrapping_mul(53));
    c = c.wrapping_add((g.len() as i64).wrapping_mul(59));
    return c;
}

entry fn checksum_law(n: i32, gn: i32) -> i64 {
    let w = hashmap_churn(n, gn);
    let r = refmap_churn(n, gn);
    if (w != r) { return 0; }   // the caller pins the checksum itself
    return r;
}

// a str-keyed instantiation rides the s lane (the class is K-generic —
// per-instantiation monomorphization); one entry, two key flavors
entry fn str_keys() -> i64 {
    let mut m: RefMap<str, Pt> = RefMap.new();
    m.put("alpha", Pt { x: 1, y: 2 });
    m.put("beta", Pt { x: 3, y: 4 });
    let mut fails: i64 = 0;
    if (m.get("alpha").y != 2) { fails += 1; }
    if (m.get("beta").x != 3) { fails += 2; }
    if (m.put("alpha", Pt { x: 9, y: 9 })) { fails += 4; }
    if (m.get("alpha").x != 9) { fails += 8; }
    if (m.len() != 2) { fails += 16; }
    return fails;
}
"#;

fn vm_refmap(src: &str) -> Vm {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    for d in [NMAP_DIR, NMAPSET_DIR] {
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
        fuel: Some(400_000_000),
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

#[test]
fn the_one_cell_law_pins_both_directions_and_multiple_holders() {
    let mut vm = vm_refmap(SRC);
    assert_eq!(
        vm.call::<_, i64>("alias_law", ()).unwrap(),
        0,
        "mutation through a get result IS the map's value"
    );
}

#[test]
fn the_alias_law_holds_across_the_wrappers_own_put_remove_paths() {
    let mut vm = vm_refmap(SRC);
    // the round-2 strengthening: aliases across replace / remove /
    // re-add in one stream, plus three same-generation holders — the
    // alias handoff makes every one of these hold BY CONSTRUCTION
    assert_eq!(
        vm.call::<_, i64>("alias_across_wrapper_ops", ()).unwrap(),
        0,
        "held aliases survive replace/remove/re-add; same-generation holders share one cell"
    );
}

#[test]
fn replace_repoints_future_gets_and_held_aliases_keep_the_old_cell() {
    let mut vm = vm_refmap(SRC);
    assert_eq!(vm.call::<_, i64>("overwrite_law", ()).unwrap(), 0);
}

#[test]
fn remove_frees_the_slot_and_a_held_alias_survives() {
    let mut vm = vm_refmap(SRC);
    assert_eq!(vm.call::<_, i64>("remove_law", ()).unwrap(), 0);
}

#[test]
fn vals_survive_sentinel_grows_with_no_drain() {
    let mut vm = vm_refmap(SRC);
    // 500 keys from cap 4: the load law (count+1)·10 >= cap·7 fires at
    // counts 2/5/11/22/44/89/179/358 — cap 1024 at the end; the sweep
    // answers fails only (0)
    assert_eq!(vm.call::<_, i64>("grow_sweep", (500i64,)).unwrap(), 0);
}

#[test]
fn the_rc_discipline_releases_at_overwrite_remove_and_teardown() {
    let mut vm = vm_refmap(SRC);
    // Byte-level release accounting (the arithmetic is in the SRC
    // comments): a big val's box pair is 80 B (box 24+8, Pt3 24+24), a
    // small one 64 B (box 24+8, Pt1 24+8); the table box is 24+144.
    // n = 90: overwrites 45, removes 15 small + 15 big, teardown the
    // remaining 30 small + 30 big.
    let big = 80i64;
    let small = 64i64;
    let table_box = 24i64 + 144;
    let h0 = vm.heap_usage() as i64;
    let t = vm.call::<_, rut_vm::OpaqueRef>("rc_build", (90i64,)).unwrap();
    let h1 = vm.heap_usage() as i64;
    assert_eq!(vm.call::<_, i64>("rc_overwrite", (t.clone(), 90i64)).unwrap(), 45);
    let h2 = vm.heap_usage() as i64;
    assert_eq!(
        h2 - h1,
        45 * small - 45 * big,
        "release AT the overwrite: 45 mints minus 45 released big pairs \
         (a skipped release would read +2880)"
    );
    assert_eq!(vm.call::<_, i64>("rc_remove", (t.clone(), 90i64)).unwrap(), 30);
    let h3 = vm.heap_usage() as i64;
    assert_eq!(
        h2 - h3,
        15 * small + 15 * big,
        "release AT the remove: 15 overwritten (small) + 15 never-overwritten (big)"
    );
    // teardown: the table box drops Rust-side with 60 live cells stored
    drop(t);
    let h4 = vm.heap_usage() as i64;
    assert_eq!(
        h3 - h4,
        30 * small + 30 * big + table_box,
        "full release at table teardown with live cells"
    );
    assert_eq!(h4, h0, "everything refunded, nothing leaked");
}

#[test]
fn the_table_box_survives_into_rust_and_arena_teardown_stays_clean() {
    let mut vm = vm_refmap(SRC);
    // the table box leaves the entry with 50 live cells stored
    let _handle = vm
        .call::<_, rut_vm::OpaqueRef>("make_map", (50i64,))
        .expect("the table's opaque handle crosses back");
    // dropping the Vm tears the arena down with live cells inside the
    // table — the owners release, no double-free (debug asserts on)
    drop(vm);
}

#[test]
fn the_get_path_is_rc_neutral_byte_accounted() {
    let mut vm = vm_refmap(SRC);
    // one stored Pt; 1000 get crossings + ?T recoveries must balance
    // to the byte — every re-handed handle dies with its loop-local
    // holder (a leaked handle reads +32000: 1000 x 32 B per box)
    let t = vm.call::<_, rut_vm::OpaqueRef>("make_map", (1i64,)).unwrap();
    let h0 = vm.heap_usage() as i64;
    assert_eq!(
        vm.call::<_, i64>("rc_get_loop", (t.clone(), 1000i64)).unwrap(),
        0
    );
    let h1 = vm.heap_usage() as i64;
    assert_eq!(
        h1 - h0,
        0,
        "1000 gets re-hand and release: +0 B (a leaked handle reads +32000)"
    );
}

#[test]
fn the_trap_matrix_is_loud() {
    // out-of-range slots, both directions — the val lanes' shared law
    // (a fresh Vm per call: a trapped machine is not re-entered)
    let mut vm = vm_refmap(SRC);
    let err = vm.call::<_, ()>("set_out_of_range", ()).unwrap_err();
    assert!(
        err.msg.contains("nmap") && err.msg.contains("out of range") && err.msg.contains("cap"),
        "{}",
        err.msg
    );
    let mut vm = vm_refmap(SRC);
    let err = vm.call::<_, ()>("get_out_of_range", ()).unwrap_err();
    assert!(
        err.msg.contains("nmap") && err.msg.contains("out of range") && err.msg.contains("cap"),
        "{}",
        err.msg
    );
    // a FOREIGN opaque — the table box itself — is rejected at the store
    let mut vm = vm_refmap(SRC);
    let err = vm.call::<_, ()>("foreign_box_val", ()).unwrap_err();
    assert!(
        err.msg.contains("map_val_set_o") && err.msg.contains("host payload box"),
        "{}",
        err.msg
    );
    // a get on a slot with no stored box is a loud caller bug
    let mut vm = vm_refmap(SRC);
    let err = vm.call::<_, ()>("get_unstored_slot", ()).unwrap_err();
    assert!(
        err.msg.contains("no val stored at slot"),
        "{}",
        err.msg
    );
    // WIRING DRIFT (the round-2 trap): a right-erasure wrong-V box
    // under a found key — `get`'s ?T downcast answers nil and RefMap
    // panics LOUD, never a silent nil through the nullable surface
    let mut vm = vm_refmap(SRC);
    let err = vm.call::<_, ()>("drift_loud_trap", ()).unwrap_err();
    assert!(
        err.msg.contains("refmap") && err.msg.contains("not the map's V"),
        "{}",
        err.msg
    );
}

#[test]
fn the_checksum_law_refmap_answers_the_sidecar_wrapper_bit_for_bit() {
    let mut vm = vm_refmap(SRC);
    // the refvals op sequence at n = 2000 / gn = 4000 through BOTH
    // val storages; the entry answers 0 on any disagreement. The
    // pinned literal is hand-reconciled from the exact counters:
    // added/replaced/hits/misses/rmw/hits2 = 2000/2000/2000/400/2000/
    // 2000, sum_before = sum(2i+15) = 4028000, sum_after = sum(i+8) =
    // 2015000, grows-sweep counters 4000/4000/4000 with sum_g = 2i's
    // sum 15996000, removal churn 1000/1000/1000, lens 2000/4000 —
    // under refvals' prime-weight table this lands on 57059800.
    assert_eq!(
        vm.call::<_, i64>("checksum_law", (2000i32, 4000i32)).unwrap(),
        57059800,
        "refcolumn == refvals' sequence, bit for bit"
    );
}

#[test]
fn str_keyed_instantiation_rides_the_s_lane() {
    let mut vm = vm_refmap(SRC);
    assert_eq!(vm.call::<_, i64>("str_keys", ()).unwrap(), 0);
}
