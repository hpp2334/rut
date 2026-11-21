//! The classifier's gate — `analyze`'s semantic-token stream (decoded
//! through the legend) and the document-symbol tree, asserted the way
//! a client consumes them. Pins: the sorted/disjoint token contract,
//! the cap law, the `::` resolution classes, the f-string tiling, the
//! AST-over-token precedence, the 15-type legend, and the
//! selection ⊆ range symbol invariant over damaged sources.

mod common;

use common::*;
use serde_json::Value;

/// analyze + decode `src`
fn toks_of(s: &mut S, uri: &str, src: &str) -> Vec<Tok> {
    let a = s.analyze(uri, src);
    decode_tokens(src, &a)
}

#[test]
fn output_is_sorted_and_disjoint() {
    let mut s = S::boot();
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../demo/src/examples/literals/src/mod.rut");
    let src = std::fs::read_to_string(&p).unwrap();
    // normalize like the server does (the uri convention: Impl)
    let src = src.replace("\r\n", "\n");
    let toks = toks_of(&mut s, "file:///t/literals.rut", &src);
    assert!(!toks.is_empty());
    let mut prev_hi = 0usize;
    for t in &toks {
        assert!(t.lo < t.hi, "empty token at {}", t.lo);
        assert!(t.lo >= prev_hi, "overlap at {}", t.lo);
        assert!(t.hi <= src.len(), "out of file");
        prev_hi = t.hi;
    }
}

#[test]
fn decls_and_keywords() {
    let mut s = S::boot();
    let src = "enum Color { Red, Green }\nfn area(r: f32) -> f32 { return r * 2.0f32; }\n";
    let toks = toks_of(&mut s, "file:///t/decls.rut", src);
    assert_eq!(types_of(&toks, "enum"), vec![t::KEYWORD]);
    assert_eq!(types_of(&toks, "Color"), vec![t::ENUM]);
    assert_eq!(types_of(&toks, "Red"), vec![t::ENUM_MEMBER]);
    assert_eq!(types_of(&toks, "Green"), vec![t::ENUM_MEMBER]);
    assert_eq!(types_of(&toks, "area"), vec![t::FUNCTION]);
    // the decl is a parameter; body uses are lowercase single-seg paths
    assert_eq!(types_of(&toks, "r"), vec![t::PARAMETER, t::VARIABLE]);
    assert_eq!(types_of(&toks, "f32"), vec![t::TYPE, t::TYPE]);
    assert_eq!(types_of(&toks, "2.0f32"), vec![t::NUMBER]);
    assert_eq!(types_of(&toks, "*"), vec![t::OPERATOR]);
}

#[test]
fn members_fields_and_locals() {
    let mut s = S::boot();
    let src = "class Circle {\n    pub r: f32;\n    fn scale(self, k: f32) -> Self { self.r = self.r * k; }\n}\n";
    let toks = toks_of(&mut s, "file:///t/members.rut", src);
    assert_eq!(types_of(&toks, "Circle"), vec![t::CLASS]);
    // the declared field + both `self.r` path tails
    assert_eq!(
        types_of(&toks, "r"),
        vec![t::PROPERTY, t::PROPERTY, t::PROPERTY]
    );
    assert_eq!(types_of(&toks, "scale"), vec![t::METHOD]);
    assert_eq!(types_of(&toks, "k"), vec![t::PARAMETER, t::VARIABLE]);
    // `self` stays a keyword everywhere (param + both path prefixes)
    assert_eq!(
        types_of(&toks, "self"),
        vec![t::KEYWORD, t::KEYWORD, t::KEYWORD]
    );
    assert_eq!(types_of(&toks, "Self"), vec![t::TYPE]);
}

#[test]
fn f_string_tiles_and_lexes_holes() {
    let mut s = S::boot();
    let src = "fn f() -> nil { log.info(f\"{color_name(Color::Red)} area={c.area()}\"); }";
    let toks = toks_of(&mut s, "file:///t/fstring.rut", src);
    // the whole literal tiles with strings + hole tokens, no overlap
    let lit = src.find("f\"").unwrap();
    let lit_end = src.find("\");").unwrap() + 1; // past the closing quote
    let inside: Vec<&Tok> = toks
        .iter()
        .filter(|t| t.lo >= lit && t.hi <= lit_end)
        .collect();
    assert!(!inside.is_empty());
    let mut prev_hi = lit;
    for t in &inside {
        assert!(t.lo >= prev_hi, "gap/overlap inside f-string at {}", t.lo);
        prev_hi = t.hi;
    }
    assert_eq!(prev_hi, lit_end);
    // hole contents classify: `Color` type, `Red` enumMember, `area` method
    assert_eq!(types_of(&toks, "Color"), vec![t::TYPE]);
    assert_eq!(types_of(&toks, "Red"), vec![t::ENUM_MEMBER]);
    assert_eq!(types_of(&toks, "area"), vec![t::METHOD]);
}

