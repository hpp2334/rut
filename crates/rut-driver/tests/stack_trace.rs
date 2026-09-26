//! `capture_stacktrace()` + the `StackTrace` builtin class (RFC 0036,
//! err-channel phase 2). The surface is RFC 0025's `builtin class` row —
//! a pure signature contract; the engine implements it. The pins here:
//!
//! - the frame walk: RAW frames, innermost first, `[c, b, a]` order at
//!   known nesting (names exact);
//! - the call-site line/col of every frame (1-based, the callee ident's
//!   column — the compiler bakes pc → (line, col) while the module
//!   source is in hand);
//! - `render()` vs RFC 0036's symbolication shape
//!   (`at name (module:line:col)`, degrading to `at name (module #f @
//!   pc p)` when stripped);
//! - out-of-range index = the LOUD trap (the index is a bug, not data);
//! - the prim/ref value paths (`len`/`line`/`col` are prims, `name`/
//!   `render` are strs, the trace itself is a shared cell);
//! - the inline law: a capture reflects the PIPELINE's frames — a small
//!   (≤24-statement) callee is checker-inlined, so its capture reports
//!   the frame it landed in, with the ORIGINAL source position;
//! - VERSION 8: the declared-surface change (the 6→7 precedent).
//!
//! House style note: bodies in these tests exceed the checker inliner's
//! 24-statement budget where REAL frames are wanted (`pad`), so the pins
//! are against genuinely separate FuncCodes.

use rut_driver::{Module, Session};
use rut_parser::Mode;

/// Compile, flatten, verify, and run `main` — the i32 checksum.
fn run_main(src: &str) -> i32 {
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module("app_main", Module { source: Some(src.into()), ..Default::default() }).unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

/// Compile for inspection: returns the flattened program + a ready Vm.
fn compile_vm(src: &str) -> (rut_core::binary::Program, rut_vm::interp::Vm) {
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module("app_main", Module { source: Some(src.into()), ..Default::default() }).unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let flat = rut_core::link::flatten(out.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat.clone()),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    (flat, vm)
}

/// `n` filler statements — the checker inliner declines bodies over 24
/// top-level statements, so a padded fn stays a REAL frame.
fn pad(n: usize) -> String {
    (0..n).map(|i| format!("    let p{i} = {i};\n")).collect()
}

/// The three-deep chain, each body padded past the inline budget.
/// Line numbers are load-bearing (pinned below).
fn chain_src() -> String {
    let mut s = String::from("fn c_big() -> str {\n");
    s.push_str(&pad(25)); // lines 2..26
    s.push_str("    let t = capture_stacktrace();\n"); // line 27, col 13
    s.push_str("    return t.render();\n");
    s.push_str("}\n"); // line 29
    s.push_str("fn b_big() -> str {\n"); // line 30
    s.push_str(&pad(25));
    s.push_str("    return c_big();\n"); // line 56, col 12
    s.push_str("}\n");
    s.push_str("fn a_big() -> str {\n");
    s.push_str(&pad(25));
    s.push_str("    return b_big();\n"); // line 84, col 12
    s.push_str("}\n");
    s.push_str("pub fn main() -> str {\n    return a_big();\n}\n"); // call at line 87, col 12
    s
}

#[test]
fn capture_lowers_to_the_capture_native() {
    let collection = rut_core::binary::Surface::default();
    let out = rut_driver::compile_program(
        "fn go() -> i32 {\n    let t = capture_stacktrace();\n    return t.len();\n}\npub fn main() -> i32 {\n    return go();\n}\n",
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string()), (3, collection, "collection".to_string())],
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    assert!(
        out.ir_dump.contains("callnat CaptureTrace"),
        "the capture must lower to the CaptureTrace native: {}",
        out.ir_dump
    );
    assert!(
        out.ir_dump.contains("callnat TraceLen"),
        "the member must lower to the TraceLen native: {}",
        out.ir_dump
    );
}

