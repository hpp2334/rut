//! Generic class bounds — `class HashMap<K requires Hashable, V>`
//! (RFC 0043 §A5): the recorded bounds gate every instantiation
//! (union- and alias-aware, via the same `admit_bounds` helper the
//! fn/method grammar uses), and the bound is what proves the
//! parameter-value → trait-slot widening inside the class body. The
//! bound is admission-only — static dispatch on the bare parameter
//! stays deferred (OQ-1); frames compile per-instantiation, so body
//! calls on a parameter-typed value are the substituted concrete type's
//! own calls.

use rut_parser::Mode;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // core bound as the one use (RFC 0028): these tests exercise class
    // bound semantics, not use discipline
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
fn bound_admits_and_rejects_at_the_class_instantiation() {
    let shape = "\
trait Hash { fn hash(self) -> u64; }
struct Token { v: i32 }
impl Hash for Token { fn hash(self) -> u64 { return 9; } }
class Box<K requires Hash, V> {
    k: K;
    v: V;
}
impl Box<K, V> {
    pub fn new(k: K, v: V) -> Self {
        return Self { k: k, v: v };
    }
    pub fn key(self) -> K { return self.k; }
    pub fn val(self) -> V { return self.v; }
}
";
    // Token implements Hash — the instantiation compiles end to end
    let out = compile(&format!(
        "{shape}\
         fn main() -> i32 {{\n\
         \x20   let b: Box<Token, i32> = Box.new(Token {{ v: 1 }}, 2);\n\
         \x20   return b.val() + b.key().v;\n\
         }}\n"
    ));
    assert!(
        out.diags.is_empty(),
        "Token satisfies `K requires Hash`: {:?}",
        out.diags
    );

    // str has no Hash impl — the instantiation diagnoses, naming the impl
    let ds = diags_of(&format!(
        "{shape}\
         fn main() -> i32 {{\n\
         \x20   let b: Box<str, i32> = Box.new(\"k\", 2);\n\
         \x20   return 0;\n\
         }}\n"
    ));
    assert!(
        ds.iter()
            .any(|d| d.contains("`str` does not satisfy `K` requires `Hash`")
                && d.contains("no impl `Hash` for `str` is registered")),
        "admission names the missing impl: {ds:?}"
    );
}

#[test]
fn bound_proves_the_widening_inside_the_class_body() {
    // `let hk: Hash = self.k;` — the recorded bound admits the K-value →
    // Hash-slot widening; the call then dispatches through the slot
    let out = compile(
        "trait Hash { fn hash(self) -> u64; }\n\
         struct Token { v: i32 }\n\
         impl Hash for Token { fn hash(self) -> u64 { return 9; } }\n\
         class Box<K requires Hash, V> {\n\
         \x20   k: K;\n\
         \x20   v: V;\n\
         }\n\
         impl Box<K, V> {\n\
         \x20   pub fn new(k: K, v: V) -> Self {\n\
         \x20       return Self { k: k, v: v };\n\
         \x20   }\n\
         \x20   pub fn key_slot(self) -> Hash {\n\
         \x20       let hk: Hash = self.k;\n\
         \x20       return hk;\n\
         \x20   }\n\
         }\n\
         fn main() -> i32 {\n\
         \x20   let b: Box<Token, i32> = Box.new(Token { v: 1 }, 2);\n\
         \x20   return b.key_slot().hash() as i32;\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "widening inside the body: {:?}", out.diags);
}

#[test]
fn class_bound_expands_through_an_alias() {
    // `type Key = Hash;` — the bound member expands before trait-vs-
    // concrete detection (RFC 0043 §A0.2), so the alias-spelled bound
    // admits the impl-registered key and rejects the rest
    let shape = "\
trait Hash { fn hash(self) -> u64; }
type Key = Hash;
struct Token { v: i32 }
impl Hash for Token { fn hash(self) -> u64 { return 9; } }
class Box<K requires Key, V> {
    k: K;
    v: V;
}
impl Box<K, V> {
    pub fn new(k: K, v: V) -> Self {
        return Self { k: k, v: v };
    }
}
";
    let out = compile(&format!(
        "{shape}\
         fn main() -> i32 {{\n\
         \x20   let b: Box<Token, i32> = Box.new(Token {{ v: 1 }}, 2);\n\
         \x20   return b.v;\n\
         }}\n"
    ));
    assert!(out.diags.is_empty(), "the alias bound admits Token: {:?}", out.diags);

    let ds = diags_of(&format!(
        "{shape}\
         fn main() -> i32 {{\n\
         \x20   let b: Box<bool, i32> = Box.new(true, 2);\n\
         \x20   return 0;\n\
         }}\n"
    ));
    assert!(
        ds.iter().any(|d| d.contains("`bool` does not satisfy `K` requires")),
        "the alias-expanded bound still gates: {ds:?}"
    );
}

#[test]
fn class_union_bound_admits_each_member() {
    // a union bound spans members — either instantiates, the rest don't
    let shape = "\
trait Hash { fn hash(self) -> u64; }
struct Token { v: i32 }
impl Hash for Token { fn hash(self) -> u64 { return 9; } }
class Box<K requires i32 | Token, V> {
    k: K;
    v: V;
}
impl Box<K, V> {
    pub fn new(k: K, v: V) -> Self {
        return Self { k: k, v: v };
    }
}
";
    let out = compile(&format!(
        "{shape}\
         fn main() -> i32 {{\n\
         \x20   let a: Box<i32, str> = Box.new(1, \"x\");\n\
         \x20   let b: Box<Token, str> = Box.new(Token {{ v: 2 }}, \"y\");\n\
         \x20   return a.k + b.k.v;\n\
         }}\n"
    ));
    assert!(out.diags.is_empty(), "both union members satisfy: {:?}", out.diags);

    let ds = diags_of(&format!(
        "{shape}\
         fn main() -> i32 {{\n\
         \x20   let b: Box<bool, str> = Box.new(true, \"x\");\n\
         \x20   return 0;\n\
         }}\n"
    ));
    assert!(
        ds.iter().any(|d| d.contains("does not satisfy `K` requires `i32 | Token`")),
        "the failure names the union: {ds:?}"
    );
}

// NOTE: the full cross-module shape (bounded class in a lib module, key +
// impl in the consumer) is out of A5 scope — used (imported) types take no
// generic arguments in this build (resolve.rs, pre-existing v1 rule). The
// bound gates through the same `find_impl_ex` registry either way.

