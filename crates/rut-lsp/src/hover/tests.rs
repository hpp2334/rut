//! Hover tests — end-to-end over parsed snippets: use sites, not decls.
//! Phase 1 adds the binding pass (identifier hovers, receiver
//! inference: field reads / chained calls / loop variables), the decl
//! layer (field + enum-member + module-let decl sites), and the M7
//! primitive blurbs.

use rut_lexer::token::{Tok, Token};

use super::*;

fn hover_at(src: &str, needle: &str) -> Option<String> {
    hover_nth(src, needle, 0)
}

/// hover the `n`-th (0-based) occurrence of `needle` — tests target
/// use sites, which usually aren't the declaration
fn hover_nth(src: &str, needle: &str, n: usize) -> Option<String> {
    hover_out_nth(src, needle, n).map(|h| h.markdown)
}

fn hover_out_nth(src: &str, needle: &str, n: usize) -> Option<HoverOut> {
    let src2 = rut_lexer::lexer::normalize(src);
    let (toks, _) = rut_lexer::lexer::lex(&src2);
    let (ast, _) = rut_parser::parse(&src2, rut_parser::Mode::Impl);
    let idx = index(&src2, &ast, &toks);
    let idxs = [&idx];
    let pos = find_ident_pos(&toks, needle, n)?;
    hover(&idxs, &toks, &ast, pos)
}

/// the whole span of the `n`-th ident `needle` — the span-first law's
/// assertion target
fn ident_span_of(toks: &[Token], name: &str, n: usize) -> rut_lexer::span::Span {
    toks.iter()
        .filter(|t| matches!(&t.tok, Tok::Ident(s) if s == name))
        .nth(n)
        .map(|t| t.span)
        .unwrap()
}

fn find_ident_pos(toks: &[Token], name: &str, n: usize) -> Option<u32> {
    toks.iter()
        .filter(|t| matches!(&t.tok, Tok::Ident(s) if s == name))
        .nth(n)
        .map(|t| t.span.lo)
}

#[test]
fn method_hover_shows_signature_doc_and_owner() {
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
    // 2nd occurrence: the use site `p.x`, not the field decl
    let md = hover_nth(src, "x", 1).unwrap();
    assert!(md.contains("```rut"), "code block: {md}");
    assert!(md.contains("x: f64"), "field decl: {md}");
    assert!(md.contains("in `Point`"), "owner: {md}");
}

#[test]
fn class_hover_shows_struct_definition() {
    let src = "\
// a circle
class Circle {
pub r: f64;
x: f64;
y: f64;
}
";
    let md = hover_at(src, "Circle").unwrap();
    assert!(md.contains("class Circle {"), "{md}");
    assert!(md.contains("pub r: f64"), "{md}");
    assert!(md.contains("x: f64"), "{md}");
    assert!(md.contains("a circle"), "doc: {md}");
}

#[test]
fn method_via_inference_and_self() {
    let src = "\
class Circle {
r: f64;
fn area(self) -> f64 { return 3.14; }
fn grow(self, k: f64) -> nil { self.r = self.r * k; }
}
fn use_it() -> f64 {
let c = Circle.new(1.0);
return c.area();
}
";
    // 2nd occurrence: the call `c.area()`, receiver inferred from
    // `let c = Circle.new(..)`
    let md = hover_nth(src, "area", 1).unwrap();
    assert!(md.contains("fn area(self) -> f64"), "{md}");
    assert!(md.contains("in `Circle`"), "{md}");
}

#[test]
fn trait_method_via_impl() {
    let src = "\
trait Drawable {
fn draw(self) -> nil;
}
class Circle {
r: f64;
}
impl Drawable for Circle {
fn draw(self) -> nil { }
}
fn render(d: Circle) -> nil {
d.draw();
}
";
    // 3rd occurrence: the call `d.draw()` — param-typed receiver,
    // resolved through the impl (the unified rule)
    let md = hover_nth(src, "draw", 2).unwrap();
    assert!(md.contains("fn draw(self) -> nil"), "{md}");
    assert!(md.contains("from `impl Drawable for Circle`"), "{md}");
}

#[test]
fn free_fn_unique_match() {
    let src = "\
// euclidean length
fn length(p: i32) -> f64 { return 1.0; }
fn main() -> nil { let x = length(3); }
";
    let md = hover_at(src, "length").unwrap();
    assert!(md.contains("fn length(p: i32) -> f64"), "{md}");
    assert!(md.contains("euclidean length"), "doc: {md}");
}