#[test]
fn types_struct_literals_and_patterns() {
    let mut s = S::boot();
    let src = "enum Color { Red, Green }\n\
               struct Point { x: f64; y: f64; }\n\
               fn make(c: Color) -> Point {\n\
                   let p = Point { x: 1, y: 2 };\n\
                   return when (c) { Color::Red -> p, _ -> p, };\n\
               }\n";
    let toks = toks_of(&mut s, "file:///t/patterns.rut", src);
    // decl name, param type, literal type
    assert_eq!(types_of(&toks, "Point"), vec![t::CLASS, t::TYPE, t::TYPE]);
    // field decls + literal labels
    assert_eq!(types_of(&toks, "x"), vec![t::PROPERTY, t::PROPERTY]);
    // enum decl name, param type, pattern type
    assert_eq!(types_of(&toks, "Color"), vec![t::ENUM, t::TYPE, t::TYPE]);
    assert_eq!(types_of(&toks, "Red"), vec![t::ENUM_MEMBER, t::ENUM_MEMBER]);
    assert_eq!(types_of(&toks, "c"), vec![t::PARAMETER, t::VARIABLE]);
    assert_eq!(
        types_of(&toks, "p"),
        vec![t::VARIABLE, t::VARIABLE, t::VARIABLE]
    );
}

#[test]
fn ast_wins_over_token_layer_at_equal_start() {
    // a field named `str` would be token-classified `type`; the AST
    // pass must replace it with `property`. (`nil` cannot be used — it
    // is a reserved word, unlike the contextual primitive names.)
    let mut s = S::boot();
    let src = "struct T { str: i32; }\n";
    let toks = toks_of(&mut s, "file:///t/strfield.rut", src);
    assert_eq!(types_of(&toks, "str"), vec![t::PROPERTY]);
}

#[test]
fn cast_types_keep_their_type_color() {
    // `x as f64` is the conversion family — the primitive after `as`
    // stays a type, not a variable
    let mut s = S::boot();
    let src = "fn f(p: Point) -> f64 { return p.x as f64; }\n";
    let toks = toks_of(&mut s, "file:///t/cast.rut", src);
    assert_eq!(types_of(&toks, "f64"), vec![t::TYPE, t::TYPE]);
    assert_eq!(types_of(&toks, "p"), vec![t::PARAMETER, t::VARIABLE]);
}

#[test]
fn bare_calls_color_their_callee_as_function() {
    // `length(pt)` / `newCanvas()` — the callee gets the function
    // color, overriding the single-seg path's variable class
    let mut s = S::boot();
    let src = "fn go() -> nil { let p = newCanvas(); blit_all(length(p), p); }\n";
    let toks = toks_of(&mut s, "file:///t/barecalls.rut", src);
    assert_eq!(types_of(&toks, "newCanvas"), vec![t::FUNCTION]);
    assert_eq!(types_of(&toks, "blit_all"), vec![t::FUNCTION]);
    assert_eq!(types_of(&toks, "length"), vec![t::FUNCTION]);
    assert_eq!(
        types_of(&toks, "p"),
        vec![t::VARIABLE, t::VARIABLE, t::VARIABLE]
    );
}

