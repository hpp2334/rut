//! Type-union bounds end to end.
//!
//! Three behaviors the phase pins down:
//! - **type unions only** — a union bound takes type NAMES; an interface
//!   member inside a union (all-interface or mixed) diagnoses, while a
//!   single-interface bound (`K requires Enc`) stays
//!   legal;
//! - **admission per member** — each named member instantiates; a
//!   record and a user class do not, and the failure names the type and
//!   the bound;
//! - **capability resolution through the union** — a method call on a
//!   union-bounded `K` (param, annotated let, a copy, or `self.f`) is
//!   written against the WHOLE bound: every member must provide the
//!   method (its own member surface — the prims' engine `builtin impl`s
//!   included), so the generic body typechecks the same way
//!   for every instantiation; each instantiation still runs the
//!   concrete member's own member (different members → different
//!   engines).

use rut_driver::{Module, ModuleBody, Session};

fn compile_app(src: &str) -> rut_driver::GraphOutput {
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module("app", Module { body: ModuleBody::Source { text: src.into(), is_decl: false }, ..Default::default() }).unwrap();
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

// ---- interfaces admit through a union bound --------------------------
// (the law FLIPPED with the structural fork: a union bound's interface
// members are legal — any-member-suffices, each admitted by the
// structural member-set match; a concrete member admits by TypeId
// membership)

#[test]
fn iface_union_admits_a_satisfying_type_and_fails_outside() {
    // a type carrying `enc` admits through the Enc leg; `str` carries no
    // members and names the whole bound
    let out = compile_app(
        "interface Enc { fn enc(self) -> bytes; }\n\
         interface Shade { fn shade(self) -> i32; }\n\
         struct Token { v: i32 }\n\
         impl Token { pub fn enc(self) -> bytes { return bytes(0); } }\n\
         fn tag<K requires Enc | Shade>(k: K) -> i64 { return 0; }\n\
         entry fn probe() -> i64 { let t: Token = Token { v: 1 }; return tag(t); }\n",
    );
    assert!(out.diags.is_empty(), "an Enc-satisfying type admits: {:?}", out.diags);

    let ds = diags_of(
        "interface Enc { fn enc(self) -> bytes; }\n\
         interface Shade { fn shade(self) -> i32; }\n\
         fn tag<K requires Enc | Shade>(k: K) -> i64 { return 0; }\n\
         entry fn probe() -> i64 { return tag(\"s\"); }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`str` does not satisfy `K` requires `Enc | Shade`")),
        "the failure names the whole bound: {ds:?}"
    );
}

#[test]
fn mixed_union_admits_concrete_and_iface_members() {
    // the union spans a concrete member and an interface member: i32
    // admits by id, a member-carrying type admits by the member-set
    // match
    let out = compile_app(
        "interface Enc { fn enc(self) -> bytes; }\n\
         struct Token { v: i32 }\n\
         impl Token { pub fn enc(self) -> bytes { return bytes(0); } }\n\
         fn tag<K requires i32 | Enc>(k: K) -> i64 { return 0; }\n\
         entry fn probe() -> i64 {\n\
             let a = tag(5);\n\
             let t: Token = Token { v: 1 };\n\
             let b = tag(t);\n\
             return a + b;\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "both members admit: {:?}", out.diags);

    // bool is neither
    let ds = diags_of(
        "interface Enc { fn enc(self) -> bytes; }\n\
         struct Token { v: i32 }\n\
         impl Token { pub fn enc(self) -> bytes { return bytes(0); } }\n\
         fn tag<K requires i32 | Enc>(k: K) -> i64 { return 0; }\n\
         entry fn probe() -> i64 { return tag(true); }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`bool` does not satisfy `K` requires `i32 | Enc`")),
        "the failure names the union: {ds:?}"
    );
}

