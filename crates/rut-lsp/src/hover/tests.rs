//! Hover tests — end-to-end over parsed snippets: use sites, not decls.

use rut_lexer::token::{Tok, Token};

use super::*;

fn hover_at(src: &str, needle: &str) -> Option<String> {
    hover_nth(src, needle, 0)
}

/// hover the `n`-th (0-based) occurrence of `needle` — tests target
/// use sites, which usually aren't the declaration
fn hover_nth(src: &str, needle: &str, n: usize) -> Option<String> {
    let src2 = rut_lexer::lexer::normalize(src);
    let (toks, _) = rut_lexer::lexer::lex(&src2);
    let (ast, _) = rut_parser::parse(&src2, rut_parser::Mode::Impl);
    let idx = index(&src2, &ast);
    let idxs = [&idx];
    let pos = find_ident_pos(&toks, needle, n)?;
    hover(&idxs, &src2, &toks, &ast, pos).map(|h| h.markdown)
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
        let (fa, _) = rut_parser::parse(&f, rut_parser::Mode::Impl);
        let mut i = index(&f, &fa);
        i.origin = origin.to_string();
        i
    };
    let a = mk("a.rut");
    let b = mk("b.rut");

    let src = "fn go() -> nil { let h = helper(); }\n";
    let s = rut_lexer::lexer::normalize(src);
    let (toks, _) = rut_lexer::lexer::lex(&s);
    let (ast, _) = rut_parser::parse(&s, rut_parser::Mode::Impl);
    let mut doc = index(&s, &ast);
    doc.origin = "main.rut".to_string();
    let idxs = [&doc, &a, &b];
    let pos = find_ident_pos(&toks, "helper", 0).unwrap();
    let md = hover(&idxs, &s, &toks, &ast, pos).unwrap().markdown;
    assert!(md.contains("multiple definitions"), "{md}");
    assert!(md.contains("a.rut") && md.contains("b.rut"), "{md}");
}

#[test]
fn host_fn_surface_favors_own_methods() {
    // std-style surface index ahead of the doc
    let surf_src = "pub host fn string_join(s: Array<str>) -> i32;\n";
    let s2 = rut_lexer::lexer::normalize(surf_src);
    let (sast, _) = rut_parser::parse(&s2, rut_parser::Mode::Decl);
    let mut surf = index(&s2, &sast);
    surf.origin = "core".to_string();

    let doc = "fn main() -> i32 { return string_join([]); }\n";
    let d2 = rut_lexer::lexer::normalize(doc);
    let (toks, _) = rut_lexer::lexer::lex(&d2);
    let (ast, _) = rut_parser::parse(&d2, rut_parser::Mode::Impl);
    let mut di = index(&d2, &ast);
    di.origin = "main.rut".to_string();
    let idxs = [&di, &surf];
    let pos = find_ident_pos(&toks, "string_join", 0).unwrap();
    let h = hover(&idxs, &d2, &toks, &ast, pos).unwrap();
    assert!(h.markdown.contains("fn string_join"), "{}", h.markdown);
    assert!(h.markdown.contains("core"), "{}", h.markdown);
}

#[test]
fn miss_is_none() {
    let src = "fn main() -> nil { let z = unknown_thing; }\n";
    assert!(hover_at(src, "unknown_thing").is_none());
}
