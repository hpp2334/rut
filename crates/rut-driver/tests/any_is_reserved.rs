//! `any` is GONE (the any-lane removal): the reservation is universal —
//! the RFC 0012 diagnostic fires in `.rut` source AND on the `.d.rut`
//! decl surface, and the driver's crossing table has no `any` arm. The
//! erasure box (`opaque(v)` + `downcast<T>`, RFC 0014) is the only
//! value lane; these tests pin the refusal end to end.

use rut_driver::{Module, ModuleBody, Session, lower_decl_module};
use rut_lexer::lexer::lex_mode;
use rut_parser::{parse, Mode};

/// The RFC 0012 diagnostic, verbatim.
const MSG: &str = "rut has no `any`; use a trait type or `opaque` (RFC 0012, RFC 0014)";

#[test]
fn a_decl_any_param_draws_the_diagnostic() {
    // the old host-decl exemption is dead: `v: any` in a `.d.rut` is
    // the reserved-word error, surfaced as a load error
    let src = "pub host fn box_put(m: opaque, v: any) -> i64;\n";
    let (_, diags) = parse(src, Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg == MSG),
        "the decl param spelling is the RFC 0012 diagnostic: {diags:?}"
    );
    let err = lower_decl_module(src, "hmap.d.rut").expect_err("the any row refuses to load");
    assert!(err.contains(MSG), "{err}");
    assert!(err.contains("the surface does not parse"), "{err}");
}

#[test]
fn a_decl_any_answer_draws_the_diagnostic() {
    let src = "pub host fn box_get(m: opaque) -> any;\n";
    let (_, diags) = parse(src, Mode::Decl);
    assert!(
        diags.iter().any(|d| d.msg == MSG),
        "the decl answer spelling is the RFC 0012 diagnostic: {diags:?}"
    );
}

#[test]
fn any_is_reserved_in_both_lex_modes() {
    // one tokenizer, one law: the word refuses identically in `.rut`
    // and `.d.rut` mode (the mode-independent reservation)
    for (label, diags) in [
        ("impl", lex_mode("let x: any = 1;").1),
        ("decl", lex_mode("pub host fn f(v: any);").1),
    ] {
        assert!(diags.iter().any(|d| d.msg == MSG), "{label}: {diags:?}");
    }
}

#[test]
fn rut_source_cannot_name_any() {
    // unchanged through the removal: a type position in .rut source
    // refuses with the same diagnostic (compile_graph surfaces it)
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module(
        "app",
        Module {
            body: ModuleBody::Source { text: "pub fn main() -> nil {\n    let x: any = 1;\n}".into(), is_decl: false },
            ..Default::default()
        },
    )
    .unwrap();
    let out = rut_driver::compile_graph(&s, "app");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("rut has no `any`")),
        "`any` is reserved — rut source must refuse it: {:?}",
        out.diags
    );
}

#[test]
fn decl_types_must_still_be_crossing_types() {
    // with the `any` arm gone, a non-crossing spelling gets the
    // crossing-set diagnostic (the grammar is concrete, RFC 0023 §1)
    let src = "pub host fn probe(m: opaque) -> Widget;\n";
    let err = lower_decl_module(src, "hmap.d.rut").expect_err("Widget is not a crossing type");
    assert!(err.contains("is not a crossing type"), "{err}");
}

#[test]
fn the_erasure_box_still_crosses() {
    // the value lane that REPLACED `any`: an `opaque` param/answer row
    // lowers, joins, and round-trips through `opaque(v)`/`downcast<T>`
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    let module = lower_decl_module(
        "pub host fn box_put(m: opaque, v: opaque) -> i64;\npub host fn box_get(m: opaque) -> ?opaque;\n",
        "hmap.d.rut",
    )
    .expect("the opaque rows lower");
    s.register_module("hmap", module).unwrap();
    s.register_module(
        "app",
        Module {
            body: ModuleBody::Source {
                text: "use hmap::{ box_put, box_get };\n\
                 pub fn main() -> i32 {\n\
                 \x20   let m: opaque = opaque(0);\n\
                 \x20   let put: i64 = box_put(m, opaque(\"ada\"));\n\
                 \x20   if (put != 1) { return -3; }\n\
                 \x20   let cell = box_get(m);\n\
                 \x20   if (cell == nil) { return -1; }\n\
                 \x20   let t = opaque.downcast<str>(cell);\n\
                 \x20   if (t == nil) { return -2; }\n\
                 \x20   return t.len();\n\
                 }\n"
                .into(),
                is_decl: false,
            },
            ..Default::default()
        },
    )
    .unwrap();
    let out = rut_driver::compile_graph(&s, "app");
    assert!(
        out.diags.is_empty(),
        "the opaque lane compiles: {:?}",
        out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>()
    );
}
