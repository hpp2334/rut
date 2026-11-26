//! Hover's gate — the envelope's markdown + range, driven through the
//! public `Lsp` over stored docs and mounted world files. Pins: the
//! verbatim signature law (the whole modifier prefix rides), the
//! owner/interface/ambiguity answers, the binding pass's inference
//! (annotated lets, loop variables, chained receivers, shadowing),
//! the primitive blurbs, the file-module edges, and the
//! hover-decorates-the-token range law.

mod common;

use common::*;

#[test]
fn method_hover_shows_signature_doc_and_owner() {
    let mut s = S::boot();
    let src = "\
// a 2D point
struct Point {
x: f64;
y: f64;
}

// euclidean length
fn length(p: Point) -> f64 {
return p.x;
}
";
    // the use site `p.x`, not the field decl
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "return p.x", 1, 9));
    assert!(md.contains("```rut"), "code block: {md}");
    assert!(md.contains("x: f64"), "field decl: {md}");
    assert!(md.contains("in `Point`"), "owner: {md}");
}

#[test]
fn class_hover_shows_struct_definition() {
    let mut s = S::boot();
    let src = "\
// a circle
class Circle {
pub r: f64;
x: f64;
y: f64;
}
";
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "Circle", 1));
    assert!(md.contains("class Circle {"), "{md}");
    assert!(md.contains("pub r: f64"), "{md}");
    assert!(md.contains("x: f64"), "{md}");
    assert!(md.contains("a circle"), "doc: {md}");
}

#[test]
fn method_via_inference_and_self() {
    let mut s = S::boot();
    let src = "\
class Circle {
r: f64;
fn area(self) -> f64 { return 3.14; }
fn grow(self, k: f64) -> nil { self.r = self.r * k; }
}
fn use_it() -> f64 {
let c = Circle::new(1.0);
return c.area();
}
";
    // the call `c.area()`, receiver inferred from `let c = Circle::new(..)`
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "c.area", 1, 2));
    assert!(md.contains("fn area(self) -> f64"), "{md}");
    assert!(md.contains("in `Circle`"), "{md}");
}

#[test]
fn interface_member_resolves_through_the_interface() {
    // an interface-typed receiver sees exactly the interface's declared
    // signatures — satisfaction is structural (a type qualifies by
    // having the members), so no impl lookup rides this path
    let mut s = S::boot();
    let src = "\
interface Drawable {
fn draw(self) -> nil;
}
class Circle {
r: f64;
}
impl Circle {
pub fn draw(self) -> nil { }
}
fn render(d: Drawable) -> nil {
d.draw();
}
";
    // the call `d.draw()` — param-typed receiver, resolved through the
    // interface itself
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "d.draw", 1, 2));
    assert!(md.contains("fn draw(self) -> nil"), "{md}");
    assert!(md.contains("declared in `Drawable`"), "{md}");
}

#[test]
fn impl_member_hover_shows_the_modifier_prefix_verbatim() {
    // the decl span starts at the member's FIRST token, so the verbatim
    // signature IS the whole prefix — `[marker]`, `pub`, `async` ride
    // the slice
    let mut s = S::boot();
    let src = "\
class Ctx {
internal: opaque;
}
impl Ctx {
[constructor] pub async fn open(cap: i32) -> Self { return Self { internal: cap }; }
}
entry fn main() -> nil {
let c = Ctx::open(1);
}
";
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "open", 1));
    assert!(
        md.contains("[constructor] pub async fn open(cap: i32) -> Self"),
        "{md}"
    );
}

#[test]
fn satisfied_member_resolves_to_the_inherent_impl() {
    // the satisfaction shape: an interface + the inherent impl that
    // carries the members — a CONCRETE receiver resolves to the impl
    // fn (the type's own surface), never to the interface's signature
    let mut s = S::boot();
    let src = "\
interface Get {
fn get(self) -> i32;
}
class A {
x: i32;
}
impl A {
pub fn get(self) -> i32 { return self.x; }
}
fn go(a: A) -> i32 {
return a.get();
}
";
    // the call `a.get()` — resolved through the impl
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "a.get", 1, 2));
    assert!(md.contains("fn get(self) -> i32"), "{md}");
    assert!(md.contains("in `A`"), "{md}");
}

#[test]
fn free_fn_unique_match() {
    let mut s = S::boot();
    let src = "\
// euclidean length
fn length(p: i32) -> f64 { return 1.0; }
fn main() -> nil { let x = length(3); }
";
    // the decl ident (`length` also names the fn — hover answers on
    // the declaration itself)
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "fn length", 1, 3));
    assert!(md.contains("fn length(p: i32) -> f64"), "{md}");
    assert!(md.contains("euclidean length"), "doc: {md}");
}

#[test]
fn item_modifier_prefixes_render_verbatim() {
    // span law v3.1 for items: the decl span starts at the item's
    // FIRST token, so the verbatim signature IS the whole prefix —
    // `pub async fn` and `entry fn` render as written
    let mut s = S::boot();
    let src = "\
pub async fn tick() -> bool { return true; }
entry fn main() -> nil { let b = tick(); }
";
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "tick", 1));
    assert!(md.contains("pub async fn tick() -> bool"), "{md}");
    let src = "\
entry fn run() -> i32 { return 7; }
entry fn main() -> nil { let n = run(); }
";
    let md = markdown(s.hover_at("file:///t/hov2.rut", src, "run", 1));
    assert!(md.contains("entry fn run() -> i32"), "{md}");
}

#[test]
fn pub_type_prefixes_render_verbatim() {
    // the type render slices the item's head (first token → the name):
    // the spelled `pub` rides it
    let mut s = S::boot();
    let src = "\
pub class Stack<T> {
items: Vec<T>;
}
entry fn main() -> nil { let s: Stack<i32> = nil; }
";
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "Stack", 1));
    assert!(md.contains("pub class Stack<T> {"), "{md}");
}

#[test]
fn generic_heads_render_verbatim_including_bounds() {
    // the head is the verbatim slice through the body's `{` — the
    // admission bound rides it exactly as written
    let mut s = S::boot();
    let src = "\
pub class Map<K requires Hashable, V> {
items: Vec<(K, V)>;
}
entry fn main() -> nil { let m: Map<i32, i32> = nil; }
";
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "Map", 1));
    assert!(
        md.contains("pub class Map<K requires Hashable, V> {"),
        "{md}"
    );
}

#[test]
fn surface_prefixes_render_verbatim() {
    // the Decl-mode rows render as written — through the door the
    // consumer's CALL sites carry the verbatim signatures (`fetch`,
    // `echo` are import-gated (`pub host` / `pub builtin` — the
    // compile binds them only where the `use` names them, the
    // file-scope law), so the doc's use edges ride; `keep` is
    // `prelude builtin` — ambient, bound in every unit, it hovers
    // with NO use edge), and the builtin class carries its spelled
    // head. The surface mounts at a path whose stem segment spells
    // the pkg head the use edges name.
    let mut s = S::boot();
    let src = "\
pub host async fn fetch(u: str) -> bytes;
pub builtin fn echo<T>(x: T) -> T;
prelude builtin fn keep<T>(x: T) -> T;
prelude builtin class Result<T> {
fn is_ok(self) -> bool;
}
";
    s.index_file("file:///t/surf/surf.d.rut", src);
    let call = "\
use surf::{ echo, fetch };
async fn go() -> bytes {
let x = echo(\"a\");
let y = keep(x);
return fetch(\"u\");
}
";
    let md = markdown(s.hover_at("file:///t/call.rut", call, "fetch", 1));
    assert!(
        md.contains("pub host async fn fetch(u: str) -> bytes"),
        "{md}"
    );
    let md = markdown(s.hover_at("file:///t/call.rut", call, "echo", 1));
    assert!(md.contains("pub builtin fn echo<T>(x: T) -> T"), "{md}");
    let md = markdown(s.hover_at("file:///t/call.rut", call, "keep", 1));
    assert!(md.contains("prelude builtin fn keep<T>(x: T) -> T"), "{md}");
    let md = markdown(s.hover_at("file:///t/surf.d.rut", src, "Result", 1));
    assert!(md.contains("prelude builtin class Result<T> {"), "{md}");
}

#[test]
fn the_newtype_head_renders_without_the_terminator() {
    // the brace-less newtype class spelling has no `{` to cut the head
    // at — the head slice trims the trailing `;`: the render is
    // `class Foo(Bar) {`, never `class Foo(Bar); {` nor a headless ` {`
    let mut s = S::boot();
    let src = "class Foo(Bar);\n";
    let md = markdown(s.hover_at("file:///t/newty.d.rut", src, "Foo", 1));
    assert!(md.contains("class Foo(Bar) {"), "{md}");
    assert!(!md.contains(";"), "no terminator artifact: {md}");
}

#[test]
fn ambiguity_lists_the_file_scope_own_ambiguity() {
    // the candidates path survives scoped: the file scope bound the
    // name to TWO of its own fns — a free fn and an impl method
    // (methods are type-scoped, the one-name law never meets them) —
    // and the honest answer lists both, never a silent pick. The
    // world's other same-named decls do NOT join the list: a sibling
    // file's twin is not the file scope's ambiguity.
    let mut s = S::boot();
    s.index_file("file:///t/a.rut", "fn helper() -> i32 { return 1; }\n");
    let src = "class Box2 {\n    n: i32;\n}\nimpl Box2 {\n    fn helper(self) -> i32 { return self.n; }\n}\nfn helper() -> i32 { return 2; }\nentry fn main() -> nil { let h = helper(); }\n";
    let md = markdown(s.hover_at("file:///t/main.rut", src, "helper();", 1));
    assert!(md.contains("multiple definitions"), "{md}");
    assert!(md.contains("main.rut"), "the file's own two decls: {md}");
    assert!(!md.contains("a.rut"), "the foreign twin never joins: {md}");
}

#[test]
fn sibling_duplicates_answer_the_file_own_binding() {
    // the one-name law's Act 2 headline: sibling modules may share a
    // name legally (Act 1) — the flat sweep answered for ALL of them
    // (a candidate list, or a foreign decl). The file scope answers
    // the queried file's OWN binding, at the call site and at the
    // decl site alike.
    let mut s = S::boot();
    s.index_file("file:///t/a.rut", "fn helper() -> i32 { return 1; }\n");
    s.index_file("file:///t/b.rut", "fn helper() -> i32 { return 2; }\n");
    let src = "// the local helper\nfn helper() -> i32 { return 3; }\nentry fn main() -> nil { let h = helper(); }\n";
    let uri = "file:///t/main.rut";
    // the call site: the file's own fn — no candidate list
    let md = markdown(s.hover_at(uri, src, "helper();", 1));
    assert!(md.contains("fn helper() -> i32"), "{md}");
    assert!(
        md.contains("the local helper"),
        "the file's own doc line: {md}"
    );
    assert!(md.contains("main.rut"), "the file's own provenance: {md}");
    assert!(!md.contains("multiple definitions"), "no candidates: {md}");
    assert!(
        !md.contains("a.rut") && !md.contains("b.rut"),
        "no foreign twin: {md}"
    );
    // the decl site answers itself
    let md = markdown(s.hover_in(uri, src, "fn helper", 1, 3));
    assert!(md.contains("the local helper"), "{md}");
}

#[test]
fn an_unbound_sibling_name_hovers_nothing() {
    // the honest unbound answer: the file never binds the name (no
    // decl, no use edge) — the siblings' decls are not the file's
    // scope, so the hover is EMPTY, never a foreign decl nor a
    // candidate list (the old flat sweep listed both twins). The
    // binding layer answers doc bindings only; an unbound bare name
    // has no local-binding fallback to give.
    let mut s = S::boot();
    s.index_file("file:///t/a.rut", "fn helper() -> i32 { return 1; }\n");
    s.index_file("file:///t/b.rut", "fn helper() -> i32 { return 2; }\n");
    let src = "entry fn main() -> nil { let h = helper(); }\n";
    let h = s.hover_at("file:///t/main.rut", src, "helper", 1);
    assert!(no_hover(&h), "an unbound sibling name answers nothing: {h}");
}

#[test]
fn a_use_bound_name_hovers_through_the_import() {
    // the file's use edges bind: the leaf (fn, type, and module let)
    // — each hover answers the EXPORTING file's decl
    let mut s = S::boot();
    s.index_file(
        "file:///t/gadgets.rut",
        "pub fn tag() -> i32 { return 1; }\npub struct Cog {\n    n: i32;\n}\npub let LIMIT: i32 = 9;\n",
    );
    let uri = "file:///t/main.rut";
    let src = "use gadgets::{ tag, Cog, LIMIT };\nentry fn main() -> nil {\n    let n = tag();\n    let c: Cog = nil;\n    let m = LIMIT;\n}\n";
    let md = markdown(s.hover_in(uri, src, "= tag();", 1, 2));
    assert!(md.contains("pub fn tag() -> i32"), "{md}");
    assert!(
        md.contains("gadgets.rut"),
        "the exporter's provenance: {md}"
    );
    let md = markdown(s.hover_in(uri, src, "let c: Cog", 1, 7));
    assert!(md.contains("struct Cog"), "{md}");
    let md = markdown(s.hover_in(uri, src, "let m = LIMIT", 1, 8));
    assert!(
        md.contains("pub let LIMIT"),
        "the use-bound module let: {md}"
    );
}

#[test]
fn host_fn_surface_favors_own_methods() {
    // std-style surface index ahead of the doc — the call site rides
    // its use edge (the file-scope law: a bare `pub host fn` the
    // compile refuses hovers nothing)
    let mut s = S::boot();
    s.index_file("core", "pub host fn string_join(s: [str]) -> i32;\n");
    let src = "use core::{ string_join };\nfn main() -> i32 { return string_join([]); }\n";
    let h = s.hover_at("file:///t/hov.rut", src, "string_join", 1);
    let md = markdown(h);
    assert!(md.contains("fn string_join"), "{md}");
    assert!(md.contains("core"), "{md}");
}

#[test]
fn nullable_alias_hover_renders_the_target() {
    // a nullable alias used to hover as `type Maybe = ;` — the target
    // silently vanished
    let mut s = S::boot();
    let src = "\
// maybe an i32
type Maybe = ?i32;
fn f(m: Maybe) -> nil { }
";
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "Maybe", 1));
    assert!(md.contains("type Maybe = ?i32;"), "{md}");
}

