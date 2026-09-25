//! Parameterized trait impls, phase 1 (the checker half): `impl
//! Readable<T> for Source<T>` registers as a TEMPLATE — the trait ref's
//! bare-parameter arguments bind the target's own type parameters to
//! placeholder types, coverage proceeds, and the v1 shape guard rejects
//! the shapes the template cannot carry (a parameter nested in a type, a
//! name that is neither a target parameter nor a type in scope) with one
//! diagnostic each. Dispatch/unification is the phase-2 half: no test
//! here may dispatch through the template.

use rut_parser::Mode;

fn compile(src: &str) -> rut_driver::ProgramOutput {
    // the core surface bound as the one use (RFC 0028): these tests
    // exercise impl registration, not use discipline
    rut_driver::compile_program(
        src,
        Mode::Impl,
        "test",
        1,
        &[(2, rut_core::binary::Surface::core(), "core".to_string())],
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
fn parameterized_trait_impl_registers_as_a_template() {
    // the head's parameter names the target's own parameter — the
    // checker's blocker ("unknown type `T`") is gone, and the concrete
    // form still compiles beside it
    let out = compile(
        "trait Readable<T> { fn atom_id(self) -> u32; }\n\
         class Source<T> { id: u32 = 0; }\n\
         impl Readable<T> for Source<T> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         impl Readable<i64> for Source<i64> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         entry fn main() {}\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
}

#[test]
fn trait_parameter_in_the_impl_methods_signature_resolves() {
    // the coverage check resolves both sides under the placeholder env —
    // a parameter-spelled impl signature must not die as an unknown type
    let ds = diags_of(
        "trait Store<T> {\n\
             fn get(self, k: T) -> u32;\n\
             fn put(self, k: T, v: u32);\n\
         }\n\
         class Source<T> { id: u32 = 0; }\n\
         impl Store<T> for Source<T> {\n\
             fn get(self, k: T) -> u32 { return self.id; }\n\
             fn put(self, k: T, v: u32) { }\n\
         }\n\
         entry fn main() {}\n",
    );
    assert!(ds.is_empty(), "{ds:?}");
}

#[test]
fn repeated_parameter_in_two_trait_slots_is_legal() {
    let ds = diags_of(
        "trait Pair2<A, B> { fn one(self) -> u32; }\n\
         class Source<T> { id: u32 = 0; }\n\
         impl Pair2<T, T> for Source<T> {\n\
             fn one(self) -> u32 { return self.id; }\n\
         }\n\
         entry fn main() {}\n",
    );
    assert!(ds.is_empty(), "{ds:?}");
}

#[test]
fn parameter_nested_in_a_trait_argument_is_a_v1_error() {
    let ds = diags_of(
        "trait Readable<T> { fn atom_id(self) -> u32; }\n\
         class Source<T> { id: u32 = 0; }\n\
         impl Readable<Vec<T>> for Source<T> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         entry fn main() {}\n",
    );
    assert_eq!(ds.len(), 1, "one diagnostic, not a cascade: {ds:?}");
    assert!(
        ds[0].contains("a parameter nested inside a type is not supported yet"),
        "{ds:?}"
    );
}

#[test]
fn trait_argument_naming_no_target_parameter_and_no_type_is_a_v1_error() {
    // `T` is not a parameter of the target (`Source<U>`) nor a type in
    // scope — the guard diagnoses the shape, once
    let ds = diags_of(
        "trait Readable<T> { fn atom_id(self) -> u32; }\n\
         class Source<U> { id: u32 = 0; }\n\
         impl Readable<T> for Source<U> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         entry fn main() {}\n",
    );
    assert_eq!(ds.len(), 1, "one diagnostic, not a cascade: {ds:?}");
    assert!(
        ds[0]
            .contains("`T` is neither a type in scope nor a type parameter of the impl target"),
        "{ds:?}"
    );
}

#[test]
fn exact_template_duplicate_still_errors() {
    // same trait template (same parameters) + same class: the ordinary
    // (trait, type) duplicate check. A template OVERLAPPING a concrete
    // impl for a specific instantiation (`Readable<T>` vs
    // `Readable<i64>`) is deliberately NOT checked at collect time —
    // the phase-2 dispatch half owns that resolution.
    let ds = diags_of(
        "trait Readable<T> { fn atom_id(self) -> u32; }\n\
         class Source<T> { id: u32 = 0; }\n\
         impl Readable<T> for Source<T> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         impl Readable<T> for Source<T> {\n\
             fn atom_id(self) -> u32 { return self.id; }\n\
         }\n\
         entry fn main() {}\n",
    );
    assert_eq!(ds.len(), 1, "one diagnostic: {ds:?}");
    assert!(
        ds[0].contains("duplicate impl for the same (trait, type) pair"),
        "{ds:?}"
    );
}
