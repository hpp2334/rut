//! The primitive-optional element store (RFC 0044 §5, the nmapset-round2
//! phase-2 repr): a `[?prim]` backing — `pouch`'s `Vec<T>.buf` included —
//! holds each element as a raw payload plus a one-byte nil tag instead of a
//! boxed one-slot cell. The element ops lower to the `OptPrim` repr family
//! (the store elision removes the per-store `makeopt`; the deref fold
//! removes the per-read mint), reads answer value semantics (a fresh opt
//! value), the release walk contributes no children for the raw store, and
//! the module binary VERSION bumps to 6 to reject stale artifacts.
//!
//! Reference payloads (`?str`, `?record`) keep the cell backing — the
//! nmapset `get`-staleness parity test in `nmapset.rs` pins that side
//! unchanged.

use std::rc::Rc;

use rut_driver::{Module, Session};

const POUCH_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/pouch");

fn session_with(app_src: &str) -> Session {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    rut_driver::mount_dir(&mut session, std::path::Path::new(POUCH_DIR))
        .expect("mount pouch");
    session
        .register_module(
            "app_main",
            Module { spec: "app_main".into(), source: Some(app_src.into()), ..Default::default() },
        )
        .unwrap();
    session
}

/// Compile, verify, and run `main` — the i64 return.
fn run_main(app_src: &str) -> i64 {
    let session = session_with(app_src);
    let out = rut_driver::compile_graph(&session, "app_main");
    assert!(
        out.diags.is_empty(),
        "{}",
        out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let prog = out.program.expect("linked program");
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(50_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), rut_vm::interp::HostRegistry::new())
        .expect("vm")
        .call::<_, i64>("main", ())
        .expect("run")
}

/// The linked program's IR dump (lowering evidence).
fn ir_dump(app_src: &str) -> String {
    let session = session_with(app_src);
    let out = rut_driver::compile_graph(&session, "app_main");
    let prog = out.program.expect("linked program");
    rut_driver::ir_dump_of(&prog.funcs, &prog.interner)
}

// ---- the lowering: element ops ride the opt-prim repr family -----------

/// A `[?i64]` churn compiles to element ops on the raw store: the store
/// carries the elided-raw repr (`OptPrimRaw`), the direct-use read carries
/// the deref-folded load repr (`OptPrimLoad`), and NO `makeopt` remains —
/// the box the `T → ?T` coercion used to mint per store/load is gone from
/// the op stream. A read whose `?i64` result binds a NAME (multi-use)
/// keeps the mint form (`OptPrim`) — the fresh opt value is built inside
/// the engine, so no box op survives in any case.
#[test]
fn opt_prim_element_ops_lower_to_the_prim_store() {
    let src = r#"
        pub fn main() -> i64 {
            let mut a: [?i64] = [nil; 8];
            a[0] = 5;
            a[1] = 6;
            let mut s: i64 = 0;
            s = s.wrapping_add(a[0]);
            for (let i = 0; i < 8; i += 1) {
                let x = a[i];
                if (x != nil) {
                    s = s.wrapping_add(x);
                }
            }
            return s;
        }
    "#;
    let dump = ir_dump(src);
    // repr codes: PrimTy::I64 wire code 7 — OptPrimRaw(I64) = 26 + 7 = 33,
    // OptPrim(I64) = 14 + 7 = 21, OptPrimLoad(I64) = 38 + 7 = 45.
    assert!(
        dump.contains(":33"),
        "stores into a `[?i64]` must bake OptPrimRaw (the elided raw form):\n{dump}"
    );
    assert!(
        dump.contains(":45"),
        "a read whose only use is the payload must bake OptPrimLoad (the deref fold):\n{dump}"
    );
    assert!(
        dump.contains(":21"),
        "a read bound to a name keeps the mint form (value semantics):\n{dump}"
    );
    assert!(
        !dump.contains("makeopt"),
        "no `T → ?T` box may survive the churn — the raw store needs no cell:\n{dump}"
    );
}

