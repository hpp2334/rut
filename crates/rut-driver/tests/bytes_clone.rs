//! `bytes.clone()` — the one copy escape hatch (RFC 0044). Every cell
//! type shares on binding; the clone member is the ONE explicit,
//! one-shot buffer copy. These tests pin the surface end-to-end: the
//! member compiles, lowers to the `BytesClone` native (a fresh buffer,
//! never an alias), runs, and `==` still CONTENT-compares a clone equal
//! to its original (the str/bytes `==` law, RFC 0012 §4). The mutation
//! isolation proof — bytes are immutable in the language, so the
//! original's storage is rewritten engine-side — lives beside the
//! native in rut-vm's `interp::tests`.

use rut_driver::{Module, Session};
use rut_parser::Mode;

/// Compile-only harness: keeps the IR dump for the lowering assertions.
fn compile_program(src: &str) -> rut_driver::ProgramOutput {
    let collection = rut_core::binary::Surface::default();
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core()), (3, collection)],
    )
}

/// Compile, flatten, verify, and run `main` — the i32 checksum.
fn run_main(app_src: &str) -> i32 {
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module("app_main", Module { source: Some(app_src.into()), ..Default::default() }).unwrap();
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

#[test]
fn clone_lowers_to_the_fresh_buffer_native() {
    let out = compile_program(
        "fn main() -> i32 {\n\
             let a = bytes.from([1u8, 2, 3]);\n\
             let b = a.clone();\n\
             if (a != b) { return 1; }\n\
             return a.len() * 10 + b.len();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    assert!(
        out.ir_dump.contains("callnat BytesClone"),
        "the clone must lower to the BytesClone native — a fresh buffer, not an alias: {}",
        out.ir_dump
    );
}

#[test]
fn clone_content_compares_equal_and_runs() {
    // `==` on bytes is CONTENT equality (RFC 0012 §4): the fresh buffer
    // is equal to its original, and a different buffer is not. The
    // clone decodes and lenses exactly like the original.
    let sum = run_main(
        "pub fn main() -> i32 {\n\
             let a = bytes.from([1u8, 2, 3]);\n\
             let b = a.clone();\n\
             let c = bytes.from([1u8, 2, 4]);\n\
             if (a != b) { return 1; }\n\
             if (a == c) { return 2; }\n\
             if (b.decode() != a.decode()) { return 3; }\n\
             return a.len() * 100 + b.len() * 10 + c.len();\n\
         }\n",
    );
    assert_eq!(sum, 333, "3+3+3 octets across original, clone, and the unlike buffer");
}

#[test]
fn clone_takes_no_arguments_and_other_members_still_diagnose() {
    let ds: Vec<String> = compile_program(
        "fn main() -> i32 {\n\
             let a = bytes.from([1u8]);\n\
             let b = a.clone(a);\n\
             return b.len();\n\
         }\n",
    )
    .diags
    .iter()
    .map(|d| d.msg.clone())
    .collect();
    assert!(
        !ds.is_empty(),
        "clone() takes no arguments — an argument must diagnose: {ds:?}"
    );
    let ds: Vec<String> = compile_program(
        "fn main() -> i32 {\n\
             let a = bytes.from([1u8]);\n\
             return a.nope();\n\
         }\n",
    )
    .diags
    .iter()
    .map(|d| d.msg.clone())
    .collect();
    assert!(
        ds.iter().any(|d| d.contains("`bytes` has no method") && d.contains("`clone`")),
        "the no-method diagnostic must list the clone member: {ds:?}"
    );
}
