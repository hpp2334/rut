//! The inherent-impl sweep's gate — task B (F60).
//!
//! The sweep stages its hits with `prefer_mounted` and answers only
//! when every staged hit is the SAME `(file, handle)`: two mounted
//! impl fns of the owner name are two declarations, and the editor
//! answers neither (one question, one answer). One mounted impl
//! still answers — the green guard beside the two-hit cell, which
//! rides the same world minus the second carrier.

mod common;

use common::*;

/// the asking root: it declares the type; the impl carriers sit
/// beside it (module `m`, module `n`), each spelling the same
/// member on the same owner
const ROOT: &str = "mod m;\nmod n;\n\nstruct Counter {\n    n: i32;\n}\n\nfn use_it(c: Counter) -> i32 {\n    return c.bump();\n}\n";

/// the one impl carrier — `m`'s spelling of the member
const M_IMPL: &str = "impl Counter {\n    pub fn bump(self) -> i32 { return self.n + 1; }\n}\n";

/// the second impl carrier — `n`'s spelling of the same member
const N_IMPL: &str = "impl Counter {\n    pub fn bump(self) -> i32 { return self.n + 2; }\n}\n";

/// two mounted impl fns of the owner name, one package: the sweep
/// stages both (both are mounts, neither is the embedded prelude) and
/// a distinct candidate is silence, never the old first-take
#[test]
fn two_mounted_impl_fns_of_the_owner_name_answer_nothing() {
    let mut s = S::boot();
    s.index_mod_at("file:///ws/pkg/src/m/mod.rut", "pkg", "m", M_IMPL);
    s.index_mod_at("file:///ws/pkg/src/n/mod.rut", "pkg", "n", N_IMPL);
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", ROOT);
    let h = s.hover_in(uri, ROOT, "c.bump();", 1, 2);
    assert!(no_hover(&h), "two distinct impl fns are silence: {h}");
    let hits = s.def_in(uri, ROOT, "c.bump();", 1, 2);
    assert!(hits.is_empty(), "no jump to either candidate: {hits:?}");
}

/// the green guard — the same world with ONE carrier still answers
/// (the same-node law refuses distinct candidates, never a lone one)
#[test]
fn one_mounted_impl_fn_of_the_owner_name_still_answers() {
    let mut s = S::boot();
    s.index_mod_at("file:///ws/pkg/src/m/mod.rut", "pkg", "m", M_IMPL);
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", ROOT);
    let md = markdown(s.hover_in(uri, ROOT, "c.bump();", 1, 2));
    assert!(md.contains("pub fn bump(self) -> i32"), "{md}");
    assert!(md.contains("in `Counter`"), "{md}");
    assert!(
        md.contains("file:///ws/pkg/src/m/mod.rut"),
        "the one carrier's provenance: {md}"
    );
}