/// The elision and the fold are LOCAL rewrites of `[?prim]` traffic: the
/// same churn over `[?str]` keeps the cell backing (`Ref` repr code 12, a
/// `makeopt` for the box) — the reference-element path is unchanged.
#[test]
fn reference_elements_keep_the_cell_backing() {
    let src = r#"
        pub fn main() -> i64 {
            let mut a: [?str] = [nil; 4];
            a[0] = "x";
            let mut n: i64 = 0;
            n = n.wrapping_add(1);
            for (let i = 0; i < 4; i += 1) {
                if (a[i] != nil) {
                    n = n.wrapping_add(1);
                }
            }
            return n;
        }
    "#;
    let dump = ir_dump(src);
    assert!(
        dump.contains("makeopt"),
        "a `?str` element boxes (the Ref law — cell backing):\n{dump}"
    );
    assert!(
        !dump.contains(":33") && !dump.contains(":45") && !dump.contains(":21"),
        "a `?str` store must not bake the opt-prim forms:\n{dump}"
    );
}

// ---- the runtime: nil round-trips + value fidelity per prim kind -------

/// nil round-trips and full-width payload fidelity on a `[?i64]`: 64-bit
/// extremes, a nil write OVER a live element, and a replace.
#[test]
fn i64_nil_round_trip_and_extremes() {
    let checksum = run_main(
        r#"
        pub fn main() -> i64 {
            let mut a: [?i64] = [nil; 16];
            for (let i = 0; i < 16; i += 1) {
                if (i % 2 == 0) {
                    a[i] = (i as i64).wrapping_mul(4294967296i64).wrapping_sub(9223372036854775807i64);
                }
            }
            a[4] = nil;
            a[8] = -9223372036854775807i64;
            let mut s: i64 = 0;
            let mut live = 0;
            for (let i = 0; i < 16; i += 1) {
                let x = a[i];
                if (x != nil) {
                    live = live + 1;
                    s = s.wrapping_add(x);
                }
            }
            // expect = the same fold over the surviving slots, nil excluded:
            // slots 0,2,6,10,12,14 hold i*2^32 - (2^63-1); slot 8 holds
            // -(2^63-1). (live=7 folded in to bind both counters to the
            // checksum.)
            let mut expect: i64 = 0i64;
            expect = expect.wrapping_add((0i64).wrapping_mul(4294967296i64).wrapping_sub(9223372036854775807i64));
            expect = expect.wrapping_add((2i64).wrapping_mul(4294967296i64).wrapping_sub(9223372036854775807i64));
            expect = expect.wrapping_add((6i64).wrapping_mul(4294967296i64).wrapping_sub(9223372036854775807i64));
            expect = expect.wrapping_add((10i64).wrapping_mul(4294967296i64).wrapping_sub(9223372036854775807i64));
            expect = expect.wrapping_add((12i64).wrapping_mul(4294967296i64).wrapping_sub(9223372036854775807i64));
            expect = expect.wrapping_add((14i64).wrapping_mul(4294967296i64).wrapping_sub(9223372036854775807i64));
            expect = expect.wrapping_add(-9223372036854775807i64);
            if (s == expect && live == 7) {
                return 111;
            }
            return s.wrapping_sub(expect).wrapping_add(live as i64);
        }
    "#,
    );
    assert_eq!(checksum, 111, "nil round-trip + i64 extreme fidelity");
}

