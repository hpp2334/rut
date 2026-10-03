//! `bytes.clone()` — the one copy escape hatch. Every cell
//! type shares on binding; the clone member is the ONE explicit,
//! one-shot buffer copy. These tests pin the surface end-to-end: the
//! member compiles, lowers to the `BytesClone` native (a fresh buffer,
//! never an alias), runs, and `==` still CONTENT-compares a clone equal
//! to its original (the str/bytes `==` law). The mutation
//! isolation proof — bytes are immutable in the language, so the
//! original's storage is rewritten engine-side — lives beside the
//! native in rut-vm's `interp::tests`.

use rut_parser::Mode;


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

/// Compile-only harness: keeps the IR dump for the lowering assertions.
fn compile_program(src: &str) -> rut_driver::ProgramOutput {
    let collection = rut_core::binary::Surface::default();
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string()), (3, collection, "collection".to_string())],
    )
}

/// Compile, flatten, verify, and run `main` — the i32 checksum.
fn run_main(app_src: &str) -> i32 {
    let out = compiled("app_main", app_src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp:: Vm::builder().program(std::rc::Rc::new(flat)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
    .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

#[test]
fn clone_lowers_to_the_fresh_buffer_native() {
    let out = compile_program(
        "entry fn main() -> i32 {\n\
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
    // `==` on bytes is CONTENT equality: the fresh buffer
    // is equal to its original, and a different buffer is not. The
    // clone decodes and lenses exactly like the original.
    let sum = run_main(
        "entry fn main() -> i32 {\n\
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
        "entry fn main() -> i32 {\n\
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
        "entry fn main() -> i32 {\n\
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