#[test]
fn symbols_outline() {
    let mut s = S::boot();
    let src = "interface Drawable { fn draw(self, g: Canvas) -> nil; }\nimpl Circle { fn draw(self, g: Canvas) -> nil {} }\nenum Color { Red }\nentry fn main() -> nil {}\n";
    let a = s.analyze("file:///t/outline.rut", src);
    let syms = a["symbols"].as_array().unwrap();
    let names: Vec<&str> = syms.iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["Drawable", "impl Circle", "Color", "main"]);
    assert_eq!(syms[0]["kind"], sk::INTERFACE, "{}", syms[0]);
    assert_eq!(syms[0]["children"].as_array().map(Vec::len), Some(1));
    assert_eq!(syms[1]["kind"], sk::MODULE, "{}", syms[1]);
    assert_eq!(syms[1]["children"].as_array().map(Vec::len), Some(1));
    assert_eq!(syms[2]["children"].as_array().map(Vec::len), Some(1));
    assert_eq!(syms[3]["kind"], sk::FUNCTION, "{}", syms[3]);
    // selectionRange stays inside range
    for x in syms {
        let sel0 = pos_pair(&x["selectionRange"]["start"]);
        let sel1 = pos_pair(&x["selectionRange"]["end"]);
        let rng0 = pos_pair(&x["range"]["start"]);
        let rng1 = pos_pair(&x["range"]["end"]);
        assert!(sel0 >= rng0 && sel1 <= rng1, "{x}");
    }
}

#[test]
fn symbols_selection_stays_inside_range_on_damaged_sources() {
    // the wire contract (LSP: selectionRange ⊆ range) holds by
    // construction at every symbol site; this is the test-time guard
    // for the sources the corpus gate never sees — recovery's real
    // diet. Every case below MUST produce diags (else it isn't damaged).
    let mut s = S::boot();
    let damaged = [
        "// c\nentry fn main( {\n  let x = ;\n",
        "enum Color { Red,\nentry fn main() -> nil {}\n",
        "enum Color { Red = , }\nentry fn main() -> nil {}\n",
        "fn () -> nil {}\nentry fn main() -> nil {}\n",
        "class C { pub : i32, }\nentry fn main() -> nil {}\n",
        "use pkg::{,,};\nentry fn main() -> nil {}\n",
        "impl { fn m(self) -> nil {} }\nentry fn main() -> nil {}\n",
    ];
    let mut walked = 0usize;
    for (i, src) in damaged.iter().enumerate() {
        let uri = format!("file:///t/damaged{i}.rut");
        let a = s.analyze(&uri, src);
        assert!(
            !a["diags"].as_array().unwrap().is_empty(),
            "not damaged: {src:?}"
        );
        let mut syms: Vec<Value> = a["symbols"].as_array().unwrap().clone();
        while let Some(x) = syms.pop() {
            let sel0 = pos_pair(&x["selectionRange"]["start"]);
            let sel1 = pos_pair(&x["selectionRange"]["end"]);
            let rng0 = pos_pair(&x["range"]["start"]);
            let rng1 = pos_pair(&x["range"]["end"]);
            assert!(
                sel0 >= rng0 && sel1 <= rng1,
                "`{}` selection outside range: {x}",
                x["name"].as_str().unwrap_or("?")
            );
            walked += 1;
            if let Some(kids) = x["children"].as_array() {
                syms.extend(kids.iter().cloned());
            }
        }
    }
    assert!(walked > 0, "damaged sources produced no symbols to check");
}

#[test]
fn the_legend_is_fifteen_standard_types() {
    // every type the legend names is a standard LSP token type and
    // every one has an emitter — no dead rows (the never-emitted
    // `comment` row was deleted, not appended to)
    let mut s = S::boot();
    let legend = s.legend();
    assert_eq!(legend.len(), 15);
    assert_eq!(legend[14], "operator");
    assert_eq!(legend[13], "namespace");
    assert_eq!(legend.last().map(String::as_str), Some("operator"));
}

#[test]
fn mod_decl_names_classify_as_namespace() {
    // `mod NAME;` declares a name-resolution head — the name reads
    // namespace, the same class the `::` path law gives the head at
    // its uses. `pub` rides (its ident never matches the name), and
    // the introducer word itself is token-owned by text.
    let mut s = S::boot();
    let src = "mod state;\npub mod theme;\nentry fn main() -> nil {}\n";
    let toks = toks_of(&mut s, "file:///t/moddecls.rut", src);
    assert_eq!(types_of(&toks, "state"), vec![t::NAMESPACE]);
    assert_eq!(types_of(&toks, "theme"), vec![t::NAMESPACE]);
    assert_eq!(types_of(&toks, "mod"), vec![t::KEYWORD, t::KEYWORD]);

    // the DECL file's file-form edges classify the same as the
    // implementation file's — ONE mod grammar, both modes
    let src = "mod net;\npub mod stream;\n";
    let a = s.analyze("file:///t/mods.d.rut", src);
    assert!(a["diags"].as_array().unwrap().is_empty(), "{a}");
    let toks = decode_tokens(src, &a);
    assert_eq!(types_of(&toks, "net"), vec![t::NAMESPACE]);
    assert_eq!(types_of(&toks, "stream"), vec![t::NAMESPACE]);

    // the inline block form is GONE from the grammar: a stray block in
    // a `.d.rut` is the ordinary ``expected `;` `` error — refusal
    // parity with an implementation file
    let src = "mod net {\n    mod stream;\n}\n";
    let a = s.analyze("file:///t/modblock.d.rut", src);
    assert!(
        a["diags"].as_array().unwrap().iter().any(|d| d["message"]
            .as_str()
            .unwrap_or("")
            .contains("expected `;`, found `{`")),
        "decl files refuse the block form with the ordinary error: {a}"
    );
    let b = s.analyze("file:///t/modblock.rut", src);
    assert!(
        b["diags"].as_array().unwrap().iter().any(|d| d["message"]
            .as_str()
            .unwrap_or("")
            .contains("expected `;`, found `{`")),
        "impl files keep the refusal: {b}"
    );
}

