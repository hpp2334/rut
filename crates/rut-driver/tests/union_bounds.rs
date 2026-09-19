//! Type-union bounds (RFC 0043 §3, native-fastpath phase 1) end to end.
//!
//! Three behaviors the phase pins down:
//! - **type unions only** — a union bound takes type NAMES; a trait
//!   member inside a union (all-trait or mixed) diagnoses, while a
//!   single-trait bound (`K requires Hashable`, mapset's law) stays
//!   legal;
//! - **admission per member** — each named member instantiates; a
//!   record and a user class do not, and the failure names the type and
//!   the bound;
//! - **capability resolution through the union** — a method call on a
//!   union-bounded `K` (param, annotated let, a copy, or `self.f`) is
//!   written against the WHOLE bound: every member must provide the
//!   method (via its impls), so the generic body typechecks the same way
//!   for every instantiation; each instantiation still dispatches the
//!   concrete member's own impl (different members → different impls).

use rut_driver::{Module, Session};

fn compile_app(src: &str) -> rut_driver::GraphOutput {
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module("app", Module { source: Some(src.into()), ..Default::default() }).unwrap();
    rut_driver::compile_graph(&s, "app")
}

fn diags_of(src: &str) -> Vec<String> {
    compile_app(src)
        .diags
        .iter()
        .map(|d| d.msg.clone())
        .collect()
}

/// Compile, flatten, verify, and run `entry` — its i64 return.
fn run_entry(src: &str, entry: &str) -> i64 {
    let out = compile_app(src);
    assert!(
        out.diags.is_empty(),
        "unexpected diags:\n{}",
        out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(20_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    vm.call::<_, i64>(entry, ()).expect("run")
}

// ---- admission: accept every member, reject a record and a class ----

const MULTI_UNION: &str = "fn take<K requires i32 | i64 | bool>(k: K) -> i64 { return 0; }\n";

#[test]
fn union_bound_admits_each_member_type() {
    let out = compile_app(&format!(
        "{MULTI_UNION}\
         entry fn probe() -> i64 {{\
             let a = take(5);\n\
             let b = take(6i64);\n\
             let c = take(true);\n\
             return a + b + c;\n\
         }}\n"
    ));
    assert!(out.diags.is_empty(), "every named member admits: {:?}", out.diags);
}

#[test]
fn union_bound_rejects_record_and_user_class_naming_the_bound() {
    // a record type is not a member
    let ds = diags_of(&format!(
        "{MULTI_UNION}\
         struct Pt {{ x: i32; }}\n\
         entry fn probe() -> i64 {{ return take(Pt {{ x: 1 }}); }}\n"
    ));
    assert!(
        ds.iter().any(|d| d.contains("`Pt` does not satisfy `K` requires `i32 | i64 | bool`")),
        "the reject names the record and the bound: {ds:?}"
    );

    // a user class is not a member either
    let ds = diags_of(&format!(
        "{MULTI_UNION}\
         class Tok {{ v: i32; }}\n\
         impl Tok {{ pub fn new() -> Self {{ return Self {{ v: 1 }}; }} }}\n\
         entry fn probe() -> i64 {{ let t: Tok = Tok.new(); return take(t); }}\n"
    ));
    assert!(
        ds.iter().any(|d| d.contains("`Tok` does not satisfy `K` requires `i32 | i64 | bool`")),
        "the reject names the class and the bound: {ds:?}"
    );
}

// ---- type unions only: traits cannot sit in a union bound ----

#[test]
fn all_trait_union_bound_diagnoses() {
    let ds = diags_of(
        "trait Enc { fn enc(self) -> bytes; }\n\
         trait Shade { fn shade(self) -> i32; }\n\
         fn tag<K requires Enc | Shade>(k: K) -> i64 { return 0; }\n\
         entry fn probe() -> i64 { return tag(\"s\"); }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`Enc` is a trait — a union bound takes type names only")),
        "an all-trait union diagnoses: {ds:?}"
    );
    assert!(
        ds.iter().any(|d| d.contains("`Shade` is a trait — a union bound takes type names only")),
        "every trait member diagnoses: {ds:?}"
    );
}

#[test]
fn mixed_union_bound_diagnoses_the_trait_member() {
    let ds = diags_of(
        "trait Enc { fn enc(self) -> bytes; }\n\
         struct Token { v: i32 }\n\
         impl Enc for Token { fn enc(self) -> bytes { return bytes(0); } }\n\
         fn tag<K requires i32 | Enc>(k: K) -> i64 { return 0; }\n\
         entry fn probe() -> i64 { return tag(5); }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`Enc` is a trait — a union bound takes type names only")),
        "the trait member of a mixed union diagnoses: {ds:?}"
    );
}

#[test]
fn trait_union_through_an_alias_diagnoses_in_bound_position() {
    // the union alias stays legal as an alias; spelling it as a BOUND
    // is trait-in-union
    let ds = diags_of(
        "trait Enc { fn enc(self) -> bytes; }\n\
         type Mix = i32 | Enc;\n\
         fn tag<K requires Mix>(k: K) -> i64 { return 0; }\n\
         entry fn probe() -> i64 { return tag(5); }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`Enc` is a trait — a union bound takes type names only")),
        "a union alias bound with a trait member diagnoses: {ds:?}"
    );
}

