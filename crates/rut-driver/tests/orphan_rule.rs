//! Cross-module structural satisfaction — the orphan rule's replacement
//! law set. The orphan gate itself is GONE (nothing registers, so there
//! is nothing to gate; `impl Trait for Type` is unparseable), and the
//! laws that replace it are about WHERE MEMBERS LIVE:
//!
//! - a consumer proves a FOREIGN interface by spelling the members on
//!   its OWN type — the boundary check at the interface-typed call
//!   admits it (the old "foreign trait + local type" shape, without any
//!   registration);
//! - a used type takes no consumer impl blocks — its members live where
//!   the type was declared (the inherent-placement law);
//! - a primitive takes no impl blocks — capability on a value type is
//!   manufactured by a wrapper (a newtype class) whose inherent impl
//!   carries the members;
//! - `?T`/`[T]` heads are exactly the same law — composites never carry
//!   members, and the wrapper manufacture is the only door;
//! - a unit compiled with no origin map has no registration machinery
//!   to consult at all — a local type's members compile untouched
//!   through the raw compile_program path (the no-map inertness, in its
//!   modern form).

use rut_driver::GraphOutput;
use rut_parser::Mode;



/// An interface pkg — LINKED into its users (its names bind as used
/// decls carrying the exporter's spec).
const IFACE: &str = "pub interface Mark { fn mark(self) -> i32; }\n";

/// A generic-exporting pkg — SPLICED into its users: its text becomes
/// leaves of the consumer's unit. The concrete box exports the wrapped
/// manufacture (a newtype over the generic, minted where the generic is
/// local — a consumer wrapping a USED generic instantiation is the one
/// shape the fork does not spell).
const BOX: &str = "\
pub class Box<T> {
    v: T;
}
impl<T> Box<T> {
    pub fn new(v: T) -> Self {
        return Self { v: v };
    }
    pub fn get(self) -> T { return self.v; }
}
pub class IntBox(Box<i32>);
impl IntBox {
    pub fn new(v: i32) -> Self { return IntBox(Box<i32>.new(v)); }
    pub fn get(self) -> i32 { return self.inner.get(); }
}
";


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
            .pkg(rut_driver::calc_pkg())
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

fn graph(modules: &[(&str, &str)]) -> GraphOutput {
    let mut chain = rut_driver::RutRun::new();
    for (spec, src) in modules {
        chain = chain.pkg(rut_driver::Pkg::source(spec, *src));
    }
    let (root, _) = modules.last().expect("root module");
    graph_of(chain.entrypoint(root).compile())
}

fn diags_of(g: &GraphOutput) -> String {
    g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
}

// ---- the legal shapes ------------------------------------------------

