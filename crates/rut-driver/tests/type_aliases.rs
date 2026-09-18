//! Type aliases, union bounds, and inline `requires` end to end
//! (RFC 0043): transparency through params/fields/chains, the
//! recursion and bound-only diagnostics, admission at the call site
//! (concrete by id, traits via the impl registry, trait objects satisfy
//! nothing), and cross-module `pub type` export.

use rut_parser::Mode;

use rut_core::types::TY_I64;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // core bound as the one use (RFC 0028): these tests exercise alias
    // and bound semantics, not use discipline
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core())],
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
         fn main() -> i32 {\n\
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
         fn main() -> i32 { let v: A = 7; return f(v) as i32; }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn forward_referenced_alias_is_legal() {
    // pass 1b validates targets after pass 1a declared every name
    let out = compile(
        "fn f(x: Later) -> Later { return x; }\n\
         type Later = i64;\n\
         fn main() -> i32 { let v: Later = 3; return f(v) as i32; }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn recursive_alias_diagnoses() {
    let ds = diags_of(
        "type A = B;\n\
         type B = A;\n\
         fn main() -> i32 { return 0; }\n",
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
         fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("recursive type alias")),
        "the self-cycle must diagnose: {ds:?}"
    );
}

#[test]
fn unions_are_bound_only() {
    // a union alias in a value position errors (`pub` so the signature
    // is compiled — the library surface, RFC 0029)
    let ds = diags_of(
        "type Num = i32 | str;\n\
         pub fn f(x: Num) -> i32 { return 0; }\n\
         fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("bound-only")),
        "union alias in a value position must diagnose: {ds:?}"
    );
    // so does an inline union type — a param list is a value position,
    // so `|` is not even valid type syntax there
    let ds = diags_of(
        "pub fn g(x: i32 | str) -> i32 { return 0; }\n\
         fn main() -> i32 { return 0; }\n",
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
fn main() -> i32 {\n\
    let a = tag(5);\n\
    let b = tag(\"s\");\n\
    return a + b;\n\
}\n\
";
    let out = compile(src);
    assert!(out.diags.is_empty(), "i32 and str both satisfy: {:?}", out.diags);

    let ds = diags_of(
        "fn tag<T requires i32 | str>(x: T) -> i32 { return 0; }\n\
         fn main() -> i32 { return tag(true); }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("does not satisfy") && d.contains("i32 | str")),
        "the failure names the bound: {ds:?}"
    );
}

#[test]
fn trait_member_bound_via_the_impl_registry() {
    let src = "\
trait Enc { fn enc(self) -> bytes; }\n\
struct Token { v: i32 }\n\
impl Enc for Token { fn enc(self) -> bytes { return bytes(0); } }\n\
fn seal<T requires Enc>(x: T) -> i32 {\n\
    let e: Enc = x;\n\
    return e.enc().len();\n\
}\n\
fn main() -> i32 { let t: Token = Token { v: 1 }; return seal(t); }\n\
";
    let out = compile(src);
    assert!(out.diags.is_empty(), "Token satisfies Enc: {:?}", out.diags);

    // str has no `Enc` impl — the bound names it
    let ds = diags_of(
        "trait Enc { fn enc(self) -> bytes; }\n\
         struct Token { v: i32 }\n\
         impl Enc for Token { fn enc(self) -> bytes { return bytes(0); } }\n\
         fn seal<T requires Enc>(x: T) -> i32 { return 0; }\n\
         fn main() -> i32 { return seal(\"nope\"); }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`str` does not satisfy") && d.contains("Enc")),
        "no impl Enc for str: {ds:?}"
    );
}

#[test]
fn trait_object_satisfies_nothing() {
    // RFC 0013 §2: a trait-object instantiation satisfies no bound —
    // only a concrete type with a registered impl does
    let ds = diags_of(
        "trait Enc { fn enc(self) -> bytes; }\n\
         struct Token { v: i32 }\n\
         impl Enc for Token { fn enc(self) -> bytes { return bytes(0); } }\n\
         fn seal<T requires Enc>(x: T) -> i32 { return 0; }\n\
         fn main() -> i32 {\n\
             let w: Enc = Token { v: 1 };\n\
             return seal(w);\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("does not satisfy")),
        "a trait object satisfies nothing: {ds:?}"
    );
}

#[test]
fn bound_may_reference_another_generic() {
    // `U requires Array<T>` — resolved under the call-site substitution
    let out = compile(
        "use core::{ Array };\n\
         fn hold<T, U requires Array<T>>(x: U) -> i32 { return 0; }\n\
         fn main() -> i32 {\n\
             let a = Array<i32>(2);\n\
             return hold<i32, Array<i32>>(a);\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);

    // U = Array<str> does not satisfy `U requires Array<i32>`
    let ds = diags_of(
        "use core::{ Array };\n\
         fn hold<T, U requires Array<T>>(x: U) -> i32 { return 0; }\n\
         fn main() -> i32 {\n\
             let s = Array<str>(2);\n\
             return hold<i32, Array<str>>(s);\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("does not satisfy")),
        "Array<str> ≠ Array<i32>: {ds:?}"
    );
}

#[test]
fn method_bounds_parse_and_collect() {
    // a bounded method parses, collects, and compiles when uncalled —
    // the bound rides the method declaration (RFC 0043)
    let out = compile(
        "trait Enc { fn enc(self) -> bytes; }\n\
         struct Token { v: i32 }\n\
         impl Enc for Token { fn enc(self) -> bytes { return bytes(0); } }\n\
         class W { v: i32; }\n\
         impl W {\n\
             fn new() -> Self { return Self { v: 0 }; }\n\
             fn wrap<T requires Enc>(self, x: T) -> i32 { return 0; }\n\
         }\n\
         fn main() -> i32 { let w: W = W.new(); return w.v; }\n",
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
         fn main() -> i32 {\n\
             let d: Meters = 9;\n\
             return m(d) as i32;\n\
         }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, surface), (3, rut_core::binary::Surface::core())],
    );
    assert!(root.diags.is_empty(), "cross-module transparency: {:?}", root.diags);

    // ... but the union alias's name never crossed — using it diagnoses
    let ds = rut_driver::compile_program(
        "use units::{ Mix };\nfn main() -> i32 { let x: Mix = 0; return 0; }\n",
        Mode::Impl,
        "app",
        2,
        &[(1, {
            let dep = rut_driver::compile_program(
                "type Mix = i32 | str;\nfn main() -> i32 { return 0; }\n",
                Mode::Impl,
                "units",
                1,
                &[],
            );
            dep.program.expect("dep").surface.clone()
        })],
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
         fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("duplicate type name `S`")),
        "{ds:?}"
    );
}
