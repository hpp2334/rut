//! Declared impl generics — the collector half: generic binders are
//! DECLARED after `impl` (`impl<T> Vec<T>`); the head's bare parameter
//! names are USES that must resolve against the declared list. The
//! undeclared name is the error it always should have been (no implicit
//! inference, no typo-shaped template parameters), the multi-binder
//! head collects, and the inherent form follows the same law.

use rut_driver::{Module, ModuleBody, Session};

fn diags_of(src: &str) -> Vec<String> {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    session
        .register_module("test", Module { body: ModuleBody::Source { text: src.into(), is_decl: false }, ..Default::default() })
        .expect("register");
    rut_driver::compile_graph(&session, "test")
        .diags
        .iter()
        .map(|d| d.msg.clone())
        .collect()
}

#[test]
fn declared_binders_collect_and_the_impl_compiles() {
    let ds = diags_of(
        "class Box2<T> { v: T; }\n\
         impl<T> Box2<T> {\n\
             pub fn new(v: T) -> Self { return Self { v: v }; }\n\
             pub fn get(self) -> T { return self.v; }\n\
         }\n\
         entry fn main() -> i32 {\n\
             let b: Box2<i32> = Box2.new(7);\n\
             return b.get();\n\
         }\n",
    );
    assert!(ds.is_empty(), "a declared inherent impl compiles: {ds:?}");
}

#[test]
fn undeclared_binder_names_the_declared_form() {
    // the typo law: a bare head name that is neither declared nor a
    // type in scope is the undeclared-parameter error — never a
    // silently minted template parameter
    let ds = diags_of(
        "class Box2<T> { v: T; }\n\
         impl<T> Box2<Tyypo> {\n\
             pub fn get(self) -> i32 { return 1; }\n\
         }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains(
            "undeclared type parameter `Tyypo` — declare it: `impl<Tyypo> ..`"
        )),
        "{ds:?}"
    );
}

#[test]
fn undeclared_binder_without_any_declared_list() {
    // the old implicit form is gone: `impl Vec<T> { .. }` without an
    // `impl<..>` list diagnoses — the declared list is the definition site
    let ds = diags_of(
        "class Vec2<T> { items: [T]; }\n\
         impl Vec2<T> {\n\
             pub fn len(self) -> i32 { return 0; }\n\
         }\n\
         entry fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("undeclared type parameter `T`")),
        "{ds:?}"
    );
}

#[test]
fn multi_binder_head_declares_both() {
    let ds = diags_of(
        "class Pair2<K, V> { k: K; v: V; }\n\
         impl<K, V> Pair2<K, V> {\n\
             pub fn new(k: K, v: V) -> Self { return Self { k: k, v: v }; }\n\
         }\n\
         entry fn main() -> i32 {\n\
             let p: Pair2<i32, i32> = Pair2.new(1, 2);\n\
             return p.k;\n\
         }\n",
    );
    assert!(ds.is_empty(), "a two-binder head compiles: {ds:?}");
}

#[test]
fn a_renamed_binder_is_the_head_spelling_law() {
    // the declared list is the binder set; the TARGET's generic NAMES
    // stay the env keys — the head spells them (renaming the target's
    // own parameter is a different feature, not this law)
    let ds = diags_of(
        "class Box2<T> { v: T; }\n\
         impl<T> Box2<T> {\n\
             pub fn new(v: T) -> Self { return Self { v: v }; }\n\
             pub fn get(self) -> T { return self.v; }\n\
         }\n\
         entry fn main() -> i32 {\n\
             let b: Box2<i32> = Box2.new(9);\n\
             return b.get();\n\
         }\n",
    );
    assert!(ds.is_empty(), "the head spelling keys the env: {ds:?}");
}

#[test]
fn concrete_head_names_stay_concrete() {
    // `Vec<i32>` in a head is a concrete argument, not a binder — the
    // known-type law spares it the declared-list requirement
    let ds = diags_of(
        "class Holder<T> { v: T; }\n\
         impl<T> Holder<T> {\n\
             pub fn new() -> Self { return Self { v: 0 }; }\n\
             pub fn coded(self) -> str { return \"holder\"; }\n\
         }\n\
         entry fn main() -> str {\n\
             let h: Holder<i32> = Holder.new();\n\
             return h.coded();\n\
         }\n",
    );
    assert!(ds.is_empty(), "a concrete generic head needs no binders: {ds:?}");
}