#[test]
fn ambiguity_lists_candidates() {
    // two workspace files each defining `helper`
    let mk = |origin: &str| {
        let f = rut_lexer::lexer::normalize("fn helper() -> nil { }\n");
        let (ftoks, _) = rut_lexer::lexer::lex(&f);
        let (fa, _) = rut_parser::parse(&f, rut_parser::Mode::Impl);
        let mut i = index(&f, &fa, &ftoks);
        i.origin = origin.to_string();
        i
    };
    let a = mk("a.rut");
    let b = mk("b.rut");

    let src = "fn go() -> nil { let h = helper(); }\n";
    let s = rut_lexer::lexer::normalize(src);
    let (toks, _) = rut_lexer::lexer::lex(&s);
    let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
    let mut doc = index(&s, &ast, &toks);
    doc.origin = "main.rut".to_string();
    let idxs = [&doc, &a, &b];
    let pos = find_ident_pos(&toks, "helper", 0).unwrap();
    let md = hover(&idxs, &toks, &ast, pos).unwrap().markdown;
    assert!(md.contains("multiple definitions"), "{md}");
    assert!(md.contains("a.rut") && md.contains("b.rut"), "{md}");
}

#[test]
fn host_fn_surface_favors_own_methods() {
    // std-style surface index ahead of the doc
    let surf_src = "pub host fn string_join(s: [str]) -> i32;\n";
    let s2 = rut_lexer::lexer::normalize(surf_src);
    let (sast, _) = rut_parser::parse(&s2, rut_parser::Mode::Decl);
    let (stoks, _) = rut_lexer::lexer::lex(&s2);
    let mut surf = index(&s2, &sast, &stoks);
    surf.origin = "core".to_string();

    let doc = "fn main() -> i32 { return string_join([]); }\n";
    let d2 = rut_lexer::lexer::normalize(doc);
    let (toks, _) = rut_lexer::lexer::lex(&d2);
    let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
    let mut di = index(&d2, &ast, &toks);
    di.origin = "main.rut".to_string();
    let idxs = [&di, &surf];
    let pos = find_ident_pos(&toks, "string_join", 0).unwrap();
    let h = hover(&idxs, &toks, &ast, pos).unwrap();
    assert!(h.markdown.contains("fn string_join"), "{}", h.markdown);
    assert!(h.markdown.contains("core"), "{}", h.markdown);
}

#[test]
fn nullable_alias_hover_renders_the_target() {
    // M4 (lsp-align survey): `ty_src` had no TyOpt arm, so a nullable
    // alias hovered as `type Maybe = ;` — the target silently vanished
    let src = "\
// maybe an i32
type Maybe = ?i32;
fn f(m: Maybe) -> nil { }
";
    let md = hover_at(src, "Maybe").unwrap();
    assert!(md.contains("type Maybe = ?i32;"), "{md}");
}

#[test]
fn nullable_let_binding_resolves_members() {
    // M4 (lsp-align survey): `ty_head` had no TyOpt arm, so a
    // `let x: ?Circle = …` binding inferred "" and the receiver hover
    // on `c.` missed
    let src = "\
class Circle {
r: f64;
// the area
fn area(self) -> f64 { return 3.14; }
}
fn go() -> f64 {
let c: ?Circle = Circle.new(1.0);
return c.area();
}
";
    let md = hover_nth(src, "area", 1).unwrap();
    assert!(md.contains("fn area(self) -> f64"), "{md}");
    assert!(md.contains("in `Circle`"), "{md}");
}

#[test]
fn miss_is_none() {
    let src = "fn main() -> nil { let z = unknown_thing; }\n";
    assert!(hover_at(src, "unknown_thing").is_none());
}

#[test]
fn inherent_impl_block_method_hover() {
    // methods live in `impl` blocks (RFC 0012 §4) — the call resolves
    // through the block's owner, not a type body
    let src = "\
struct Counter {
n: i32;
}
impl Counter {
// add one
fn bump(mut self) -> i32 { self.n += 1; return self.n; }
}
fn use_it(c: Counter) -> i32 {
return c.bump();
}
";
    let md = hover_nth(src, "bump", 1).unwrap();
    assert!(md.contains("fn bump(mut self) -> i32"), "{md}");
    assert!(md.contains("in `impl Counter`"), "{md}");
    assert!(md.contains("add one"), "doc: {md}");
}