#[test]
fn nullable_let_binding_resolves_members() {
    let mut s = S::boot();
    let src = "\
class Circle {
r: f64;
// the area
fn area(self) -> f64 { return 3.14; }
}
fn go() -> f64 {
let c: ?Circle = Circle::new(1.0);
return c.area();
}
";
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "c.area", 1, 2));
    assert!(md.contains("fn area(self) -> f64"), "{md}");
    assert!(md.contains("in `Circle`"), "{md}");
}

#[test]
fn miss_is_none() {
    let mut s = S::boot();
    let src = "fn main() -> nil { let z = unknown_thing; }\n";
    let h = s.hover_at("file:///t/hov.rut", src, "unknown_thing", 1);
    assert!(no_hover(&h), "{h}");
}

#[test]
fn inherent_impl_block_method_hover() {
    // methods live in `impl` blocks — the call resolves through the
    // block's target type (the owner string IS the type's name)
    let mut s = S::boot();
    let src = "\
struct Counter {
n: i32;
}
impl Counter {
// add one
fn bump(self) -> i32 { self.n += 1; return self.n; }
}
fn use_it(c: Counter) -> i32 {
return c.bump();
}
";
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "c.bump", 1, 2));
    assert!(md.contains("fn bump(self) -> i32"), "{md}");
    assert!(md.contains("in `Counter`"), "{md}");
    assert!(md.contains("add one"), "doc: {md}");
}

#[test]
fn foreign_interface_member_requires_use() {
    // the use-both gate: the interface lives in another module, so its
    // members resolve only once the document `use`s it
    let mut s = S::boot();
    s.index_file(
        "file:///t/greets.rut",
        "interface Greeter {\nfn greet(self) -> nil;\n}\n",
    );

    let gated = "fn go(g: Greeter) -> nil { g.greet(); }\n";
    let h = s.hover_in("file:///t/main.rut", gated, "g.greet", 1, 2);
    assert!(
        no_hover(&h),
        "unused foreign interface must not resolve: {h}"
    );

    let used = "use greets::{ Greeter };\nfn go(g: Greeter) -> nil { g.greet(); }\n";
    let md = markdown(s.hover_in("file:///t/main.rut", used, "g.greet", 1, 2));
    assert!(md.contains("fn greet(self) -> nil"), "{md}");
    assert!(md.contains("declared in `Greeter`"), "{md}");
}

// ---- the binding pass + decl layer + M7 ----

#[test]
fn identifier_hover_shows_the_declared_annotation() {
    // display-side inference — an annotated let renders its annotation
    // as written (`?Circle` survives)
    let mut s = S::boot();
    let src = "\
class Circle {
r: f64;
}
fn go() -> f64 {
let c: ?Circle = nil;
return 3.14;
}
";
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "let c: ?Circle", 1, 4));
    assert!(md.contains("let c: ?Circle"), "{md}");
    assert!(md.contains("let binding"), "{md}");
    assert!(!md.contains("inferred"), "declared, not inferred: {md}");
}

#[test]
fn identifier_hover_shows_the_inferred_type() {
    let mut s = S::boot();
    let src = "\
class Circle {
r: f64;
}
fn go() -> f64 {
let c = Circle::new(1.0);
return c.r;
}
";
    // the use of `c` in `c.r` — inferred from the `Circle::new` init
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "c.r", 1, 0));
    assert!(md.contains("let c: Circle"), "{md}");
    assert!(md.contains("type inferred"), "{md}");
}

#[test]
fn param_hover_shows_the_declared_type() {
    let mut s = S::boot();
    let src = "\
struct Point {
x: f64;
}
fn length(p: Point) -> f64 {
return p.x;
}
";
    // the param's USE inside the body
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "p.x", 1, 0));
    assert!(md.contains("p: Point"), "{md}");
    assert!(md.contains("parameter"), "{md}");
}

#[test]
fn field_decl_hover_names_the_owner() {
    // hovering `item` inside the struct was NULL — member lookup needs
    // a dot; the decl layer's exact span answers
    let mut s = S::boot();
    let src = "\
class Circle {
r: f64;
}
struct Slot {
item: ?Circle;
}
";
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "item", 1));
    assert!(md.contains("item: ?Circle"), "{md}");
    assert!(md.contains("in `Slot`"), "{md}");
}

#[test]
fn enum_member_decl_hover_shows_the_enum() {
    let mut s = S::boot();
    let src = "enum Color { Red, Green }\nfn f(c: Color) -> nil { }\n";
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "Red", 1));
    assert!(md.contains("enum Color { Red, Green }"), "{md}");
}

#[test]
fn module_let_decl_and_use_hover() {
    let mut s = S::boot();
    let src = "\
let MAX: i32 = 100;
fn f() -> i32 {
return MAX;
}
";
    // the decl site — the verbatim slice carries the annotation
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "MAX", 1));
    assert!(md.contains("let MAX: i32 = 100"), "{md}");
    assert!(md.contains("module let"), "{md}");
    // the use site — the unique module-let match
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "MAX", 2));
    assert!(md.contains("let MAX: i32 = 100"), "{md}");
}

#[test]
fn module_let_initializer_infers() {
    // a module let with no annotation still answers (its head feeds
    // receiver inference)
    let mut s = S::boot();
    let src = "let W = 3.5;\nfn f() -> f64 { return W; }\n";
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "W", 1));
    assert!(md.contains("let W = 3.5;"), "{md}");
    assert!(md.contains("`f32`"), "{md}");
}

#[test]
fn field_read_receiver_resolves_members() {
    let mut s = S::boot();
    let src = "\
class Circle {
r: f64;
}
struct Wrap {
c: Circle;
}
fn area(wrap: Wrap) -> f64 {
let inner = wrap.c;
return inner.r;
}
";
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "let inner", 1, 4));
    assert!(md.contains("let inner: Circle"), "{md}");
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "inner.r", 1, 6));
    assert!(md.contains("r: f64"), "{md}");
    assert!(md.contains("in `Circle`"), "{md}");
}

#[test]
fn chained_call_receiver_resolves_members() {
    // `c.grown(2.0).r` — the chain shape in the doc; the field itself
    // answers with its owner (the member the chain resolves to)
    let mut s = S::boot();
    let src = "\
class Circle {
r: f64;
}
impl Circle {
fn grown(self, k: f64) -> Circle { return self; }
}
fn go(c: Circle) -> f64 {
return c.grown(2.0).r;
}
";
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "r: f64", 1, 0));
    assert!(md.contains("r: f64"), "{md}");
    assert!(md.contains("in `Circle`"), "{md}");
}

#[test]
fn fn_result_receiver_resolves_members() {
    // a free fn's declared ret feeds the same chain
    let mut s = S::boot();
    let src = "\
class Circle {
r: f64;
}
fn mk() -> Circle { return Circle::new(1.0); }
fn go() -> f64 {
let d = mk();
return d.r;
}
";
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "let d", 1, 4));
    assert!(md.contains("let d: Circle"), "{md}");
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "d.r", 1, 2));
    assert!(md.contains("in `Circle`"), "{md}");
}

#[test]
fn self_ret_chain_types_the_binding() {
    // the `-> Self` rung types the binding through the chain — the
    // builder shape (`Thing().m()` answers `Thing`, not the empty
    // display the un-resolved `Self` used to render)
    let mut s = S::boot();
    let src = "\
class Thing {
c: i32;
}
impl Thing {
fn m(self) -> Self { return self; }
}
fn go() -> nil {
let r = Thing().m();
}
";
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "let r", 1, 4));
    assert!(md.contains("let r: Thing"), "{md}");
}

#[test]
fn for_of_var_resolves_members() {
    // the for-of binding is a receiver today's naive inference misses;
    // `[Point]` → `Point` (the elem rule)
    let mut s = S::boot();
    let src = "\
struct Point {
x: f64;
y: f64;
}
fn sum(points: [Point]) -> f64 {
let total = 0.0;
for (let p of points) {
total += p.x;
}
return total;
}
";
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "let p of", 1, 4));
    assert!(md.contains("p: Point"), "{md}");
    assert!(md.contains("loop variable"), "{md}");
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "p.x", 1, 2));
    assert!(md.contains("x: f64"), "{md}");
}

#[test]
fn for_c_counter_binding_resolves() {
    // the default: an unsuffixed `0` is `i32`
    let mut s = S::boot();
    let src = "\
fn f(n: i32) -> i32 {
let acc = 0;
for (let i = 0; i < n; i += 1) {
acc += i;
}
return acc;
}
";
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "i < n", 1, 0));
    assert!(md.contains("i: i32"), "{md}");
    // the counter is visible in the update too
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "i += 1", 1, 0));
    assert!(md.contains("i: i32"), "{md}");
}

#[test]
fn loop_header_const_renders_in_the_hover() {
    // the header's `const` rides the binding record — the hover shows
    // the binding as it binds; a plain loop var stays unadorned
    let mut s = S::boot();
    let src = "\
struct Task {
done: bool;
}
fn flip(tasks: [Task]) -> nil {
for (const t of tasks) {
t.done = true;
}
for (let u of tasks) {
u.done = u.done;
}
}
";
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "t.done = true", 1, 0));
    assert!(md.contains("const t: Task"), "{md}");
    assert!(md.contains("loop variable"), "{md}");
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "u.done = u.done", 1, 0));
    assert!(md.contains("u: Task"), "{md}");
    assert!(!md.contains("const"), "no `const` without the flag: {md}");
    // the let binding renders its head keyword too
    let src = "\
fn f() -> i32 {
const base = 10;
let acc = 0;
for (let i = 0; i < base; i += 1) {
acc += i;
}
return acc;
}
";
    let md = markdown(s.hover_in("file:///t/hov2.rut", src, "i < base", 1, 4));
    assert!(md.contains("const base: i32"), "{md}");
    let md = markdown(s.hover_in("file:///t/hov2.rut", src, "acc += i", 1, 0));
    assert!(md.contains("let acc: i32"), "{md}");
    let md = markdown(s.hover_in("file:///t/hov2.rut", src, "i += 1", 1, 0));
    assert!(md.contains("i: i32"), "{md}");
    assert!(!md.contains("const i"), "for-c carries no const: {md}");
}

#[test]
fn shadowing_resolves_to_the_nearest_latest() {
    // an inner re-binding wins inside its scope; the outer binding
    // resumes after the block ends
    let mut s = S::boot();
    let src = "\
class Circle {
r: f64;
}
struct Dot {
x: f64;
}
fn go(c: Circle) -> f64 {
let x = c;
if true {
let x = Dot { x: 1.0 };
return x.x;
}
return 0.0;
}
";
    // inside the if: the Dot binding (declared later, scope contains)
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "return x.x", 1, 7));
    assert!(md.contains("let x: Dot"), "{md}");
    // the literal's label — still the Dot binding
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "x: 1.0", 1, 0));
    assert!(md.contains("let x: Dot"), "{md}");
}

#[test]
fn scope_exit_restores_the_outer_binding() {
    let mut s = S::boot();
    let src = "\
class Circle {
r: f64;
}
struct Dot {
x: f64;
}
fn go(c: Circle) -> f64 {
let x = c;
if true {
let x = Dot { x: 1.0 };
}
return x.r;
}
";
    // after the if block: the inner Dot binding's scope ended — the
    // `x` in `return x.r` is the outer Circle binding again
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "return x.r", 1, 7));
    assert!(md.contains("let x: Circle"), "{md}");
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "x.r", 1, 2));
    assert!(md.contains("r: f64"), "{md}");
}

#[test]
fn hover_at_the_end_of_buffer_still_resolves_the_receiver() {
    // the end-of-buffer scope fix's blast-radius guard: the doc ends
    // at the incomplete statement, the cursor sits ON the block span's
    // hi — the fn body's scope extends through the source end
    let mut s = S::boot();
    let src = "class List {\n    items: i32;\n}\nimpl List {\n    fn count(self) -> i32 { return 0; }\n}\nfn go() -> nil {\n    let list = List.new();\n    return list.count;";
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "return list.count", 1, 8));
    assert!(md.contains("let list: List"), "{md}");
    // the member after the dot rides the same resolution
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "return list.count", 1, 12));
    assert!(md.contains("fn count(self) -> i32"), "{md}");
}