/// u64 payloads across the 2^63 boundary round-trip bit-exactly through the
/// raw store (the tag must not eat a payload bit, and the widening read
/// must not sign-extend).
#[test]
fn u64_high_bit_payloads_round_trip() {
    let checksum = run_main(
        r#"
        pub fn main() -> i64 {
            let mut u: [?u64] = [nil; 4];
            u[0] = 9223372036854775808u64;
            u[1] = 18446744073709551615u64;
            u[2] = 9223372036854775807u64;
            let mut a: u64 = 0;
            for (let i = 0; i < 4; i += 1) {
                let x = u[i];
                if (x != nil) {
                    a = a.wrapping_add(x);
                }
            }
            let want: u64 = 18446744073709551614u64;
            if (a == want) {
                return 111;
            }
            return -111;
        }
    "#,
    );
    assert_eq!(checksum, 111, "u64 2^63..2^64-1 payloads must round-trip exactly");
}

/// f64 and the narrow ints (i32/i16) round-trip through their payload
/// widths; a narrow store's high bits do not bleed into the tag.
#[test]
fn float_and_narrow_int_payloads_round_trip() {
    let checksum = run_main(
        r#"
        pub fn main() -> i64 {
            let f: [?f64] = [3.5, -0.25, nil];
            let mut fs: f64 = 0.0;
            for (let i = 0; i < 3; i += 1) {
                let x = f[i];
                if (x != nil) {
                    fs = fs + x;
                }
            }
            let mut n: [?i32] = [nil; 3];
            n[0] = -2147483647;
            n[1] = 2147483647;
            let mut ns: i64 = 0;
            for (let i = 0; i < 3; i += 1) {
                if (n[i] != nil) {
                    ns = ns.wrapping_add(n[i] as i64);
                }
            }
            let mut b: [?i16] = [nil; 2];
            b[0] = -32767;
            b[1] = 32767;
            let mut bs: i64 = 0;
            for (let i = 0; i < 2; i += 1) {
                if (b[i] != nil) {
                    bs = bs.wrapping_add(b[i] as i64);
                }
            }
            let fok = fs == 3.25;
            let nok = ns == 0;
            let bok = bs == 0;
            if (fok && nok && bok) {
                return 222;
            }
            return -222;
        }
    "#,
    );
    assert_eq!(checksum, 222, "f64 + narrow-int payload fidelity");
}

/// Iteration order is the index order — the checksum law: mixed nil/some
/// elements visit nil where a nil is stored (no compaction, no reorder).
#[test]
fn iteration_order_unchanged() {
    let checksum = run_main(
        r#"
        pub fn main() -> i64 {
            let mut a: [?i32] = [nil; 12];
            for (let i = 0; i < 12; i += 1) {
                if (i % 3 != 0) {
                    a[i] = i;
                }
            }
            let mut acc: i64 = 0;
            for (let i = 0; i < 12; i += 1) {
                let x = a[i];
                if (x == nil) {
                    acc = acc.wrapping_mul(31).wrapping_add(7);
                } else {
                    acc = acc.wrapping_mul(31).wrapping_add(x as i64);
                }
            }
            return acc;
        }
    "#,
    );
    // reference fold: for i in 0..12: v = if i%3==0 {7} else {i}
    let mut expect: i64 = 0;
    for i in 0..12i64 {
        let v = if i % 3 == 0 { 7 } else { i };
        expect = expect.wrapping_mul(31).wrapping_add(v);
    }
    assert_eq!(checksum, expect, "iteration must visit elements in index order, nils included");
}

/// The growable path: `Vec<i64>`'s `buf: [?i64]` rides the same raw store —
/// push through several grows, replace, and read back.
#[test]
fn vec_backing_rides_the_prim_store() {
    let checksum = run_main(
        r#"
        use pouch::{ Vec };
        pub fn main() -> i64 {
            let mut v: Vec<i64> = Vec.new();
            for (let i = 0; i < 5000; i += 1) {
                v.push((i as i64).wrapping_mul(3));
            }
            for (let i = 0; i < 5000; i += 10) {
                v[i] = -1;
            }
            let mut s: i64 = 0;
            for (let x of v) {
                s = s.wrapping_add(x);
            }
            let mut want: i64 = 0;
            for (let i = 0; i < 5000; i += 1) {
                if (i % 10 == 0) {
                    want = want.wrapping_add(-1);
                } else {
                    want = want.wrapping_add((i as i64).wrapping_mul(3));
                }
            }
            if (s == want && v.len() == 5000) {
                return 333;
            }
            return -333;
        }
    "#,
    );
    assert_eq!(checksum, 333, "Vec<i64> over the raw backing must push/grow/replace/iterate exactly");
}