#[test]
fn foreign_trait_method_requires_use() {
    // the use-both gate (RFC 0012 §6): the trait lives in another module,
    // so the call resolves only once the document `use`s it
    let surf_src = "trait Greeter {\nfn greet(self) -> nil;\n}\n";
    let s2 = rut_lexer::lexer::normalize(surf_src);
    let (sast, _) = rut_parser::parse(&s2, rut_parser::Mode::Impl);
    let (stoks, _) = rut_lexer::lexer::lex(&s2);
    let mut surf = index(&s2, &sast, &stoks);
    surf.origin = "greets.rut".to_string();

    let mk = |doc: &str| {
        let d2 = rut_lexer::lexer::normalize(doc);
        let (toks, _) = rut_lexer::lexer::lex(&d2);
        let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
        let mut di = index(&d2, &ast, &toks);
        di.origin = "main.rut".to_string();
        let idxs = [&di, &surf];
        let pos = find_ident_pos(&toks, "greet", 1).unwrap();
        hover(&idxs, &toks, &ast, pos).map(|h| h.markdown)
    };

    let gated = "class Robot { }\nimpl Greeter for Robot { fn greet(self) -> nil { } }\nfn go(r: Robot) -> nil { r.greet(); }\n";
    assert!(mk(gated).is_none(), "unused foreign trait must not resolve");

    let used = "use greets::{ Greeter };\nclass Robot { }\nimpl Greeter for Robot { fn greet(self) -> nil { } }\nfn go(r: Robot) -> nil { r.greet(); }\n";
    let md = mk(used).unwrap();
    assert!(md.contains("fn greet(self) -> nil"), "{md}");
    assert!(md.contains("from `impl Greeter for Robot`"), "{md}");
}

// ---- phase 1 (lsp-features): the binding pass + decl layer + M7 ----

#[test]
fn identifier_hover_shows_the_declared_annotation() {
    // the survey's ruling: display-side inference — an annotated let
    // renders its annotation as written (`?Circle` survives)
    let src = "\
class Circle {
r: f64;
}
fn go() -> f64 {
let c: ?Circle = nil;
return 3.14;
}
";
    // 1st occurrence = the decl ident, 2nd = the same name nowhere else;
    // hover the decl ident itself
    let md = hover_at(src, "c").unwrap();
    assert!(md.contains("let c: ?Circle"), "{md}");
    assert!(md.contains("let binding"), "{md}");
    assert!(!md.contains("inferred"), "declared, not inferred: {md}");
}

#[test]
fn identifier_hover_shows_the_inferred_type() {
    let src = "\
class Circle {
r: f64;
}
fn go() -> f64 {
let c = Circle.new(1.0);
return c.r;
}
";
    // the use of `c` in `c.r` — inferred from the `Circle.new` init
    let md = hover_nth(src, "c", 1).unwrap();
    assert!(md.contains("let c: Circle"), "{md}");
    assert!(md.contains("type inferred"), "{md}");
}

#[test]
fn param_hover_shows_the_declared_type() {
    let src = "\
struct Point {
x: f64;
}
fn length(p: Point) -> f64 {
return p.x;
}
";
    // the param's USE inside the body
    let md = hover_nth(src, "p", 1).unwrap();
    assert!(md.contains("p: Point"), "{md}");
    assert!(md.contains("parameter"), "{md}");
}

#[test]
fn field_decl_hover_names_the_owner() {
    // survey §1.3 gap 3: hovering `item` inside the struct was NULL —
    // member lookup needs a dot; the decl layer's exact span answers
    let src = "\
class Circle {
r: f64;
}
struct Slot {
item: ?Circle;
}
";
    let md = hover_at(src, "item").unwrap();
    assert!(md.contains("item: ?Circle"), "{md}");
    assert!(md.contains("in `Slot`"), "{md}");
}

#[test]
fn enum_member_decl_hover_shows_the_enum() {
    let src = "enum Color { Red, Green }\nfn f(c: Color) -> nil { }\n";
    let md = hover_at(src, "Red").unwrap();
    assert!(md.contains("enum Color { Red, Green }"), "{md}");
}