#[test]
fn colon2_value_paths_classify_by_the_resolution_law() {
    // `::` spells name resolution: interior segments are heads (a
    // lowercase head is a module → namespace, a capitalized one a
    // type), the leaf is the member (a lowercase fn, a capitalized
    // enum member/static); a lone lowercase seg stays a local read and
    // a property never appears — `.` spells value access
    let mut s = S::boot();
    let src = "mod layout;\nenum Color { Red }\nentry fn main() -> nil {\n    let a = layout::mk();\n    let b = Color::Red;\n    let c = i32::abs(-1);\n    let d = b;\n}\n";
    let toks = toks_of(&mut s, "file:///t/colon2.rut", src);
    // the module head — namespace, not a local variable. The DECL's own
    // name classifies the same way: the declared head and the path
    // head are the same kind of thing, so both read namespace
    assert_eq!(types_of(&toks, "layout"), vec![t::NAMESPACE, t::NAMESPACE]);
    // the member fn through the module — function, not a property
    assert_eq!(types_of(&toks, "mk"), vec![t::FUNCTION]);
    // the enum head + its member — type, then enumMember (decl + use)
    assert_eq!(types_of(&toks, "Color"), vec![t::ENUM, t::TYPE]);
    assert_eq!(types_of(&toks, "Red"), vec![t::ENUM_MEMBER, t::ENUM_MEMBER]);
    // a primitive head stays a type at an interior position (the
    // token layer's class, confirmed by the path walk)
    assert_eq!(types_of(&toks, "i32"), vec![t::TYPE]);
    assert_eq!(types_of(&toks, "abs"), vec![t::FUNCTION]);
    // the lone lowercase path is still the local read (decl + use)
    assert_eq!(types_of(&toks, "b"), vec![t::VARIABLE, t::VARIABLE]);
    // the law: `::` never colors a property
    assert!(!toks.iter().any(|t| t.ty == t::PROPERTY), "{toks:?}");
}

#[test]
fn a_name_spelled_default_is_an_ordinary_variable() {
    // `default` is no keyword anywhere — the parser never matches it,
    // so the AST pass owns the name: decl and use both classify as
    // plain variables
    let mut s = S::boot();
    let src = "entry fn main() -> nil {\n    let default = 1;\n    let x = default;\n}\n";
    let toks = toks_of(&mut s, "file:///t/default.rut", src);
    assert_eq!(types_of(&toks, "default"), vec![t::VARIABLE, t::VARIABLE]);
}

#[test]
fn use_lines_classify_by_the_capitalization_law() {
    // the use line colors best-effort, without resolution: a path
    // segment reads as its head (lowercase → namespace, capitalized →
    // type), an imported name as the leaf member (capitalized → type,
    // lowercase → function); `entry` before `fn` is token-owned by text
    let mut s = S::boot();
    let src = "use ink::{ Logger };\nentry fn main() -> nil {}\n";
    let toks = toks_of(&mut s, "file:///t/useline.rut", src);
    assert_eq!(types_of(&toks, "ink"), vec![t::NAMESPACE]);
    assert_eq!(types_of(&toks, "Logger"), vec![t::TYPE]);
    assert_eq!(types_of(&toks, "entry"), vec![t::KEYWORD]);

    // a lowercase import colors a function; a keyword-named head stays
    // token-owned (`mod` joins the classifier's keyword set — the text
    // class wins in the token stream even in a legal path slot)
    let src = "use mod::{ helper };\n";
    let toks = toks_of(&mut s, "file:///t/usemod.rut", src);
    assert_eq!(types_of(&toks, "mod"), vec![t::KEYWORD]);
    assert_eq!(types_of(&toks, "helper"), vec![t::FUNCTION]);
}

