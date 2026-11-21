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
    // `echo`, `keep` are bodiless surface rows; their hovers answer at
    // uses), and the builtin class carries its spelled head
    let mut s = S::boot();
    let src = "\
pub host async fn fetch(u: str) -> bytes;
pub builtin fn echo<T>(x: T) -> T;
prelude builtin fn keep<T>(x: T) -> T;
prelude builtin class Result<T> {
fn is_ok(self) -> bool;
}
";
    s.index_file("file:///t/surf.d.rut", src);
    let call = "\
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
fn ambiguity_lists_candidates() {
    // two workspace files each defining `helper`
    let mut s = S::boot();
    s.index_file("file:///t/a.rut", "fn helper() -> nil { }\n");
    s.index_file("file:///t/b.rut", "fn helper() -> nil { }\n");
    let src = "fn go() -> nil { let h = helper(); }\n";
    let md = markdown(s.hover_at("file:///t/main.rut", src, "helper", 1));
    assert!(md.contains("multiple definitions"), "{md}");
    assert!(md.contains("a.rut") && md.contains("b.rut"), "{md}");
}

#[test]
fn host_fn_surface_favors_own_methods() {
    // std-style surface index ahead of the doc
    let mut s = S::boot();
    s.index_file("core", "pub host fn string_join(s: [str]) -> i32;\n");
    let src = "fn main() -> i32 { return string_join([]); }\n";
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

#[test]
fn mod_decl_hover_shows_kind_mount_and_children() {
    // `pub mod layout;` in the doc — kind, the mounted file, the
    // child count read off the mounted index in the chain
    let mut s = S::boot();
    mount_gadgets(&mut s);
    let doc = "pub mod layout;\nmod secret;\nentry fn main() -> nil { }\n";
    let uri = "file:///ws/gadgets/main.rut";
    // stamp the doc's twin so the own-package edges key
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
    s.index_file("file:///t/surf.d.rut", src);
    let call =
        "fn go(x: f64) -> f64 {\n    let a = abs(x);\n    let b = ceil(a);\n    return b;\n}\n";
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
    let src = "fn go(x: f64) -> f64 {\n    return abs(x);\n}\n";
    let h = s.hover_at("file:///t/hov.rut", src, "abs", 1);
    let md = markdown(h);
    assert!(
        md.contains("fn abs<T requires f32 | f64>(x: T) -> T"),
        "{md}"
    );
    assert!(!md.contains(";pub"), "the next-decl artifact leaked: {md}");
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