#[test]
fn module_let_decl_and_use_hover() {
    let src = "\
let MAX: i32 = 100;
fn f() -> i32 {
return MAX;
}
";
    // the decl site — the decl layer's exact span; the verbatim slice
    // carries the annotation
    let md = hover_at(src, "MAX").unwrap();
    assert!(md.contains("let MAX: i32 = 100"), "{md}");
    assert!(md.contains("module let"), "{md}");
    // the use site — the unique module-let match
    let md = hover_nth(src, "MAX", 1).unwrap();
    assert!(md.contains("let MAX: i32 = 100"), "{md}");
}

#[test]
fn module_let_initializer_infers() {
    // a module let with no annotation still answers (its head feeds
    // receiver inference through `module_let_ty`)
    let src = "let W = 3.5;\nfn f() -> f64 { return W; }\n";
    let md = hover_at(src, "W").unwrap();
    assert!(md.contains("let W = 3.5;"), "{md}");
    assert!(md.contains("`f32`"), "{md}");
}

#[test]
fn field_read_receiver_resolves_members() {
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
    let md = hover_at(src, "inner").unwrap();
    assert!(md.contains("let inner: Circle"), "{md}");
    let md = hover_nth(src, "r", 1).unwrap();
    assert!(md.contains("r: f64"), "{md}");
    assert!(md.contains("in `Circle`"), "{md}");
}

#[test]
fn chained_call_receiver_resolves_members() {
    // survey §1.3 gap 4b: `c.grown(2.0).r` — the receiver is a method
    // result; the chain rides the impl method's declared ret
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
    let md = hover_at(src, "r").unwrap();
    assert!(md.contains("r: f64"), "{md}");
    assert!(md.contains("in `Circle`"), "{md}");
}

#[test]
fn fn_result_receiver_resolves_members() {
    // a free fn's declared ret feeds the same chain
    let src = "\
class Circle {
r: f64;
}
fn mk() -> Circle { return Circle.new(1.0); }
fn go() -> f64 {
let d = mk();
return d.r;
}
";
    let md = hover_at(src, "d").unwrap();
    assert!(md.contains("let d: Circle"), "{md}");
    let md = hover_nth(src, "r", 1).unwrap();
    assert!(md.contains("in `Circle`"), "{md}");
}

#[test]
fn for_of_var_resolves_members() {
    // survey §1.3 gap 4c: the for-of binding is a receiver today's
    // inference misses entirely; `[Point]` → `Point` (the elem rule)
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
    let md = hover_nth(src, "p", 1).unwrap();
    assert!(md.contains("p: Point"), "{md}");
    assert!(md.contains("loop variable"), "{md}");
    let md = hover_nth(src, "x", 1).unwrap();
    assert!(md.contains("x: f64"), "{md}");
}

#[test]
fn for_c_counter_binding_resolves() {
    // the RFC 0007 §1 default: an unsuffixed `0` is `i32`
    let src = "\
fn f(n: i32) -> i32 {
let acc = 0;
for (let i = 0; i < n; i += 1) {
acc += i;
}
return acc;
}
";
    let md = hover_nth(src, "i", 1).unwrap();
    assert!(md.contains("i: i32"), "{md}");
    // the counter is visible in the condition too
    let md = hover_nth(src, "i", 2).unwrap();
    assert!(md.contains("i: i32"), "{md}");
}

#[test]
fn shadowing_resolves_to_the_nearest_latest() {
    // the rule infer_local's flat scan lacked: an inner re-binding wins
    // inside its scope; the outer binding resumes after the block ends
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
    let md = hover_nth(src, "x", 2).unwrap();
    assert!(md.contains("let x: Dot"), "{md}");
    // the field access `x.x`'s receiver — still the Dot
    let md = hover_nth(src, "x", 3).unwrap();
    assert!(md.contains("let x: Dot"), "{md}");
}

#[test]
fn scope_exit_restores_the_outer_binding() {
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
    // `x` in `return x.r` is the outer Circle binding again. Occurrence
    // 4: Dot's own field decl, the two decls, the literal label, then
    // the return's use
    let md = hover_nth(src, "x", 4).unwrap();
    assert!(md.contains("let x: Circle"), "{md}");
    let md = hover_nth(src, "r", 1).unwrap();
    assert!(md.contains("r: f64"), "{md}");
}

