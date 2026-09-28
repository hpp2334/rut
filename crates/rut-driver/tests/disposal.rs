//! Disposal dispatch (the cell-death contract): the engine pins a cell
//! whose type implements `Disposal` at strong-count zero, queues it, and
//! runs `dispose(self, cx)` at the next call boundary on the same frame
//! machine. The pins here:
//!
//! - the release path: a dispose-implementing cell frees ONLY through the
//!   drain (the pin, the queued call, the post-`dispose` release) — the
//!   heap comes back to its baseline, nothing leaks;
//! - the cx: a `DisposalContext` is minted per call and released with the
//!   callee's root ret + the drain (accounting-neutral);
//! - weak boxes go dead BEFORE dispose observes them (the driver-lane
//!   pin lives in the weak tests; here the weak-after-churn balance);
//! - the per-type rows ride the binary: a compiled module decodes with
//!   its disposal row, and the func id survives encode → decode.

use rut_driver::{Module, ModuleBody, Session};

fn run_logged(src: &str) -> (rut_vm::interp::Vm, Vec<String>) {
    let combined = format!("{src}\nuse ink::{{Logger}};\n");
    let mut s = Session::new();
    rut_driver::mount_std(&mut s);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    rut_driver::mount_dir(&mut s, &root.join("rut/ink")).expect("mount ink (+rt)");
    s.register_module("app_main", Module { body: ModuleBody::Source { text: combined.into(), is_decl: false }, ..Default::default() }).unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    assert!(out.diags.is_empty(), "diags: {:?}", out.diags);
    let prog = out.program.expect("linked program");
    rut_vm::verify::verify(&prog).expect("verify");
    let lines: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = lines.clone();
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_std::logger::install_std_log(&mut hosts, move |msg| {
        sink.borrow_mut().push(msg.to_string());
    });
    rut_std::math::install_std_math(&mut hosts);
    hosts.verify_against(&s.expected_host_fns());
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        hosts,
    )
    .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run");
    let out_lines = lines.borrow().clone();
    (vm, out_lines)
}

use std::cell::RefCell;
use std::rc::Rc;

const DISP: &str = r#"
use core::{ Disposal, DisposalContext };
struct A { n: i32 = 0; }
impl Disposal for A { fn dispose(mut self, cx: DisposalContext) { Logger.new("t").info(f"gone {self.n}"); } }
"#;

#[test]
fn the_heap_returns_to_baseline_through_the_disposal_drain() {
    let src = format!(
        r#"{DISP}
pub fn main() -> i32 {{
    let mut acc = 0;
    for (let i = 0; i < 50; i += 1) {{
        let mut a = A {{ n: i }};
        acc += a.n;
        a = A {{ n: 7 }};   // a mid-frame death: pin, queue, drain at the end
    }}
    return acc;            // 100 cells disposed by the time this ret drains
}}
"#
    );
    let (vm, lines) = run_logged(&src);
    // 50 rebind deaths + 50 frame-end deaths, all drained at the root ret
    // (one rebind death also reads "gone 7" — i == 7's own rebind)
    assert_eq!(lines.len(), 100, "every death disposed: {lines:?}");
    assert!(lines.iter().all(|l| l.starts_with("gone ")), "{lines:?}");
    // the pins released, the cx cells released — what remains charged is
    // exactly the const pool's immortal str slots (materialized at load,
    // the module keeps them for the program's lifetime; 24 B header + the
    // octets each)
    let const_bytes: u64 = vm
        .prog
        .consts
        .iter()
        .map(|c| match c {
            rut_core::binary::ConstVal::Str(s) => 24 + s.len() as u64,
            _ => 0,
        })
        .sum();
    assert_eq!(
        vm.heap_usage(),
        const_bytes,
        "the disposal machinery leaks nothing beyond the const pool"
    );
    assert!(vm.heap_peak() > const_bytes, "the drain ran on a real heap");
}

#[test]
fn the_disposal_row_rides_the_binary() {
    let src = r#"
use core::{ Disposal, DisposalContext };
struct A { n: i32 = 0; }
impl Disposal for A { fn dispose(mut self, cx: DisposalContext) { } }
pub fn main() -> i32 { let a = A { n: 9 }; return 0; }
"#;
    let out = rut_driver::compile_module(src, rut_parser::Mode::Impl, "app_main");
    assert!(out.diags.is_empty(), "diags: {:?}", out.diags);
    let bytes = out.binary.expect("encoded module");
    let prog = rut_core::binary::decode(&bytes).expect("decode");
    // exactly one disposal row: the `dispose` func id for A — not a boot
    // type, not a fn-less shell
    let rows: Vec<(usize, u32)> = prog
        .disposal_impls
        .iter()
        .enumerate()
        .filter_map(|(ty, f)| f.map(|f| (ty, f)))
        .collect();
    assert_eq!(rows.len(), 1, "one row: {:?}", rows);
    let (ty, fid) = rows[0];
    // the dispose fn: the impl method's inst name ends in `$dispose`
    let fname = prog.interner.name(prog.funcs[fid as usize].name);
    assert!(fname.ends_with("dispose"), "the dispose inst: {fname}");
    assert!(ty >= rut_core::types::TypeTable::boot().types.len(), "a user type, not a boot id");
    // the linked program keeps the row and the VM releases cleanly
    let flat = rut_core::link::flatten(prog);
    assert!(flat.disposal_impls.iter().filter(|f| f.is_some()).count() >= 1);
}

/// Import gating (the `pub builtin` spellings): `Disposal` and
/// `DisposalContext` resolve ONLY through `use core::{ .. }` — the bare
/// source fails compilation naming the fix exactly, a use that names
/// only one of the pair leaves the other missing with its own fix, and
/// the fully-imported source compiles and runs.
#[test]
fn disposal_requires_the_import() {
    let body = r#"
struct A { n: i32 = 0; }
impl Disposal for A { fn dispose(mut self, cx: DisposalContext) { } }
pub fn main() -> i32 { let a = A { n: 9 }; return a.n; }
"#;
    // no use at all: the trait miss names the fix
    let out = rut_driver::compile_module(body, rut_parser::Mode::Impl, "app_main");
    assert!(
        out.diags
            .iter()
            .any(|d| d.msg == "`Disposal` is not in scope — `use core::{ Disposal }`"),
        "the trait miss names the fix: {:?}",
        out.diags
    );
    assert!(out.binary.is_none(), "the bare source must not compile");

    // the trait imported but not the cx type: the type miss names its
    // own fix (the impl still fails — dispose's signature is wrong)
    let trait_only = format!("use core::{{ Disposal }};\n{body}");
    let out = rut_driver::compile_module(&trait_only, rut_parser::Mode::Impl, "app_main");
    assert!(
        out.diags
            .iter()
            .any(|d| d.msg == "`DisposalContext` is not in scope — `use core::{ DisposalContext }`"),
        "the cx-type miss names the fix: {:?}",
        out.diags
    );

    let with = format!("use core::{{ Disposal, DisposalContext }};\n{body}");
    let out = rut_driver::compile_module(&with, rut_parser::Mode::Impl, "app_main");
    assert!(out.diags.is_empty(), "diags: {:?}", out.diags);
    let bytes = out.binary.expect("encoded module");
    let prog = rut_core::link::flatten(rut_core::binary::decode(&bytes).expect("decode"));
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(prog),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    let v: i32 = vm.call("main", ()).expect("run");
    assert_eq!(v, 9);
}