#[test]
fn frame_names_order_and_depth_are_exact() {
    // a_big -> b_big -> c_big captures: [c_big, b_big, a_big, main],
    // names EXACT, depth exact — innermost first
    let full = {
        let mut s2 = String::from("fn c_big() -> i32 {\n");
        s2.push_str(&pad(25));
        s2.push_str("    let t = capture_stacktrace();\n");
        s2.push_str("    if (t.len() != 4) { return 10; }\n"); // c,b,a,main
        s2.push_str("    if (t.name(0) != \"c_big\") { return 11; }\n");
        s2.push_str("    if (t.name(1) != \"b_big\") { return 12; }\n");
        s2.push_str("    if (t.name(2) != \"a_big\") { return 13; }\n");
        s2.push_str("    if (t.name(3) != \"main\") { return 14; }\n");
        s2.push_str("    return 7;\n");
        s2.push_str("}\n");
        s2.push_str("fn b_big() -> i32 {\n");
        s2.push_str(&pad(25));
        s2.push_str("    return c_big();\n");
        s2.push_str("}\n");
        s2.push_str("fn a_big() -> i32 {\n");
        s2.push_str(&pad(25));
        s2.push_str("    return b_big();\n");
        s2.push_str("}\n");
        s2.push_str("pub fn main() -> i32 {\n");
        s2.push_str("    return a_big();\n");
        s2.push_str("}\n");
        s2
    };
    assert_eq!(run_main(&full), 7);
}

#[test]
fn call_site_lines_and_cols_are_exact() {
    // the render carries every frame's call site: line = the call line,
    // col = the 1-based column of the CALLEE IDENT. chain_src pins the
    // sites: c_big captures at 27:13, b_big calls c_big at 56:12,
    // a_big calls b_big at 84:12, main calls a_big at 87:12.
    let src = chain_src();
    let (_flat, mut vm) = compile_vm(&src);
    let r: String = vm.call::<_, String>("main", ()).expect("run");
    let lines: Vec<&str> = r.lines().collect();
    assert_eq!(lines[0], "at c_big (core:27:13)", "capture site: the CallNat op's own position");
    assert_eq!(lines[1], "at b_big (core:56:12)", "the call that pushed the c_big frame");
    assert_eq!(lines[2], "at a_big (core:84:12)");
    assert_eq!(lines[3], "at main (core:87:12)");
    assert_eq!(lines.len(), 4, "innermost first, one line per frame: {r}");
}

#[test]
fn members_report_positions_individually() {
    // line(i)/col(i) — the lazy per-index path — agree with render()'s
    // positions: frame 0 = the capture site, frame 1 = the call in main
    let main_call_line = 35; // counted: 1 sig + 25 pad + 27 capture + 4 ifs + 32 ret + 33 brace + 34 sig + 35 call
    let mut sum = String::from("fn c_big() -> i32 {\n");
    sum.push_str(&pad(25)); // lines 2..26
    sum.push_str("    let t = capture_stacktrace();\n"); // line 27, col 13
    sum.push_str("    if (t.line(0) != 27) { return 1; }\n");
    sum.push_str("    if (t.col(0) != 13) { return 2; }\n");
    sum.push_str(&format!("    if (t.line(1) != {main_call_line}) {{ return 3; }}\n"));
    sum.push_str("    if (t.col(1) != 12) { return 4; }\n");
    sum.push_str("    return 9;\n");
    sum.push_str("}\n"); // line 29
    sum.push_str("pub fn main() -> i32 {\n    return c_big();\n}\n"); // call at line 30, col 12
    assert_eq!(run_main(&sum), 9);
}

