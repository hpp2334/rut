//! Generic class bounds — `class Box<K requires Hash, V>`
//!: the recorded bounds gate every instantiation
//! (union- and alias-aware, via the same `admit_bounds` helper the
//! fn/method grammar uses), and the bound is what proves the
//! parameter-value → interface-slot widening inside the class body. The
//! bound is admission-only — static dispatch on the bare parameter
//! stays deferred; frames compile per-instantiation, so body
//! calls on a parameter-typed value are the substituted concrete type's
//! own calls. Interface bounds admit by the structural member-set
//! match; an interface-typed instantiation satisfies nothing.

use rut_parser::Mode;


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

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // core bound as the one use: these tests exercise class
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
interface Hash { fn hash(self) -> u64; }
struct Token { v: i32 }
impl Token { pub fn hash(self) -> u64 { return 9; } }
class Box<K requires Hash, V> {
    k: K;
    v: V;
}
impl<K, V> Box<K, V> {
    pub fn new(k: K, v: V) -> Self {
        return Self { k: k, v: v };
    }
    pub fn key(self) -> K { return self.k; }
    pub fn val(self) -> V { return self.v; }
}
";
    // Token has the member — the instantiation compiles end to end
    let out = compile(&format!(
        "{shape}\
         entry fn main() -> i32 {{\n\
         \x20   let b: Box<Token, i32> = Box.new(Token {{ v: 1 }}, 2);\n\
         \x20   return b.val() + b.key().v;\n\
         }}\n"
    ));
    assert!(
        out.diags.is_empty(),
        "Token satisfies `K requires Hash`: {:?}",
        out.diags
    );

    // str has no members — the instantiation diagnoses, naming the member
    let ds = diags_of(&format!(
        "{shape}\
         entry fn main() -> i32 {{\n\
         \x20   let b: Box<str, i32> = Box.new(\"k\", 2);\n\
         \x20   return 0;\n\
         }}\n"
    ));
    assert!(
        ds.iter()
            .any(|d| d.contains("`str` does not satisfy `Hash`")
                && d.contains("no member `hash`")),
        "admission names the missing member: {ds:?}"
    );
}

#[test]
fn bound_proves_the_widening_inside_the_class_body() {
    // `let hk: Hash = self.k;` — the recorded bound admits the K-value →
    // interface-slot widening (Token is a ref-repr class, the only shape
    // that boxes); the call then dispatches through the slot
    let out = compile(
        "interface Hash { fn hash(self) -> u64; }\n\
         class Token { v: i32 }\n\
         impl Token {\n\
         \x20   pub fn new() -> Self { return Self { v: 1 }; }\n\
         \x20   pub fn hash(self) -> u64 { return 9; }\n\
         }\n\
         class Box<K requires Hash, V> {\n\
         \x20   k: K;\n\
         \x20   v: V;\n\
         }\n\
         impl<K, V> Box<K, V> {\n\
         \x20   pub fn new(k: K, v: V) -> Self {\n\
         \x20       return Self { k: k, v: v };\n\
         \x20   }\n\
         \x20   pub fn key_slot(self) -> Hash {\n\
         \x20       let hk: Hash = self.k;\n\
         \x20       return hk;\n\
         \x20   }\n\
         }\n\
         entry fn main() -> i32 {\n\
         \x20   let b: Box<Token, i32> = Box.new(Token.new(), 2);\n\
         \x20   return b.key_slot().hash() as i32;\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "widening inside the body: {:?}", out.diags);
}

#[test]
fn class_bound_expands_through_an_alias() {
    // `type Key = Hash;` — the bound member expands before interface-vs-
    // concrete detection, so the alias-spelled bound
    // admits the member-carrying key and rejects the rest
    let shape = "\
interface Hash { fn hash(self) -> u64; }
type Key = Hash;
struct Token { v: i32 }
impl Token { pub fn hash(self) -> u64 { return 9; } }
class Box<K requires Key, V> {
    k: K;
    v: V;
}
impl<K, V> Box<K, V> {
    pub fn new(k: K, v: V) -> Self {
        return Self { k: k, v: v };
    }
}
";
    let out = compile(&format!(
        "{shape}\
         entry fn main() -> i32 {{\n\
         \x20   let b: Box<Token, i32> = Box.new(Token {{ v: 1 }}, 2);\n\
         \x20   return b.v;\n\
         }}\n"
    ));
    assert!(out.diags.is_empty(), "the alias bound admits Token: {:?}", out.diags);

    let ds = diags_of(&format!(
        "{shape}\
         entry fn main() -> i32 {{\n\
         \x20   let b: Box<bool, i32> = Box.new(true, 2);\n\
         \x20   return 0;\n\
         }}\n"
    ));
    assert!(
        ds.iter().any(|d| d.contains("`bool` does not satisfy")),
        "the alias-expanded bound still gates: {ds:?}"
    );
}

#[test]
fn class_union_bound_admits_each_member() {
    // a union bound spans members — either instantiates, the rest don't
    let shape = "\
interface Hash { fn hash(self) -> u64; }
struct Token { v: i32 }
impl Token { pub fn hash(self) -> u64 { return 9; } }
class Box<K requires i32 | Token, V> {
    k: K;
    v: V;
}
impl<K, V> Box<K, V> {
    pub fn new(k: K, v: V) -> Self {
        return Self { k: k, v: v };
    }
}
";
    let out = compile(&format!(
        "{shape}\
         entry fn main() -> i32 {{\n\
         \x20   let a: Box<i32, str> = Box.new(1, \"x\");\n\
         \x20   let b: Box<Token, str> = Box.new(Token {{ v: 2 }}, \"y\");\n\
         \x20   return a.k + b.k.v;\n\
         }}\n"
    ));
    assert!(out.diags.is_empty(), "both union members satisfy: {:?}", out.diags);

    let ds = diags_of(&format!(
        "{shape}\
         entry fn main() -> i32 {{\n\
         \x20   let b: Box<bool, str> = Box.new(true, \"x\");\n\
         \x20   return 0;\n\
         }}\n"
    ));
    assert!(
        ds.iter().any(|d| d.contains("does not satisfy `K` requires `i32 | Token`")),
        "the failure names the union: {ds:?}"
    );
}

// NOTE: the full cross-module shape (bounded class in a lib module, key
// spelled in the consumer) is out of A5 scope — used (imported) types
// take no generic arguments in this build (resolve.rs, pre-existing v1
// rule). The bound gates through the same structural member-set match
// either way.