#[test]
fn binding_decl_span_is_byte_exact() {
    // the span-first law: hovering a use decorates exactly the use
    // token — the envelope's range is the hovered ident's extent
    let mut s = S::boot();
    let src = "fn go() -> nil {\n    let total = 1;\n    let u = total;\n}\n";
    let h = s.hover_in("file:///t/hov.rut", src, "let u = total", 1, 8);
    let want = byte_of(src, "total", 2);
    assert_eq!(
        span_of(src, &h),
        (want as usize, want as usize + 5),
        "the hover decorates the use token: {h}"
    );
}

#[test]
fn primitive_type_token_hover() {
    // M7: `i32` in type position — a static blurb, width + range
    let mut s = S::boot();
    let src = "fn f(x: i32) -> i32 { return x; }\n";
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "i32", 1));
    assert!(md.contains("i32"), "{md}");
    assert!(md.contains("signed 32-bit"), "{md}");
    assert!(md.contains("-2147483648 ..= 2147483647"), "{md}");
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "i32", 2));
    assert!(md.contains("signed 32-bit"), "{md}");
}

#[test]
fn primitive_bool_and_float_hover() {
    let mut s = S::boot();
    let src = "fn f(b: bool, w: f64) -> f64 { return w; }\n";
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "bool", 1));
    assert!(md.contains("true` / `false"), "{md}");
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "f64", 1));
    assert!(md.contains("binary64"), "{md}");
}

#[test]
fn surface_primitives_keep_their_rich_hover() {
    // `str`/`bytes`/`opaque` have surface decls — those must win over
    // the static blurb (the `primitive str {` law)
    let mut s = S::boot();
    s.index_file(
        "file:///t/core.d.rut",
        "prelude builtin primitive str {\nfn len(self) -> i32;\n}\n",
    );
    let src = "fn f(s: str) -> str { return s; }\n";
    let md = markdown(s.hover_at("file:///t/hov.rut", src, "str", 1));
    assert!(md.contains("primitive str {"), "{md}");
}

#[test]
fn lambda_params_scope_to_the_lambda() {
    let mut s = S::boot();
    let src = "\
class Circle {
r: f64;
}
fn go(cs: [Circle]) -> f64 {
let f = fn(c: Circle) -> f64 { return c.r; };
return 0.0;
}
";
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "return c.r", 1, 7));
    assert!(md.contains("c: Circle"), "{md}");
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "return c.r", 1, 9));
    assert!(md.contains("r: f64"), "{md}");
}

#[test]
fn self_receiver_field_read_infers() {
    // `self.r` inside an inherent impl — the receiver chain rides the
    // enclosing type
    let mut s = S::boot();
    let src = "\
class Circle {
r: f64;
}
impl Circle {
fn grown(self, k: f64) -> Circle { return self; }
fn twice(self) -> f64 {
let rr = self.r;
return rr;
}
}
";
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "let rr", 1, 4));
    assert!(md.contains("let rr: f64"), "{md}");
}

// ---- file modules: the namespace edges ----

/// the gadget dep tree, stamped through the scan lane the way the
/// faces' mount lays it down
fn mount_gadgets(s: &mut S) {
    s.index_mod_at(
        "file:///ws/gadgets/mod.rut",
        "gadgets",
        "",
        "pub mod layout;\nmod secret;\npub fn tag() -> i32 { return 1; }\n",
    );
    s.index_mod_at(
        "file:///ws/gadgets/layout/mod.rut",
        "gadgets",
        "layout",
        "pub mod grid;\npub struct Column {\n    w: i32;\n}\n",
    );
    s.index_mod_at(
        "file:///ws/gadgets/layout/grid/mod.rut",
        "gadgets",
        "layout/grid",
        "pub struct Cell {\n    x: i32;\n}\n",
    );
    s.index_mod_at(
        "file:///ws/gadgets/secret/mod.rut",
        "gadgets",
        "secret",
        "pub fn open() -> i32 { return 1; }\n",
    );
}

#[test]
fn qualified_type_positions_render_with_colon2() {
    // `ty_src` joins a type path's segments with `::` — the
    // name-resolution law's spelling (a dotted join would read value
    // access into the annotation)
    let mut s = S::boot();
    let src = "mod layout;\nentry fn main() -> nil {\n    let p: layout::Point = nil;\n}\n";
    let md = markdown(s.hover_in("file:///t/hov.rut", src, "let p:", 1, 4));
    assert!(md.contains("let p: layout::Point"), "{md}");
}

both_uri_shapes! {
#[test]
fn mod_decl_hover_shows_kind_mount_and_children() {
    // `pub mod layout;` in the doc — kind, the mounted file, the
    // child count read off the mounted index in the chain
    let mut s = S::boot();
    mount_gadgets(&mut s);
    let doc = "pub mod layout;\nmod secret;\nentry fn main() -> nil { }\n";
    let uri = "file:///ws/gadgets/main.rut";
    // stamp the uri (the index lane's push) so the own-package edges key
    s.index_mod_at(uri, "gadgets", "", doc);
    let md = markdown(s.hover_at(uri, doc, "layout", 1));
    assert!(md.contains("pub mod layout;"), "{md}");
    assert!(md.contains("file module — `layout/mod.rut`"), "{md}");
    assert!(md.contains("1 child module"), "{md}");
    // the private edge spells `mod`, the mounted twin has no children
    let md = markdown(s.hover_at(uri, doc, "secret", 1));
    assert!(md.contains("\nmod secret;"), "{md}");
    assert!(md.contains("file module — `secret/mod.rut`"), "{md}");
    assert!(md.contains("no child modules"), "{md}");
}
}

#[test]
fn use_segment_hover_resolves_through_the_dep_tree() {
    // `layout` inside `use gadgets::layout::{ Column };` — the
    // resolved module (kind, mounted file, child count)
    let mut s = S::boot();
    mount_gadgets(&mut s);
    let doc = "use gadgets::layout::{ Column };\nentry fn main() -> nil {\n    let c: Column<i32> = Column { w: 1 };\n}\n";
    let md = markdown(s.hover_at("file:///t/hov.rut", doc, "layout", 1));
    assert!(md.contains("layout/mod.rut"), "{md}");
    assert!(md.contains("file module"), "{md}");
    assert!(md.contains("1 child module"), "{md}");
    // the pkg head resolves the dep's root module
    let md = markdown(s.hover_at("file:///t/hov.rut", doc, "gadgets", 1));
    assert!(md.contains("mod.rut") && md.contains("file module"), "{md}");
    assert!(
        md.contains("2 child modules"),
        "the root mounts layout + secret: {md}"
    );
    // a use the chain cannot walk: no hover, never wrong text
    let doc2 = "use gadgets::secret::open;\nentry fn main() -> nil { }\n";
    let h = s.hover_at("file:///t/hov2.rut", doc2, "secret", 1);
    assert!(no_hover(&h), "a non-pub edge resolves to no hover: {h}");
}

#[test]
fn use_leaf_through_mods_hovers() {
    // the leaf name still hovers its type through the use graph
    let mut s = S::boot();
    mount_gadgets(&mut s);
    let doc = "use gadgets::layout::{ Column };\nentry fn main() -> nil {\n    let c: Column<i32> = Column { w: 1 };\n}\n";
    let md = markdown(s.hover_at("file:///t/hov.rut", doc, "Column", 2));
    assert!(md.contains("struct Column"), "{md}");
}

#[test]
fn host_fn_surface_signature_cuts_at_its_own_semicolon() {
    // a bodiless fn's parser span runs to the NEXT decl's end — the
    // verbatim slice must cut at the item's OWN `;`, or `pub host fn
    // abs(x: f64) -> f64;` rendered `...f64;pub` in the detail
    let mut s = S::boot();
    let src = "pub host fn abs(x: f64) -> f64;\npub host fn ceil(x: f64) -> f64;\n";
    s.index_file("file:///t/surf/surf.d.rut", src);
    let call =
        "use surf::{ abs, ceil };\nfn go(x: f64) -> f64 {\n    let a = abs(x);\n    let b = ceil(a);\n    return b;\n}\n";
    let md = markdown(s.hover_at("file:///t/call.rut", call, "abs", 1));
    assert!(
        md.contains("pub host fn abs(x: f64) -> f64"),
        "no `;pub` artifact: {md}"
    );
    assert!(!md.contains(";pub"), "the next-decl artifact leaked: {md}");
    let md = markdown(s.hover_at("file:///t/call.rut", call, "ceil", 1));
    assert!(
        md.contains("pub host fn ceil(x: f64) -> f64"),
        "each row cuts at its own `;`: {md}"
    );
}

both_uri_shapes! {
#[test]
fn host_fn_hover_via_the_calc_fixture_has_no_next_decl_artifact() {
    // calc rides the PUBLIC dep lane — the repo's own source mounted
    // as a fixture (the root shell + the math module beside it),
    // exactly what the walk indexes; the hover markdown carries the
    // clean signature only
    let mut s = S::boot();
    s.index_mod_at(
        "rut/calc/src/mod.rut",
        "calc",
        "",
        include_str!("../../../rut/calc/src/mod.rut"),
    );
    s.index_mod_at(
        "rut/calc/src/math.rut",
        "calc",
        "math",
        include_str!("../../../rut/calc/src/math.rut"),
    );
    let src = "use calc::math::{ abs };\nfn go(x: f64) -> f64 {\n    return abs(x);\n}\n";
    let h = s.hover_at("file:///t/hov.rut", src, "abs", 1);
    let md = markdown(h);
    assert!(
        md.contains("fn abs<T requires f32 | f64>(x: T) -> T"),
        "{md}"
    );
    assert!(!md.contains(";pub"), "the next-decl artifact leaked: {md}");
}
}

#[test]
fn decl_site_hovers_decorate_the_name_token() {
    // the round-2 pin: hovering a DECLARED name decorates exactly the
    // name token — the let head, a param, a fn decl, a type decl —
    // never the whole statement/decl extent
    let mut s = S::boot();
    let src = "\
fn go(x: i32) -> i32 {
let total = x;
return total;
}
struct Point {
x: f64;
}
";
    let uri = "file:///t/hov.rut";
    for (needle, occ, off, len) in [
        ("let total", 1, 4, 5),    // the let's declared name
        ("fn go(", 1, 3, 2),       // the fn decl's name
        ("fn go(", 1, 6, 1),       // the param's declared name
        ("struct Point", 1, 7, 5), // the struct decl's name
        ("x: f64", 1, 0, 1),       // the field decl's name
    ] {
        let h = s.hover_in(uri, src, needle, occ, off);
        let want = byte_of(src, needle, occ) + off;
        assert_eq!(
            span_of(src, &h),
            (want as usize, want as usize + len),
            "`{needle}` must decorate the hovered token: {h}"
        );
    }
}

#[test]
fn member_hovers_decorate_the_member_token() {
    // the round-3 pin: hovering a member USE — a method call, a field
    // read — decorates exactly the member token, never the whole
    // call/field-access node
    let mut s = S::boot();
    let src = "class Circle {\nr: f64;\n}\nimpl Circle {\nfn area(self) -> f64 { return self.r; }\n}\nfn go(c: Circle) -> f64 {\nreturn c.area(1.0) + c.r;\n}\n";
    let uri = "file:///t/hov.rut";
    let h = s.hover_in(uri, src, "c.area", 1, 2);
    let want = byte_of(src, "c.area", 1) + 2;
    assert_eq!(span_of(src, &h), (want as usize, want as usize + 4), "{h}");
    let h = s.hover_in(uri, src, "+ c.r", 1, 4);
    let want = byte_of(src, "+ c.r", 1) + 4;
    assert_eq!(span_of(src, &h), (want as usize, want as usize + 1), "{h}");
}

#[test]
fn await_unwrap_hover_agrees_with_the_hint() {
    // the one-source law over the Future family: the hint's `: bytes`
    // IS the hover's binding line (the tooltip renders the markdown)
    let mut s = S::boot();
    let src = "class Response {\nn: i32;\n}\nimpl Response {\npub fn body(self) -> Future<bytes> { return nil; }\n}\n\
               async fn work(r: Response) -> bytes {\nconst b = await r.body();\nreturn b;\n}\n";
    let uri = "file:///t/hov.rut";
    let md = markdown(s.hover_in(uri, src, "const b = await", 1, 6));
    assert!(md.contains("const b: bytes"), "{md}");
    assert!(md.contains("type inferred"), "{md}");
    // the use site answers the same text
    let md = markdown(s.hover_in(uri, src, "return b;", 1, 7));
    assert!(md.contains("const b: bytes"), "{md}");
}

#[test]
fn the_async_family_hovers_engine_true_texts() {
    // an unawaited async fn call, an async block's returns, and the
    // chained await — each hover types the binding the way the engine
    // values it
    let mut s = S::boot();
    let src = "async fn get() -> i32 { return 1; }\nentry fn main() -> nil {\nconst f = get();\nconst g = async { return 5; };\nconst v = await f;\n}\n";
    let uri = "file:///t/hov.rut";
    let md = markdown(s.hover_in(uri, src, "const f = get", 1, 6));
    assert!(md.contains("const f: Future<i32>"), "{md}");
    let md = markdown(s.hover_in(uri, src, "const g = async", 1, 6));
    assert!(md.contains("const g: Future<i32>"), "{md}");
    let md = markdown(s.hover_in(uri, src, "const v = await", 1, 6));
    assert!(md.contains("const v: i32"), "{md}");
}

