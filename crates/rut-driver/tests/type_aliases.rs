//! Type aliases, union bounds, and inline `requires` end to end
//!: transparency through params/fields/chains, the
//! recursion and bound-only diagnostics, admission at the call site
//! (concrete by id, interfaces via the structural member-set match,
//! interface objects satisfy nothing), and cross-module `pub type`
//! export.

use rut_parser::Mode;

use rut_core::types::TY_I64;


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
            .pkg(rut_driver::calc_pkg())
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

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // core bound as the one use: these tests exercise alias
    // and bound semantics, not use discipline
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string())],
    )
}

fn diags_of(src: &str) -> Vec<String> {
    compile(src).diags.iter().map(|d| d.msg.clone()).collect()
}

#[test]
fn alias_is_transparent_in_params_fields_and_lets() {
    let out = compile(
        "type Meters = i64;\n\
         struct Trip { dist: Meters }\n\
         fn walk(m: Meters) -> Meters { return m; }\n\
         entry fn main() -> i32 {\n\
             let d: Meters = 42;\n\
             let t: Trip = Trip { dist: d };\n\
             let n = walk(t.dist);\n\
             return n as i32;\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn alias_chains_expand() {
    let out = compile(
        "type A = B;\n\
         type B = C;\n\
         type C = i64;\n\
         fn f(x: A) -> C { return x; }\n\
         entry fn main() -> i32 { let v: A = 7; return f(v) as i32; }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn forward_referenced_alias_is_legal() {
    // pass 1b validates targets after pass 1a declared every name
    let out = compile(
        "fn f(x: Later) -> Later { return x; }\n\
         type Later = i64;\n\
         entry fn main() -> i32 { let v: Later = 3; return f(v) as i32; }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn recursive_alias_diagnoses() {
    let ds = diags_of(
        "type A = B;\n\
         type B = A;\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("recursive type alias")),
        "the cycle must diagnose: {ds:?}"
    );
}

#[test]
fn self_alias_diagnoses() {
    let ds = diags_of(
        "type A = A;\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("recursive type alias")),
        "the self-cycle must diagnose: {ds:?}"
    );
}

#[test]
fn unions_are_bound_only() {
    // a union alias in a value position errors (`pub` so the signature
    // is compiled — the library surface)
    let ds = diags_of(
        "type Num = i32 | str;\n\
         pub fn f(x: Num) -> i32 { return 0; }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("bound-only")),
        "union alias in a value position must diagnose: {ds:?}"
    );
    // so does an inline union type — a param list is a value position,
    // so `|` is not even valid type syntax there
    let ds = diags_of(
        "pub fn g(x: i32 | str) -> i32 { return 0; }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        !ds.is_empty(),
        "an inline union in a value position is a compile error: {ds:?}"
    );
}

#[test]
fn bound_admits_each_union_member_and_fails_outside() {
    let src = "\
fn tag<T requires i32 | str>(x: T) -> i32 { return 0; }\n\
entry fn main() -> i32 {\n\
    let a = tag(5);\n\
    let b = tag(\"s\");\n\
    return a + b;\n\
}\n\
";
    let out = compile(src);
    assert!(out.diags.is_empty(), "i32 and str both satisfy: {:?}", out.diags);

    let ds = diags_of(
        "fn tag<T requires i32 | str>(x: T) -> i32 { return 0; }\n\
         entry fn main() -> i32 { return tag(true); }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("does not satisfy") && d.contains("i32 | str")),
        "the failure names the bound: {ds:?}"
    );
}

#[test]
fn iface_member_bound_via_structural_satisfaction() {
    let src = "\
interface Enc { fn enc(self) -> bytes; }
class Token { v: i32 }
impl Token {
    pub fn new() -> Self { return Token { v: 1 }; }
    pub fn enc(self) -> bytes { return bytes(0); }
}
fn seal<T requires Enc>(x: T) -> i32 {
    let e: Enc = x;
    return e.enc().len();
}
entry fn main() -> i32 { let t: Token = Token.new(); return seal(t); }
";
    let out = compile(src);
    assert!(out.diags.is_empty(), "Token satisfies Enc: {:?}", out.diags);

    // str has no members — the bound names it
    let ds = diags_of(
        "interface Enc { fn enc(self) -> bytes; }\n\
         class Token { v: i32 }\n\
         impl Token {\n\
             pub fn new() -> Self { return Token { v: 1 }; }\n\
             pub fn enc(self) -> bytes { return bytes(0); }\n\
         }\n\
         fn seal<T requires Enc>(x: T) -> i32 { return 0; }\n\
         entry fn main() -> i32 { return seal(\"nope\"); }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`str` does not satisfy") && d.contains("Enc")),
        "no satisfaction for str: {ds:?}"
    );
}

#[test]
fn iface_object_satisfies_nothing() {
    // an interface-object value satisfies no bound —
    // only a concrete type with the members does
    let ds = diags_of(
        "interface Enc { fn enc(self) -> bytes; }\n\
         class Token { v: i32 }\n\
         impl Token {\n\
             pub fn new() -> Self { return Token { v: 1 }; }\n\
             pub fn enc(self) -> bytes { return bytes(0); }\n\
         }\n\
         fn seal<T requires Enc>(x: T) -> i32 { return 0; }\n\
         entry fn main() -> i32 {\n\
             let w: Enc = Token.new();\n\
             return seal(w);\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("does not satisfy")),
        "an interface object satisfies nothing: {ds:?}"
    );
}

#[test]
fn bound_may_reference_another_generic() {
    // `U requires [T]` — resolved under the call-site substitution
    let out = compile(
        "         fn hold<T, U requires [T]>(x: U) -> i32 { return 0; }\n\
         entry fn main() -> i32 {\n\
             let a = [0; 2];\n\
             return hold<i32, [i32]>(a);\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);

    // U = [str] does not satisfy `U requires [i32]`
    let ds = diags_of(
        "         fn hold<T, U requires [T]>(x: U) -> i32 { return 0; }\n\
         entry fn main() -> i32 {\n\
             let s = [\"\", \"\"];\n\
             return hold<i32, [str]>(s);\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("does not satisfy")),
        "[str] ≠ [i32]: {ds:?}"
    );
}

#[test]
fn method_bounds_parse_and_collect() {
    // a bounded method parses, collects, and compiles when uncalled —
    // the bound rides the method declaration
    let out = compile(
        "interface Enc { fn enc(self) -> bytes; }\n\
         class Token { v: i32 }\n\
         impl Token { pub fn enc(self) -> bytes { return bytes(0); } }\n\
         class W { v: i32; }\n\
         impl W {\n\
             fn new() -> Self { return Self { v: 0 }; }\n\
             fn wrap<T requires Enc>(self, x: T) -> i32 { return 0; }\n\
         }\n\
         entry fn main() -> i32 { let w: W = W.new(); return w.v; }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn alias_exports_transparently_across_modules() {
    // dep "units" scope 1: `pub type Meters = i64;` adds a surface row
    // keyed by i64's id — the root uses the NAME with zero changes
    let dep = rut_driver::compile_program(
        "pub type Meters = i64;\n\
         type Mix = i32 | str;\n\
         pub fn m(v: Meters) -> Meters { return v; }\n",
        Mode::Impl,
        "units",
        1,
        &[],
    );
    assert!(dep.diags.is_empty(), "{:?}", dep.diags);
    let dep = dep.program.expect("dep program");
    let surface = dep.surface.clone();
    let meters = surface
        .type_exports
        .iter()
        .find(|t| surface.names.name(t.name) == "Meters")
        .expect("Meters exported");
    // the alias binds the TARGET's id: i64 lives in the shared boot
    // table, so the row points at scope 0 with i64's local
    assert_eq!(meters.scope, Some(rut_core::BOOT_SCOPE));
    assert_eq!(meters.local, rut_core::local_of(TY_I64));
    assert!(!meters.is_class && !meters.is_generic);
    // the union alias stays module-local — never exported
    assert!(
        !surface.type_exports.iter().any(|t| surface.names.name(t.name) == "Mix"),
        "union aliases are not exported"
    );

    let root = rut_driver::compile_program(
        "use units::{ Meters, m };\n\
         entry fn main() -> i32 {\n\
             let d: Meters = 9;\n\
             return m(d) as i32;\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface, "units".to_string()), (3, rut_core::binary::Surface::core(), "core".to_string())],
    );
    assert!(root.diags.is_empty(), "cross-module transparency: {:?}", root.diags);

    // ... but the union alias's name never crossed — using it diagnoses
    let ds = rut_driver::compile_program(
        "use units::{ Mix };\nentry fn main() -> i32 { let x: Mix = 0; return 0; }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, {
            let dep = rut_driver::compile_program(
                "type Mix = i32 | str;\nentry fn main() -> i32 { return 0; }\n",
                Mode::Impl,
                "units",
                1,
                &[],
            );
            dep.program.expect("dep").surface.clone()
        }, "units".to_string())],
    )
    .diags
    .iter()
    .map(|d| d.msg.clone())
    .collect::<Vec<_>>();
    assert!(
        ds.iter().any(|d| d.contains("Mix")),
        "a cross-module union alias use fails as unknown: {ds:?}"
    );
}