#[test]
fn multi_seg_use_paths_classify_each_lane() {
    // `use pkg::mod::{ name };` — the interior segments are the module
    // heads (namespace), the brace leaf the import (function when
    // lowercase) — mirroring the corpus's `use http_host::net::{ .. };`
    let mut s = S::boot();
    let src = "use http_host::net::{ http_send };\nentry fn main() -> nil {}\n";
    let toks = toks_of(&mut s, "file:///t/multiseg.rut", src);
    assert_eq!(types_of(&toks, "http_host"), vec![t::NAMESPACE]);
    assert_eq!(types_of(&toks, "net"), vec![t::NAMESPACE]);
    assert_eq!(types_of(&toks, "http_send"), vec![t::FUNCTION]);
}

#[test]
fn comments_never_enter_the_token_stream() {
    // comments have NO legend row — the token stream never carries
    // one, a `//` inside a string literal stays ONE string token, and
    // the classified spans stay sorted/disjoint/in-file around them
    // (the comment side table itself is the lexer's own lane, beyond
    // this door)
    let mut s = S::boot();
    let src = "let s = \"a\"/*c*/;\nlet t = \"// not a comment\"; // real\n// lead\n";
    let a = s.analyze("file:///t/comments.rut", src);
    let toks = decode_tokens(src, &a);
    // the string interior `//` is TOKEN interior — one string token
    // covers it whole
    assert_eq!(types_of(&toks, "\"// not a comment\""), vec![t::STRING]);
    // no token's text carries a comment introducer
    for t in &toks {
        assert!(
            !t.text.starts_with("//") && !t.text.starts_with("/*"),
            "a comment leaked into the stream: {t:?}"
        );
        assert!(
            t.lo < t.hi && t.hi <= src.len(),
            "in-file, non-empty: {t:?}"
        );
    }
    // sorted + disjoint
    let mut prev_hi = 0usize;
    for t in &toks {
        assert!(t.lo >= prev_hi, "overlap at {}", t.lo);
        prev_hi = t.hi;
    }
    assert!(toks.iter().any(|t| t.ty == t::KEYWORD), "{toks:?}");
}

#[test]
fn broken_source_still_classifies_and_recovers_symbols() {
    // missing closing brace — the recovery tree simply yields fewer
    // names; `entry` is token-owned by text now, so like the reserved
    // words it never drops its class; the diags name the break and the
    // symbol tree still answers (the recovering entry — the editor law)
    let mut s = S::boot();
    let src = "// still colored\nentry fn main( {\n  let x = ;\n";
    let a = s.analyze("file:///t/broken.rut", src);
    assert!(
        !a["diags"].as_array().unwrap().is_empty(),
        "the break is diagnosed"
    );
    let toks = decode_tokens(src, &a);
    assert_eq!(types_of(&toks, "entry"), vec![t::KEYWORD]);
    assert_eq!(types_of(&toks, "let"), vec![t::KEYWORD]);
    assert_eq!(types_of(&toks, "fn"), vec![t::KEYWORD]);
    // the intact call shape beside the damage still colors (`main` a
    // function), the reserved words never drop theirs
    assert_eq!(types_of(&toks, "main"), vec![t::FUNCTION], "{toks:?}");
    // a damaged `.d.rut` still answers a tree (symbols present)
    let a = s.analyze("file:///t/broken.d.rut", "class B {\n    fn m();\n");
    let _syms = a["symbols"].as_array().unwrap();
}

#[test]
fn doc_chains_graft_onto_decl_nodes() {
    // the editor law Document bakes: include_comments is ON, so the
    // doc chain lands on the decl node itself (D7) — through the door
    // the hover carries the chain lines
    let mut s = S::boot();
    let src = "// the total\n// in two lines\nlet TOTAL = 10;\n";
    let h = s.hover_at("file:///t/total.rut", src, "TOTAL", 1);
    let md = markdown(h);
    assert!(md.contains("the total"), "{md}");
    assert!(md.contains("in two lines"), "{md}");
}
