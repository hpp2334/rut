//! Method-call generic args validate (the `ExprRef::Method` lane): the
//! site's explicit list answers to the RESOLVED method's declared
//! binders — more args than binders is the arity error (the house
//! text, at the list's own span: first spelled arg through last), and
//! every spelled arg resolves as a TYPE through the same lanes an
//! `x: T` slot rides (the unknown-name family, at the arg's own
//! span). The under-spelled call keeps its inference, the bare call
//! on a non-generic method stays silent, and the named regression —
//! `log.warn<T>("hi")` on the tree ink pkg's non-generic `Logger`
//! method — used to run clean through the extern-class lane.

use rut_ast::parser::Mode;
use rut_lexer::diag::Diag;
use rut_lexer::span::Span;
use rut_vm::interp::{HostRegistry, Limits, Vm};

const INK_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/ink");

fn compile(src: &str) -> rut_driver::ProgramOutput {
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string())],
    )
}

/// The app compiled over the tree's ink pkg (its `ink_host` path dep
/// rides the walk): diags only — the arity refusal needs no host.
fn ink_diags(app_src: &str) -> Vec<Diag> {
    let mut loaded = rut_native::dir_pkgs(std::path::Path::new(INK_DIR)).expect("mount ink");
    loaded.pkgs.push(
        rut_driver::Mounted::source("app", app_src)
            .manifest(rut_driver::Manifest::lib("app", &["ink"])),
    );
    match loaded
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
        .entrypoint("app")
        .compile()
    {
        Ok(c) => c.graph.diags,
        Err(e) => vec![Diag::new(Span::new(0, 0), e.msg)],
    }
}

/// compile + run: the entry fn hands an i32 back through the typed
/// host boundary (the return-value pin).
fn run_returns(src: &str) -> Result<i32, String> {
    let out = compile(src);
    if !out.diags.is_empty() {
        return Err(out
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("; "));
    }
    let prog = rut_core::link::flatten(out.program.expect("linked program"));
    let limits = Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = Vm::builder()
        .program(std::rc::Rc::new(prog))
        .limits(limits)
        .hosts(HostRegistry::new())
        .build()
        .map_err(|e| format!("{e:?}"))?;
    vm.call::<_, i32>("main", ()).map_err(|e| format!("{e:?}"))
}

/// `recv.meth<T>(..)` on a non-generic LOCAL method: the arity error,
/// exact house text, one diagnostic, at the generic-list span (the
/// spelled arg node's own span).
#[test]
fn non_generic_method_type_args_are_the_arity_error() {
    let src = "class Box2 {\n\
               \x20   v: i32 = 0;\n\
               }\n\
               impl Box2 {\n\
               \x20   fn bump(self) -> i32 { return self.v; }\n\
               }\n\
               entry fn main() -> i32 {\n\
               \x20   let b = Box2 {};\n\
               \x20   return b.bump<i64>();\n\
               }\n";
    let ds = compile(src).diags;
    assert_eq!(ds.len(), 1, "one diagnostic, not a cascade: {ds:?}");
    assert_eq!(
        ds[0].msg, "`bump` takes 0 type argument(s), 1 given",
        "{ds:?}"
    );
    // the list's own span: the spelled arg (`i64` inside `b.bump<i64>()`)
    let lo = src.find("bump<i64>").expect("spelled list") as u32 + 5;
    assert_eq!(ds[0].span, Span::new(lo, lo + 3), "{ds:?}");
}

/// THE NAMED REGRESSION (the extern-class lane): `log.warn<T>("hi")`
/// on the tree ink pkg's non-generic `Logger::warn` used to run clean
/// — the site's list rode along silently and the program printed. It
/// is the arity error now, same house text, at the list's own span.
#[test]
fn extern_method_type_args_are_the_arity_error() {
    let src = "use ink::{ Logger };\n\
               \n\
               entry fn main() {\n\
               \x20   let log = Logger::new(\"app\");\n\
               \x20   log.warn<T>(\"hi\");\n\
               }\n";
    let ds = ink_diags(src);
    assert_eq!(ds.len(), 1, "one diagnostic, not a cascade: {ds:?}");
    assert_eq!(
        ds[0].msg, "`warn` takes 0 type argument(s), 1 given",
        "{ds:?}"
    );
    let lo = src.find("warn<T>").expect("spelled list") as u32 + 5;
    assert_eq!(ds[0].span, Span::new(lo, lo + 1), "{ds:?}");
}

/// A genuinely generic method called with an unresolvable arg: the
/// resolution error — the unknown-name family — at the ARG's own
/// span (the same lanes an `x: T` slot rides).
#[test]
fn unknown_type_argument_errs_at_the_arg_span() {
    let src = "class Ctx2 {\n\
               \x20   n: i32 = 0;\n\
               }\n\
               impl Ctx2 {\n\
               \x20   fn get<T, A>(self, a: A) -> i32 { let _ = a; return self.n; }\n\
               }\n\
               entry fn main() -> i32 {\n\
               \x20   let c = Ctx2 {};\n\
               \x20   return c.get<Bogus>(1);\n\
               }\n";
    let ds = compile(src).diags;
    assert_eq!(ds.len(), 1, "one diagnostic, not a cascade: {ds:?}");
    assert_eq!(ds[0].msg, "unknown type `Bogus`", "{ds:?}");
    let lo = src.find("get<Bogus>").expect("spelled arg") as u32 + 4;
    assert_eq!(ds[0].span, Span::new(lo, lo + 5), "{ds:?}");
}

/// The LEGAL shapes stay legal and run: the exact spelling on a
/// generic method, the under-spelled call riding inference, and the
/// bare call on a non-generic method (zero args on zero params,
/// silent) — one program, one return-value pin.
#[test]
fn legal_spelled_inferred_and_bare_calls_compile_and_run() {
    let v = run_returns(
        "class Store2 {\n\
         \x20   n: i32 = 7;\n\
         }\n\
         impl Store2 {\n\
         \x20   fn get<T>(self, a: T) -> i32 { let _ = a; return self.n; }\n\
         \x20   fn one(self) -> i32 { return 1; }\n\
         }\n\
         entry fn main() -> i32 {\n\
         \x20   let s = Store2 { n: 7 };\n\
         \x20   let exact = s.get<u32>(1);\n\
         \x20   let inferred = s.get(\"two\");\n\
         \x20   let bare = s.one();\n\
         \x20   return exact + inferred + bare;\n\
         }\n",
    )
    .expect("runs");
    assert_eq!(v, 15, "7 + 7 + 1 — every legal shape answered");
}