both_uri_shapes! {
#[test]
fn member_hover_gates_the_foreign_privates() {
    // the member-vis law on hover: a foreign class's private field is
    // a MISS (the compile refuses it — Act 1), the pub impl method
    // still renders, and the doc's own private field still hovers
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/http/mod.rut",
        "http",
        "",
        "pub class RequestBuilder {\nheaders: str;\n}\n\
         impl RequestBuilder {\npub fn build(self) -> i32 { return 0; }\n}\n\
         pub fn builder() -> RequestBuilder { return nil; }\n",
    );
    let src = "use http::{ builder };\nfn go() -> nil {\n    let b = builder();\n    b.headers;\n    b.build();\n}\n";
    let h = s.hover_in("file:///t/main.rut", src, "b.headers", 1, 2);
    assert!(
        no_hover(&h),
        "a private foreign field hovers nothing: {h:?}"
    );
    let md = markdown(s.hover_in("file:///t/main.rut", src, "b.build", 1, 2));
    assert!(md.contains("pub fn build(self) -> i32"), "{md}");

    // the doc's own private field: the declaring file IS the site —
    // the dial passes and the member hover answers
    let own =
        "class C {\nhidden: i32;\n}\nimpl C {\nfn read(self) -> i32 { return self.hidden; }\n}\n";
    let md = markdown(s.hover_in("file:///t/own.rut", own, "self.hidden", 1, 5));
    assert!(md.contains("hidden"), "own privates still hover: {md}");
}
}

// ---- qualified paths: the module namespace lane ----

/// the layout pkg tree — the driver's mixed-tree shape (compiles with
/// bare `mod` edges and qualified access from the root): the root
/// stamps as the pkg's own, the children ride their mod paths
fn mount_layout_pkg(s: &mut S) {
    s.index_mod_at(
        "file:///ws/pkg/src/layout/mod.rut",
        "pkg",
        "layout",
        "mod grid;\npub fn span() -> i32 { return 1; }\npub struct Point {\n    x: i32;\n}\n",
    );
    s.index_mod_at(
        "file:///ws/pkg/src/layout/grid.rut",
        "pkg",
        "layout/grid",
        "pub fn cell() -> i32 { return 2; }\n",
    );
}

both_uri_shapes! {
#[test]
fn qualified_path_through_own_mod_edge_hovers() {
    // `layout::span()` with only `mod layout;` — the compiler resolves
    // it (the mixed-tree pin), so the file scope's module namespace
    // lane answers the walked module's own decl: the verbatim
    // signature + the declaring module's provenance
    let mut s = S::boot();
    mount_layout_pkg(&mut s);
    let doc = "mod layout;\nentry fn main() -> i32 { return layout::span(); }\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    let md = markdown(s.hover_in(uri, doc, "layout::span()", 1, 9));
    assert!(md.contains("pub fn span() -> i32"), "{md}");
    assert!(
        md.contains("layout/mod.rut"),
        "the module's provenance: {md}"
    );
    // the qualified path's own HEAD segment renders the module (the
    // mid-segment cursor rides the lane's target)
    let md = markdown(s.hover_in(uri, doc, "layout::span()", 1, 3));
    assert!(md.contains("file module"), "{md}");
    assert!(md.contains("layout/mod.rut"), "{md}");
}
}

#[test]
fn qualified_path_hovers_from_an_unstamped_doc() {
    // the open scratch-file shape: no stamp laid down — the doc's own
    // `mod layout;` edge reads off the DeclMap and the unnamed-owner
    // law finds the mounted child
    let mut s = S::boot();
    mount_layout_pkg(&mut s);
    let doc = "mod layout;\nentry fn main() -> i32 { return layout::span(); }\n";
    let md = markdown(s.hover_in("file:///t/scratch.rut", doc, "layout::span()", 1, 9));
    assert!(md.contains("pub fn span() -> i32"), "{md}");
}

#[test]
fn qualified_type_position_answers_through_the_lane() {
    // the grammar admits qualified TYPE paths (`let p: layout::Point`)
    // — the same lane answers type positions
    let mut s = S::boot();
    mount_layout_pkg(&mut s);
    let doc = "mod layout;\nentry fn main() -> nil { let p: layout::Point = nil; }\n";
    let md = markdown(s.hover_in("file:///ws/pkg/src/mod.rut", doc, "layout::Point", 1, 12));
    assert!(md.contains("pub struct Point"), "{md}");
}

both_uri_shapes! {
#[test]
fn qualified_path_through_use_bound_module_hovers() {
    // `math::sqrt(x)` with only `use calc::{ math };` — the use-bound
    // module opens its namespace (the cross-package pub walk gates)
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/calc/src/mod.rut",
        "calc",
        "",
        "pub mod math;\n",
    );
    s.index_mod_at(
        "file:///deps/calc/src/math.rut",
        "calc",
        "math",
        "pub fn sqrt(x: f64) -> f64 { return x; }\npub let PI: f64 = 3.14;\n",
    );
    let doc = "use calc::{ math };\nfn go(x: f64) -> f64 {\n    return math::sqrt(x);\n}\n";
    let md = markdown(s.hover_in("file:///t/main.rut", doc, "math::sqrt", 1, 9));
    assert!(md.contains("pub fn sqrt(x: f64) -> f64"), "{md}");
    assert!(md.contains("math.rut"), "the module's provenance: {md}");
    // the module's pub let: silent behind a use-bound module head
    // (module lets cross no package boundary through a qualified path)
    let doc2 = "use calc::{ math };\nfn go() -> f64 {\n    return math::PI;\n}\n";
    let h = s.hover_in("file:///t/main2.rut", doc2, "math::PI", 1, 7);
    assert!(
        no_hover(&h),
        "a module let is silent behind a use-bound head: {h}"
    );
}
}

both_uri_shapes! {
#[test]
fn qualified_path_walks_nested_modules() {
    // `layout::grid::cell()` — the walk crosses both mod edges and
    // answers in the deepest module's file scope
    let mut s = S::boot();
    mount_layout_pkg(&mut s);
    let doc = "mod layout;\nentry fn main() -> i32 { return layout::grid::cell(); }\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    let md = markdown(s.hover_in(uri, doc, "layout::grid::cell", 1, 15));
    assert!(md.contains("pub fn cell() -> i32"), "{md}");
    assert!(md.contains("grid.rut"), "{md}");
}
}

both_uri_shapes! {
#[test]
fn qualified_path_through_the_own_pkg_head_hovers() {
    // the stamped doc's own package name opens its root module — the
    // own lane's pkg-head rule (`pkg::layout::span()`)
    let mut s = S::boot();
    mount_layout_pkg(&mut s);
    let doc = "mod layout;\nentry fn main() -> i32 { return pkg::layout::span(); }\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    let md = markdown(s.hover_in(uri, doc, "pkg::layout::span", 1, 15));
    assert!(md.contains("pub fn span() -> i32"), "{md}");
}
}

both_uri_shapes! {
#[test]
fn qualified_path_without_the_binding_answers_nothing() {
    // scope-true: the file binds no `layout` (no mod edge, no use) —
    // the qualified spelling answers NOTHING, and a same-named fn in
    // an unrelated sibling module never leaks in
    let mut s = S::boot();
    s.index_mod_at(
        "file:///ws/pkg/src/layout/mod.rut",
        "pkg",
        "layout",
        "pub fn span() -> i32 { return 1; }\n",
    );
    s.index_file(
        "file:///ws/other/lib.rut",
        "pub fn span() -> i32 { return 9; }\n",
    );
    let doc = "entry fn main() -> i32 { return layout::span(); }\n";
    let h = s.hover_in("file:///t/scratch.rut", doc, "layout::span()", 1, 9);
    assert!(
        no_hover(&h),
        "an unbound qualified head answers nothing: {h}"
    );
}
}

both_uri_shapes! {
#[test]
fn pkg_rooted_dep_name_answers_through_the_lane() {
    // HIGH 1: `calc::math::sqrt(x)` with calc ONLY a declared dep —
    // the manifest's own naming, no use anywhere — the compiler
    // answers it (the fully-qualified-module-path pin), so the lane
    // does too. The tree-carrying rule rides along: a FLAT dep roots
    // nothing (the package-head-in-a-position pin). The type cell
    // REFUSES in the compiler (the type ladder walks the own family
    // only) — silent here too; the non-pub leaf stays silent
    // cross-package.
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/calc/src/mod.rut",
        "calc",
        "",
        "pub mod math;\npub let ROOT: i32 = 1;\n",
    );
    s.index_mod_at(
        "file:///deps/calc/src/math.rut",
        "calc",
        "math",
        "pub fn sqrt(x: f64) -> f64 { return x; }\npub struct Angle {\n    deg: i32;\n}\npub let PI2: f64 = 6.28;\nfn hidden() -> i32 { return 0; }\n",
    );
    let uri = "file:///t/main.rut";
    // the fn cell
    let doc = "entry fn main() -> f64 {\n    return calc::math::sqrt(4.0);\n}\n";
    let md = markdown(s.hover_in(uri, doc, "calc::math::sqrt", 1, 13));
    assert!(md.contains("pub fn sqrt(x: f64) -> f64"), "{md}");
    assert!(md.contains("math.rut"), "the module's provenance: {md}");
    // the let cell: module lets cross NO package boundary through a
    // qualified path (the compiler refuses `mathkit::advanced::SCALE`
    // and the use-bound `advanced::SCALE` in every lane) — silent
    let doc3 = "entry fn main() -> f64 {\n    return calc::math::PI2;\n}\n";
    let h = s.hover_in(uri, doc3, "calc::math::PI2", 1, 13);
    assert!(
        no_hover(&h),
        "a module let is silent behind a dep head: {h}"
    );
    // the type cell: the type ladder takes no dep head
    let doc2 = "entry fn main() -> i32 {\n    let a: calc::math::Angle = nil;\n    return 0;\n}\n";
    let h = s.hover_in(uri, doc2, "calc::math::Angle", 1, 14);
    assert!(no_hover(&h), "the type ladder takes no dep head: {h}");
    // the non-pub leaf is silent cross-package
    let doc4 = "entry fn main() -> i32 {\n    return calc::math::hidden();\n}\n";
    let h = s.hover_in(uri, doc4, "calc::math::hidden", 1, 13);
    assert!(no_hover(&h), "a non-pub leaf is silent cross-package: {h}");
    // a FLAT dep roots nothing
    s.index_mod_at(
        "file:///deps/flat/src/lib.rut",
        "flatdep",
        "",
        "pub fn top() -> i32 { return 10; }\n",
    );
    let flatdoc = "entry fn main() -> i32 {\n    return flatdep::top();\n}\n";
    let h = s.hover_in("file:///t/flat.rut", flatdoc, "flatdep::top", 1, 10);
    assert!(no_hover(&h), "a flat dep roots nothing: {h}");
}
}

both_uri_shapes! {
#[test]
fn qualified_leaves_are_the_module_own_decls_only() {
    // HIGH 2: a CROSS-package qualified leaf is DECLS-ONLY — the
    // target's use edges never open its qualified surface (the
    // compiler: "`two` is not declared in module `layout` — it lives
    // in module `helper`"). A PRIVATE use, a `pub use` of a
    // DEPENDENCY name (the surface row is the USE lane's — the
    // consumer's `use layout::{ two }` binds it; a qualified path
    // never chases it across the boundary), and an ambient/prelude
    // name all answer NOTHING; a non-pub leaf is silent from outside
    // the declaring subtree.
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/helper/src/mod.rut",
        "helper",
        "",
        "pub fn two() -> i32 { return 20; }\n",
    );
    const LAYOUT_URI: &str = "file:///ws/pkg/src/layout/mod.rut";
    const ROOT_URI: &str = "file:///ws/pkg/src/mod.rut";
    let variants = [
        (
            "private use",
            "use helper::{ two };\npub fn span() -> i32 { return 1; }\n",
        ),
        (
            "pub use",
            "pub use helper::{ two };\npub fn span() -> i32 { return 1; }\n",
        ),
    ];
    let doc = "mod layout;\nentry fn main() -> i32 { return layout::two(); }\n";
    for (tag, layout_src) in variants {
        s.index_mod_at(LAYOUT_URI, "pkg", "layout", layout_src);
        let h = s.hover_in(ROOT_URI, doc, "layout::two()", 1, 9);
        assert!(no_hover(&h), "[{tag}] a use-row leaf is silent: {h}");
        let hits = s.def_in(ROOT_URI, doc, "layout::two()", 1, 9);
        assert!(
            hits.is_empty(),
            "[{tag}] no jump through a use row: {hits:?}"
        );
    }
    // the ambient/prelude name of the target: not declared there
    s.index_mod_at(
        LAYOUT_URI,
        "pkg",
        "layout",
        "pub fn span() -> i32 { return 1; }\npub let LIMIT: i32 = 5;\nfn internal() -> i32 { return 2; }\n",
    );
    let amb = "mod layout;\nentry fn main() -> i32 { return layout::keep(1); }\n";
    let h = s.hover_in(ROOT_URI, amb, "layout::keep", 1, 9);
    assert!(
        no_hover(&h),
        "an ambient name is no leaf of the module: {h}"
    );
    // the non-pub leaf from OUTSIDE the declaring subtree
    let internal = "mod layout;\nentry fn main() -> i32 { return layout::internal(); }\n";
    let h = s.hover_in(ROOT_URI, internal, "layout::internal", 1, 9);
    assert!(no_hover(&h), "a private leaf is silent from outside: {h}");
}
}