#[test]
fn binding_decl_span_is_byte_exact() {
    // the span-first law: the record's decl_ident_span equals the
    // declaring ident's token span — phase 2's definition target
    let src = "fn go() -> nil {\n    let total = 1;\n    let u = total;\n}\n";
    let src2 = rut_lexer::lexer::normalize(src);
    let (toks, _) = rut_lexer::lexer::lex(&src2);
    let (ast, _) = rut_parser::parse(&src2, rut_parser::Mode::Impl);
    let idx = index(&src2, &ast, &toks);
    let idxs = [&idx];
    let binds = bindings::collect(&ast, &toks, &idxs);
    let total = bindings::resolve(&binds, src2.rfind("total").unwrap() as u32, "total").unwrap();
    let decl_span = toks
        .iter()
        .find(|t| matches!(&t.tok, Tok::Ident(s) if s == "total"))
        .unwrap()
        .span;
    assert_eq!(total.decl_ident_span, decl_span);
    // the hover decorates exactly the token under the cursor
    let use_pos = src2.rfind("total").unwrap() as u32;
    let out = hover(&idxs, &toks, &ast, use_pos).unwrap();
    let use_span = toks
        .iter()
        .filter(|t| matches!(&t.tok, Tok::Ident(s) if s == "total"))
        .last()
        .unwrap()
        .span;
    assert_eq!(out.span, use_span);
}

#[test]
fn primitive_type_token_hover() {
    // M7: `i32` in type position — a static blurb, width + range
    let src = "fn f(x: i32) -> i32 { return x; }\n";
    let md = hover_at(src, "i32").unwrap();
    assert!(md.contains("i32"), "{md}");
    assert!(md.contains("signed 32-bit"), "{md}");
    assert!(md.contains("-2147483648 ..= 2147483647"), "{md}");
    assert!(md.contains("2147483647"), "{md}");
    let md = hover_nth(src, "i32", 1).unwrap();
    assert!(md.contains("signed 32-bit"), "{md}");
}

#[test]
fn primitive_bool_and_float_hover() {
    let src = "fn f(b: bool, w: f64) -> f64 { return w; }\n";
    let md = hover_at(src, "bool").unwrap();
    assert!(md.contains("true` / `false"), "{md}");
    let md = hover_at(src, "f64").unwrap();
    assert!(md.contains("binary64"), "{md}");
}

#[test]
fn surface_primitives_keep_their_rich_hover() {
    // `str`/`bytes`/`opaque` have surface decls — those must win over
    // the static blurb (the lsp-align smoke's `primitive str {` law)
    let core_src = "builtin primitive str {\nfn len(self) -> i32;\n}\n";
    let c2 = rut_lexer::lexer::normalize(core_src);
    let (ctoks, _) = rut_lexer::lexer::lex(&c2);
    let (cast, _) = rut_parser::parse(&c2, rut_parser::Mode::Decl);
    let core = index(&c2, &cast, &ctoks);
    let doc = "fn f(s: str) -> str { return s; }\n";
    let d2 = rut_lexer::lexer::normalize(doc);
    let (toks, _) = rut_lexer::lexer::lex(&d2);
    let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
    let di = index(&d2, &ast, &toks);
    let idxs = [&di, &core];
    let pos = find_ident_pos(&toks, "str", 0).unwrap();
    let md = hover(&idxs, &toks, &ast, pos).unwrap().markdown;
    assert!(md.contains("primitive str {"), "{md}");
}

#[test]
fn lambda_params_scope_to_the_lambda() {
    let src = "\
class Circle {
r: f64;
}
fn go(cs: [Circle]) -> f64 {
let f = fn(c: Circle) -> f64 { return c.r; };
return 0.0;
}
";
    let md = hover_nth(src, "c", 1).unwrap();
    assert!(md.contains("c: Circle"), "{md}");
    let md = hover_nth(src, "r", 1).unwrap();
    assert!(md.contains("r: f64"), "{md}");
}

#[test]
fn self_receiver_field_read_infers() {
    // `self.r` inside an inherent impl — the receiver chain rides the
    // enclosing type (recv_type's own law, through inference now)
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
    let md = hover_at(src, "rr").unwrap();
    assert!(md.contains("let rr: f64"), "{md}");
}
