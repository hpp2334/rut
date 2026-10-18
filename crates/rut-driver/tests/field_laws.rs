//! The record-literal field laws: every field is covered — spelled in
//! the literal or covered by the field's declared initializer — and no
//! field is spelled twice. Omitted-without-initializer and duplicates
//! diagnose; the zero-fill of yesteryear is gone.

use rut_parser::Mode;


/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(rut_driver::RutRun::new().pkg(rut_driver::Mounted::source(spec, src)).entrypoint(spec).compile())
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

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // core + pouch bound as the uses; these
    // tests exercise literal laws, not use discipline
    let collection = rut_core::binary::Surface::default();
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string()), (3, collection, "collection".to_string())],
    )
}

fn diags_of(src: &str) -> Vec<String> {
    compile(src)
        .diags
        .iter()
        .map(|d| d.msg.clone())
        .collect()
}

fn one_diag(src: &str) -> String {
    let d = diags_of(src);
    assert_eq!(d.len(), 1, "expected exactly one diagnostic, got {d:?}");
    d[0].clone()
}

/// Compile, flatten, verify, and run a single-module
/// `main` returning i32.
fn run_main(src: &str) -> i32 {
    let out = compile(src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = rut_core::link::flatten(out.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp:: Vm::builder().program(std::rc::Rc::new(prog)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
    .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

// ---- the every-field-covered law -------------------------------------

#[test]
fn omitted_field_without_an_initializer_diagnoses() {
    // `P { a: 1 }` for `{ a: i32, b: str }`: `b` takes no silent zero
    // value — the literal must cover it or the field must declare an
    // initializer
    let msg = one_diag(
        "struct P { a: i32; b: str; }\n\
         entry fn main() -> i32 {\n\
             let p = P { a: 1 };\n\
             return p.a;\n\
         }\n",
    );
    assert!(
        msg.contains("missing field `b` in the `P` literal (the every-field-covered law)"),
        "got: {msg}"
    );
}

#[test]
fn missing_class_literal_field_names_the_field_and_type() {
    // the same law on the class instance literal (struct vs class
    // changes nothing at the literal): the diag names the uncovered
    // field and the literal's type
    let msg = one_diag(
        "class Tagged { x: i32; tag: str; }\n\
         entry fn main() -> i32 {\n\
             let t = Tagged { x: 1 };\n\
             return t.x;\n\
         }\n",
    );
    assert!(
        msg.contains("missing field `tag` in the `Tagged` literal"),
        "got: {msg}"
    );
}

#[test]
fn omitted_field_with_a_declared_initializer_takes_it() {
    // the legal omission: the field's initializer covers it — `p.b`
    // observes `"hi"`, not the zero string
    let n = run_main(
        "struct P { a: i32; b: str = \"hi\"; }\n\
         entry fn main() -> i32 {\n\
             let p = P { a: 1 };\n\
             if (p.b == \"hi\") { return p.a + 10; }\n\
             return -1;\n\
         }\n",
    );
    assert_eq!(n, 11);
}

#[test]
fn class_literal_omission_takes_the_declared_initializer() {
    // the class instance literal answers to the same law: every field
    // declared an initializer, so the empty spelling is legal — and
    // the initializer's value is what runs
    let n = run_main(
        "class C { n: i32 = 42; }\n\
         entry fn main() -> i32 {\n\
             let c = C { };\n\
             return c.n;\n\
         }\n",
    );
    assert_eq!(n, 42);
}

// ---- the duplicate-field law ------------------------------------------

#[test]
fn duplicate_struct_literal_field_diagnoses() {
    // last-wins is gone: the second spelling of a field is a duplicate
    let msg = one_diag(
        "struct P { a: i32; }\n\
         entry fn main() -> i32 {\n\
             let p = P { a: 1, a: 2 };\n\
             return p.a;\n\
         }\n",
    );
    assert!(
        msg.contains("duplicate field `a` in the `P` literal"),
        "got: {msg}"
    );
}

#[test]
fn duplicate_class_literal_field_diagnoses() {
    // one path for struct and class: the class literal refuses the
    // second spelling too
    let msg = one_diag(
        "class Dup { a: i32; }\n\
         entry fn main() -> i32 {\n\
             let d = Dup { a: 1, a: 2 };\n\
             return d.a;\n\
         }\n",
    );
    assert!(
        msg.contains("duplicate field `a` in the `Dup` literal"),
        "got: {msg}"
    );
}