#[test]
fn out_of_range_index_traps_loud() {
    // `grab` is tiny, so it checker-inlines into main — the trace has one
    // frame (see the inline-law test); the LOUD part is what's pinned:
    let src = "fn grab() -> i32 {\n    let t = capture_stacktrace();\n    return t.name(5).len();\n}\npub fn main() -> i32 {\n    return grab();\n}\n";
    let (_flat, mut vm) = compile_vm(src);
    let err = vm.call::<_, i32>("main", ()).unwrap_err();
    assert_eq!(err.kind, rut_vm::TrapKind::IndexOutOfBounds);
    assert!(
        err.msg.contains("StackTrace index 5 out of range"),
        "the loud message names the index and the len: {}",
        err.msg
    );
    assert!(err.msg.contains("len is 1"), "the trace's real depth is disclosed: {}", err.msg);

    // a negative index is out of range too — the index is a bug, not data
    let src = "fn grab() -> i32 {\n    let t = capture_stacktrace();\n    return t.line(-1);\n}\npub fn main() -> i32 {\n    return grab();\n}\n";
    let (_flat, mut vm) = compile_vm(src);
    let err = vm.call::<_, i32>("main", ()).unwrap_err();
    assert_eq!(err.kind, rut_vm::TrapKind::IndexOutOfBounds);
    assert!(err.msg.contains("StackTrace index -1"), "{}", err.msg);
}

#[test]
fn prim_and_ref_member_paths_flow() {
    // len/line/col are prims (arithmetic works), name is a str (content
    // compares), the trace itself is a shared cell (passes through a
    // call argument by reference and stays readable on both sides)
    let src = r#"fn read(t: StackTrace, side: i32) -> i32 {
    let p0 = 0;
    let p1 = 1;
    let p2 = 2;
    let p3 = 3;
    let p4 = 4;
    let p5 = 5;
    let p6 = 6;
    let p7 = 7;
    let p8 = 8;
    let p9 = 9;
    let p10 = 10;
    let p11 = 11;
    let p12 = 12;
    let p13 = 13;
    let p14 = 14;
    let p15 = 15;
    let p16 = 16;
    let p17 = 17;
    let p18 = 18;
    let p19 = 19;
    let p20 = 20;
    let p21 = 21;
    let p22 = 22;
    let p23 = 23;
    let p24 = 24;
    if (side == 0) { return t.len(); }
    if (t.name(0) != "grab") { return 50; }
    return t.col(0) + 100;
}
fn grab() -> i32 {
    let p0 = 0;
    let p1 = 1;
    let p2 = 2;
    let p3 = 3;
    let p4 = 4;
    let p5 = 5;
    let p6 = 6;
    let p7 = 7;
    let p8 = 8;
    let p9 = 9;
    let p10 = 10;
    let p11 = 11;
    let p12 = 12;
    let p13 = 13;
    let p14 = 14;
    let p15 = 15;
    let p16 = 16;
    let p17 = 17;
    let p18 = 18;
    let p19 = 19;
    let p20 = 20;
    let p21 = 21;
    let p22 = 22;
    let p23 = 23;
    let p24 = 24;
    let t = capture_stacktrace();
    let a = read(t, 0);
    let b = read(t, 1);
    return a * 1000 + b;
}
pub fn main() -> i32 {
    return grab();
}
"#;
    // a = len (2 frames: grab + main) → 2; b = col(0)+100 = 13+100 = 113
    assert_eq!(run_main(src), 2113);
}

#[test]
fn capture_reflects_the_pipelines_frames_the_inline_law() {
    // a small callee is checker-inlined (the fusion-era inliner, ≤24
    // statements): the capture inside it reports the frame it LANDED in
    // — `main` — with the ORIGINAL capture-site position. The trace is
    // the pipeline's truth, not the source's shape.
    let src = "fn tiny() -> i32 {\n    let t = capture_stacktrace();\n    if (t.len() != 1) { return 1; }\n    if (t.name(0) != \"main\") { return 2; }\n    if (t.line(0) != 2) { return 3; }\n    if (t.col(0) != 13) { return 4; }\n    return 5;\n}\npub fn main() -> i32 {\n    return tiny();\n}\n";
    assert_eq!(run_main(src), 5);
}

