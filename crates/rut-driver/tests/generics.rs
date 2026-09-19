//! Generic user types (RFC 0013 monomorphization): one concrete type and
//! one set of methods per instantiation.

use rut_parser::Mode;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // the core surface bound as the one use (RFC 0028): these tests
    // exercise generic monomorphization, not use discipline — the
    // prelude is used, never ambient
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core())],
    )
}

#[test]
fn generic_class_monomorphizes_per_instantiation() {
    let out = compile(
        "class Box<T> {\n\
             value: T;\n\
         }\n\
         impl Box<T> {\n\
             fn new(v: T) -> Self { return Self { value: v }; }\n\
             fn get(self) -> T { return self.value; }\n\
         }\n\
         fn main() -> i32 {\n\
             let b: Box<i32> = Box.new(41);\n\
             return b.get() + 1;\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    let boxes = p.types.types.iter().filter(|t| p.name_of(t.name) == "Box<i32>").count();
    assert_eq!(boxes, 1, "Box<i32> instantiated once");
    // body compiled: `new` (static) + `main`; `get` is a small instance
    // method and inlines at its call site (RFC 0005 sequence lowering)
    assert!(p.funcs.len() >= 2, "funcs: {:?}", p.funcs.iter().map(|f| p.name_of(f.name)).collect::<Vec<_>>());
    assert!(p.funcs.iter().any(|f| p.name_of(f.name) == "main"));
}

#[test]
fn two_instantiations_are_distinct() {
    let out = compile(
        "class Pair<T> {\n\
             a: T;\n\
             b: T;\n\
         }\n\
         impl Pair<T> {\n\
             fn new(a: T, b: T) -> Self { return Self { a: a, b: b }; }\n\
             fn fst(self) -> T { return self.a; }\n\
         }\n\
         fn main() -> i32 {\n\
             let x: Pair<i32> = Pair.new(1, 2);\n\
             let y: Pair<i64> = Pair.new(3, 4);\n\
             return x.fst();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    assert_eq!(p.types.types.iter().filter(|t| p.name_of(t.name) == "Pair<i32>").count(), 1);
    assert_eq!(p.types.types.iter().filter(|t| p.name_of(t.name) == "Pair<i64>").count(), 1);
}

#[test]
fn recursive_generic_terminates() {
    // Node<T> { next: ?Node<T> } (v1.1: the recursive edge is a nullable,
    // nil = end) — the cache must break the cycle
    let out = compile(
        "class Node<T> {\n\
             value: T;\n\
             next: ?Node<T>;\n\
         }\n\
         impl Node<T> {\n\
             fn new(v: T) -> Self { return Self { value: v, next: nil }; }\n\
         }\n\
         fn main() -> i32 { let n: Node<i32> = Node.new(1); return n.value; }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    assert_eq!(p.types.types.iter().filter(|t| p.name_of(t.name) == "Node<i32>").count(), 1);
}

#[test]
fn explicit_generic_static_path() {
    let out = compile(
        "class Box<T> {\n\
             value: T;\n\
         }\n\
         impl Box<T> {\n\
             fn new(v: T) -> Self { return Self { value: v }; }\n\
             fn get(self) -> T { return self.value; }\n\
         }\n\
         fn main() -> i32 {\n\
             let b = Box<i32>.new(7);\n\
             return b.get();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn vec_over_pointer_array_compiles() {
    // the Vec shape over the nullable-array backing (RFC 0005 §9 + RFC 0044):
    // `[nil; cap]` is the only generic zero; stores share `v`, and the
    // fused loads deref (RFC 0032 §1.1)
    let out = compile(
        "class Vec<T> {\n\
             buf: [?T];\n\
             len: i32;\n\
         }\n\
         impl Vec<T> {\n\
             fn new() -> Self { return Vec.with_capacity(0); }\n\
             fn with_capacity(cap: i32) -> Self { return Self { buf: [nil; cap], len: 0 }; }\n\
             fn len(self) -> i32 { return self.len; }\n\
             fn push(mut self, v: T) -> nil {\n\
                 if (self.len == self.buf.len()) {\n\
                     let mut cap = self.buf.len() * 2;\n\
                     if (cap == 0) { cap = 4; }\n\
                     let mut next: [?T] = [nil; cap];\n\
                     for (let i = 0; i < self.len; i += 1) { next[i] = self.buf[i]; }\n\
                     self.buf = next;\n\
                 }\n\
                 self.buf[self.len] = v;\n\
                 self.len += 1;\n\
             }\n\
             fn pop(mut self) -> T {\n\
                 self.len -= 1;\n\
                 return self.buf[self.len];\n\
             }\n\
         }\n\
         fn main() -> i32 {\n\
             let mut v: Vec<i32> = Vec.new();\n\
             v.push(1);\n\
             v.push(2);\n\
             return v.len();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    assert_eq!(p.types.types.iter().filter(|t| p.name_of(t.name) == "Vec<i32>").count(), 1);
}