both_uri_shapes! {
#[test]
fn qualified_leaf_inside_the_declaring_subtree_answers() {
    // the visibility matrix's inside-the-subtree cell: grid's own
    // file reads its own bare fn through the qualified spelling —
    // the gate allows the reader inside the declaring subtree (the
    // compiler compiles it)
    let mut s = S::boot();
    s.index_mod_at(
        "file:///ws/pkg/src/layout/mod.rut",
        "pkg",
        "layout",
        "pub mod grid;\n",
    );
    let grid =
        "fn secret() -> i32 { return 1; }\npub fn local() -> i32 { return grid::secret(); }\n";
    let grid_uri = "file:///ws/pkg/src/layout/grid.rut";
    s.index_mod_at(grid_uri, "pkg", "layout/grid", grid);
    let md = markdown(s.hover_in(grid_uri, grid, "grid::secret", 1, 9));
    assert!(md.contains("fn secret() -> i32"), "{md}");
}
}

both_uri_shapes! {
#[test]
fn qualified_tail_never_falls_through_to_the_file_own_name() {
    // the nastiest scope-false shape: the doc DECLARES its own
    // `fn span` and binds no `layout` — the qualified spelling's tail
    // must answer NOTHING (never the file's own same-named fn: an
    // authoritative null, not a fall-through the bare lanes would
    // turn into the doc's own decl)
    let mut s = S::boot();
    s.index_mod_at(
        "file:///ws/pkg/src/layout/mod.rut",
        "pkg",
        "layout",
        "pub fn span() -> i32 { return 1; }\n",
    );
    let doc = "fn span() -> i32 { return 9; }\nentry fn main() -> i32 { return layout::span(); }\n";
    let uri = "file:///t/main.rut";
    let h = s.hover_in(uri, doc, "layout::span()", 1, 9);
    assert!(no_hover(&h), "an unbound head answers nothing: {h}");
    let hits = s.def_in(uri, doc, "layout::span()", 1, 9);
    assert!(
        hits.is_empty(),
        "never the file's own same-named fn: {hits:?}"
    );
}
}

both_uri_shapes! {
#[test]
fn qualified_path_through_an_ancestor_edge_hovers() {
    // the ancestor's edge opens the sibling module (the own lane's
    // walk; the compiler compiles the sibling spelling from a child)
    // — and the sibling's PRIVATE leaf stays silent (private to its
    // own module, the visibility matrix's refusal)
    let mut s = S::boot();
    s.index_mod_at(
        "file:///ws/app/src/helpers.rut",
        "app",
        "helpers",
        "mod other;\npub mod deep;\n",
    );
    s.index_mod_at(
        "file:///ws/app/src/helpers/other.rut",
        "app",
        "helpers/other",
        "pub fn thing() -> i32 { return 3; }\n",
    );
    let deep = "pub fn dig() -> i32 { return other::thing(); }\n";
    let deep_uri = "file:///ws/app/src/helpers/deep.rut";
    s.index_mod_at(deep_uri, "app", "helpers/deep", deep);
    let md = markdown(s.hover_in(deep_uri, deep, "other::thing", 1, 9));
    assert!(md.contains("pub fn thing() -> i32"), "{md}");
    // the private sibling leaf
    s.index_mod_at(
        "file:///ws/app/src/helpers/other.rut",
        "app",
        "helpers/other",
        "fn thing() -> i32 { return 3; }\n",
    );
    let h = s.hover_in(deep_uri, deep, "other::thing", 1, 9);
    assert!(no_hover(&h), "a sibling's private leaf is silent: {h}");
}
}

/// the counter module world — the receiver-inference shapes' fixture
/// (the driver's D2 world: the compiler accepts every shape probed)
fn mount_counter_pkg(s: &mut S) {
    s.index_mod_at(
        "file:///ws/pkg/src/m.rut",
        "pkg",
        "m",
        "pub struct Counter {\n    n: i32;\n}\nimpl Counter {\n    pub fn bump(self) -> i32 { return self.n + 1; }\n}\npub fn mk(n: i32) -> Counter { return Counter { n: n }; }\n",
    );
}

both_uri_shapes! {
#[test]
fn qualified_annotation_receiver_resolves_members() {
    // `let c: m::Counter = m::mk(41); c.bump()` — the annotation
    // resolves through the type ladder's qualified lane to the
    // declaring view, the bare name rides the computed-head lane,
    // and the member answers (the compiler compiles the shape)
    let mut s = S::boot();
    mount_counter_pkg(&mut s);
    let doc = "mod m;\nentry fn main() -> i32 {\n    let c: m::Counter = m::mk(41);\n    return c.bump();\n}\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    let md = markdown(s.hover_in(uri, doc, "c.bump();", 1, 2));
    assert!(md.contains("pub fn bump(self) -> i32"), "{md}");
    assert!(md.contains("in `Counter`"), "the owner: {md}");
}
}

both_uri_shapes! {
#[test]
fn qualified_inferred_receiver_resolves_members() {
    // `let c = m::mk(41); c.bump()` — the qualified callee's written
    // ret types the binding (`Counter`), the member answers
    let mut s = S::boot();
    mount_counter_pkg(&mut s);
    let doc = "mod m;\nentry fn main() -> i32 {\n    let c = m::mk(41);\n    return c.bump();\n}\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    // the binding itself types as Counter
    let md = markdown(s.hover_in(uri, doc, "let c = m::mk", 1, 4));
    assert!(md.contains("let c: Counter"), "{md}");
    assert!(md.contains("type inferred"), "{md}");
    // the member rides the same resolution
    let md = markdown(s.hover_in(uri, doc, "c.bump();", 1, 2));
    assert!(md.contains("pub fn bump(self) -> i32"), "{md}");
    assert!(md.contains("in `Counter`"), "{md}");
}
}

both_uri_shapes! {
#[test]
fn qualified_chained_receiver_resolves_members() {
    // `m::mk(41).bump()` — the chained callee's ret drives the member
    let mut s = S::boot();
    mount_counter_pkg(&mut s);
    let doc = "mod m;\nentry fn main() -> i32 {\n    return m::mk(41).bump();\n}\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    let md = markdown(s.hover_in(uri, doc, ".bump();", 1, 1));
    assert!(md.contains("pub fn bump(self) -> i32"), "{md}");
    assert!(md.contains("in `Counter`"), "{md}");
}
}

both_uri_shapes! {
#[test]
fn use_bound_bare_call_receiver_resolves_members() {
    // `use m::{ mk }; let c = mk(41); c.bump()` — the bare callee's
    // ret is a sibling-module type; the member answers through the
    // computed-head lane
    let mut s = S::boot();
    mount_counter_pkg(&mut s);
    let doc = "mod m;\nuse m::{ mk };\nentry fn main() -> i32 {\n    let c = mk(41);\n    return c.bump();\n}\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    let md = markdown(s.hover_in(uri, doc, "c.bump();", 1, 2));
    assert!(md.contains("pub fn bump(self) -> i32"), "{md}");
    assert!(md.contains("in `Counter`"), "{md}");
}
}

both_uri_shapes! {
#[test]
fn unknown_member_on_a_resolved_receiver_stays_silent() {
    // the negative: the receiver resolves, the member does not exist
    // — the member lane's None is FINAL (never the bare-name lanes)
    let mut s = S::boot();
    mount_counter_pkg(&mut s);
    let doc = "mod m;\nentry fn main() -> i32 {\n    let c: m::Counter = m::mk(41);\n    return c.nonexist();\n}\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    let h = s.hover_in(uri, doc, "c.nonexist();", 1, 2);
    assert!(no_hover(&h), "an unknown member is silent: {h}");
    let hits = s.def_in(uri, doc, "c.nonexist();", 1, 2);
    assert!(hits.is_empty(), "no jump: {hits:?}");
}
}

both_uri_shapes! {
#[test]
fn field_typed_receiver_with_a_sibling_module_type_resolves_members() {
    // the field row: a qualified FIELD annotation (`c: m::Counter`)
    // types the field through the type ladder's qualified lane — the
    // compiler compiles the shape, the member answers
    let mut s = S::boot();
    s.index_mod_at(
        "file:///ws/pkg/src/m.rut",
        "pkg",
        "m",
        "pub struct Counter {\n    n: i32;\n}\nimpl Counter {\n    pub fn bump(self) -> i32 { return self.n + 1; }\n}\npub fn mk(n: i32) -> Counter { return Counter { n: n }; }\n",
    );
    let doc = "mod m;\nstruct Holder {\n    c: m::Counter;\n}\nentry fn main() -> i32 {\n    let h = Holder { c: m::mk(1) };\n    return h.c.bump();\n}\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    let md = markdown(s.hover_in(uri, doc, "h.c.bump();", 1, 4));
    assert!(md.contains("pub fn bump(self) -> i32"), "{md}");
    assert!(md.contains("in `Counter`"), "{md}");
}
}

both_uri_shapes! {
#[test]
fn is_cast_narrowed_receiver_resolves_members() {
    // the is-cast row (the cast's type operand is a SPELLING: the
    // bare `Counter` must be bound — the compiler refuses the
    // unbound shape). The member rides the binding's type.
    let mut s = S::boot();
    mount_counter_pkg(&mut s);
    let doc = "mod m;\nuse m::{ Counter };\nentry fn main() -> i32 {\n    let q = m::mk(41);\n    if (q is Counter) { return q.bump(); }\n    return 0;\n}\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    let md = markdown(s.hover_in(uri, doc, "q.bump();", 1, 2));
    assert!(md.contains("pub fn bump(self) -> i32"), "{md}");
    assert!(md.contains("in `Counter`"), "{md}");
}
}

both_uri_shapes! {
#[test]
fn generic_qualified_receiver_resolves_members() {
    // the generic row: a qualified generic annotation
    // (`b: m::Box2<i32>`) and a qualified generic call — the member
    // rides the substituted ret
    let mut s = S::boot();
    s.index_mod_at(
        "file:///ws/pkg/src/m.rut",
        "pkg",
        "m",
        "pub struct Box2<T> {\n    v: T;\n}\nimpl<T> Box2<T> {\n    pub fn get(self) -> T { return self.v; }\n}\npub fn mk_box<T>(v: T) -> Box2<T> { return Box2 { v: v }; }\n",
    );
    let doc = "mod m;\nentry fn main() -> i32 {\n    let b: m::Box2<i32> = m::mk_box(41);\n    return b.get();\n}\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    let md = markdown(s.hover_in(uri, doc, "b.get();", 1, 2));
    assert!(md.contains("pub fn get(self) -> T"), "{md}");
    assert!(md.contains("in `Box2`"), "{md}");
}
}

both_uri_shapes! {
#[test]
fn intra_package_module_let_answers_through_the_lane() {
    // the LET cell behind the own edge: module lets cross no package
    // boundary qualified, but INTRA-package heads answer them (the
    // compiler compiles `a::LIMIT`)
    let mut s = S::boot();
    s.index_mod_at(
        "file:///deps/gadgets/src/gadgets.rut",
        "gadgets",
        "",
        "pub mod core_mod;\n",
    );
    s.index_mod_at(
        "file:///deps/gadgets/src/core_mod.rut",
        "gadgets",
        "core_mod",
        "pub let LIMIT: i32 = 9;\npub fn tag() -> i32 { return LIMIT; }\n",
    );
    let doc = "mod core_mod;\nentry fn main() -> i32 {\n    return core_mod::LIMIT;\n}\n";
    let uri = "file:///t/main.rut";
    let md = markdown(s.hover_in(uri, doc, "core_mod::LIMIT", 1, 13));
    assert!(md.contains("pub let LIMIT: i32 = 9"), "{md}");
    let hits = s.def_in(uri, doc, "core_mod::LIMIT", 1, 13);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0]["uri"], "file:///deps/gadgets/src/core_mod.rut");
}
}

// ---- round 4: the rut-new layout — the root landing and the
//      intra-package use edge, through the extension's own mounts ----