#[test]
fn recursion_pads_every_frame() {
    // each recursive activation is a real frame; the padded body keeps
    // the checker inliner out so the count is the source's shape
    let mut src = String::from("fn down(n: i32) -> i32 {\n");
    src.push_str(&pad(25));
    src.push_str("    if (n == 0) {\n        let t = capture_stacktrace();\n        return t.len() * 100 + t.name(0).len();\n    }\n");
    src.push_str("    return down(n - 1);\n");
    src.push_str("}\n");
    src.push_str("pub fn main() -> i32 {\n    return down(2);\n}\n");
    // frames: down, down, down, main → len 4; name(0) is "down" (4 chars)
    assert_eq!(run_main(&src), 404);
}

#[test]
fn render_degrades_when_stripped() {
    // a stripped build carries no position table: line/col read 0 and
    // render degrades to RFC 0036's pc-only shape
    let src = chain_src();
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module("app_main", Module { source: Some(src.into()), ..Default::default() }).unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let mut prog = out.program.expect("linked program");
    for f in prog.funcs.iter_mut() {
        f.pos.clear(); // --release: spans' positions stripped
    }
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits::default();
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    let r: String = vm.call::<_, String>("main", ()).expect("run");
    let lines: Vec<&str> = r.lines().collect();
    // pcs: the capture sits at op 25 of c_big (25 pad statements push it
    // there); each padded caller's call op is at op 25 too; main's call
    // is op 0. The func indices are link order.
    assert_eq!(lines[0], "at c_big (core #3 @ pc 25)");
    assert_eq!(lines[1], "at b_big (core #2 @ pc 25)");
    assert_eq!(lines[2], "at a_big (core #1 @ pc 25)");
    assert_eq!(lines[3], "at main (core #0 @ pc 0)");
}

#[test]
fn discarded_capture_releases_cleanly() {
    // `capture_stacktrace();` as a statement: the minted cell releases
    // (no leak), the run stays green under a tight heap budget
    let mut src = String::from("fn noiser() -> i32 {\n");
    src.push_str(&pad(25));
    src.push_str("    capture_stacktrace();\n");
    src.push_str("    return 3;\n");
    src.push_str("}\n");
    src.push_str("pub fn main() -> i32 {\n    let a = noiser();\n    capture_stacktrace();\n    return a;\n}\n");
    assert_eq!(run_main(&src), 3);
}

#[test]
fn version_nine_rejects_stale_artifacts() {
    // v13 is the weak batch (RFC 0017 v1: `TyKind::Weak` + the two Weak
    // ops — new encoded vocabulary, the bump law); v12 was the char
    // exorcism (the nmap-hostvals batch phase 1: the
    // enumerated `char` finishes dying — the char prim tag (11) withdrawn,
    // const tag 3 (`ConstVal::Char`) withdrawn, opcode 48 (`StrCharAt`)
    // REPLACED by `StrCodeAt` (91), never re-meaninged); v11 was the
    // opaque-is law (the opaque-is batch phase 1, the v9
    // policy-bump precedent: `is` answers by the box); v10 was the
    // general scan/classify + builder surface (the json-perf
    // batch phase 2, the 7→8 declared-surface precedent); v9 was the
    // orphan rule (RFC 0012 §2a, the 5→6 rejection-addition precedent);
    // v8 was the declared-surface change (the 6→7 precedent): a v7
    // header is rejected with the standard version error
    let mut bytes = Vec::new();
    bytes.extend_from_slice(rut_core::binary::MAGIC);
    bytes.extend_from_slice(&7u32.to_le_bytes());
    let err = rut_core::binary::decode(&bytes).unwrap_err();
    assert!(err.contains("unsupported module binary version 7"), "{err}");
    // and a fresh compile round-trips under v13
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module("app_main", Module { source: Some("pub fn main() -> i32 { return 4; }".into()), ..Default::default() }).unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    let prog = out.program.expect("program");
    let bytes = rut_core::binary::encode(&prog);
    assert_eq!(&bytes[4..8], &13u32.to_le_bytes(), "the header carries v13");
    assert!(rut_core::binary::decode(&bytes).is_ok());
}
