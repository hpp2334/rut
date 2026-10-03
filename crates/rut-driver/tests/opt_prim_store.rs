//! The primitive-optional element store (the nmapset-round2
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


const POUCH_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/pouch");




/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(rut_driver::RutRun::new().pkg(rut_driver::Pkg::source(spec, src)).entrypoint(spec).compile())
}

/// [`compiled`] with calc offered (the old `mount_std` shape: core
/// auto-rides, `calc` is an ordinary pkg).
#[allow(dead_code)]
fn compiled_std(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            .pkg(rut_driver::Pkg::source(spec, src))
            .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
            .entrypoint(spec)
            .compile(),
    )
}

#[allow(dead_code)]
fn graph_of(c: Result<rut_driver::Compiled, rut_driver::RunError>) -> rut_driver::GraphOutput {
    match c {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(rut_lexer::span::Span::new(0, 0), e.msg)],
            program: None,
        },
    }
}

fn world_with(app_src: &str) -> rut_driver::Loaded {
    let mut loaded = rut_native::dir_pkgs(std::path::Path::new(POUCH_DIR)).expect("mount pouch");
    loaded.pkgs.push(rut_driver::Pkg::source("app_main", app_src));
    loaded
}

/// Compile, verify, and run `main` — the i64 return.
fn run_main(app_src: &str) -> i64 {
    let out = graph_of(
        rut_driver::RutRun::new()
            .pkgs(&world_with(app_src))
            .entrypoint("app_main")
            .compile(),
    );
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
    rut_vm::interp:: Vm::builder().program(Rc::new(prog)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
        .expect("vm")
        .call::<_, i64>("main", ())
        .expect("run")
}

/// The linked program's IR dump (lowering evidence).
fn ir_dump(app_src: &str) -> String {
    let out = graph_of(
        rut_driver::RutRun::new()
            .pkgs(&world_with(app_src))
            .entrypoint("app_main")
            .compile(),
    );
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
        entry fn main() -> i64 {
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
        entry fn main() -> i64 {
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
        entry fn main() -> i64 {
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
        entry fn main() -> i64 {
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
        entry fn main() -> i64 {
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
        entry fn main() -> i64 {
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
        entry fn main() -> i64 {
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
        entry fn main() -> i64 {
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
        entry fn main() -> i64 {
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
    let mut world = world_with(src);
    // nmapset pulls the `nmap_host` host pkg through its own [deps]
    world.pkgs.extend(
        rut_native::dir_pkgs(std::path::Path::new(NMAPSET_DIR))
            .expect("mount nmapset")
            .pkgs,
    );
    let ctx = rut_driver::host_pkg_ctx(&world.pkgs);
    for f in ["map_new", "map_len", "map_hput", "map_hfind", "map_hremove"] {
        assert!(
            ctx.rows_of("nmap_host").is_some_and(|r| r.contains_key(f)),
            "the nmap_host surface must cross through the [deps] mount: {ctx:?}"
        );
    }
    let out = graph_of(
        rut_driver::RutRun::new()
            .pkgs(&world)
            .entrypoint("app_main")
            .compile(),
    );
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
    hosts.install_host_pkg(&ctx, rut_std::nmap::pkg());
    hosts.verify_against(&ctx.flatten());
    let mut vm = rut_vm::interp:: Vm::builder().program(Rc::new(prog)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(hosts).build()
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
        entry fn main() -> i64 {
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
    let out = graph_of(
        rut_driver::RutRun::new()
            .pkgs(&world_with(src))
            .entrypoint("app_main")
            .compile(),
    );
    let prog = out.program.expect("linked program");
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(50_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp:: Vm::builder().program(Rc::new(prog)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
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

/// A current module decodes; the same bytes with the version word rolled
/// back are rejected with the standard clear error (stale artifacts carry
/// the wire vocabulary of an older law, which the new engines must not
/// misread).
#[test]
fn version_gate_rejects_stale_artifacts() {
    use rut_core::binary::{decode, VERSION};
    // v12 is the char exorcism (nmap-hostvals batch phase 1: the
    // enumerated `char` finishes dying — the char prim tag (11) is
    // withdrawn from `PrimTy`, const tag 3 (`ConstVal::Char`) is
    // withdrawn, and opcode 48 (`StrCharAt`) is REPLACED by `StrCodeAt`
    // (91), never re-meaninged; every codepoint rides a plain u32 and
    // the new `Nat::StrFromCode` (21) rides the same bump); v11 was the
    // opaque-is law (the opaque-is batch phase 1: `is` on an
    // opaque box answers BY THE BOX — the IsType/IsIface op bodies read
    // `cell.ty`, `o is X` misses for every payload X, `o is opaque`
    // (or an alias) stays true, `downcast<T>` is the only recovery; a
    // POLICY bump per the v9 precedent — the op stream is
    // byte-identical, but the same `istype` bytes answer differently
    // under the new engine, so old artifacts are NOT
    // behavior-identical); v10 was the general scan/classify + builder
    // surface (the json-perf
    // batch phase 2: the `StrScan`/`StrStartsWith`/`StrBuf*` natives +
    // the `StrBuf` boot type, the 7→8 declared-surface precedent); v9
    // was the orphan rule (the orphan-rule batch's
    // rejection addition, the 5→6 precedent); v8 was the err-channel
    // phase 2 declared-surface change (`capture_stacktrace()` + the
    // `StackTrace` builtin class + the `pos` span table);
    // v13 is the weak batch (`TyKind::Weak` + the two
    // Weak ops — new encoded vocabulary, the bump law); v14 is the
    // disposal surface (the `DisposalContext` boot type — id 22, kind
    // tag 17 — plus the `Disposal`/`DisposalContext` surface rows, the
    // v10 declared-surface precedent); v15 is the disposal dispatch —
    // the binary gains the per-type `dispose` section beside the
    // vtables (the v14 interim carried the surface decl-only and had
    // no such section, so stale artifacts are refused rather than
    // misread); v16 is surfaces on the wire — the encoded program
    // carries its full exported surface after the main tables
    // (namespace, fns, consts, types + scope blocks + type exports,
    // native rows with their ambient bits, and the reserved
    // inherent-impl table): a declared-surface change, and stale v15
    // artifacts carry no surface section at all;
    // v17 was owner-anchored instantiation — type exports carry their
    // generic parameter lists and the program carries the
    // instantiation ledger (type rows + fn identities keyed by
    // `(owner pkg, decl, arguments)`), so link unifies one
    // instantiation program-wide and consumers resolve requests
    // against a packaged binary's ledger; v18 adds the exported
    // generic fns' placeholder signatures; v19 withdraws the builder's
    // engine surface (kind tag 15, native-type tag 2, the five
    // `Nat::StrBuf*` rows — every later nat's tag shifts down); v20
    // closes the async classes (`Future`/`RunContext` became CLOSED
    // builtin classes and `Iterable`/`Disposal` the bracket markers,
    // and the inherent-method surface rows gained the
    // designated-slot marker byte — a stale v19 artifact misparses the
    // first marked method row); v21 is the structural-interfaces fork —
    // the trait tables leave the wire (`SurfaceImpl` rows and the
    // native-trait name table are gone; `SurfaceIface` crosses
    // interface decls, the program carries `ifaces`/`iface_slots`)
    assert_eq!(VERSION, 21, "the structural-interfaces fork owns this VERSION bump");
    let out = rut_driver::compile_module(
        "entry fn main() -> i64 { let mut a: [?i64] = [nil; 2]; a[0] = 1; let x = a[0]; return x; }",
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