#[test]
fn single_trait_bound_stays_legal() {
    // the regression guard: `K requires Enc` (one trait, no union) is the
    // mapset law — admission via the impl registry, untouched
    let out = compile_app(
        "trait Enc { fn enc(self) -> bytes; }\n\
         struct Token { v: i32 }\n\
         impl Enc for Token { fn enc(self) -> bytes { return bytes(0); } }\n\
         fn seal<T requires Enc>(x: T) -> i64 { return 0; }\n\
         entry fn probe() -> i64 { let t: Token = Token { v: 1 }; return seal(t); }\n",
    );
    assert!(out.diags.is_empty(), "a single-trait bound admits: {:?}", out.diags);
}

// ---- capability resolution through the union ----

const LANE_PRELUDE: &str = "\
trait Lane { fn lane(self) -> i32; }\n\
impl Lane for i32 { fn lane(self) -> i32 { return 7; } }\n\
impl Lane for i64 { fn lane(self) -> i32 { return 13; } }\n";

#[test]
fn method_call_resolves_when_every_member_has_the_method() {
    // the scratch trait IS the phase-1 fixture (the real private KeyLane
    // arrives in phase 3): every member implements `Lane`, so the body
    // against `K` typechecks and dispatches per member
    let out = run_entry(
        &format!(
            "{LANE_PRELUDE}\
             entry fn dispatch() -> i64 {{\
                 let a = lane_of(5);\n\
                 let b = lane_of(6i64);\n\
                 let c = copy_of(7);\n\
             return ((a + b) * 10 + c) as i64;\n\
             }}\n\
             fn lane_of<K requires i32 | i64>(k: K) -> i32 {{ return k.lane(); }}\n\
             fn copy_of<K requires i32 | i64>(k: K) -> i32 {{ let kk = k; return kk.lane(); }}\n"
        ),
        "dispatch",
    );
    // lane_of(5) → 7 (i32's impl), lane_of(6i64) → 13 (i64's), copy → 7:
    // distinct members dispatched their distinct impls at run time
    assert_eq!(out, ((7 + 13) as i64) * 10 + 7);
}

#[test]
fn method_call_diagnoses_when_a_member_lacks_the_method() {
    // `bool` carries no `Lane` impl — the union admits `bool` values, so
    // a body calling `k.lane()` violates the whole-bound contract and the
    // instantiation diagnoses at compile time (naming the member and the
    // allowed set), member-instantiated or not
    let ds = diags_of(&format!(
        "{LANE_PRELUDE}\
         fn lane_of<K requires i32 | bool>(k: K) -> i32 {{ return k.lane(); }}\n\
         entry fn probe() -> i64 {{ let a = lane_of(5); return a as i64; }}\n"
    ));
    assert!(
        ds.iter().any(|d| d.contains("`bool` does not provide `lane`")
            && d.contains("`K` requires `i32 | bool`")
            && d.contains("every member")),
        "the capability failure names the member and the allowed set: {ds:?}"
    );
}

#[test]
fn class_body_method_call_through_the_union_bound() {
    // the phase-3 shape: a class field typed by a union-bounded generic,
    // a method call through `self.field` inside the class body
    let src = format!(
        "{LANE_PRELUDE}\
         class Box<K requires i32 | i64, V> {{\n\
         \x20   k: K;\n\
         \x20   v: V;\n\
         }}\n\
         impl Box<K, V> {{\n\
         \x20   pub fn new(k: K, v: V) -> Self {{ return Self {{ k: k, v: v }}; }}\n\
         \x20   pub fn go(self) -> i32 {{ let kk = self.k; return kk.lane() * 10; }}\n\
         }}\n"
    );

    let full = format!(
        "{src}\
         entry fn check() -> i64 {{\
             let a: Box<i32, str> = Box.new(5, \"x\");\n\
             let b: Box<i64, str> = Box.new(6i64, \"y\");\n\
             return (a.go() as i64) + (b.go() as i64);\n\
         }}\n"
    );
    let out = compile_app(&full);
    assert!(out.diags.is_empty(), "both members satisfy the class bound: {:?}", out.diags);

    // a member WITHOUT the capability inside a class union bound
    let ds = diags_of(&format!(
        "{LANE_PRELUDE}\
         class Box<K requires i32 | bool, V> {{\n\
         \x20   k: K;\n\
         \x20   v: V;\n\
         }}\n\
         impl Box<K, V> {{\n\
         \x20   pub fn new(k: K, v: V) -> Self {{ return Self {{ k: k, v: v }}; }}\n\
         \x20   pub fn go(self) -> i32 {{ let kk = self.k; return kk.lane(); }}\n\
         }}\n\
         entry fn probe() -> i64 {{\
             let b: Box<i32, str> = Box.new(5, \"x\");\n\
             return b.go() as i64;\n\
         }}\n"
    ));
    assert!(
        ds.iter().any(|d| d.contains("`bool` does not provide `lane`")
            && d.contains("`K` requires `i32 | bool`")),
        "the class-body capability failure names the member and the bound: {ds:?}"
    );
}