#[test]
fn consumer_proves_a_foreign_interface_on_its_own_type() {
    // the digest shape: the interface links from `fmt` (origin fmt),
    // the type and its members are the consumer's own — the boundary
    // check at the interface-typed call runs the member-set match
    let g = graph(&[
        ("fmt", IFACE),
        ("app", "\
use fmt::{Mark};
pub fn prove(x: Mark) -> i32 { return x.mark(); }
struct Thing { n: i32 }
impl Thing {
    pub fn mark(self) -> i32 { return self.n; }
}
entry fn main() -> i32 {
    let t = Thing { n: 7 };
    return prove(t);
}
"),
    ]);
    assert!(g.diags.is_empty(), "{}", diags_of(&g));
    assert!(g.program.is_some());
}

#[test]
fn the_owning_pkg_proves_its_interface_on_its_own_type() {
    // the trivially-legal shape every program relies on: interface and
    // type in one module, the members on the type's inherent impl
    let g = graph(&[(
        "app",
        "\
interface Mark { fn mark(self) -> i32; }
struct Thing { n: i32 }
impl Thing {
    pub fn mark(self) -> i32 { return self.n; }
}
entry fn main() -> i32 { let t = Thing { n: 7 }; return t.mark(); }
",
    )]);
    assert!(g.diags.is_empty(), "{}", diags_of(&g));
}

#[test]
fn a_consumer_wraps_a_foreign_type_to_prove_the_interface() {
    // the structural fork's third-party shape: the consumer cannot grow
    // members on a used type, so it spells a WRAPPER (a newtype class)
    // over the foreign value and carries the members there — the
    // boundary check admits the wrapper, never the bare type
    let g = graph(&[
        ("mark", IFACE),
        ("box", BOX),
        ("app", "\
use mark::{Mark};
use box::{IntBox};
class MarkedBox(IntBox);
impl MarkedBox {
    pub fn mark(self) -> i32 { return self.inner.get(); }
}
entry fn main() -> i32 {
    let m: MarkedBox = MarkedBox(IntBox.new(7));
    return m.mark();
}
"),
    ]);
    assert!(g.diags.is_empty(), "{}", diags_of(&g));
    assert!(g.program.is_some());
}

// ---- the placement errors (the orphan gate's replacements) -----------

#[test]
fn inherent_impl_on_a_used_class_is_refused() {
    // the probe A law, restated for linked packages: an INHERENT block
    // on a used class lives in the type's module — the class's surface
    // carries its methods, and a consumer's block would need the
    // private layout. The old `impl Trait for UsedType` escape is gone
    // with the grammar; satisfaction of a foreign interface is proven
    // on the consumer's OWN types (or a wrapper over the foreign value).
    let g = graph(&[
        ("box", BOX),
        ("app", "\
use box::{Box};
impl<T> Box<T> {
    pub fn probe_hi(self) -> i64 { return 7; }
}
entry fn main() -> i32 { return 0; }
"),
    ]);
    assert!(g.program.is_none(), "the inherent orphan must refuse");
    let ds = diags_of(&g);
    assert!(
        ds.contains("inherent impls live in the type's module"),
        "{ds}"
    );
}

#[test]
fn impl_for_is_unparseable() {
    // the grammar law: `impl I for T` is gone — the impl head is a type
    // and the body must follow, so the deleted branch dies at the
    // parser, naming the one inherent form
    let g = graph(&[
        ("mark", IFACE),
        ("app", "\
use mark::{Mark};
struct Thing { n: i32 }
impl Mark for Thing {
    fn mark(self) -> i32 { return self.n; }
}
entry fn main() -> i32 { return 0; }
"),
    ]);
    assert!(g.program.is_none(), "the trait-impl spelling must refuse");
    let ds = diags_of(&g);
    assert!(
        ds.contains("expected {, found `for`"),
        "{ds}"
    );
}

#[test]
fn primitive_head_takes_no_impl_blocks() {
    // the builtin clause, restated: a primitive is in NO pkg and takes
    // NO impl blocks — the wrapper manufacture is the sanctioned door
    // (the diagnostic names it)
    let g = graph(&[
        ("app", "\
impl str {
    pub fn mark(self) -> i32 { return 1; }
}
entry fn main() -> i32 { return 0; }
"),
    ]);
    assert!(g.program.is_none(), "the prim impl must refuse");
    let ds = diags_of(&g);
    assert!(
        ds.contains("a primitive takes no impl blocks")
            && ds.contains("wrapper"),
        "{ds}"
    );
}

#[test]
fn opt_and_array_heads_take_no_impl_blocks() {
    // the `?T`/`[T]` matrix: a composite head carries no members and
    // takes no impl block here — the impl-target diagnosis covers the
    // whole composite family (the old local-trait escape died with the
    // grammar)
    for head in ["?T", "[T]"] {
        let src = format!(
            "\
impl<T> {head} {{
    pub fn mark(self) -> i32 {{ return 1; }}
}}
entry fn main() -> i32 {{ return 0; }}
"
        );
        let g = graph(&[("app", src.as_str())]);
        assert!(
            g.diags.iter().any(|d| d.msg.contains("impl target must be a struct, class, or enum of this module")),
            "{head}: {}",
            diags_of(&g)
        );
    }
}

// ---- the real std pkgs ------------------------------------------------

#[test]
fn consumer_proves_jsonserialize_on_its_own_type() {
    // the survey probe B's modern replacement: json's interface, the
    // consumer's own type, the members on the consumer's inherent impl
    // — the boundary check at the encodeJson entry admits it and the
    // document comes back
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut world = Vec::new();
    for dir in ["rut/pouch", "rut/nmapset", "rut/json"] {
        world.extend(rut_driver::dir_pkgs(&root.join(dir)).expect("mount tree pkg").pkgs);
    }
    let mut chain = rut_driver::RutRun::new();
    for p in &world {
        chain = chain.pkg(p.clone());
    }
    let g = graph_of(
        chain
            .pkg(rut_driver::calc_pkg())
            .pkg(rut_driver::Pkg::source(
                "main",
                "\
use json::{ encodeJson, JsonSerialize, JsonWriter, EncodeJsonError, JsonVec };
use pouch::{ Vec };

class Todo { name: str; n: i64; }

impl Todo {
    pub fn encode(self, mut w: JsonWriter) -> ?EncodeJsonError {
        w.begin_object();
        w.key(\"name\");
        w.write_str(self.name);
        w.key(\"n\");
        w.write_i64(self.n);
        w.end_object();
        return nil;
    }
}

entry fn main() -> ?str {
    let mut v = Vec<Todo>.new();
    v.push(Todo { name: \"a\", n: 1 });
    let (doc, e) = encodeJson(JsonVec(v));
    if (e != nil) { return nil; }
    return doc;
}
",
            ))
            .entrypoint("main")
            .compile(),
    );
    let ds = diags_of(&g);
    assert!(g.program.is_some(), "{ds}");
}

// ---- the no-map inertness ---------------------------------------------

#[test]
fn no_map_unit_is_inert() {
    // the single-file law (the survey §2.3's fallback, in its modern
    // form): a unit compiled with no origin map has no registration
    // machinery at all — a local type's inherent members compile
    // untouched through the raw compile_program path
    let out = rut_driver::compile_program(
        "\
interface Mark { fn mark(self) -> i32; }
struct Thing { n: i32 }
impl Thing {
    pub fn mark(self) -> i32 { return self.n; }
}
entry fn main() -> i32 { let t = Thing { n: 7 }; return t.mark(); }
",
        Mode::Impl,
        "app",
        1,
        &[],
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    assert!(out.program.is_some());
}