both_uri_shapes! {
    #[test]
    fn the_own_pkg_head_from_a_child_lands_the_root_module() {
        // DEFECT 1: `app::LIMIT` from a child of the rut-new layout —
        // the compiler compiles it, and the own-package head names the
        // ROOT module. The open document's uri names the package too
        // (that is the layout); it must never land the head's root
        // lookup — the answer is the root's own decl, with the root
        // file's provenance
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let root = rut_new::at("src/mod.rut");
        let md = markdown(s.hover_in(
            &uri,
            "fn k() -> i32 { return app::LIMIT; }\n",
            "app::LIMIT",
            1,
            6,
        ));
        assert!(md.contains("pub let LIMIT: i32 = 7"), "{md}");
        assert!(
            md.contains(&format!("— {root}:4")),
            "the ROOT's decl, never the open document: {md}"
        );
        let md = markdown(s.hover_in(
            &uri,
            "fn k() -> i32 { return app::rootfn(); }\n",
            "app::rootfn",
            1,
            6,
        ));
        assert!(md.contains("pub fn rootfn() -> i32"), "{md}");
        let md = markdown(s.hover_in(
            &uri,
            "fn k(t: app::RootTy) -> i32 { return 0; }\n",
            "app::RootTy",
            1,
            6,
        ));
        assert!(md.contains("pub struct RootTy"), "{md}");
        // the root's private fn: a descendant sees its ancestors'
        // privates (the compiler compiles it)
        let md = markdown(s.hover_in(
            &uri,
            "fn k() -> i32 { return app::rootpriv(); }\n",
            "app::rootpriv",
            1,
            6,
        ));
        assert!(md.contains("fn rootpriv() -> i32"), "{md}");
        // ... and the module the root declares, behind the head
        let md = markdown(s.hover_in(
            &uri,
            "fn k() -> i32 { return app::m::mk(41).bump(); }\n",
            "app::m::mk",
            1,
            8,
        ));
        assert!(md.contains("pub fn mk(n: i32) -> Counter"), "{md}");
        assert!(
            md.contains(&format!("— {}:13", rut_new::at("src/m/mod.rut"))),
            "{md}"
        );
        // ... from a grandchild, the root's edge two levels up
        let sub = rut_new::at("src/child/sub/mod.rut");
        // one world: the child's buffer is the truth, and this test's
        // earlier probes synced other probe texts into it — its own
        // text rides again before the grandchild names it (the old
        // answer read the indexed copy behind the open buffer)
        s.analyze(&uri, rut_new::CHILD_HEAD);
        let md = markdown(s.hover_in(
            &sub,
            "fn k() -> i32 { return m::mk(41).bump(); }\n",
            "m::mk",
            1,
            3,
        ));
        assert!(md.contains("pub fn mk(n: i32) -> Counter"), "{md}");
        // ... and the ancestor itself, named through the root's edge
        let md = markdown(s.hover_in(&sub, "fn k() -> i32 { return child::kid(); }\n", "child::kid", 1, 8));
        assert!(md.contains("pub fn kid() -> i32"), "{md}");
        // ... and from the root document itself (it IS the root — its
        // own entry answers, the lane lands on it by its stamp)
        let root_doc = format!("{}fn k() -> i32 {{ return app::m::mk(41).bump(); }}\n", rut_new::ROOT);
        let md = markdown(s.hover_in(&root, &root_doc, "app::m::mk", 1, 8));
        assert!(md.contains("pub fn mk(n: i32) -> Counter"), "{md}");
        assert!(md.contains(&format!("— {}:13", rut_new::at("src/m/mod.rut"))), "{md}");
        // the own-family negatives the compiler refuses: a private
        // leaf of m from outside its subtree, the root's re-export row
        // (`pub use m::{ mk }` is no declaration of the root)
        for (src, needle, off) in [
            ("fn k() -> i32 { return app::m::secret(); }\n", "app::m::secret", 8),
            ("fn k() -> i32 { return app::mk(41).bump(); }\n", "app::mk", 6),
        ] {
            let h = s.hover_in(&uri, src, needle, 1, off);
            assert!(no_hover(&h), "[{needle:?}] a refused leaf hovers nothing: {h:?}");
        }
    }
}

both_uri_shapes! {
    #[test]
    fn a_root_declared_edge_used_from_a_child_hovers() {
        // DEFECT 1: `mod m;` declared by the ROOT, used from a child —
        // the compiler resolves the leaf through the ancestor walk
        // (`unknown name `m::mk`` was the pre-fix silence)
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let m = rut_new::at("src/m/mod.rut");
        let md = markdown(s.hover_in(
            &uri,
            "fn k() -> i32 { return m::mk(41).bump(); }\n",
            "m::mk",
            1,
            3,
        ));
        assert!(md.contains("pub fn mk(n: i32) -> Counter"), "{md}");
        assert!(md.contains(&format!("— {m}:13")), "{md}");
        let md = markdown(s.hover_in(&uri, "fn k() -> i32 { return m::STEP; }\n", "m::STEP", 1, 3));
        assert!(md.contains("pub let STEP: i32 = 2"), "{md}");
        let md = markdown(s.hover_in(
            &uri,
            "fn k() -> i32 { return m::pkgfn(); }\n",
            "m::pkgfn",
            1,
            3,
        ));
        assert!(md.contains("pub(pkg) fn pkgfn() -> i32"), "{md}");
        let md = markdown(s.hover_in(
            &uri,
            "fn k(c: m::Counter) -> i32 { return 0; }\n",
            "m::Counter",
            1,
            3,
        ));
        assert!(md.contains("pub struct Counter"), "{md}");
        // the nested module's fn, and the segments that name modules
        let src = "fn k() -> i32 { return m::grid::cell(); }\n";
        let md = markdown(s.hover_in(&uri, src, "m::grid::cell", 1, 9));
        assert!(md.contains("pub fn cell() -> i32"), "{md}");
        let md = markdown(s.hover_in(&uri, src, "m::grid::cell", 1, 0));
        assert!(md.contains("file module"), "the head is m: {md}");
        assert!(md.contains(&format!("— {m}:1")), "{md}");
        let md = markdown(s.hover_in(&uri, src, "m::grid::cell", 1, 3));
        assert!(md.contains("file module"), "the mid segment is grid: {md}");
        assert!(md.contains(&format!("— {}:1", rut_new::at("src/m/grid/mod.rut"))), "{md}");
        // the leaf kinds the compiler refuses through a qualified
        // position: a private of m from a sibling subtree, m's private
        // use rows, an ambient name (m's `pub use` row answers — the
        // re-export pins below)
        for (src, needle) in [
            ("fn k() -> i32 { return m::secret(); }\n", "m::secret"),
            ("fn k() -> i32 { return m::cell(); }\n", "m::cell"),
            ("fn k() -> i32 { return m::GK; }\n", "m::GK"),
            ("fn k() -> nil { m::panic(\"x\"); }\n", "m::panic"),
        ] {
            let h = s.hover_in(&uri, src, needle, 1, 3);
            assert!(no_hover(&h), "[{needle:?}] a refused leaf hovers nothing: {h:?}");
        }
    }
}

both_uri_shapes! {
    #[test]
    fn the_receiver_family_behind_a_root_declared_edge_hovers() {
        // the member lane behind the ROOT's edge: the annotated, the
        // inferred, the chained, and the field-typed receiver — all
        // compile, all answer (the inference's qualified hook reads
        // the doc's own stamp now)
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let m = rut_new::at("src/m/mod.rut");
        let bump = |s: &mut S, src: &'static str, needle: &'static str, off: u32| {
            let md = markdown(s.hover_in(&uri, src, needle, 1, off));
            assert!(md.contains("pub fn bump(self) -> i32"), "{md}");
            assert!(md.contains("in `Counter`"), "{md}");
            assert!(md.contains(&format!("— {m}:11")), "{md}");
        };
        bump(
            &mut s,
            "fn k() -> i32 { let c: m::Counter = m::mk(41); return c.bump(); }\n",
            ".bump()",
            1,
        );
        let src = "fn k() -> i32 { let c = m::mk(41); return c.bump(); }\n";
        bump(&mut s, src, ".bump()", 1);
        let md = markdown(s.hover_in(&uri, src, "let c", 1, 4));
        assert!(md.contains("let c: Counter"), "the inferred binding: {md}");
        bump(
            &mut s,
            "fn k() -> i32 { return m::mk(41).bump(); }\n",
            ".bump()",
            1,
        );
        bump(
            &mut s,
            "struct Holder { c: m::Counter }\nfn k() -> i32 { let h = Holder { c: m::mk(1) }; return h.c.bump(); }\n",
            ".bump()",
            1,
        );
        // the `is`-test receiver, and the own-package head in the
        // annotation
        bump(
            &mut s,
            "use m::{ Counter };\nfn k() -> i32 { let q = m::mk(41); if (q is Counter) { return q.bump(); } return 0; }\n",
            "q.bump()",
            2,
        );
        bump(
            &mut s,
            "fn k() -> i32 { let c: app::m::Counter = app::m::mk(41); return c.bump(); }\n",
            ".bump()",
            1,
        );
        // a generic receiver
        let md = markdown(s.hover_in(
            &uri,
            "fn k() -> i32 { let b: m::Box2<i32> = m::mk_box(41); return b.get(); }\n",
            ".get()",
            1,
            1,
        ));
        assert!(md.contains("pub fn get(self) -> T"), "{md}");
        assert!(md.contains("in `Box2`"), "{md}");
        assert!(md.contains(&format!("— {m}:21")), "{md}");
    }
}

both_uri_shapes! {
    #[test]
    fn a_dep_roots_own_member_is_no_leaf_of_a_position() {
        // `calc::ctop()` — the dep root's OWN fn behind the pkg-rooted
        // name — refuses in the compiler ("unknown name `calc::ctop`",
        // a tree dep and a flat dep alike: the use is the only door to
        // a root's members); the lane roots INSIDE the dep's tree only
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let h = s.hover_in(
            &uri,
            "fn k() -> i32 { return calc::ctop(); }\n",
            "calc::ctop",
            1,
            6,
        );
        assert!(no_hover(&h), "the dep root's own member: {h:?}");
        // the control: the tree path answers
        let md = markdown(s.hover_in(
            &uri,
            "fn k() -> i32 { return calc::math::cmk(1).deg(); }\n",
            "calc::math::cmk",
            1,
            12,
        ));
        assert!(md.contains("pub fn cmk(d: i32) -> Calc"), "{md}");
        assert!(
            md.contains(&format!("— {}:13", rut_new::calc("src/math/mod.rut"))),
            "{md}"
        );
    }
}

both_uri_shapes! {
    #[test]
    fn an_intra_package_use_edge_hovers_through_the_module_lane() {
        // DEFECT 2: `use m::{ mk }` — the compiler's intra-package
        // import; the file scope's use tier answers it through the
        // module lane (the own-package head walks, the leaf is the
        // module's own visible decl) — at the call site AND on the
        // use leaf
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let m = rut_new::at("src/m/mod.rut");
        let grid = rut_new::at("src/m/grid/mod.rut");
        let root = rut_new::at("src/mod.rut");
        // a fn
        let src = "use m::{ mk };\nfn k() -> i32 { return mk(41).bump(); }\n";
        for (needle, off) in [("mk(41)", 0), ("{ mk }", 2)] {
            let md = markdown(s.hover_in(&uri, src, needle, 1, off));
            assert!(md.contains("pub fn mk(n: i32) -> Counter"), "[{needle}] {md}");
            assert!(md.contains(&format!("— {m}:13")), "[{needle}] {md}");
        }
        // a type — and the member behind it
        let src = "use m::{ Counter };\nfn k(c: Counter) -> i32 { return c.bump(); }\n";
        for (needle, off) in [("c: Counter", 3), ("{ Counter }", 2)] {
            let md = markdown(s.hover_in(&uri, src, needle, 1, off));
            assert!(md.contains("pub struct Counter"), "[{needle}] {md}");
        }
        let md = markdown(s.hover_in(&uri, src, ".bump()", 1, 1));
        assert!(md.contains("pub fn bump(self) -> i32"), "{md}");
        // a module let
        let src = "use m::{ STEP };\nfn k() -> i32 { return STEP; }\n";
        let md = markdown(s.hover_in(&uri, src, "return STEP", 1, 7));
        assert!(md.contains("pub let STEP: i32 = 2"), "{md}");
        let md = markdown(s.hover_in(&uri, src, "{ STEP }", 1, 2));
        assert!(md.contains("pub let STEP: i32 = 2"), "{md}");
        // nested: `use m::grid::{ cell }` — and the path's segments
        let src = "use m::grid::{ cell };\nfn k() -> i32 { return cell(); }\n";
        let md = markdown(s.hover_in(&uri, src, "cell()", 1, 0));
        assert!(md.contains("pub fn cell() -> i32"), "{md}");
        assert!(md.contains(&format!("— {grid}:1")), "{md}");
        let md = markdown(s.hover_in(&uri, src, "use m", 1, 4));
        assert!(md.contains("file module") && md.contains(&format!("— {m}:1")), "{md}");
        let md = markdown(s.hover_in(&uri, src, "m::grid:", 1, 3));
        assert!(md.contains("file module") && md.contains(&format!("— {grid}:1")), "{md}");
        // the spelled leaves: `use m::{ mk }` binds the module's
        // visible own decls
        for (leaf, call, needle, off, text) in [
            ("mk", "mk(41).bump()", "mk(41)", 0, "pub fn mk(n: i32) -> Counter"),
            ("STEP", "STEP", "return STEP", 7, "pub let STEP: i32 = 2"),
            ("pkgfn", "pkgfn()", "return pkgfn", 7, "pub(pkg) fn pkgfn() -> i32"),
        ] {
            let src = format!("use m::{{ {leaf} }};\nfn k() -> i32 {{ return {call}; }}\n");
            let md = markdown(s.hover_in(&uri, &src, needle, 1, off));
            assert!(md.contains(text), "[{call}] {md}");
        }
        // the own-package NAME as the use head, from a child
        let src = "use app::{ LIMIT };\nfn k() -> i32 { return LIMIT; }\n";
        let md = markdown(s.hover_in(&uri, src, "return LIMIT", 1, 7));
        assert!(md.contains("pub let LIMIT: i32 = 7"), "{md}");
        assert!(md.contains(&format!("— {root}:4")), "{md}");
        // ... and the ancestor's edge from a grandchild
        let sub = rut_new::at("src/child/sub/mod.rut");
        let src = "use m::{ mk };\nfn k() -> i32 { return mk(41).bump(); }\n";
        let md = markdown(s.hover_in(&sub, src, "mk(41)", 1, 0));
        assert!(md.contains("pub fn mk(n: i32) -> Counter"), "{md}");
    }
}

