//! The orphan rule (RFC 0012 §2a, the orphan-rule batch phase 1): for
//! every `impl Trait for Type` at least one of the pair is defined in
//! the pkg whose source declared the block — tracked through the
//! mount/splice model by the unit's origin map (a spliced leaf keeps its
//! origin pkg; a bound name carries its exporter's spec; a builtin —
//! `?T`, `[T]`, a primitive, `opaque` — is in no pkg, and only a local
//! trait may be implemented for one).
//!
//! Covered: the legal shapes (local type + foreign trait; foreign type +
//! local trait; both local), the orphan errors (both foreign; builtin
//! self type + foreign trait; `?T`/`[T]` heads; generic heads by the
//! head's locality), the diagnostic's exact text (the survey §3.1's two
//! renderings, probe B's shape first), the mount model (the same pair is
//! legal in the owning pkg's unit and an orphan in a consumer's — the
//! origin, not the unit, decides), and the no-map inertness (a
//! single-file unit compiles its local impls untouched).

use rut_driver::{GraphOutput, Module, Session};
use rut_parser::Mode;

/// A non-generic trait pkg — LINKED into its users (its names bind as
/// used decls carrying the exporter's spec).
const TR: &str = "pub trait Mark { fn mark(self) -> i32; }\n";

/// A generic-exporting pkg — SPLICED into its users: its text becomes
/// leaves of the consumer's unit, and its origin survives the splice.
const FMT: &str = "\
pub trait Show { fn show(self) -> str; }
pub class Box<T> {
    v: T;
}
impl Box<T> {
    pub fn new(v: T) -> Self {
        return Self { v: v };
    }
}
";

/// A generic-exporting pkg that owns a side of a pair with `tr` — the
/// mount model's legal home for `impl Mark for Set<T>`.
const COLL: &str = "\
use tr::{Mark};
pub class Set<T> {
    v: T;
}
impl Set<T> {
    pub fn new(v: T) -> Self {
        return Self { v: v };
    }
}
impl Mark for Set<T> {
    fn mark(self) -> i32 { return 1; }
}
";

fn graph(modules: &[(&str, &str)]) -> GraphOutput {
    let mut s = Session::new();
    for (spec, src) in modules {
        let _ = s.register_module(
            spec,
            Module { spec: spec.to_string(), source: Some(src.to_string()), ..Default::default() },
        );
    }
    let (root, _) = modules.last().expect("root module");
    rut_driver::compile_graph(&s, root)
}

fn diags_of(g: &GraphOutput) -> String {
    g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
}

// ---- the legal shapes ------------------------------------------------