#[test]
fn iface_union_through_an_alias_admits() {
    // the union alias carries the interface leg fine as a BOUND; the
    // admission is the same any-member-suffices match
    let out = compile_app(
        "interface Enc { fn enc(self) -> bytes; }\n\
         struct Token { v: i32 }\n\
         impl Token { pub fn enc(self) -> bytes { return bytes(0); } }\n\
         type Mix = i32 | Enc;\n\
         fn tag<K requires Mix>(k: K) -> i64 { return 0; }\n\
         entry fn probe() -> i64 {\n\
             let a = tag(5);\n\
             let t: Token = Token { v: 1 };\n\
             return a + tag(t);\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "the alias-spelled union admits both: {:?}", out.diags);
}

#[test]
fn single_iface_bound_stays_legal() {
    // the regression guard: `K requires Enc` (one interface, no union) is
    // the single-interface law — admission via the structural member-set
    // match
    let out = compile_app(
        "interface Enc { fn enc(self) -> bytes; }\n\
         struct Token { v: i32 }\n\
         impl Token { pub fn enc(self) -> bytes { return bytes(0); } }\n\
         fn seal<T requires Enc>(x: T) -> i64 { return 0; }\n\
         entry fn probe() -> i64 { let t: Token = Token { v: 1 }; return seal(t); }\n",
    );
    assert!(out.diags.is_empty(), "a single-interface bound admits: {:?}", out.diags);
}

// ---- capability resolution through the union ----

// The prim-impl fixtures died with the registry (a primitive carries no
// user members). The capability law rides the prims' own engine surface:
// `wrapping_mul` is a `builtin impl` member on the integer prims, so an
// i32|i64 bound provides it and a bool member does not.
#[test]
fn method_call_resolves_when_every_member_has_the_method() {
    // every member provides `wrapping_mul` (its engine `builtin impl`),
    // so the body against `K` typechecks and each instantiation runs its
    // own member
    let out = run_entry(
        "entry fn dispatch() -> i64 {\
             let a = lane_of(5);\n\
             let b = lane_of(6i64);\n\
             let c = copy_of(7);\n\
         return ((a + b) * 10 + c) as i64;\n\
         }\n\
         fn lane_of<K requires i32 | i64>(k: K) -> i64 { return k.wrapping_mul(3) as i64; }\n\
         fn copy_of<K requires i32 | i64>(k: K) -> i64 { let kk = k; return kk.wrapping_mul(3) as i64; }\n",
        "dispatch",
    );
    // lane_of(5) → 15 (i32's member), lane_of(6i64) → 18 (i64's),
    // copy → 21: distinct members ran their distinct members at run time
    assert_eq!(out, (15 + 18) * 10 + 21);
}

#[test]
fn method_call_diagnoses_when_a_member_lacks_the_method() {
    // `bool` carries no `wrapping_mul` member — the union admits `bool`
    // values, so a body calling `k.wrapping_mul(..)` violates the
    // whole-bound contract and the instantiation diagnoses at compile
    // time (naming the member and the allowed set), member-instantiated
    // or not
    let ds = diags_of(
        "fn lane_of<K requires i32 | bool>(k: K) -> i64 { return k.wrapping_mul(3) as i64; }\n\
         entry fn probe() -> i64 { let a = lane_of(5); return a; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`bool` does not provide `wrapping_mul`")
            && d.contains("`K` requires `i32 | bool`")
            && d.contains("every member")),
        "the capability failure names the member and the allowed set: {ds:?}"
    );
}

#[test]
fn class_body_method_call_through_the_union_bound() {
    // a class field typed by a union-bounded generic,
    // a method call through `self.field` inside the class body
    let src = "\
         class Box<K requires i32 | i64, V> {\n\
         \x20   k: K;\n\
         \x20   v: V;\n\
         }\n\
         impl<K, V> Box<K, V> {\n\
         \x20   pub fn new(k: K, v: V) -> Self { return Self { k: k, v: v }; }\n\
         \x20   pub fn go(self) -> i32 { let kk = self.k; return kk.wrapping_mul(10) as i32; }\n\
         }\n";

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
    let ds = diags_of(
        "class Box<K requires i32 | bool, V> {\n\
         \x20   k: K;\n\
         \x20   v: V;\n\
         }\n\
         impl<K, V> Box<K, V> {\n\
         \x20   pub fn new(k: K, v: V) -> Self { return Self { k: k, v: v }; }\n\
         \x20   pub fn go(self) -> i32 { let kk = self.k; return kk.wrapping_mul(10) as i32; }\n\
         }\n\
         entry fn probe() -> i64 {\
             let b: Box<i32, str> = Box.new(5, \"x\");\n\
             return b.go() as i64;\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`bool` does not provide `wrapping_mul`")
            && d.contains("`K` requires `i32 | bool`")),
        "the class-body capability failure names the member and the bound: {ds:?}"
    );
}