#[test]
fn alias_shadows_nothing_and_duplicate_names_diagnose() {
    // an alias colliding with a declared type is a duplicate name
    let ds = diags_of(
        "struct S { v: i32 }\n\
         type S = i64;\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("duplicate type name `S`")),
        "{ds:?}"
    );
}

// ---- the type-name-law batch (phase 1): one name = one type ----------
//
// The alias-row form is repealed: the head admits no members (a
// generic head is a parse error again), the concrete-shadows-generic
// lift is gone from `declare_alias`, and the duplicate-type-name check
// is the plain symmetric four-way — the probed five-kind x both-orders
// matrix diagnoses uniformly.

#[test]
fn alias_head_params_are_a_parse_error_again() {
    // the row grammar's head seam reverts: `type Name<K, ..>` never
    // reaches the checker — the parser expects `=` after the name
    let ds = diags_of(
        "type HashMap<K, i64> = i64;\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("expected =") && d.contains("found `<`")),
        "a generic alias head is a parse error again: {ds:?}"
    );
    // the empty head died with the same seam
    let ds = diags_of(
        "type Foo<> = i64;\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("expected =")),
        "`<>` is no special grammar: {ds:?}"
    );
}

#[test]
fn one_name_one_decl_diagnoses_in_both_orders() {
    // alias-vs-class, both orders — the row-vs-class shape (silent in
    // the class-first order under the removed lift) cannot even parse
    // now; a PLAIN alias colliding with a class diagnoses both ways
    for src in [
        "class C { }\n type C = i64;\n entry fn main() -> i32 { return 0; }\n",
        "type C = i64;\n class C { }\n entry fn main() -> i32 { return 0; }\n",
    ] {
        let ds = diags_of(src);
        assert!(
            ds.iter().any(|d| d.contains("duplicate type name `C`")),
            "alias x class must diagnose both orders: {ds:?}"
        );
    }
    // alias-vs-generic-struct, both orders (the lifted leg-2 shape)
    for src in [
        "struct W<T> { v: T }\n type W = i64;\n entry fn main() -> i32 { return 0; }\n",
        "type W = i64;\n struct W<T> { v: T }\n entry fn main() -> i32 { return 0; }\n",
    ] {
        let ds = diags_of(src);
        assert!(
            ds.iter().any(|d| d.contains("duplicate type name `W`")),
            "alias x generic struct must diagnose both orders: {ds:?}"
        );
    }
    // alias-vs-alias (the row-vs-row family shape — two decls, one name)
    let ds = diags_of(
        "type A = i64;\n type A = str;\n entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("duplicate type name `A`")),
        "two aliases, one name: {ds:?}"
    );
    // alias-vs-enum and alias-vs-interface (the legs that never had a lift)
    let ds = diags_of("enum E { M }\n type E = i64;\n entry fn main() -> i32 { return 0; }\n");
    assert!(ds.iter().any(|d| d.contains("duplicate type name `E`")), "{ds:?}");
    let ds = diags_of("interface T { }\n type T = i64;\n entry fn main() -> i32 { return 0; }\n");
    assert!(ds.iter().any(|d| d.contains("duplicate type name `T`")), "{ds:?}");
}

#[test]
fn alias_target_resolves_at_declaration_used_or_not() {
    // the RHS hole is closed for good: the target head resolves in
    // pass 1b at the DECL — an unused alias with a nonsense target
    // diagnoses with nothing ever expanding it
    let ds = diags_of(
        "type Foo = NotAType;\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("unknown type `NotAType`")),
        "an unused nonsense alias target must diagnose at the decl: {ds:?}"
    );
    // and the arity of a parameterized target is checked at the decl
    // too (a plain alias takes no arguments)
    let ds = diags_of(
        "type Bar = Missing<i64>;\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("unknown type `Missing`")),
        "an unknown parameterized target head diagnoses at the decl: {ds:?}"
    );
}