#[test]
fn local_type_and_foreign_trait_is_legal() {
    // the digest shape: the trait splices from `fmt` (origin fmt), the
    // type is the consumer's own — the type-local side of §2a
    let g = graph(&[
        ("tr", TR),
        ("fmt", FMT),
        ("app", "\
use fmt::{Show};
struct Thing { n: i32 }
impl Show for Thing {
    fn show(self) -> str { return \"thing\"; }
}
fn main() -> i32 {
    let t = Thing { n: 1 };
    let s = t.show();
    return s.len() as i32;
}
"),
    ]);
    assert!(g.diags.is_empty(), "{}", diags_of(&g));
    assert!(g.program.is_some());
}

#[test]
fn foreign_type_and_local_trait_is_legal() {
    // the json-group shape: the type splices from `coll` (origin coll),
    // the trait is the consumer's own — the trait-local side of §2a
    let g = graph(&[
        ("tr", TR),
        ("coll", COLL),
        ("app", "\
use coll::{Set};
trait Named { fn tag(self) -> str; }
impl Named for Set<T> {
    fn tag(self) -> str { return \"set\"; }
}
fn main() -> i32 { return 0; }
"),
    ]);
    assert!(g.diags.is_empty(), "{}", diags_of(&g));
    assert!(g.program.is_some());
}

#[test]
fn both_local_is_legal() {
    // the trivially-legal shape every pre-rule program relied on
    let g = graph(&[(
        "app",
        "\
trait Mark { fn mark(self) -> i32; }
struct Thing { n: i32 }
impl Mark for Thing {
    fn mark(self) -> i32 { return self.n; }
}
fn main() -> i32 { let t = Thing { n: 7 }; return t.mark(); }
",
    )]);
    assert!(g.diags.is_empty(), "{}", diags_of(&g));
}

#[test]
fn inherent_impl_on_a_spliced_foreign_class_stays_legal() {
    // probe A (the orphan-rule survey §1.5): an INHERENT block on a
    // spliced foreign class compiles — the splice hole is recorded out
    // of scope, and this pin keeps the rule scoped to trait impls
    let g = graph(&[
        ("tr", TR),
        ("coll", COLL),
        ("app", "\
use coll::{Set};
impl Set<T> {
    pub fn probe_hi(self) -> i64 { return 7; }
}
fn main() -> i32 { return 0; }
"),
    ]);
    assert!(g.diags.is_empty(), "{}", diags_of(&g));
}

// ---- the orphan errors ----------------------------------------------

#[test]
fn both_foreign_is_the_orphan_error() {
    // probe B (the survey §1.5/§2.7): the consumer unit splices `fmt`
    // and binds `tr`; the block is the consumer's own, and neither side
    // of the pair is defined in it — the §3.1 rendering, pinned verbatim
    let g = graph(&[
        ("tr", TR),
        ("fmt", FMT),
        ("app", "\
use fmt::{Show, Box};
use tr::{Mark};
impl Mark for Box<T> {
    fn mark(self) -> i32 { return 1; }
}
fn main() -> i32 { return 0; }
"),
    ]);
    assert!(g.program.is_none(), "the orphan must refuse to compile");
    let ds = diags_of(&g);
    assert!(
        ds.contains(
            "orphan impl: neither `Mark` nor `Box` is defined in this pkg — \
             `Mark` is tr's, `Box` is fmt's; an `impl Trait for Type` needs \
             at least one of the pair declared in its own pkg (RFC 0012 §2a)"
        ),
        "{ds}"
    );
}

#[test]
fn builtin_head_and_foreign_trait_is_the_orphan_error() {
    // the §2a builtin clause: a primitive is in NO pkg, so a foreign
    // trait's impl for one errs — the builtin-head rendering, pinned
    // verbatim (`?T` peels the same way; the matrix below)
    let g = graph(&[
        ("tr", TR),
        ("fmt", FMT),
        ("app", "\
use tr::{Mark};
impl Mark for str {
    fn mark(self) -> i32 { return 1; }
}
fn main() -> i32 { return 0; }
"),
    ]);
    assert!(g.program.is_none(), "the orphan must refuse to compile");
    let ds = diags_of(&g);
    assert!(
        ds.contains(
            "orphan impl: neither `Mark` nor `str` is defined in this pkg — \
             `Mark` is tr's, `str` is a builtin, in no pkg; only a trait of \
             this pkg may be implemented for a builtin (RFC 0012 §2a)"
        ),
        "{ds}"
    );
}

#[test]
fn opt_and_array_heads_peel_to_no_pkg() {
    // the `?T`/`[T]` matrix: the head peels to its element — a type
    // parameter, so the head's locality is "builtin, in no pkg". Under
    // a foreign trait the block is an orphan; under a local trait it is
    // legal (json's twelve).
    let orphan_opt = graph(&[
        ("tr", TR),
        (
            "app",
            "\
use tr::{Mark};
impl Mark for ?T {
    fn mark(self) -> i32 { return 1; }
}
fn main() -> i32 { return 0; }
",
        ),
    ]);
    assert!(orphan_opt.program.is_none(), "{}", diags_of(&orphan_opt));
    assert!(
        orphan_opt.diags.iter().any(|d| d.msg.contains("orphan impl")
            && d.msg.contains("`?T` is a builtin, in no pkg")),
        "{}",
        diags_of(&orphan_opt)
    );

    let orphan_array = graph(&[
        ("tr", TR),
        (
            "app",
            "\
use tr::{Mark};
impl Mark for [T] {
    fn mark(self) -> i32 { return 1; }
}
fn main() -> i32 { return 0; }
",
        ),
    ]);
    assert!(orphan_array.program.is_none(), "{}", diags_of(&orphan_array));
    assert!(
        orphan_array.diags.iter().any(|d| d.msg.contains("orphan impl")
            && d.msg.contains("`[T]` is a builtin, in no pkg")),
        "{}",
        diags_of(&orphan_array)
    );

    let legal = graph(&[(
        "app",
        "\
trait Mark { fn mark(self) -> i32; }
impl Mark for ?T {
    fn mark(self) -> i32 { return 1; }
}
impl Mark for [T] {
    fn mark(self) -> i32 { return 2; }
}
fn main() -> i32 { return 0; }
",
    )]);
    assert!(legal.diags.is_empty(), "{}", diags_of(&legal));
}

#[test]
fn generic_heads_classify_by_the_head() {
    // locality is of the HEAD: `Box` splices from `fmt` (origin fmt),
    // so the consumer's `impl Mark for Box<T>` is an orphan — the type
    // PARAMETER never satisfies locality; the same head under a local
    // trait is legal
    let orphan = graph(&[
        ("tr", TR),
        ("fmt", FMT),
        (
            "app",
            "\
use fmt::{Box};
use tr::{Mark};
impl Mark for Box<T> {
    fn mark(self) -> i32 { return 1; }
}
fn main() -> i32 { return 0; }
",
        ),
    ]);
    assert!(orphan.program.is_none(), "{}", diags_of(&orphan));
    assert!(
        orphan.diags.iter().any(|d| d.msg.contains("`Box` is fmt's")),
        "{}",
        diags_of(&orphan)
    );

    let legal = graph(&[
        ("fmt", FMT),
        (
            "app",
            "\
use fmt::{Box};
trait Mark { fn mark(self) -> i32; }
impl Mark for Box<T> {
    fn mark(self) -> i32 { return 1; }
}
fn main() -> i32 { return 0; }
",
        ),
    ]);
    assert!(legal.diags.is_empty(), "{}", diags_of(&legal));
}

// ---- the mount model -------------------------------------------------

#[test]
fn the_origin_not_the_unit_decides() {
    // `coll` owns `impl Mark for Set<T>` (trait foreign, type local):
    // legal in coll's own compile AND in every unit that splices it —
    // the block's origin is coll's leaf. The SAME pair written in a
    // consumer's own block is the orphan: nothing about the unit's
    // compilation changes the block's declaring pkg.
    let g = graph(&[
        ("tr", TR),
        ("coll", COLL),
        ("app", "\
use coll::{Set};
use tr::{Mark};
impl Mark for Set<T> {
    fn mark(self) -> i32 { return 2; }
}
fn main() -> i32 { return 0; }
"),
    ]);
    assert!(g.program.is_none(), "{}", diags_of(&g));
    let ds = diags_of(&g);
    assert!(
        ds.contains("orphan impl: neither `Mark` nor `Set` is defined in this pkg"),
        "{ds}"
    );
    // placement precedes registration: the orphan gate fires on the
    // app's block before the duplicate-pair check ever sees the pair
    assert!(!ds.contains("duplicate impl"), "{ds}");
}

// ---- the survey's canonical case (the real std pkgs) ------------------

#[test]
fn consumer_impl_of_jsonserialize_for_hashset_errors() {
    // the survey §1.5 probe B verbatim — the rule's first error on the
    // real mounted pkgs: json's trait, nmapset's container, the block
    // the consumer's own. The §3.1 rendering, pinned exactly.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    for dir in ["rut/pouch", "rut/nmapset", "rut/json"] {
        rut_driver::mount_dir(&mut s, &root.join(dir)).expect("mount tree pkg");
    }
    rut_driver::assemble_peers(&mut s).expect("assemble peer groups");
    s.register_module(
        "main",
        Module {
            spec: "main".into(),
            source: Some(
                "\
use json::{ JsonSerialize, JsonWriter, EncodeJsonError };
use nmapset::{ HashSet };

impl JsonSerialize for HashSet<T> {
    fn encode(self, mut w: JsonWriter) -> ?EncodeJsonError { return nil; }
}

entry fn main() -> nil {
    let x: i64 = 0;
}
"
                .into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();
    let g = rut_driver::compile_graph(&s, "main");
    assert!(g.program.is_none(), "the orphan must refuse to compile");
    let ds = diags_of(&g);
    assert!(
        ds.contains(
            "orphan impl: neither `JsonSerialize` nor `HashSet` is defined in this pkg — \
             `JsonSerialize` is json's, `HashSet` is nmapset's; an `impl Trait for Type` \
             needs at least one of the pair declared in its own pkg (RFC 0012 §2a)"
        ),
        "{ds}"
    );
}

// ---- the no-map inertness ---------------------------------------------

#[test]
fn no_map_unit_is_inert() {
    // the single-file law (the survey §2.3's fallback): a unit compiled
    // with no origin map has every definition's origin = its own spec —
    // the check reduces to names that resolve here, and a local impl
    // compiles untouched through the raw compile_program path
    let out = rut_driver::compile_program(
        "\
trait Mark { fn mark(self) -> i32; }
struct Thing { n: i32 }
impl Mark for Thing {
    fn mark(self) -> i32 { return self.n; }
}
impl Mark for ?T {
    fn mark(self) -> i32 { return 1; }
}
impl Mark for [T] {
    fn mark(self) -> i32 { return 2; }
}
pub fn main() -> i32 { let t = Thing { n: 7 }; return t.mark(); }
",
        Mode::Impl,
        "app",
        1,
        &[],
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    assert!(out.program.is_some());
}