/// A fresh read answers VALUE semantics: reading a slot, storing a new
/// value, and reading again — the first read's value must be unchanged
/// (it is a copy, not the store's cell).
#[test]
fn reads_are_values_not_handles() {
    let checksum = run_main(
        r#"
        pub fn main() -> i64 {
            let mut a: [?i64] = [nil; 2];
            a[0] = 10;
            let x = a[0];
            a[0] = 20;
            let y = a[0];
            a[1] = nil;
            let nil1 = a[1] == nil;
            let ok1 = x == 10;
            let ok2 = y == 20;
            // an array LITERAL of `?prim` elements rides the same store —
            // and its boxed elements must die with it (release parity)
            let mut lit: [?i64] = [70, nil, 7];
            let mut s: i64 = 0;
            for (let i = 0; i < 3; i += 1) {
                let e = lit[i];
                if (e != nil) {
                    s = s.wrapping_add(e);
                }
            }
            if (ok1 && ok2 && nil1 && s == 77) {
                return 444;
            }
            return -444;
        }
    "#,
    );
    assert_eq!(checksum, 444, "a `[?prim]` read mints a fresh value — a later store must not rewrite it");
}

// ---- the raw sidecar under nmapset + the heap-accounting law ----------

const NMAPSET_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/nmapset");

