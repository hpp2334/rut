//! A6 surface migrations (RFC 0005 §9): the `[v; n]` repeat construction,
//! the `&x` address-of (the `make_ptr` spelling is gone), and the purge of
//! the `Array` name — the type is `[T]`, and use sites diagnose with the
//! removal and its replacement.

use rut_parser::Mode;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // core + pouch bound as the uses (RFC 0028); these tests exercise the
    // migrated surface, not use discipline
    let collection = rut_core::binary::Surface::default();
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core()), (3, collection)],
    )
}

fn diags_of(src: &str) -> Vec<String> {
    compile(src)
        .diags
        .iter()
        .map(|d| d.msg.clone())
        .collect()
}

#[test]
fn repeat_compiles_scalar_nil_and_ref_fills() {
    // a scalar fill, a nil fill over a pointer array, and a ref fill
    // (the cell handle shared by every slot)
    let out = compile(
        "struct P { x: i32 = 0 }\n\
         fn main() -> i32 {\n\
             let a: [i32] = [0; 8];\n\
             let b: [*i32] = [nil; 4];\n\
             let p = &P { x: 1 };\n\
             let c: [*P] = [p; 3];\n\
             let n: i32 = 5;\n\
             let d: [f64] = [1.5; n * 2];\n\
             return a.len() + b.len() + c.len() + d.len();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn nil_fill_lowers_to_arrnew_alone() {
    // the nil fill IS the zero-fill: the memset-class op needs no loop
    let out = compile(
        "fn main() -> i32 {\n\
             let b: [*i32] = [nil; 4];\n\
             return b.len();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let ir = out.ir_dump;
    let arrnews = ir.matches("arrnew").count();
    let arrsets = ir.matches("arrset").count();
    assert_eq!(arrnews, 1, "one ArrNew: {ir}");
    assert_eq!(arrsets, 0, "a nil fill must not emit a fill loop: {ir}");
}

#[test]
fn address_of_boxes_the_operand() {
    // `&v` ≡ the old `make_ptr(v)`: a `*T` the deref reads back
    let out = compile(
        "struct P { x: i32 = 0 }\n\
         fn poke(p: *P) -> i32 { return p.x; }\n\
         fn main() -> i32 {\n\
             let v = P { x: 9 };\n\
             let p = &v;\n\
             let q: *i32 = &7;\n\
             return poke(p) + *q;\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let ir = out.ir_dump;
    assert!(ir.contains("makeptr"), "the box op must be there: {ir}");
}

#[test]
fn the_array_name_diagnoses_with_the_removal() {
    // type position
    let ds = diags_of(
        "pub fn f(a: Array<i32>) -> i32 { return a.len(); }\n\
         fn main() -> i32 { return 0; }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`Array` was removed")),
        "type-position Array must self-diagnose: {ds:?}"
    );
    // the old type-call construction
    let ds = diags_of(
        "fn main() -> i32 {\n\
             let a = Array<i32>(4);\n\
             return a.len();\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`Array` was removed") && d.contains("[v; n]")),
        "Array<T>(n) must point at `[v; n]`: {ds:?}"
    );
    // and the removed make_ptr spelling
    let ds = diags_of(
        "fn main() -> i32 {\n\
             let p = make_ptr(7);\n\
             return *p;\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`make_ptr(v)` was removed") && d.contains("`&v`")),
        "make_ptr must point at `&v`: {ds:?}"
    );
}

#[test]
fn bracket_types_need_no_use_statement() {
    // `[T]` is grammar — a module that never named `Array` still spells it
    let out = compile(
        "pub fn join_all(parts: [str]) -> str { return string_join(parts); }\n\
         fn main() -> i32 { return 0; }\n",
    );
    assert!(
        out.diags.iter().any(|d| d.msg.contains("string_join")),
        "string_join itself stays use-gated: {:?}",
        out.diags
    );
    let out = compile(
        "fn len3(xs: [*i32]) -> i32 { return xs.len(); }\n\
         fn main() -> i32 {\n\
             let a: [i32] = [0; 3];\n\
             return a.len() + len3([nil; 1]);\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn pointer_backed_vec_shape_fuses_through_the_deref() {
    // the DataBuf shape over `[*T]`: the fused index read/write derefs
    // and boxes (RFC 0032 §1.1 over the pointer-array backing)
    let out = compile(
        "class Box2<T> {\n\
             buf: [*T];\n\
             len: i32;\n\
         }\n\
         impl Box2<T> {\n\
             fn new() -> Self { return Self { buf: [nil; 4], len: 0 }; }\n\
             fn push(mut self, v: T) -> nil { self.buf[self.len] = &v; self.len += 1; }\n\
             fn get(self, i: i32) -> T { return *self.buf[i]; }\n\
         }\n\
         fn main() -> i32 {\n\
             let mut b: Box2<i32> = Box2.new();\n\
             b.push(5);\n\
             b.push(7);\n\
             b[0] = 6;\n\
             return b[0] + b[1];\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn impl_over_the_array_type_still_compiles() {
    // `impl [T] { .. }` — the generic template through the element
    let out = compile(
        "impl [T] {\n\
             fn first(self) -> i32 { return 7; }\n\
         }\n\
         fn main() -> i32 {\n\
             let a = [0; 2];\n\
             return a.first();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}
