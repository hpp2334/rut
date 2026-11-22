//! nmap's white-box suite, moved out of `src/nmap.rs`: the
//! `NativeTable` internals pinned at the Rust boundary — the pinned
//! hash words, the packed answers, the handle births, the val lanes,
//! the seal laws, the sv windows, the iteration cursor — re-expressed
//! over the PRODUCTION crossings (`hput`/`hfind`/`hremove`, the
//! `*_ref` borrowed cores the `pkg()` registrations ride) with no
//! test-only wrappers in between. The end-to-end, language-driven
//! coverage (rut programs over the wrapper face) lives in
//! `rut-cli/tests/nmap.rs`.

use rut_core::types::{TY_BOOL, TY_BYTES, TY_I32, TY_I64, TY_I8, TY_STR, TY_U32, TY_U64};
use rut_std::nmap::{
    hash_bytes, hash_payload, iter_key_kind, mint_key_box, pack_answer, seal_answer, seal_transfer,
    sv_range, IterCursor, KeyKind, KeyRef, KeyVal, NativeTable,
};
use rut_vm::interp::Vm;
use rut_vm::{Opaque, OpaqueRef, Slot, Trap, TrapKind, ValSlot, Value};
use std::rc::Rc;

/// A bare Vm (the rut-vm store tests' pattern): the release-context
/// law's `Vm::retain`/`Vm::release` wrappers are what the value-aware
/// paths take, and the heap accounting (`heap_usage`) reads back the
/// balance.
fn bare_vm() -> Vm {
    let prog = rut_core::binary::Program {
        types: rut_core::types::TypeTable::boot(),
        ..rut_core::binary::Program::default()
    };
    rut_vm::interp::Vm::builder()
        .program(Rc::new(prog))
        .limits(rut_vm::interp::Limits::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("bare vm")
}

/// The pinned integer/bool hash word — the pinned-literal tests
/// name the exact bits.
fn mix64(bits: u64) -> u64 {
    (14695981039346656037u64 ^ bits).wrapping_mul(1099511628211u64)
}

fn bits(v: u64) -> KeyVal {
    KeyVal::Bits(v)
}

/// The shallow-size tripwire (the refcolumn batch's precedent):
/// `Opaque::alloc` charges this at every `map_new`, so the heap
/// movers' arithmetic pins on it. The HashMap core made the table
/// SMALLER — the parallel key/hash/state/val/handle vecs are gone.
#[test]
fn native_table_shallow_size_is_pinned() {
    assert_eq!(std::mem::size_of::<NativeTable>(), 40);
}

#[test]
fn the_capacity_law_is_hashmaps_reserve_semantics() {
    // the open-addressing power-of-two law is gone with the probing
    // core; `map_new`'s `cap` is the table's reserve — a lower bound
    // on capacity, read back through the observation door `cap()` (the
    // `map_cap` crossing retired with the wrapper's sidecar
    // pre-size: nothing rut-side has anything left to size)
    assert_eq!(NativeTable::new(0).cap(), 0);
    assert_eq!(NativeTable::new(-5).cap(), 0);
    assert!(NativeTable::new(4).cap() >= 4);
    assert!(NativeTable::new(1000).cap() >= 1000);
}

#[test]
fn key_kinds_admit_but_never_mix() {
    let vm = bare_vm();
    let mut t = NativeTable::new(8);
    assert_eq!(
        t.hput(KeyKind::Bits, bits(1)).unwrap(),
        pack_answer(0, true)
    );
    // the same kind admits; a different one is a trap, not a miss
    assert_eq!(
        t.hput(KeyKind::Bits, bits(2)).unwrap(),
        pack_answer(1, true)
    );
    let err = t.hput(KeyKind::Str, KeyVal::Str("x".into())).unwrap_err();
    assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
    let err = t.hfind(KeyKind::Str, &KeyVal::Str("x".into())).unwrap_err();
    assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
    let err = t
        .hremove(&vm, KeyKind::Str, &KeyVal::Str("x".into()))
        .unwrap_err();
    assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
    assert_eq!(t.len(), 2, "the rejected keys stored nothing");
    assert_eq!(t.kind(), KeyKind::Bits);
}

#[test]
fn str_key_equality_is_content_equality() {
    let vm = bare_vm();
    // one table, one key kind — homogeneity is the contract
    let mut t = NativeTable::new(8);
    let a = t.hput_ref(KeyRef::Str("alpha")).unwrap();
    assert_eq!(a & 1, 1, "fresh");
    let alpha = (a >> 1) as i32;
    // a fresh copy of the same octets finds the same entry
    assert_eq!(t.hfind_ref(KeyRef::Str("alpha")).unwrap(), alpha);
    assert_eq!(t.hfind_ref(KeyRef::Str("alph")).unwrap(), -1);
    // a replace answers the same handle, not newly
    assert_eq!(
        t.hput_ref(KeyRef::Str("alpha")).unwrap(),
        pack_answer(alpha, false)
    );
    assert_eq!(t.len(), 1);
    assert_eq!(t.hremove_ref(&vm, KeyRef::Str("alpha")).unwrap(), alpha);
    assert_eq!(t.hfind_ref(KeyRef::Str("alpha")).unwrap(), -1);

    let mut t = NativeTable::new(8);
    let b = t
        .hput(KeyKind::Bytes, KeyVal::Bytes(vec![9, 8, 7]))
        .unwrap();
    assert_eq!(b & 1, 1, "fresh");
    let hd = (b >> 1) as i32;
    assert_eq!(
        t.hfind(KeyKind::Bytes, &KeyVal::Bytes(vec![9, 8, 7]))
            .unwrap(),
        hd
    );
    assert_eq!(
        t.hfind(KeyKind::Bytes, &KeyVal::Bytes(vec![9, 8])).unwrap(),
        -1
    );
}

// ---- the hash law (the checksum constants — unchanged) ------------

/// The checksum law, pinned as LITERALS: hash_payload must answer
/// the pinned constants' exact words (the `mix64` / `fnv1a64`
/// constants, originally ported bit-for-bit from the removed
/// mapset pkg) — a one-ulp difference breaks the bench gate, which
/// pins the value-derived checksums in `expected.json`. These are
/// also the words `KeyVal`'s `Hash` writes into the map.
#[test]
fn hash_payload_bits_are_pinned_bit_for_bit() {
    // mix64 — the integer/bool lane: (offset ^ bits) * prime
    assert_eq!(hash_payload(&KeyVal::Bits(0)), 12638153115695167455);
    assert_eq!(hash_payload(&KeyVal::Bits(1)), 12638152016183539244);
    assert_eq!(hash_payload(&KeyVal::Bits(11)), 12638163011299821354);
    assert_eq!(hash_payload(&KeyVal::Bits(255)), 12638352127299873646);
    // u64's sign-bit set lives through raw: bits 0xFFFF..FF (an
    // i64's -1, sign-extended) hash 0x509c41b379fe466e
    assert_eq!(hash_payload(&KeyVal::Bits(u64::MAX)), 5808589858502755950);
    // FNV-1a 64 over the octets — the str/bytes lanes
    assert_eq!(
        hash_payload(&KeyVal::Str("alpha".into())),
        9999721509958787115
    );
    assert_eq!(
        hash_payload(&KeyVal::Str("beta".into())),
        8513880941419438247
    );
    assert_eq!(
        hash_payload(&KeyVal::Bytes(vec![1, 2, 3])),
        15035938162879559083
    );
}

/// Widening distinctness: `-1i8`, `255u8`, and `255i64` are three
/// keys the closed set can carry, and the map must see what the
/// crossing bits law sees — signed widths cross SIGN-extended
/// (`-1i8` hashes as bits `0xFFFF..FF`), unsigned widths
/// ZERO-extend, and equal bits collapse to one key (`255u8` ==
/// `255i64`, the homogeneous-K contract).
#[test]
fn widening_distinctness_matches_the_bits_law() {
    let mut t = NativeTable::new(8);
    // -1i8 crosses as i64 -1: bits 0xFFFF..FFFF
    let neg = t.hput(KeyKind::Bits, bits((-1i64) as u64)).unwrap();
    assert_eq!(neg & 1, 1, "fresh");
    // 255u8 and 255i64 are the SAME key (bits 255) — the i/u lanes
    // share the Bits kind, exactly as the crossing law does
    let u8_255 = t.hput(KeyKind::Bits, bits(255)).unwrap();
    assert_eq!(u8_255 & 1, 1, "255 is a fresh key");
    assert_eq!(
        t.hput(KeyKind::Bits, bits(255i64 as u64)).unwrap(),
        u8_255 & !1,
        "same handle, newly = false — equal bits collapse"
    );
    assert_eq!(t.len(), 2);
    // and neither matches -1: the three plan keys are distinct
    // (255u8/255i64 to each other, both from -1i8)
    let found_neg = t.hfind(KeyKind::Bits, &bits((-1i64) as u64)).unwrap();
    assert_ne!(found_neg, (u8_255 >> 1) as i32);
    assert!(found_neg >= 0, "-1i8 is stored, not missing");
    assert_eq!(
        t.hfind(KeyKind::Bits, &bits(255)).unwrap(),
        (u8_255 >> 1) as i32
    );
    assert_eq!(t.hfind(KeyKind::Bits, &bits(256)).unwrap(), -1);
}

/// The bool lane: `true`/`false` hash mix64(1)/mix64(0) — the
/// wrapper's bool hash's exact bits, and the words the map's
/// identity hasher buckets on.
#[test]
fn the_bool_lane_hashes_the_wrappers_bools() {
    let vm = bare_vm();
    let mut t = NativeTable::new(8);
    let tru = t.hput(KeyKind::Bits, bits(1)).unwrap();
    assert_eq!(tru & 1, 1, "fresh");
    assert_eq!(t.hfind(KeyKind::Bits, &bits(1)).unwrap(), (tru >> 1) as i32);
    assert_eq!(
        t.hfind(KeyKind::Bits, &bits(0)).unwrap(),
        -1,
        "false misses"
    );
    let fal = t.hput(KeyKind::Bits, bits(0)).unwrap();
    assert_eq!(fal & 1, 1);
    assert_eq!(t.hfind(KeyKind::Bits, &bits(0)).unwrap(), (fal >> 1) as i32);
    assert_eq!(
        t.hremove(&vm, KeyKind::Bits, &bits(0)).unwrap(),
        (fal >> 1) as i32
    );
    // hash_payload IS mix64(0)/mix64(1) for the bool bits
    assert_eq!(hash_payload(&KeyVal::Bits(0)), mix64(0));
    assert_eq!(hash_payload(&KeyVal::Bits(1)), mix64(1));
}

// ---- the handle-rebased val lanes ---------------------------------

/// Raw-bit round-trips through the handle-rebased lanes: the u64
/// sign range, f64 bit patterns (`to_bits`), replace at the same
/// handle, the EMPTY-trap law (§0.8 g — an hput birth holds no
/// bits; reading them is a caller bug, never a silent zero), and
/// the bounds law (live births; a removed key's handle traps by
/// name, never reinterpreted).
#[test]
fn val_lanes_round_trip_raw_bits_and_trap_on_empty_and_dead_handles() {
    let vm = bare_vm();
    let mut t = NativeTable::new(8);
    for k in 0u64..5 {
        let ans = t.hput(KeyKind::Bits, bits(k)).unwrap();
        let hd = (ans >> 1) as i32;
        // the h-family birth holds Empty — a valued read TRAPS
        let err = t.val_get_u(hd).unwrap_err();
        assert!(err.msg.contains("no bits"), "{}", err.msg);
        t.val_set_u(&vm, hd, u64::MAX - k).unwrap();
    }
    for k in 0u64..5 {
        let hd = t.hfind(KeyKind::Bits, &bits(k)).unwrap();
        assert_eq!(t.val_get_u(hd).unwrap(), u64::MAX - k, "key {k}");
    }
    // f64 patterns ride the same Bits storage through the f lane
    let vals = [
        1.5f64,
        -2.25,
        0.0,
        -0.0, // the sign bit is the payload — not "equal" to 0.0's bits
        f64::MIN_POSITIVE,
        f64::MAX,
        f64::MIN,
    ];
    let hd = t.hfind(KeyKind::Bits, &bits(0)).unwrap();
    for (i, v) in vals.iter().enumerate() {
        t.val_set_f(&vm, hd, *v).unwrap();
        assert_eq!(t.val_get_f(hd).unwrap(), *v, "val {i}");
        assert_eq!(t.val_get_u(hd).unwrap(), v.to_bits(), "the raw word {i}");
    }
    // -0.0 vs 0.0: distinct bit patterns in ONE storage
    t.val_set_f(&vm, hd, 0.0).unwrap();
    let zero = t.val_get_u(hd).unwrap();
    t.val_set_f(&vm, hd, -0.0).unwrap();
    assert_ne!(t.val_get_u(hd).unwrap(), zero);
    // bounds = live births: negative and unborn handles trap
    for h in [-1, 5, i32::MAX] {
        let err = t.val_get_u(h).unwrap_err();
        assert_eq!(err.kind, TrapKind::Invalid);
        assert!(err.msg.contains("out of range"), "{}", err.msg);
        let err = t.val_set_u(&vm, h, 1).unwrap_err();
        assert!(err.msg.contains("out of range"), "{}", err.msg);
    }
    // remove kills the handle: a removed key's handle stays inside
    // the births range (monotonic, never recycled) but has no live
    // entry — the trap names exactly that
    let hd = t.hfind(KeyKind::Bits, &bits(1)).unwrap();
    assert_eq!(t.hremove(&vm, KeyKind::Bits, &bits(1)).unwrap(), hd);
    let err = t.val_get_u(hd).unwrap_err();
    assert!(err.msg.contains("no live entry"), "{}", err.msg);
    let err = t.val_set_u(&vm, hd, 9).unwrap_err();
    assert!(err.msg.contains("no live entry"), "{}", err.msg);
}

/// THE PARITY LAW: one churn sequence — puts with
/// internal growth, replaces, removes, re-adds — driven through the
/// h-family + the raw val lanes must agree with a std `HashMap`
/// reference on every observable: the live set, each key's value,
/// len, and the handle identity (`hfind` answers what `hput` packed).
#[test]
fn the_handle_rebased_val_lanes_match_a_reference_map() {
    let vm = bare_vm();
    let mut t = NativeTable::new(4);
    let mut reference: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
    let mut handles: std::collections::HashMap<u64, i32> = std::collections::HashMap::new();
    let val_of = |k: u64| -> u64 {
        if k.is_multiple_of(7) {
            (-(k as f64)).to_bits()
        } else {
            k * 7 + 1
        }
    };
    // 150 puts (growth is std's, internal), val stored per birth
    for k in 0u64..150 {
        let ans = t.hput(KeyKind::Bits, bits(k)).unwrap();
        assert_eq!(ans & 1, 1, "key {k} fresh");
        let hd = (ans >> 1) as i32;
        handles.insert(k, hd);
        t.val_set_u(&vm, hd, val_of(k)).unwrap();
        reference.insert(k, val_of(k));
    }
    assert_eq!(t.len(), 150);
    assert!(t.cap() >= 150, "growth happened internally: {}", t.cap());
    // replaces on the % 4 keys: same handle, new bits
    for k in (0u64..150).step_by(4) {
        let ans = t.hput(KeyKind::Bits, bits(k)).unwrap();
        assert_eq!(ans, pack_answer(handles[&k], false), "replace key {k}");
        t.val_set_u(&vm, handles[&k], val_of(k).wrapping_add(1))
            .unwrap();
        reference.insert(k, val_of(k).wrapping_add(1));
    }
    // removes on the % 3 keys: the dead handle traps afterwards
    for k in (0u64..150).step_by(3) {
        let hd = handles.remove(&k).unwrap();
        assert_eq!(t.hremove(&vm, KeyKind::Bits, &bits(k)).unwrap(), hd);
        reference.remove(&k);
        let err = t.val_get_u(hd).unwrap_err();
        assert!(err.msg.contains("no live entry"), "key {k}: {}", err.msg);
    }
    // re-adds on half the removed keys: a NEW birth, fresh val
    for k in (0u64..150).step_by(6) {
        let ans = t.hput(KeyKind::Bits, bits(k)).unwrap();
        assert_eq!(ans & 1, 1, "re-add key {k} is a new birth");
        let hd = (ans >> 1) as i32;
        assert_ne!(hd, handles.get(&k).copied().unwrap_or(-1), "never recycled");
        handles.insert(k, hd);
        t.val_set_u(&vm, hd, val_of(k).wrapping_add(2)).unwrap();
        reference.insert(k, val_of(k).wrapping_add(2));
    }
    // every survivor reads back exact, and identity holds
    assert_eq!(t.len(), reference.len() as i32);
    for (k, v) in &reference {
        let hd = handles[k];
        assert_eq!(t.hfind(KeyKind::Bits, &bits(*k)).unwrap(), hd, "key {k}");
        assert_eq!(t.val_get_u(hd).unwrap(), *v, "key {k}");
    }
}

// ---- the valued hv lanes (nmap-hostvals P5; the seal law, 2a) ------

/// The valued lanes' storage law end to end: a stored box (the
/// erasure box, `Ref`) round-trips by IDENTITY — `hvget`
/// answers the SAME cell, two gets name one box (the aliasing
/// law) — a miss is `None`, replace releases-and-stores with the
/// SAME handle and `newly = false`, `hremove` (the `map_hvremove`
/// body) answers the dead key's handle and frees the box (the
/// inner str with it — the heap balance closes), and the EMPTY
/// trap fires when an h-family birth is read through a valued
/// lane (§0.8 g). A raw `Bits` cell still answers at the CORE
/// level (the raw lanes store it), while [`seal_answer`] — the
/// row's `?opaque` conversion — refuses it by name.
#[test]
fn the_valued_lanes_round_trip_boxes_trap_on_empty_and_refuse_raw_bits_at_the_row() {
    let mut vm = bare_vm();
    let mut t = NativeTable::new(8);
    let base = vm.heap_usage();
    // seal a value the way the rows do: mint a box, take a handle,
    // hand the rc across with [`seal_transfer`] (the same helper
    // the `v: opaque` registrations run; two boxes of one payload
    // type — equal charges, exact deltas)
    let b0 = Opaque::alloc(&mut vm, 7i64).unwrap();
    let p0 = b0.handle().ptr();
    let sealed0 = seal_transfer(b0.handle().clone(), &mut vm).unwrap();
    let held = vm.heap_usage();
    assert!(held > base, "the box charges its payload: {base} -> {held}");
    // a stored box reads back as the SAME cell — never a copy
    let ans = t.hvput(&vm, KeyKind::Bits, bits(0), sealed0).unwrap();
    assert_eq!(ans, pack_answer(0, true), "fresh birth");
    drop(b0); // the mint rc folds; the store's own rc holds the box
    assert_eq!(
        vm.heap_usage(),
        held,
        "the store took over the box's rc, no new charge"
    );
    let stored = t.hvget(KeyKind::Bits, &bits(0)).unwrap();
    match stored {
        Some(ValSlot::Ref(s)) => assert!(unsafe { s.r } == p0, "identity"),
        other => panic!("expected Ref, got {other:?}"),
    }
    let again = t.hvget(KeyKind::Bits, &bits(0)).unwrap();
    assert_eq!(stored, again, "two gets, one cell");
    // the row's ?opaque conversion: a fresh OWNING handle over the
    // same box (the register's rc; the store keeps its own)
    let handle = seal_answer(&vm, stored).unwrap().expect("some");
    assert!(handle.ptr() == p0);
    drop(handle); // the extra rc folds away
    assert_eq!(vm.heap_usage(), held, "the answer's rc released");
    // the miss is None
    assert_eq!(t.hvget(KeyKind::Bits, &bits(1)).unwrap(), None);
    // replace: same handle, newly = false, the new box stored —
    // the displaced box releases in-crossing
    let b1 = Opaque::alloc(&mut vm, 9i64).unwrap();
    let sealed1 = seal_transfer(b1.handle().clone(), &mut vm).unwrap();
    drop(b1);
    let _before_replace = vm.heap_usage();
    let ans = t.hvput(&vm, KeyKind::Bits, bits(0), sealed1).unwrap();
    assert_eq!(ans, pack_answer(0, false), "replace");
    assert_eq!(
        vm.heap_usage(),
        held,
        "the displaced box freed in-crossing (equal charges)"
    );
    // the h-family placeholder: an hput birth read through a valued
    // lane TRAPS loudly (§0.8 g)
    let ans = t.hput(KeyKind::Bits, bits(9)).unwrap();
    let hd = (ans >> 1) as i32;
    let err = t.hvget(KeyKind::Bits, &bits(9)).unwrap_err();
    assert_eq!(err.kind, TrapKind::Invalid);
    assert!(
        err.msg.contains("Empty") && err.msg.contains("placeholder"),
        "{}",
        err.msg
    );
    // and a valued put OVER that same key replaces the placeholder
    let ans = t
        .hvput(&vm, KeyKind::Bits, bits(9), ValSlot::Bits(Slot::int(90)))
        .unwrap();
    assert_eq!(ans, pack_answer(hd, false), "the birth's handle, not newly");
    // remove answers the dead key's handle; the box dies with it
    assert_eq!(t.hremove(&vm, KeyKind::Bits, &bits(0)).unwrap(), 0);
    assert_eq!(t.hvget(KeyKind::Bits, &bits(0)).unwrap(), None);
    assert_eq!(t.hremove(&vm, KeyKind::Bits, &bits(0)).unwrap(), -1);
    assert_eq!(
        vm.heap_usage(),
        base,
        "the removed key's box freed in-crossing"
    );
    // a raw `map_val_set_*` store is BITS, not a seal box: the core
    // answers it (storage is value-shape-agnostic), the row's
    // ?opaque conversion refuses it by name
    t.val_set_u(&vm, hd, 90).unwrap();
    match t.hvget(KeyKind::Bits, &bits(9)).unwrap() {
        Some(ValSlot::Bits(s)) => assert_eq!(unsafe { s.i }, 90),
        other => panic!("expected Bits at the core, got {other:?}"),
    }
    let err = seal_answer(&vm, t.hvget(KeyKind::Bits, &bits(9)).unwrap()).unwrap_err();
    assert!(
        err.msg.contains("raw bits") && err.msg.contains("not a seal box"),
        "{}",
        err.msg
    );
    // floats never reach the table: the typed key rows admit only
    // the closed set (str/bits/bytes), so there is no float key
    // crossing left to refuse — the admission trap is the wrapper's
    // union bound (compile time), not a host decode
    assert_eq!(t.len(), 1, "nine survives; zero was removed");
}

/// The parity law over the VALUED lanes: one churn sequence — puts
/// with growth, replaces, removes, re-adds — driven through
/// `hvput`/`hvget`/`hremove` must agree with a std `HashMap`
/// reference on every observable (the live set, each key's value,
/// len, the handle identity). Values ride the raw `Bits` spelling
/// here — the core storage is value-shape-agnostic (the raw
/// `map_val_set_*` lanes store the same field); the row-level seal
/// law is the box test above and the rut-cli end-to-end suite.
#[test]
fn the_valued_lanes_match_a_reference_map() {
    let vm = bare_vm();
    let mut t = NativeTable::new(4);
    let mut reference: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
    let mut handles: std::collections::HashMap<u64, i32> = std::collections::HashMap::new();
    let val_of = |k: u64| k * 7 + 1;
    for k in 0u64..150 {
        let ans = t
            .hvput(
                &vm,
                KeyKind::Bits,
                bits(k),
                ValSlot::Bits(Slot::int(val_of(k) as i64)),
            )
            .unwrap();
        assert_eq!(ans & 1, 1, "key {k} fresh");
        handles.insert(k, (ans >> 1) as i32);
        reference.insert(k, val_of(k));
    }
    assert_eq!(t.len(), 150);
    for k in (0u64..150).step_by(4) {
        let ans = t
            .hvput(
                &vm,
                KeyKind::Bits,
                bits(k),
                ValSlot::Bits(Slot::int((val_of(k) + 1) as i64)),
            )
            .unwrap();
        assert_eq!(ans, pack_answer(handles[&k], false), "replace key {k}");
        reference.insert(k, val_of(k) + 1);
    }
    for k in (0u64..150).step_by(3) {
        let hd = handles.remove(&k).unwrap();
        assert_eq!(t.hremove(&vm, KeyKind::Bits, &bits(k)).unwrap(), hd);
        reference.remove(&k);
        assert_eq!(
            t.hvget(KeyKind::Bits, &bits(k)).unwrap(),
            None,
            "key {k} gone"
        );
    }
    assert_eq!(t.len(), reference.len() as i32);
    for (k, v) in &reference {
        let hd = handles[k];
        assert_eq!(
            t.hvget(KeyKind::Bits, &bits(*k)).unwrap(),
            Some(ValSlot::Bits(Slot::int(*v as i64))),
            "key {k}"
        );
        assert_eq!(
            t.hvget(KeyKind::Bits, &bits(*k)).unwrap(),
            t.hvget(KeyKind::Bits, &bits(*k)).unwrap(),
            "identity: two gets read the same storage"
        );
        let _ = hd;
    }
}

/// The finalize hook is WIRED and runs at store-entry death: the
/// hosted mint attaches it, and rc-0's release walk runs it (the
/// Bits values here release as nothing; the clear law and the
/// no-leak law are the unit-visible part). The Ref-valued balance —
/// real cells released exactly once — is pinned end to end by the
/// rut-cli valued-lane suite's heap accounting.
#[test]
fn the_finalize_hook_runs_at_entry_death() {
    let mut vm = bare_vm();
    let base = vm.heap_usage();
    let mut t = NativeTable::new(8);
    for k in 0u64..8 {
        t.hvput(
            &vm,
            KeyKind::Bits,
            bits(k),
            ValSlot::Bits(Slot::int(k as i64 * 3)),
        )
        .unwrap();
    }
    assert_eq!(t.len(), 8);
    let b = Opaque::alloc_hosted(&mut vm, t).expect("hosted mint");
    let held = vm.heap_usage();
    assert!(held > base, "the box charges its payload: {base} -> {held}");
    drop(b);
    assert_eq!(
        vm.heap_usage(),
        base,
        "rc-0 freed the payload; finalize ran inside the walk"
    );
}

// ---- the sv lanes (strings-round1, phase 1) ------------------------

/// Hash-pin for the range hasher: `hash_bytes` is FNV-1a 64 with
/// the ONE constants — bit-for-bit what `hash_payload` answers for
/// the same octets (the Str/Bytes arms route through it), the empty
/// range is the bare offset basis, and a carved window of a longer
/// parent hashes exactly like the whole text.
#[test]
fn hash_bytes_is_bit_for_bit_the_payload_hasher() {
    // the s-lane pins from the hash test above, over raw ranges
    assert_eq!(hash_bytes(b"alpha"), 9999721509958787115);
    assert_eq!(hash_bytes(b"beta"), 8513880941419438247);
    assert_eq!(
        hash_bytes(b""),
        14695981039346656037,
        "the bare offset basis"
    );
    assert_eq!(
        hash_bytes(b"alpha"),
        hash_payload(&KeyVal::Str("alpha".into()))
    );
    assert_eq!(
        hash_bytes(b"beta"),
        hash_payload(&KeyVal::Str("beta".into()))
    );
    // multi-byte content: the range hash runs over the OCTETS
    let s = "héllo→";
    assert_eq!(
        hash_bytes(s.as_bytes()),
        hash_payload(&KeyVal::Str(s.into()))
    );
    // the bytes lane rides the same hasher
    assert_eq!(
        hash_bytes(&[1, 2, 3]),
        hash_payload(&KeyVal::Bytes(vec![1, 2, 3]))
    );
    // a carved window of a longer parent = the whole text's bits
    let parent = "alpha-beta";
    assert_eq!(hash_bytes(&parent.as_bytes()[0..5]), hash_bytes(b"alpha"));
    assert_eq!(
        hash_bytes(&parent.as_bytes()[6..10]),
        hash_payload(&KeyVal::Str("beta".into()))
    );
}

/// The sv range check: byte offsets over the parent's octets, both
/// ends on UTF-8 codepoint boundaries; negative, out-of-range, and
/// mid-codepoint windows are the house `Invalid` trap (named, never
/// a silent mis-read). The empty range is legal at every boundary.
#[test]
fn sv_range_traps_mid_codepoint_and_out_of_range() {
    let ascii = "abcdef";
    assert_eq!(sv_range(ascii, 0, 6).unwrap(), "abcdef", "the full range");
    assert_eq!(sv_range(ascii, 2, 3).unwrap(), "cde", "a middle window");
    assert_eq!(sv_range(ascii, 6, 0).unwrap(), "", "the empty tail window");
    assert_eq!(sv_range(ascii, 0, 0).unwrap(), "", "the empty head window");

    // h(1) é(2) l(1) l(1) o(1) = 6 bytes; é spans bytes 1..3
    let utf8 = "héllo";
    assert_eq!(sv_range(utf8, 0, 3).unwrap(), "hé");
    assert_eq!(sv_range(utf8, 3, 3).unwrap(), "llo");
    assert_eq!(sv_range(utf8, 0, 6).unwrap(), utf8);
    // a window STARTING mid-codepoint traps
    let err = sv_range(utf8, 2, 2).unwrap_err();
    assert_eq!(err.kind, TrapKind::Invalid);
    assert!(err.msg.contains("not a UTF-8 boundary"), "{}", err.msg);
    assert!(err.msg.contains("offset 2"), "{}", err.msg);
    // a window ENDING mid-codepoint traps the same way
    let err = sv_range(utf8, 0, 2).unwrap_err();
    assert!(
        err.msg.contains("end offset 2") && err.msg.contains("not a UTF-8 boundary"),
        "{}",
        err.msg
    );

    // negative offsets and lengths
    for (off, len) in [(-1, 2), (2, -1), (i32::MIN, 0), (0, i32::MIN)] {
        let err = sv_range(utf8, off, len).unwrap_err();
        assert_eq!(err.kind, TrapKind::Invalid);
        assert!(err.msg.contains("out of range"), "{off}/{len}: {}", err.msg);
    }
    // past the end — usize math, so the i32 corners cannot overflow
    // the check
    let err = sv_range(utf8, 0, 7).unwrap_err();
    assert!(err.msg.contains("out of range"), "{}", err.msg);
    let err = sv_range(utf8, i32::MAX, 1).unwrap_err();
    assert!(err.msg.contains("out of range"), "{}", err.msg);
    let err = sv_range("", 0, 1).unwrap_err();
    assert!(err.msg.contains("out of range"), "{}", err.msg);
    // but the empty range on an empty parent is fine
    assert_eq!(sv_range("", 0, 0).unwrap(), "");
}

/// The parity law (strings-round1 phase 1), re-based: the sv lanes
/// and the s lanes on the SAME content build the SAME map — same
/// kind, same handles, same find answers — with replaces and
/// removes crossing lanes, both directions. Identity: an sv key and
/// the equal-content str key are ONE entry.
#[test]
fn sv_and_s_lanes_assign_identical_handles_both_directions() {
    let vm = bare_vm();
    // the sv side's parent; the windows are BYTE ranges carved out
    // of it, and the texts are what they carve (the test's ground
    // truth — checked below)
    let parent = "alpha-beta-gamma-delta";
    let windows: [(i32, i32); 8] = [
        (0, 5),  // alpha
        (6, 4),  // beta
        (11, 5), // gamma
        (17, 5), // delta
        (0, 4),  // alph — a prefix of alpha, distinct content
        (5, 5),  // -beta
        (10, 6), // -gamma
        (16, 6), // -delta
    ];
    let texts: [String; 8] = [
        "alpha".into(),
        "beta".into(),
        "gamma".into(),
        "delta".into(),
        "alph".into(),
        "-beta".into(),
        "-gamma".into(),
        "-delta".into(),
    ];
    for ((off, len), text) in windows.iter().zip(texts.iter()) {
        assert_eq!(&parent[*off as usize..(off + len) as usize], text);
    }

    // one op stream, three builds: all-sv, all-s, mixed lanes —
    // growth is std's, internal (the sv legs run the registration
    // body verbatim: `sv_range` then the borrowed put, still
    // zero-copy)
    let build = |mode: u8| -> NativeTable {
        let mut t = NativeTable::new(4);
        for i in 0..8 {
            let ans = match mode {
                0 => t.hput_ref(KeyRef::Str(
                    sv_range(parent, windows[i].0, windows[i].1).unwrap(),
                )),
                1 => t.hput_ref(KeyRef::Str(texts[i].as_str())),
                _ => {
                    if i % 2 == 0 {
                        t.hput_ref(KeyRef::Str(
                            sv_range(parent, windows[i].0, windows[i].1).unwrap(),
                        ))
                    } else {
                        t.hput_ref(KeyRef::Str(texts[i].as_str()))
                    }
                }
            }
            .unwrap();
            assert_eq!(ans & 1, 1, "key {i} inserts fresh");
        }
        t
    };
    let a = build(0);
    let b = build(1);
    let c = build(2);
    assert_eq!(a.len(), 8);
    assert_eq!(b.len(), 8);
    assert_eq!(c.len(), 8);
    assert_eq!(a.kind(), KeyKind::Str);
    // the three builds are THE SAME MAP: same handles per content,
    // by construction (identity is the content, never an address)
    for i in 0..8 {
        let (off, len) = windows[i];
        let ha = a
            .hfind_ref(KeyRef::Str(sv_range(parent, off, len).unwrap()))
            .unwrap();
        assert!(ha >= 0, "key {i} lost on the sv build");
        assert_eq!(ha, a.hfind_ref(KeyRef::Str(texts[i].as_str())).unwrap());
        assert_eq!(
            ha,
            b.hfind_ref(KeyRef::Str(sv_range(parent, off, len).unwrap()))
                .unwrap()
        );
        assert_eq!(ha, b.hfind_ref(KeyRef::Str(texts[i].as_str())).unwrap());
        assert_eq!(
            ha,
            c.hfind_ref(KeyRef::Str(texts[i].as_str())).unwrap(),
            "the mixed build too"
        );
    }

    // replaces cross lanes: a put through the OPPOSITE lane answers
    // the same handle, newly = false, and stores no second entry
    let mut a = a;
    let alpha = a
        .hfind_ref(KeyRef::Str(sv_range(parent, 0, 5).unwrap()))
        .unwrap();
    assert_eq!(
        a.hput_ref(KeyRef::Str("alpha")).unwrap(),
        pack_answer(alpha, false)
    );
    assert_eq!(a.len(), 8);
    let beta = a.hfind_ref(KeyRef::Str("beta")).unwrap();
    assert_eq!(
        a.hput_ref(KeyRef::Str(sv_range(parent, 6, 4).unwrap()))
            .unwrap(),
        pack_answer(beta, false)
    );
    assert_eq!(a.len(), 8);

    // removes cross lanes: sv remove of the s-replaced key answers
    // its handle; the owned lane then misses too; a re-add is a NEW
    // birth (handles are never recycled)
    assert_eq!(
        a.hremove_ref(&vm, KeyRef::Str(sv_range(parent, 0, 5).unwrap()))
            .unwrap(),
        alpha
    );
    assert_eq!(a.hremove_ref(&vm, KeyRef::Str("alpha")).unwrap(), -1);
    assert_eq!(
        a.hfind_ref(KeyRef::Str(sv_range(parent, 0, 5).unwrap()))
            .unwrap(),
        -1
    );
    assert_eq!(a.hfind_ref(KeyRef::Str("alpha")).unwrap(), -1);
    assert_eq!(a.len(), 7);
    let ans = a.hput_ref(KeyRef::Str("alpha")).unwrap();
    assert_eq!(ans & 1, 1, "the re-add is a new birth");
    assert!(((ans >> 1) as i32) > alpha, "births are monotonic");
}

/// The sv lanes admit like the s lanes: a Str table takes them, a
/// Bits table traps mixed kinds BEFORE touching the map (never a
/// silent absent), and a fresh table's first sv put fixes its kind.
#[test]
fn sv_lanes_admit_and_trap_like_the_s_lanes() {
    let vm = bare_vm();
    let mut t = NativeTable::new(8);
    assert_eq!(t.hput(KeyKind::Bits, bits(1)).unwrap() & 1, 1);
    let err = t
        .hfind_ref(KeyRef::Str(sv_range("ab", 0, 2).unwrap()))
        .unwrap_err();
    assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
    let err = t
        .hput_ref(KeyRef::Str(sv_range("ab", 0, 2).unwrap()))
        .unwrap_err();
    assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
    let err = t
        .hremove_ref(&vm, KeyRef::Str(sv_range("ab", 0, 2).unwrap()))
        .unwrap_err();
    assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
    let err = t.hput_ref(KeyRef::Str("ab")).unwrap_err();
    assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
    assert_eq!(t.len(), 1, "the rejected keys stored nothing");
    // a fresh table admits the sv lanes (Unset -> Str)
    let mut t = NativeTable::new(8);
    let ans = t
        .hput_ref(KeyRef::Str(sv_range("xy", 0, 2).unwrap()))
        .unwrap();
    assert_eq!(ans & 1, 1);
    assert_eq!(t.kind(), KeyKind::Str);
}

/// Multi-grow sweep through the sv path, the k-mer shape: 150
/// distinct 3-byte windows carved from ONE generated parent, std's
/// internal growth, remove/re-add churn across lanes — every key
/// findable at the end, both lanes agreeing on every handle.
#[test]
fn sv_lanes_survive_multi_grow_sweeps() {
    let vm = bare_vm();
    // a marker parent: window i (bytes 3i..3i+3) spells
    // ('A'+i%26)('a'+i/26)('0'+i%10) — unique per i BY CONSTRUCTION
    // (the first two bytes determine i for i < 156), so no periodic
    // collision can sneak into the sweep
    let mut parent = String::new();
    for i in 0..150usize {
        parent.push((b'A' + (i % 26) as u8) as char);
        parent.push((b'a' + (i / 26) as u8) as char);
        parent.push((b'0' + (i % 10) as u8) as char);
    }
    let off_of = |i: usize| (3 * i) as i32;
    let text_of = |i: usize| parent[3 * i..3 * i + 3].to_string();

    let mut t = NativeTable::new(4);
    let mut distinct = std::collections::HashSet::new();
    for i in 0..150usize {
        let ans = t
            .hput_ref(KeyRef::Str(sv_range(&parent, off_of(i), 3).unwrap()))
            .unwrap();
        assert_eq!(ans & 1, 1, "window {i} inserts fresh");
        distinct.insert(text_of(i));
    }
    assert_eq!(distinct.len(), 150, "the sweep's keys are distinct");
    assert_eq!(t.len(), 150);
    assert!(t.cap() >= 150, "growth is std's and internal: {}", t.cap());

    // churn: remove every 3rd key through the sv lane, re-add
    // through the s lane (mixed-lane churn over the same map)
    for i in (0..150).step_by(3) {
        assert!(
            t.hremove_ref(&vm, KeyRef::Str(sv_range(&parent, off_of(i), 3).unwrap()))
                .unwrap()
                >= 0
        );
    }
    assert_eq!(t.len(), 100);
    for i in (0..150).step_by(3) {
        let ans = t.hput_ref(KeyRef::Str(text_of(i).as_str())).unwrap();
        assert_eq!(ans & 1, 1, "re-add {i} is a fresh birth");
    }
    assert_eq!(t.len(), 150);
    // every key findable at the end, BOTH lanes agreeing per handle
    for i in 0..150usize {
        let via_sv = t
            .hfind_ref(KeyRef::Str(sv_range(&parent, off_of(i), 3).unwrap()))
            .unwrap();
        assert!(via_sv >= 0, "key {i} lost");
        assert_eq!(
            via_sv,
            t.hfind_ref(KeyRef::Str(text_of(i).as_str())).unwrap(),
            "key {i}"
        );
    }
}

// ---- the fused handle lanes (nmapset-hostops) ---------------------

#[test]
fn the_packed_answer_law_is_bit0_newly_rest_handle() {
    // bit 0 = the newly bit, the rest the handle — distinct pairs
    // answer distinctly, and handle 0 rides both flags (the probe's
    // own PACK row, pinned)
    assert_eq!(pack_answer(0, false), 0);
    assert_eq!(pack_answer(0, true), 1);
    assert_eq!(pack_answer(1, false), 2);
    assert_eq!(pack_answer(1, true), 3);
    assert_eq!(pack_answer(1_000_000_000, true), 2_000_000_001);
    for hd in [0i32, 1, 7, 1 << 20] {
        for newly in [false, true] {
            let ans = pack_answer(hd, newly);
            assert_eq!((ans & 1) == 1, newly);
            assert_eq!((ans >> 1) as i32, hd);
        }
    }
    // collision-freedom over the whole realistic range: the map is
    // injective by construction (one bit vs the shifted rest)
    let mut seen = std::collections::HashSet::new();
    for hd in 0..2000i32 {
        assert!(seen.insert(pack_answer(hd, true)));
        assert!(seen.insert(pack_answer(hd, false)));
    }
}

#[test]
fn hput_answers_handles_and_hfind_agrees() {
    let mut t = NativeTable::new(8);
    // three fresh births: handles 0, 1, 2 — each newly
    for i in 0..3u64 {
        let ans = t.hput(KeyKind::Bits, bits(i)).unwrap();
        assert_eq!(ans, pack_answer(i as i32, true), "birth {i}");
    }
    // replace: the SAME handle, newly = false
    assert_eq!(
        t.hput(KeyKind::Bits, bits(1)).unwrap(),
        pack_answer(1, false)
    );
    // hfind answers the handle; a miss is -1
    assert_eq!(t.hfind(KeyKind::Bits, &bits(0)).unwrap(), 0);
    assert_eq!(t.hfind(KeyKind::Bits, &bits(1)).unwrap(), 1);
    assert_eq!(t.hfind(KeyKind::Bits, &bits(99)).unwrap(), -1);
    assert_eq!(t.len(), 3);
}

#[test]
fn handles_survive_multi_grow_sweeps_untouched() {
    // the takeover's core law: the handle is the key's identity —
    // growth (std's, internal) rehashes nothing the handle can see,
    // and the wrapper's sidecar (indexed by it) never drains
    let mut t = NativeTable::new(4);
    let mut handles = Vec::new();
    for i in 0..500u64 {
        let ans = t
            .hput(KeyKind::Bits, bits(i.wrapping_mul(0x9E3779B1)))
            .unwrap();
        assert_eq!(ans & 1, 1, "every key fresh");
        handles.push((ans >> 1) as i32);
    }
    assert!(
        t.cap() >= 500,
        "the sweep grew the map internally: {}",
        t.cap()
    );
    assert_eq!(t.len(), 500);
    for (i, hd) in handles.iter().enumerate() {
        let key = (i as u64).wrapping_mul(0x9E3779B1);
        assert_eq!(t.hfind(KeyKind::Bits, &bits(key)).unwrap(), *hd, "key {i}");
    }
    // handles are the dense births 0..500
    let mut sorted = handles.clone();
    sorted.sort();
    for (i, hd) in sorted.iter().enumerate() {
        assert_eq!(*hd, i as i32);
    }
}

#[test]
fn hremove_answers_the_dead_handle_and_reinsert_is_a_new_birth() {
    let vm = bare_vm();
    let mut t = NativeTable::new(8);
    t.hput(KeyKind::Bits, bits(7)).unwrap(); // handle 0
    t.hput(KeyKind::Bits, bits(8)).unwrap(); // handle 1
                                             // remove answers the DEAD key's OWN handle — the wrapper nils
                                             // exactly that sidecar slot
    assert_eq!(t.hremove(&vm, KeyKind::Bits, &bits(7)).unwrap(), 0);
    assert_eq!(t.hremove(&vm, KeyKind::Bits, &bits(7)).unwrap(), -1);
    assert_eq!(t.hfind(KeyKind::Bits, &bits(7)).unwrap(), -1);
    // monotonic v1: the re-inserted key is a NEW birth (handle 2),
    // never the recycled 0 — the removed slot's sidecar nil stays
    // correct (slot 0 was nilled; the new birth writes slot 2)
    let ans = t.hput(KeyKind::Bits, bits(7)).unwrap();
    assert_eq!(ans, pack_answer(2, true));
    // the survivor is untouched
    assert_eq!(t.hfind(KeyKind::Bits, &bits(8)).unwrap(), 1);
}

#[test]
fn the_sv_and_s_lanes_answer_one_handle() {
    let vm = bare_vm();
    // the parity law extended to identity: an sv key and the
    // equal-content str key are ONE key — same entry, same HANDLE —
    // through every fused op, both directions. The full answers
    // differ in the newly bit by design (the first put is a fresh
    // birth, the second finds it); the HANDLE must not.
    let mut t = NativeTable::new(8);
    let parent = "xx-alpha-xx-beta-xx";
    let via_sv = t
        .hput_ref(KeyRef::Str(sv_range(parent, 3, 5).unwrap()))
        .unwrap(); // "alpha"
    let via_s = t.hput_ref(KeyRef::Str("alpha")).unwrap();
    assert_eq!(via_sv & 1, 1, "the sv put was the fresh birth");
    assert_eq!(via_s & 1, 0, "the s put found the sv key: a replace");
    assert_eq!(via_sv >> 1, via_s >> 1, "ONE key, ONE handle");
    assert_eq!(
        t.hfind_ref(KeyRef::Str("alpha")).unwrap(),
        t.hfind_ref(KeyRef::Str(sv_range(parent, 3, 5).unwrap()))
            .unwrap()
    );
    assert_eq!(
        t.hremove_ref(&vm, KeyRef::Str(sv_range(parent, 3, 5).unwrap()))
            .unwrap(),
        (via_s >> 1) as i32
    );
    assert_eq!(t.hfind_ref(KeyRef::Str("alpha")).unwrap(), -1);
}

#[test]
fn bytes_and_bool_lanes_ride_the_fused_family() {
    let vm = bare_vm();
    // bytes on its own table (kinds are homogeneous per table)
    let mut t = NativeTable::new(8);
    assert_eq!(
        t.hput(KeyKind::Bytes, KeyVal::Bytes(vec![1, 2, 3]))
            .unwrap(),
        pack_answer(0, true)
    );
    assert_eq!(
        t.hfind(KeyKind::Bytes, &KeyVal::Bytes(vec![1, 2, 3]))
            .unwrap(),
        0
    );
    assert_eq!(
        t.hput(KeyKind::Bytes, KeyVal::Bytes(vec![1, 2, 3]))
            .unwrap(),
        pack_answer(0, false)
    );
    assert_eq!(
        t.hremove(&vm, KeyKind::Bytes, &KeyVal::Bytes(vec![1, 2, 3]))
            .unwrap(),
        0
    );
    // bool on its own table — `true` is bits 1 on the b lane
    let mut b = NativeTable::new(8);
    assert_eq!(
        b.hput(KeyKind::Bits, bits(1)).unwrap(),
        pack_answer(0, true),
        "bool true = bits 1"
    );
    assert_eq!(b.hfind(KeyKind::Bits, &bits(1)).unwrap(), 0);
    assert_eq!(b.hremove(&vm, KeyKind::Bits, &bits(1)).unwrap(), 0);
    // kind mixing still traps through the fused path
    assert!(t.hput(KeyKind::Str, KeyVal::Str("s".into())).is_err());
}

// ---- the iteration lane (the `for..of` law) -------------------------

/// A cursor over a table filled through the fused lanes — the tests'
/// shared shape. `keys` must be inserted in the intended birth order.
fn cursor_of(vm: &mut Vm, t: &NativeTable, ty: u32) -> Result<Opaque<IterCursor>, Trap> {
    let kind = iter_key_kind(ty)?;
    let keys = t.admit(kind).map(|_| t.iter_snapshot())?;
    Opaque::alloc(vm, IterCursor { keys, ty, pos: 0 })
}

/// The snapshot walks BIRTH order — insertion order of live entries;
/// a remove-then-reinsert is a NEW birth and walks last. The mint
/// answers each key as a fresh `?opaque` box, nil at the end.
#[test]
fn the_snapshot_walks_birth_order() {
    let mut t = NativeTable::new(8);
    for v in [30, 10, 20] {
        t.hput(KeyKind::Bits, bits(v)).unwrap();
    }
    let mut vm = bare_vm();
    let cur = cursor_of(&mut vm, &t, TY_I64).unwrap();
    let mut walked = Vec::new();
    loop {
        let next = cur
            .with_mut(&mut vm, |vm, c| {
                let key = match c.keys.get(c.pos) {
                    Some(k) => k.clone(),
                    None => return Ok(None),
                };
                c.pos += 1;
                mint_key_box(vm, c.ty, &key).map(Some)
            })
            .unwrap()
            .unwrap();
        let Some(boxed) = next else { break };
        let (ty, v) = rut_vm::rut_box_payload(&vm, &boxed).unwrap();
        assert_eq!(ty, TY_I64);
        let Value::I64(word) = v else { unreachable!() };
        walked.push(word as u64);
    }
    assert_eq!(walked, vec![30, 10, 20], "birth order, not bucket order");
}

/// The mint is the put decode's exact inverse: every int width reads
/// back bit-for-bit through the box (i8 sign-extended, u64 raw,
/// bool the 0/1 word), str/bytes mint FRESH equal-content cells.
#[test]
fn the_mint_round_trips_every_key_flavor() {
    let mut vm = bare_vm();
    let read = |vm: &Vm, boxed: &OpaqueRef| -> (u32, Value) {
        rut_vm::rut_box_payload(vm, boxed).unwrap()
    };
    // bits: the sign/zero-extension law, width by width. The words
    // are the REALISTIC crossing words — signed widths sign-extend,
    // unsigned widths zero-extend (the widening-distinctness law).
    for (word, ty, expect) in [
        (0xFFFF_FFFF_FFFF_FFFBu64, TY_I8, -5i64), // -5i8 sign-extended
        (0xFFFF_FFFF_FFFF_FFFBu64, TY_I64, -5i64), // -5i64 raw
        (0x8000_0000_0000_0005, TY_U64, -9223372036854775803i64), // the raw word
        (0x0000_0000_FFFF_FFFB, TY_U32, 4294967291i64), // zero-extended
        (5, TY_I32, 5),
        (1, TY_BOOL, 1),
        (0, TY_BOOL, 0),
    ] {
        let boxed = mint_key_box(&mut vm, ty, &KeyVal::Bits(word)).unwrap();
        let (got_ty, got) = read(&vm, &boxed);
        assert_eq!(got_ty, ty, "word {word:#x}");
        match got {
            Value::I64(w) => assert_eq!(w, expect, "word {word:#x} ty {ty}"),
            Value::Bool(b) => assert_eq!(b as i64, expect, "word {word:#x}"),
            other => panic!("unexpected payload {other:?}"),
        }
    }
    // octets: fresh cells, equal content
    let boxed = mint_key_box(&mut vm, TY_STR, &KeyVal::Str("alpha".into())).unwrap();
    let (ty, v) = read(&vm, &boxed);
    assert_eq!(ty, TY_STR);
    assert_eq!(v, Value::Str("alpha".into()));
    let boxed = mint_key_box(&mut vm, TY_BYTES, &KeyVal::Bytes(vec![1, 2, 3])).unwrap();
    let (ty, v) = read(&vm, &boxed);
    assert_eq!(ty, TY_BYTES);
    assert_eq!(v, Value::Bytes(vec![1, 2, 3]));
}

/// The decode's defense rows: a type outside the closed set traps;
/// a flavor disagreement between the mint's `ty` and the snapshot
/// traps; `map_iter_new`'s admission runs before any cursor exists.
#[test]
fn iter_defense_traps_name_the_law() {
    let mut t = NativeTable::new(8);
    t.hput(KeyKind::Bits, bits(1)).unwrap();
    // outside the closed key set: a float type id, a garbage id
    assert!(iter_key_kind(rut_core::types::TY_F64).is_err());
    assert!(iter_key_kind(999_999).is_err());
    // flavor mismatch: a Bits table walked under the str type
    let mut vm = bare_vm();
    let err = match cursor_of(&mut vm, &t, TY_STR) {
        Err(e) => e,
        Ok(_) => panic!("the flavor mismatch must trap"),
    };
    assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
    // the mint itself refuses a mismatched pair (defense in depth)
    let err = mint_key_box(&mut vm, TY_STR, &KeyVal::Bits(1)).unwrap_err();
    assert!(err.msg.contains("flavor mismatch"), "{}", err.msg);
    // an empty table walks to nil immediately
    let empty = NativeTable::new(8);
    let cur = cursor_of(&mut vm, &empty, TY_I64).unwrap();
    let next = cur.with_mut(&mut vm, |vm, c| {
        let key = match c.keys.get(c.pos) {
            Some(k) => k.clone(),
            None => return Ok(None),
        };
        c.pos += 1;
        mint_key_box(vm, c.ty, &key).map(Some)
    });
    assert!(matches!(next, Ok(Ok(None))), "empty snapshot = nil");
}

/// Birth order through remove + re-insert: the reinserted key is a
/// NEW birth and walks after the survivors.
#[test]
fn reinsert_moves_the_key_to_the_walks_end() {
    let vm = bare_vm();
    let mut t = NativeTable::new(8);
    for v in [1, 2, 3] {
        t.hput(KeyKind::Bits, bits(v)).unwrap();
    }
    t.hremove(&vm, KeyKind::Bits, &bits(2)).unwrap();
    t.hput(KeyKind::Bits, bits(2)).unwrap(); // a NEW birth
    let keys: Vec<u64> = t
        .iter_snapshot()
        .into_iter()
        .map(|k| match k {
            KeyVal::Bits(v) => v,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(keys, vec![1, 3, 2]);
}