/// `HashMap<i32, i64>` — a PRIMITIVE value V: the `[?V]` sidecar rides the
/// raw store, so the wrapper's surface law is unchanged (put/get/has/
/// remove over 1000 keys, several grows, replaces, removes) while the
/// stored elements are raw payloads. The checksum derives from the values,
/// so a wrong tag or payload width cannot pass.
#[test]
fn nmapset_primitive_values_round_trip_through_the_raw_sidecar() {
    let src = r#"
        use nmapset::{ HashMap };
        pub fn main() -> i64 {
            let mut m: HashMap<i32, i64> = HashMap.new();
            for (let i = 0; i < 1000; i += 1) {
                if (m.put(i, (i as i64).wrapping_mul(1000003i64))) {
                    let _ = 0;
                }
            }
            for (let i = 0; i < 1000; i += 2) {
                let _ = m.put(i, -1);
            }
            for (let i = 0; i < 1000; i += 4) {
                let _ = m.remove(i);
            }
            let mut s: i64 = 0;
            let mut live = 0;
            for (let i = 0; i < 1000; i += 1) {
                let p = m.get(i);
                if (p != nil) {
                    live = live + 1;
                    s = s.wrapping_add(p);
                }
            }
            // reference fold: odd i -> i*1000003; i ≡ 2 mod 4 -> -1;
            // i ≡ 0 mod 4 removed.
            let mut want: i64 = 0;
            let mut wantlive = 0;
            for (let i = 0; i < 1000; i += 1) {
                if (i % 4 == 2) {
                    want = want.wrapping_add(-1);
                    wantlive = wantlive + 1;
                } else if (i % 2 == 1) {
                    want = want.wrapping_add((i as i64).wrapping_mul(1000003i64));
                    wantlive = wantlive + 1;
                }
            }
            if (s == want && live == wantlive && m.len() == 750) {
                return 555;
            }
            return -555;
        }
    "#;
    let mut session = session_with(src);
    // nmapset pulls the `nmap_host` host pkg through its own [deps]
    rut_driver::mount_dir(&mut session, std::path::Path::new(NMAPSET_DIR))
        .expect("mount nmapset");
    let expected = session.expected_host_fns();
    for f in ["map_new", "map_entry", "map_find", "map_remove", "map_grow", "map_take_reloc", "map_cap", "map_len"] {
        assert!(
            expected.contains_key(&format!("nmap_host::{f}")),
            "the nmap_host surface must cross through the [deps] mount: {expected:?}"
        );
    }
    let out = rut_driver::compile_graph(&session, "app_main");
    assert!(
        out.diags.is_empty(),
        "{}",
        out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let prog = out.program.expect("linked program");
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(50_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_std::nmap::install_std_nmap(&mut hosts);
    hosts.verify_against(&expected);
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts)
        .expect("vm");
    let v: i64 = vm.call("main", ()).expect("run");
    assert_eq!(v, 555, "primitive V round-trips through the raw sidecar");
}

/// The release walk contributes NO children for a raw-store array (the
/// plan's "drop/release skips slot release"): a long `Vec<i64>` run's VM
/// heap peak stays at the block + transient level — the boxed era's
/// per-element cells (32-40 B each, plus the release-walk collect) would
/// blow far past this bound.
#[test]
fn prim_store_release_walk_skips_element_slots() {
    let src = r#"
        use pouch::{ Vec };
        pub fn main() -> i64 {
            let mut v: Vec<i64> = Vec.new();
            for (let i = 0; i < 100000; i += 1) {
                v.push(i as i64);
            }
            let mut s: i64 = 0;
            for (let x of v) {
                s = s.wrapping_add(x);
            }
            return s;
        }
    "#;
    let session = session_with(src);
    let out = rut_driver::compile_graph(&session, "app_main");
    let prog = out.program.expect("linked program");
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(50_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), rut_vm::interp::HostRegistry::new())
        .expect("vm");
    let v: i64 = vm.call("main", ()).expect("run");
    assert_eq!(v, (0i64..100000).sum::<i64>());
    // live elements cap at 131072 slots x 9 B = 1.18 MiB; the peak adds the
    // previous block + per-access transient opt mints (freed at once). The
    // boxed-cell era peaked at ~4+ MB for this exact loop.
    let peak = vm.heap_peak();
    assert!(
        peak < 2_600_000,
        "raw-store heap peak must stay near the block cost, got {peak} B"
    );
}

// ---- serialization: the version gate ----------------------------------

/// A VERSION-7 module decodes; the same bytes with the version word rolled
/// back to 6 are rejected with the standard clear error (stale v6 artifacts
/// carry the `(T, bool)` downcast lowering the new engines must not run on
/// the `?T` surface).
#[test]
fn version_gate_rejects_stale_artifacts() {
    use rut_core::binary::{decode, VERSION};
    // v8 is the err-channel phase 2 declared-surface change
    // (`capture_stacktrace()` + the `StackTrace` builtin class + the
    // `pos` span table, RFC 0036) — the 6→7 precedent this pin already
    // enforced for the primitive-optional element store
    assert_eq!(VERSION, 8, "the declared-surface change is the only allowed VERSION bump this phase");
    let out = rut_driver::compile_module(
        "pub fn main() -> i64 { let mut a: [?i64] = [nil; 2]; a[0] = 1; let x = a[0]; return x; }",
        rut_parser::Mode::Impl,
        "gate",
    );
    assert!(out.diags.is_empty(), "{}", out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n"));
    let bytes = out.binary.expect("encoded module");
    assert!(decode(&bytes).is_ok(), "current artifacts decode");

    let mut stale = bytes.clone();
    let ver_at = 4; // MAGIC (4) then the version u32
    stale[ver_at..ver_at + 4].copy_from_slice(&7u32.to_le_bytes());
    let err = decode(&stale).expect_err("a v7 artifact must be rejected");
    assert!(
        err.contains("unsupported module binary version 7"),
        "the standard clear error: {err}"
    );
}