both_uri_shapes! {
    #[test]
    fn a_use_leaf_the_compiler_refuses_hovers_nothing() {
        // the negatives, each a use line the compiler refuses: a
        // private of the module (import is no visibility bypass), the
        // module's own private import; cross-package: a dep's private
        // fn and its private import. Silent at the call site AND on
        // the leaf.
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        for (src, needles) in [
            ("use m::{ secret };\nfn k() -> i32 { return secret(); }\n", vec![("secret()", 0), ("{ secret }", 2)]),
            ("use m::{ cell };\nfn k() -> i32 { return cell(); }\n", vec![("cell()", 0), ("{ cell }", 2)]),
            ("use m::*;\nfn k() -> i32 { return mk(41); }\n", vec![("mk(41)", 0)]),
            ("use calc::math::{ hidden };\nfn k() -> i32 { return 0; }\n", vec![("{ hidden }", 2)]),
            ("use calc::math::{ cell };\nfn k() -> i32 { return 0; }\n", vec![("{ cell }", 2)]),
        ] {
            for (needle, off) in needles {
                let h = s.hover_in(&uri, src, needle, 1, off);
                assert!(no_hover(&h), "[{src:?} @ {needle:?}] a refused leaf: {h:?}");
            }
        }
    }
}

both_uri_shapes! {
    #[test]
    fn a_re_exported_leaf_and_its_qualified_spelling_hover_the_declaration() {
        // the own lane's `pub use` row NAMES its target: `m` re-exports
        // `grid::G`, so the leaf (`use m::{ G }`), its use-line
        // spelling, and the qualified spelling (`m::G`) all answer the
        // DECLARATION in grid — one walk, one answer.
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let grid = rut_new::at("src/m/grid/mod.rut");
        let leaf = "use m::{ G };\nfn k() -> i32 { return G; }\n";
        for (needle, off, what) in [
            ("return G", 7, "the re-exported leaf's use site"),
            ("{ G }", 2, "the leaf's spelling"),
        ] {
            let md = markdown(s.hover_in(&uri, leaf, needle, 1, off));
            assert!(md.contains("pub let G: i32 = 1"), "[{what}] {md}");
            assert!(md.contains(&format!("— {grid}:2")), "[{what}] {md}");
        }
        let qualified = "fn k() -> i32 { return m::G; }\n";
        let md = markdown(s.hover_in(&uri, qualified, "m::G", 1, 3));
        assert!(md.contains("pub let G: i32 = 1"), "the qualified spelling: {md}");
        assert!(md.contains(&format!("— {grid}:2")), "the qualified spelling: {md}");
    }
}

both_uri_shapes! {
    #[test]
    fn a_twice_bound_name_hovers_its_candidates() {
        // the mid-edit shape: a file scope that binds a name TWICE by
        // DISTINCT bindings is a file the compiler refuses at the
        // second spelling — the hover LISTS every binding through the
        // candidates path (each with its kind and provenance: which
        // decl, which `use` leaf and its source), never a pick between
        // them; definition stays silent on the same name
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let m = rut_new::at("src/m/mod.rut");
        // two same-named decls in two packages are legal; two leaves
        // of one name in a third file refuse — each candidate rides
        // the `use` that bound it, beside its own decl's provenance
        let math = format!("{}pub fn mk(n: i32) -> Calc {{ return Calc {{ d: n }}; }}\n", rut_new::CALC_MATH);
        s.index_mod_at(&rut_new::calc("src/math/mod.rut"), "calc", "math", &math);
        let src = "use m::{ mk };\nuse calc::math::{ mk };\nfn k() -> i32 { return mk(41).bump(); }\n";
        let want = format!(
            "multiple definitions:\n- `pub fn mk(n: i32) -> Counter` — fn — via `use m::{{ mk }}` — {m}:13\n- `pub fn mk(n: i32) -> Calc` — fn — via `use calc::math::{{ mk }}` — {}:17",
            rut_new::calc("src/math/mod.rut")
        );
        // the call site AND each leaf — one name, one list
        for (needle, occ, off) in [("mk(41)", 1, 0), ("{ mk }", 1, 2), ("{ mk }", 2, 2)] {
            let md = markdown(s.hover_in(&uri, src, needle, occ, off));
            assert_eq!(md, want, "[{needle} #{occ}] the candidates: ");
        }
        // an own decl beside a leaf: the registry refuses the file —
        // the own decl first, then the leaf's row
        let src = "use m::{ mk };\nfn mk(n: i32) -> i32 { return n; }\nfn k() -> i32 { return mk(41); }\n";
        let want = format!(
            "multiple definitions:\n- `fn mk(n: i32) -> i32` — fn — {uri}:2\n- `pub fn mk(n: i32) -> Counter` — fn — via `use m::{{ mk }}` — {m}:13"
        );
        assert_eq!(markdown(s.hover_in(&uri, src, "mk(41)", 1, 0)), want);
        assert_eq!(markdown(s.hover_in(&uri, src, "{ mk }", 1, 2)), want, "the leaf too");
        // the decl SITE stays exact (decl-site navigation)
        let md = markdown(s.hover_in(&uri, src, "fn mk(", 1, 3));
        assert!(md.contains("fn mk(n: i32) -> i32"), "the decl itself: {md}");
        assert!(!md.contains("multiple definitions"), "the decl site is no list: {md}");
    }
}

both_uri_shapes! {
    #[test]
    fn a_twice_bound_type_or_let_hovers_its_candidates() {
        // the same law for the registry's other kinds — a type, a
        // module let: each candidate with its kind and the `use` that
        // bound it
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let m = rut_new::at("src/m/mod.rut");
        let src = "use m::{ Counter };\nuse app::m::{ Counter };\nfn k(c: Counter) -> i32 { return c.bump(); }\n";
        let want = format!(
            "multiple definitions:\n- `pub struct Counter` — struct — via `use m::{{ Counter }}` — {m}:7\n- `pub struct Counter` — struct — via `use app::m::{{ Counter }}` — {m}:7"
        );
        assert_eq!(markdown(s.hover_in(&uri, src, "c: Counter", 1, 3)), want);
        let src = "use m::{ STEP };\nuse app::m::{ STEP };\nfn k() -> i32 { return STEP; }\n";
        let want = format!(
            "multiple definitions:\n- `pub let STEP: i32 = 2;` — module let — via `use m::{{ STEP }}` — {m}:16\n- `pub let STEP: i32 = 2;` — module let — via `use app::m::{{ STEP }}` — {m}:16"
        );
        assert_eq!(markdown(s.hover_in(&uri, src, "return STEP", 1, 7)), want);
    }
}

both_uri_shapes! {
    #[test]
    fn two_own_decls_of_a_name_hover_both() {
        // decl + decl — "duplicate name `mk` — the fn already declares
        // it in this file": both decls are the candidates, each at its
        // own line (no `use` rode); the decl sites stay exact
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let src = "fn mk(n: i32) -> i32 { return n; }\nclass mk {\n    n: i32,\n}\nfn k() -> i32 { return mk(41); }\n";
        let want = format!(
            "multiple definitions:\n- `class mk` — class — {uri}:2\n- `fn mk(n: i32) -> i32` — fn — {uri}:1"
        );
        assert_eq!(markdown(s.hover_in(&uri, src, "mk(41)", 1, 0)), want);
        let md = markdown(s.hover_in(&uri, src, "fn mk(", 1, 3));
        assert!(md.contains("fn mk(n: i32) -> i32"), "the fn's own site: {md}");
        assert!(!md.contains("multiple definitions"), "a decl site is no list: {md}");
        let md = markdown(s.hover_in(&uri, src, "class mk", 1, 6));
        assert!(md.contains("class mk {"), "the class's own site: {md}");
        assert!(!md.contains("multiple definitions"), "a decl site is no list: {md}");
    }
}

both_uri_shapes! {
    #[test]
    fn a_module_leaf_use_hovers_the_module() {
        // `use calc::{ math }` — the leaf names a module (the
        // compiler's module-leaf binding): it renders like a path
        // segment does, the walked module's own file
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let src = "use calc::{ math };\nfn k() -> i32 { return math::cmk(1).deg(); }\n";
        let md = markdown(s.hover_in(&uri, src, "{ math }", 1, 2));
        assert!(md.contains("file module"), "{md}");
        assert!(
            md.contains(&format!("— {}:1", rut_new::calc("src/math/mod.rut"))),
            "{md}"
        );
        let md = markdown(s.hover_in(&uri, src, "calc::{ m", 1, 1));
        assert!(md.contains("file module"), "the pkg head: {md}");
    }
}

both_uri_shapes! {
    #[test]
    fn two_lanes_mounting_one_file_never_double_a_binding() {
        // the extension mounts every workspace file TWICE — the
        // indexer's `file://` stamp and the walk's rooted string — one
        // entry per file (the store's replace-by-FILE law); a name
        // bound once never hovers `multiple definitions`, and a module
        // let stays unique-or-none's ONE
        let (mut s, _io) = rut_new::boot();
        let uri = rut_new::at("src/child/mod.rut");
        let src = "use app::m::{ mk };\nfn k() -> i32 { return mk(41).bump(); }\n";
        let md = markdown(s.hover_in(&uri, src, "mk(41)", 1, 0));
        assert!(md.contains("pub fn mk(n: i32) -> Counter"), "{md}");
        assert!(!md.contains("multiple definitions"), "{md}");
        let src = "use app::{ LIMIT };\nfn k() -> i32 { return LIMIT; }\n";
        let md = markdown(s.hover_in(&uri, src, "return LIMIT", 1, 7));
        assert!(md.contains("pub let LIMIT: i32 = 7"), "{md}");
        // the walk's indexed copy of the doc's file lags an open edit —
        // the live document's decls answer for the doc's own names
        let src = "fn own() -> i32 { return 1; }\nfn k() -> i32 { return own(); }\n";
        let md = markdown(s.hover_in(&uri, src, "return own", 1, 7));
        assert!(md.contains("fn own() -> i32"), "{md}");
        assert!(md.contains(&format!("— {uri}:1")), "the live document's own row: {md}");
    }
}

// ---- the class callee's constructor section (the one-name law's
//      Act 3 — the original ask) ----

both_uri_shapes! {
#[test]
fn a_class_callee_hover_shows_the_class_block_and_the_constructor_section() {
    // hover on a class callee — `Logger("hello")` — shows the class
    // block (primary) AND the designated ctor's verbatim impl slice
    // appended (it carries `[constructor] pub fn new(..) -> Self`
    // whole)
    let mut s = S::boot();
    let src = "class Logger {\n    msg: str;\n}\nimpl Logger {\n    [constructor] pub fn new(msg: str) -> Self { return Self { msg: msg }; }\n    pub fn show(self) -> str { return self.msg; }\n}\nentry fn main() -> nil {\n    let l = Logger(\"hello\");\n}\n";
    let md = markdown(s.hover_in("file:///t/main.rut", src, "= Logger(", 1, 2));
    assert!(md.contains("class Logger {"), "the class block stays primary: {md}");
    assert!(md.contains("constructor:"), "the ctor section appends: {md}");
    assert!(
        md.contains("[constructor] pub fn new(msg: str) -> Self"),
        "the ctor's verbatim impl slice: {md}"
    );
}
}

both_uri_shapes! {
#[test]
fn a_no_ctor_class_callee_hover_appends_nothing() {
    // a class with no designated ctor and no mint: the callee hover
    // is the class block alone, exactly as before this phase
    let mut s = S::boot();
    let src = "class Plain {\n    n: i32;\n}\nentry fn main() -> nil {\n    let _p = Plain();\n}\n";
    let md = markdown(s.hover_in("file:///t/main.rut", src, "= Plain(", 1, 2));
    assert!(md.contains("class Plain {"), "{md}");
    assert!(!md.contains("constructor:"), "no ctor, no section: {md}");
    assert!(!md.contains("newtype mint"), "{md}");
}
}

both_uri_shapes! {
#[test]
fn a_newtype_callee_hover_shows_the_mint_line() {
    // a positional newtype's only constructor is the compiler-provided
    // mint (`compile_newtype_ctor`: exactly one wrapped value, the
    // wrapper back; the desugar names the field `inner`) — the
    // synthesized line is source-less BY DESIGN (the LSP verbatim law
    // renders inferred/semantic facts)
    let mut s = S::boot();
    let src = "// meters\nclass Meters(f64);\nentry fn main() -> nil {\n    let m = Meters(1.0);\n}\n";
    let md = markdown(s.hover_in("file:///t/main.rut", src, "= Meters(", 1, 2));
    assert!(
        md.contains("class Meters(f64) {"),
        "the class block stays primary: {md}"
    );
    assert!(
        md.contains("constructor — the newtype mint: Meters(inner: f64) -> Meters"),
        "{md}"
    );
    assert!(
        !md.contains("constructor:"),
        "the mint is no designated ctor section: {md}"
    );
}
}

both_uri_shapes! {
#[test]
fn an_argument_path_hovers_the_binding_never_the_ctor() {
    // the identity discriminator: an ARGUMENT path shares the parent
    // CallNode but is not the callee slot — `Logger(msg)`'s `msg`
    // answers the binding hover, and a CAPITALIZED argument (`show
    // (Logger)`) still answers the bare class block, never the ctor
    // section
    let mut s = S::boot();
    let src = "class Logger {\n    msg: str;\n}\nimpl Logger {\n    [constructor] pub fn new(msg: str) -> Self { return Self { msg: msg }; }\n    pub fn show(self) -> str { return self.msg; }\n}\nfn use_it(msg: str) -> str {\n    let l = Logger(msg);\n    return l.show();\n}\nfn take_val(v: Logger) -> str {\n    return v.msg;\n}\nfn show(v: Logger) -> str {\n    return v.msg;\n}\nentry fn main() -> nil {\n    let l = Logger(\"hi\");\n    let _a = show(Logger);\n}\n";
    let md = markdown(s.hover_in("file:///t/main.rut", src, "= Logger(msg)", 1, 10));
    assert!(md.contains("msg: str"), "{md}");
    assert!(md.contains("parameter"), "{md}");
    assert!(!md.contains("constructor:"), "{md}");
    // a capitalized ARGUMENT path: the same parent CallNode, the
    // callee slot's identity is the discriminator
    let md = markdown(s.hover_in("file:///t/main.rut", src, "= show(Logger)", 1, 7));
    assert!(md.contains("class Logger {"), "{md}");
    assert!(!md.contains("constructor:"), "an argument never triggers: {md}");
}
}

both_uri_shapes! {
#[test]
fn a_type_position_and_the_decl_site_carry_no_ctor_section() {
    // type positions (`l: Logger`) and decl sites answer exactly as
    // before — the section rides the CALLEE position only
    let mut s = S::boot();
    let src = "class Logger {\n    msg: str;\n}\nimpl Logger {\n    [constructor] pub fn new(msg: str) -> Self { return Self { msg: msg }; }\n}\nfn take(l: Logger) -> str {\n    return l.msg;\n}\n";
    let md = markdown(s.hover_in("file:///t/main.rut", src, "l: Logger", 1, 4));
    assert!(md.contains("class Logger {"), "{md}");
    assert!(!md.contains("constructor:"), "a type position appends nothing: {md}");
    let md = markdown(s.hover_in("file:///t/main.rut", src, "class Logger", 1, 6));
    assert!(md.contains("class Logger {"), "{md}");
    assert!(!md.contains("constructor:"), "the decl site appends nothing: {md}");
}
}

both_uri_shapes! {
#[test]
fn a_member_call_tail_hovers_the_impl_fn_unchanged() {
    // `Logger::new(..)`'s tail resolves through the member law → the
    // impl fn's verbatim render — no ctor section appends there
    let mut s = S::boot();
    let src = "class Logger {\n    msg: str;\n}\nimpl Logger {\n    [constructor] pub fn new(msg: str) -> Self { return Self { msg: msg }; }\n}\nentry fn main() -> nil {\n    let l = Logger::new(\"hi\");\n}\n";
    let md = markdown(s.hover_in("file:///t/main.rut", src, "Logger::new", 1, 10));
    assert!(md.contains("[constructor] pub fn new(msg: str) -> Self"), "{md}");
    assert!(md.contains("in `Logger`"), "{md}");
    assert!(
        !md.contains("constructor:"),
        "the tail is a member answer, no section: {md}"
    );
}
}

both_uri_shapes! {
#[test]
fn the_self_lanes_carry_no_ctor_section() {
    // `Self` (the value spelling) and `Self` (the type slot) render
    // the enclosing type, unchanged — no ctor section
    let mut s = S::boot();
    let src = "class Logger {\n    msg: str;\n}\nimpl Logger {\n    [constructor] pub fn new(msg: str) -> Self { return Self { msg: msg }; }\n}\nentry fn main() -> nil {\n    let l = Logger(\"hi\");\n}\n";
    let md = markdown(s.hover_in("file:///t/main.rut", src, "return Self", 1, 7));
    assert!(md.contains("class Logger {"), "{md}");
    assert!(!md.contains("constructor:"), "the Self lane appends nothing: {md}");
    let md = markdown(s.hover_in("file:///t/main.rut", src, ") -> Self {", 1, 5));
    assert!(md.contains("class Logger {"), "{md}");
    assert!(!md.contains("constructor:"), "the SelfTy lane appends nothing: {md}");
}
}

both_uri_shapes! {
#[test]
fn a_cross_module_ctor_impl_is_no_ctor_of_the_class() {
    // the compiler REFUSES an inherent impl outside the class's own
    // module ("an inherent impl for `Logger` must live in `Logger`'s
    // module") — a cross-module `[constructor]` impl is NOT a ctor of
    // that class: hover shows the class block with NO ctor section
    // (never-wrong; the diagnostics panel owns the refusal)
    let mut s = S::boot();
    s.index_file(
        "file:///ws/lib/log.rut",
        "impl Logger {\n    // the maker\n    [constructor] pub fn new(msg: str) -> Self { return Self { msg: msg }; }\n}\n",
    );
    let src = "class Logger {\n    msg: str;\n}\nentry fn main() -> nil {\n    let l = Logger(\"hello\");\n}\n";
    let md = markdown(s.hover_in("file:///t/main.rut", src, "= Logger(", 1, 2));
    assert!(md.contains("class Logger {"), "{md}");
    assert!(!md.contains("constructor:"), "a foreign impl is no ctor: {md}");
    assert!(!md.contains("log.rut"), "no foreign provenance: {md}");
    assert!(!md.contains("the maker"), "{md}");
}
}

both_uri_shapes! {
#[test]
fn a_qualified_ctor_callee_appends_the_section() {
    // `shape::Logger("x")` — the qualified callee resolves through the
    // module namespace lane; the class block stays primary, the ctor
    // section appends as the bare callee's does
    let mut s = S::boot();
    s.index_mod_at(
        "file:///ws/pkg/src/shape/mod.rut",
        "pkg",
        "shape",
        "pub class Logger {\n    msg: str;\n}\nimpl Logger {\n    [constructor] pub fn new(msg: str) -> Self { return Self { msg: msg }; }\n}\n",
    );
    let doc = "mod shape;\nentry fn main() -> nil { let l = shape::Logger(\"x\"); }\n";
    let uri = "file:///ws/pkg/src/mod.rut";
    s.index_mod_at(uri, "pkg", "", doc);
    let md = markdown(s.hover_in(uri, doc, "shape::Logger(", 1, 8));
    assert!(md.contains("class Logger {"), "the class block stays primary: {md}");
    assert!(md.contains("constructor:"), "{md}");
    assert!(md.contains("[constructor] pub fn new(msg: str) -> Self"), "{md}");
    // the head segment still renders the module, no ctor section
    let md = markdown(s.hover_in(uri, doc, "shape::Logger(", 1, 2));
    assert!(md.contains("file module"), "{md}");
    assert!(!md.contains("constructor:"), "{md}");
}
}

// ---- the file-scope law on the ctor door (review round 1): the ctor
//      is the RESOLVED class's own file's, never a name-keyed flat
//      sweep over the chain ----

both_uri_shapes! {
#[test]
fn the_bound_packages_class_donates_the_ctor_section() {
    // P1 — two packages each carry `class Logger` with a DIFFERENT
    // ctor; the file binds each in turn (`use aaa::{Logger}` /
    // `use bbb::{Logger}`) and the section is the BOUND package's, in
    // both mount orders — chain order must not decide
    let aaa = "pub class Logger {\n    n: i32;\n}\nimpl Logger {\n    [constructor] pub fn mk(n: i32) -> Self { return Self { n: n }; }\n}\n";
    let bbb = "pub class Logger {\n    msg: str;\n}\nimpl Logger {\n    [constructor] pub fn build(msg: str) -> Self { return Self { msg: msg }; }\n}\n";
    for aaa_first in [true, false] {
        let mut s = S::boot();
        if aaa_first {
            s.index_file("file:///ws/aaa.rut", aaa);
            s.index_file("file:///ws/bbb.rut", bbb);
        } else {
            s.index_file("file:///ws/bbb.rut", bbb);
            s.index_file("file:///ws/aaa.rut", aaa);
        }
        let src = "use aaa::{Logger};\nentry fn main() -> nil {\n    let l = Logger(1);\n}\n";
        let md = markdown(s.hover_in("file:///t/main.rut", src, "= Logger(", 1, 2));
        assert!(md.contains("n: i32"), "aaa's class block: {md}");
        assert!(!md.contains("msg: str"), "never bbb's fields: {md}");
        assert!(md.contains("constructor:"), "{md}");
        assert!(
            md.contains("[constructor] pub fn mk(n: i32) -> Self"),
            "aaa's ctor: {md}"
        );
        assert!(!md.contains("fn build"), "never bbb's ctor: {md}");
        let src = "use bbb::{Logger};\nentry fn main() -> nil {\n    let l = Logger(\"x\");\n}\n";
        let md = markdown(s.hover_in("file:///t/main.rut", src, "= Logger(", 1, 2));
        assert!(md.contains("msg: str"), "bbb's class block: {md}");
        assert!(!md.contains("n: i32"), "{md}");
        assert!(
            md.contains("[constructor] pub fn build(msg: str) -> Self"),
            "bbb's ctor: {md}"
        );
        assert!(!md.contains("fn mk"), "{md}");
    }
}
}

both_uri_shapes! {
#[test]
fn a_foreign_same_named_class_donates_nothing_to_the_files_own_no_ctor_class() {
    // P4 — the file's OWN no-ctor `class Logger` beside a mounted
    // foreign `Logger` with a ctor: the compiler refuses the call
    // ("`Logger` constructs through its class methods") — the foreign
    // ctor must not donate a section
    let mut s = S::boot();
    s.index_file(
        "file:///ws/aaa.rut",
        "pub class Logger {\n    n: i32;\n}\nimpl Logger {\n    [constructor] pub fn mk(n: i32) -> Self { return Self { n: n }; }\n}\n",
    );
    let src = "class Logger {\n    msg: str;\n}\nentry fn main() -> nil {\n    let l = Logger(\"x\");\n}\n";
    let md = markdown(s.hover_in("file:///t/main.rut", src, "= Logger(", 1, 2));
    assert!(md.contains("msg: str"), "the file's own class block: {md}");
    assert!(!md.contains("n: i32"), "{md}");
    assert!(!md.contains("constructor:"), "no foreign ctor donates: {md}");
}
}

both_uri_shapes! {
#[test]
fn an_unbound_class_name_hovers_nothing() {
    // P2's hover half — an unbound capitalized name that is a class
    // elsewhere answers nothing (the file scope's law; sig help was
    // the leak, hover was already gated)
    let mut s = S::boot();
    s.index_file(
        "file:///ws/aaa.rut",
        "pub class Widget {\n    v: i32;\n}\nimpl Widget {\n    [constructor] pub fn new(v: i32) -> Self { return Self { v: v }; }\n}\n",
    );
    let src = "entry fn main() -> nil {\n    let w = Widget(1);\n}\n";
    let h = s.hover_in("file:///t/main.rut", src, "= Widget(", 1, 2);
    assert!(no_hover(&h), "{h}");
}
}

both_uri_shapes! {
#[test]
fn an_ambiguous_own_class_lists_candidates_with_no_ctor_section() {
    // D's hover half — two own `class Logger` decls (the one-name law
    // refuses; a mid-edit state): hover lists the candidates, no ctor
    // section, never a pick
    let mut s = S::boot();
    let src = "class Logger {\n    n: i32;\n}\nclass Logger {\n    msg: str;\n}\nimpl Logger {\n    [constructor] pub fn build(msg: str) -> Self { return Self { msg: msg }; }\n}\nentry fn main() -> nil {\n    let l = Logger(1);\n}\n";
    let md = markdown(s.hover_in("file:///t/main.rut", src, "= Logger(", 1, 2));
    assert!(md.contains("multiple definitions"), "{md}");
    assert!(!md.contains("constructor:"), "{md}");
}
}

both_uri_shapes! {
#[test]
fn a_generic_newtype_callee_mints_with_its_binder() {
    // `class Box<T>(T);` — the mint carries the class's generics on
    // BOTH sides: Box<T>(inner: T) -> Box<T> (the wrapped type is the
    // desugared field's type text — the binder as written)
    let mut s = S::boot();
    let src = "class Box<T>(T);\nentry fn main() -> nil {\n    let b = Box(1);\n}\n";
    let md = markdown(s.hover_in("file:///t/main.rut", src, "= Box(", 1, 2));
    assert!(md.contains("class Box<T>(T) {"), "{md}");
    assert!(
        md.contains("constructor — the newtype mint: Box<T>(inner: T) -> Box<T>"),
        "{md}"
    );
}
}

#[test]
fn qualified_tail_caret_past_the_name_still_answers_the_member() {
    // END-OF-NAME CARET (Act 4 review): the caret one byte past a
    // qualified tail's last character — `Color::Red|` — rides the
    // tail: hover renders the enum (the member's own block), and the
    // references count the declaring ident beside the use site. The
    // base probe answered null/[] there (definition's cell of this
    // shape is pinned by
    // `definition_at_a_tail_s_end_byte_still_rides_the_tail`).
    let mut s = S::boot();
    let uri = "file:///t/main.rut";
    let src = "enum Color { Red, Green }\nentry fn main() -> nil {\n    let c = Color::Red;\n}\n";
    s.analyze(uri, src);
    // the caret past the tail `Red` (the `;` that follows)
    let (l, c) = S::pos_at_byte(src, byte_of(src, "Color::Red", 1) + 10);
    let md = markdown(s.hover_pos(uri, l, c));
    assert!(md.contains("enum Color { Red, Green }"), "{md}");
    let refs = s.refs_pos(uri, (l, c), true);
    assert_eq!(refs.len(), 2, "{refs:?}");
    assert_eq!(pos_pair(&refs[0]["range"]["start"]), (0, 13), "{refs:?}");
    assert_eq!(pos_pair(&refs[1]["range"]["start"]), (2, 19), "{refs:?}");
}
