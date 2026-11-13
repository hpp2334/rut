//! Corpus conformance — the runnable example projects
//! (`examples/**`), the playground classics (`demo/src/examples/`),
//! the stdlib (`rut/**`), and the bench workloads: every `*.rut` in
//! the repo parses with zero diags (declaration mode for `*.d.rut`).
//! Together they are the parser's conformance suite.
//!
//! The file also carries the grammar-law pins that rode the v1
//! corpus.rs (restored from port-pending at phase 4, names and laws
//! intact, re-expressed over the slot enums). Two laws from the
//! deferred file already live elsewhere and are not duplicated here:
//! the depth budget pins in `deep.rs`, and the dump-through-format
//! round-trip in `rut-fmt`'s corpus test — the fmt lane owns it, since
//! rut-ast cannot dev-depend on rut-fmt without a cycle.

mod common;

use common::parse;
use rut_ast::parser::Mode;
use rut_ast::{ItemRef, ParamRef, TyRef, Vis};

fn corpus() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let roots = [
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../demo/src/examples"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../rut"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benches/workloads"),
    ];
    let mut stack = roots.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().map(|x| x == "rut").unwrap_or(false) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn corpus_parses_clean() {
    let files = corpus();
    assert!(!files.is_empty(), "the corpus roots must exist");
    let mut failures = Vec::new();
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else {
            continue;
        };
        let mode = if f.to_string_lossy().ends_with(".d.rut") {
            rut_ast::parser::Mode::Decl
        } else {
            rut_ast::parser::Mode::Impl
        };
        let (ast, diags) = common::parse(&src, mode);
        if !diags.is_empty() {
            let first = &diags[0];
            failures.push(format!("{}:{}: {}", f.display(), first.span.lo, first.msg));
            continue;
        }
        // the tree itself must be sound: every item slot resolves
        let _ = ast.root().items(&ast).len();
    }
    assert!(
        failures.is_empty(),
        "{} of {} corpus files failed:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}

#[test]
fn enum_member_values_are_gone() {
    // `= int` after a member no longer parses: one "expected `,` or `}`"
    // diag pointing at the `=`, no cascade, and the enum still collects
    // its members — the stray run skips to `,` or the closing `}` (a
    // broken member region may leave an Error slot beside the survivors;
    // the count here is the LIVE members)
    for (src, want_members) in [("enum T { a = 1 }", 1), ("enum E { A = 1, B }", 2)] {
        let (ast, diags) = parse(src, Mode::Impl);
        assert_eq!(diags.len(), 1, "one diag, no cascade: {src} -> {diags:?}");
        assert!(
            diags[0].msg.contains("expected `,` or `}`, found `=`"),
            "want the member-value diag, got: {diags:?}"
        );
        let items = ast.root().items(&ast);
        let Some(ItemRef::Enum(h)) = items.iter().find(|i| matches!(i, ItemRef::Enum(_))) else {
            panic!("enum item still parses: {diags:?}")
        };
        let members = &h.get(&ast).members;
        let live = members.iter().filter(|m| !m.is_error()).count();
        assert_eq!(live, want_members, "{src}: members: {members:?}");
    }
}

#[test]
fn reserved_words_explain_themselves() {
    for (word, want) in [("private", "pub"), ("void", "nil"), ("in", "of")] {
        let src = format!("fn f() -> nil {{ let x = {word}; }}");
        let (_, diags) = parse(&src, Mode::Impl);
        assert!(
            diags.iter().any(|d| d.msg.contains(want)),
            "`{word}` diag should name its rut replacement `{want}`: {diags:?}"
        );
    }
}

#[test]
fn interface_is_reserved_and_trait_is_not() {
    // `interface` moved from the lexer's hard-ban list to the parser's
    // reserved table (it gates struct-literal/lambda reads of the
    // word); `trait` is an ordinary identifier again — it lexes and
    // parses clean everywhere
    assert!(rut_ast::parser::is_reserved_kw("interface"));
    let src = "fn f() -> nil { let trait = 1; let y = trait; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(
        diags.is_empty(),
        "`trait` must lex and parse as an identifier: {diags:?}"
    );
    // ...and `interface` no longer hard-errors at lex — the word flows
    // through the parser like any identifier token
    let src = "fn f() -> nil { let x = interface; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(
        !diags.iter().any(|d| d.msg.contains("reserved word")),
        "`interface` must not be a lexer hard-ban anymore: {diags:?}"
    );
}

#[test]
fn retired_reservations_are_ordinary_identifiers() {
    // the 13 absence-phrased reservations retired: the words are plain
    // identifiers now, in BOTH modes — they parse clean, and the old
    // hard lex errors (`switch`, `match`, `null`, `var`, `delete`,
    // `any`, …) are gone. `let x = null;` degrades to an
    // unresolved-name error at RESOLUTION, not a lex/parse diag.
    for (label, src, mode) in [
        (
            "binding",
            "fn f() -> nil { let null = 5; let var = null; }",
            Mode::Impl,
        ),
        ("fn name", "fn match() -> nil { }", Mode::Impl),
        ("struct name", "struct var { n: i32 }", Mode::Impl),
        (
            "method call",
            "fn f() -> nil { let m = 1; let k = m.delete(m); }",
            Mode::Impl,
        ),
        (
            "with ident",
            "fn f() -> nil { let with = 1; let const = with; }",
            Mode::Impl,
        ),
        (
            "decl param + answer",
            "pub host fn del(m: opaque, k: bytes) -> any;",
            Mode::Decl,
        ),
    ] {
        let (_, diags) = parse(src, mode);
        assert!(
            diags.is_empty(),
            "{label}: retired reservation drew a diag: {diags:?}"
        );
    }
}

#[test]
fn fstring_holes_allow_string_arguments() {
    // corpus-driven (closures-generics.rut L32): hole termination is
    // brace-based, so string literals lex fine inside holes — the
    // "bind it first" stays a style note, not a lex error
    let src =
        "fn f() -> nil { let name = Option.some(\"x\"); print(f\"n={name.unwrap_or(\"?\")}\"); }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn tuples_parse_and_destructure() {
    // tuples are records with numeric fields
    let src = "fn f() -> nil { let x = (1, 2); let (a, b) = x; let n = x.0; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // a one-element paren is still a grouping, not a tuple
    let src = "fn f() -> nil { let x = (1); }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn pointers_and_nil_parse() {
    // `?T` types, the `nil` literal
    let src = "fn f() -> nil { let p: ?i32 = 7; if (p != nil) { } }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // anonymous closures: `fn (params) { .. }` — no arrow
    let src = "fn f() -> nil { let g = fn (a: i32) -> bool { return a > 0; }; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn nil_is_the_empty_type_and_value() {
    // v1.2: `unit` is gone; `nil` names the empty type and its value
    let src = "fn f() -> nil { let u: nil = nil; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn empty_parens_are_rejected_with_nil_hint() {
    // v1.2: neither `()` the value nor `()` the type survives; both
    // diagnostics must point at `nil`
    let (_, diags) = parse("fn f() -> nil { let x = (); }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`nil`")),
        "`()` value diag should point at `nil`: {diags:?}"
    );
    let (_, diags) = parse("fn f() -> () { }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`nil`")),
        "`()` type diag should point at `nil`: {diags:?}"
    );
    let (_, diags) = parse("fn f() -> nil { let x: () = nil; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`nil`")),
        "`()` annotation diag should point at `nil`: {diags:?}"
    );
}

#[test]
fn bracket_array_and_async_parse() {
    // `[T]` is the array type spelling
    let src = "fn f(xs: [i32]) -> i32 { return xs.len(); }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // `async fn` — the async spelling
    let src = "async fn tick() -> nil { }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // the fn type with an omitted return is `nil`
    let src = "fn run(f: fn(i32)) -> nil { f(1); }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn repeat_and_address_of_parse() {
    // `[v; n]` — a VALUE and a count; no type-in-expression form
    let src = "fn f() -> nil { let a: [i32] = [0; 8]; let b: [?i32] = [nil; 8]; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // the count is a full expression
    let src = "fn f(n: i32) -> nil { let a: [f64] = [1.5; n * 2 + 1]; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // bindings share — a `T` widens into `?T`, no `&` needed
    let src = "fn f() -> nil { let p: ?i32 = 7; let q: ?Point = Point { x: 1 }; let y: i32 = p; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // binary `&` and `*` still parse
    let src = "fn f(a: i32, b: i32) -> i32 { return a & b; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    let src = "fn f(a: i32, b: i32) -> i32 { return a * b; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn nullable_types_parse() {
    // user ruling: `?` is the ONE nullable spelling — prefix,
    // binding the following type TERM. `[?T]` is `[T | nil]` (nullable
    // ELEMENT — TyArray(TyOpt)), while `?[T]` is `[T] | nil` (nullable
    // ARRAY — TyOpt(TyArray)); `??T` chains, `?[?T]` nests the other way.
    let src = "fn f(a: [?i32], b: ?[i32], c: ??i32, d: ?[?i32]) -> ?i32 { return a[0]; }";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    let items = ast.root().items(&ast);
    let ItemRef::Fn(fh) = *items
        .iter()
        .find(|i| matches!(i, ItemRef::Fn(_)))
        .expect("fn")
    else {
        unreachable!()
    };
    let d = fh.get(&ast);
    let param_ty = |i: usize| -> Option<TyRef> {
        match d.params[i] {
            ParamRef::Param(h) => h.get(&ast).ty,
            ref p => panic!("param {i}, got {p:?}"),
        }
    };
    // `[?i32]` — the array OUTSIDE, the nullable on the ELEMENT
    let TyRef::Array(t0) = param_ty(0).unwrap() else {
        panic!("`[?i32]` must be TyArray(TyOpt), got {:?}", param_ty(0));
    };
    assert!(
        matches!(t0.get(&ast).elem, TyRef::Opt(_)),
        "the element carries the `?`"
    );
    // `?[i32]` — the nullable OUTSIDE, the array inside
    let TyRef::Opt(t1) = param_ty(1).unwrap() else {
        panic!("`?[i32]` must be TyOpt(TyArray), got {:?}", param_ty(1));
    };
    assert!(
        matches!(t1.get(&ast).inner, TyRef::Array(_)),
        "the array is the payload"
    );
    // `??i32` chains — Opt(Opt(path))
    let TyRef::Opt(t2) = param_ty(2).unwrap() else {
        panic!("`??i32` must be TyOpt, got {:?}", param_ty(2));
    };
    assert!(
        matches!(t2.get(&ast).inner, TyRef::Opt(_)),
        "the chain nests"
    );
    // `?[?i32]` — both at once: Opt(Array(Opt))
    let TyRef::Opt(t3) = param_ty(3).unwrap() else {
        panic!("`?[?i32]` must be TyOpt, got {:?}", param_ty(3));
    };
    let inner3 = t3.get(&ast).inner;
    let TyRef::Array(t3in) = inner3 else {
        panic!("the payload is the array, got {inner3:?}");
    };
    assert!(
        matches!(t3in.get(&ast).elem, TyRef::Opt(_)),
        "the element is nullable"
    );
    // the return keeps the same rule
    let Some(r) = d.ret else {
        panic!("`-> ?i32` must be TyOpt");
    };
    assert!(
        matches!(r, TyRef::Opt(_)),
        "`-> ?i32` must be TyOpt, got {r:?}"
    );
    // `?` composes with generics and fn types
    let src = "fn f(v: Vec<?Point>) -> fn(?i32) -> ?str { panic(\"\"); }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    // each union member carries its own `?` — prefix, before the member
    let src = "type U = ?i32 | ?str;";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn the_postfix_nullable_spelling_is_removed() {
    // the user ruling: `T?` / `[T]?` no longer parse — each trailing `?`
    // diagnoses pointing at the prefix spelling, and the bare type is
    // the recovery result (the Diag fails the compile either way)
    for src in [
        "fn f(p: i32?) -> nil { }",
        "fn f(p: [i32]?) -> nil { }",
        "fn f() -> [i32]? { return nil; }",
        "type U = i32? | str?;",
    ] {
        let (_, diags) = parse(src, Mode::Impl);
        assert!(
            diags
                .iter()
                .any(|d| d.msg.contains("the postfix spelling `T?` was removed")
                    && d.msg.contains("`?T`")),
            "`{src}` must diagnose the removed postfix spelling: {diags:?}"
        );
    }
}

#[test]
fn removed_pointer_spellings_diagnose() {
    // the `*T` type spelling points at `?T`...
    let (_, diags) = parse("fn f(p: *i32) -> nil { }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("`?T`")),
        "`*T` should diagnose `?T`: {diags:?}"
    );
    // ...and the `&x` / `*x` prefix forms point at direct passing
    let (_, diags) = parse("fn f() -> nil { let p: ?i32 = &7; }", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("share by reference")),
        "`&x` should diagnose the sharing law: {diags:?}"
    );
    let (_, diags) = parse(
        "fn f() -> nil { let p: ?i32 = 7; let y: i32 = *p; }",
        Mode::Impl,
    );
    assert!(
        diags.iter().any(|d| d.msg.contains("share by reference")),
        "`*x` should diagnose the sharing law: {diags:?}"
    );
}

#[test]
fn the_array_name_is_gone_from_the_grammar() {
    // the `Array` name is removed — the type is `[T]`, construction is
    // `[v; n]`; `[]` stays the empty list literal
    let src = "fn f() -> nil { let a: [i32] = []; }";
    let (_, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
}

// ---- file modules: mod decls, use paths, vis forms ----

#[test]
fn mod_decls_parse_with_vis_variants() {
    // `pub? mod NAME;` — a declaration only; the body lives in the
    // sibling `NAME/mod.rut`
    let src = "\
mod layout;
pub mod widget;
pub(pkg) mod ui;
pub(super) mod internal;
";
    let (ast, diags) = parse(src, Mode::Impl);
    assert!(diags.is_empty(), "expected a clean parse: {diags:?}");
    let items = ast.root().items(&ast);
    assert_eq!(items.len(), 4, "one decl per line: {items:?}");
    let decl = |i: usize| match items[i] {
        ItemRef::ModDecl(h) => {
            let d = h.get(&ast);
            (d.vis(), ast.name(d.name).to_string())
        }
        k => panic!("expected a mod decl at {i}, got {k:?}"),
    };
    assert_eq!(decl(0), (Vis::Self_, "layout".to_string()));
    assert_eq!(decl(1), (Vis::Pub, "widget".to_string()));
    assert_eq!(decl(2), (Vis::Pkg, "ui".to_string()));
    assert_eq!(decl(3), (Vis::Super, "internal".to_string()));
}

#[test]
fn use_paths_split_path_and_names() {
    // the leaf brace list / single name split off the mod path;
    // path[0] is always the package
    let (ast, diags) = parse("use pkg::A::B::{C, D};\n", Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    let items = ast.root().items(&ast);
    match items[0] {
        ItemRef::Use(h) => {
            let u = h.get(&ast);
            let segs: Vec<&str> = u.path.iter().map(|&s| ast.name(s)).collect();
            let nms: Vec<&str> = u.names.iter().map(|&n| ast.name(n)).collect();
            assert_eq!(segs, vec!["pkg", "A", "B"], "the mod path");
            assert_eq!(nms, vec!["C", "D"], "the leaf names");
        }
        k => panic!("expected a use item, got {k:?}"),
    }
    // the single-name leaf: the LAST segment pops into names
    let (ast, diags) = parse("use pkg::A::B::C;\n", Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    let items = ast.root().items(&ast);
    match items[0] {
        ItemRef::Use(h) => {
            let u = h.get(&ast);
            let segs: Vec<&str> = u.path.iter().map(|&s| ast.name(s)).collect();
            assert_eq!(segs, vec!["pkg", "A", "B"]);
            assert_eq!(u.names.len(), 1);
            assert_eq!(ast.name(u.names[0]), "C");
        }
        k => panic!("expected a use item, got {k:?}"),
    }
}

#[test]
fn flat_uses_degenerate_to_the_package_path() {
    // `use pkg::{A, B};` / `use pkg::A;` — behavior identical to the
    // flat grammar: path = [pkg]
    for (src, want_names) in [
        ("use pkg::{A, B};\n", vec!["A", "B"]),
        ("use pkg::A;\n", vec!["A"]),
    ] {
        let (ast, diags) = parse(src, Mode::Impl);
        assert!(diags.is_empty(), "{src}: {diags:?}");
        let items = ast.root().items(&ast);
        match items[0] {
            ItemRef::Use(h) => {
                let u = h.get(&ast);
                assert_eq!(u.path.len(), 1, "{src}: the degenerate path");
                assert_eq!(ast.name(u.path[0]), "pkg");
                let nms: Vec<&str> = u.names.iter().map(|&n| ast.name(n)).collect();
                assert_eq!(nms, want_names, "{src}");
            }
            k => panic!("{src}: expected a use item, got {k:?}"),
        }
    }
}

#[test]
fn use_with_an_empty_leaf_diagnoses() {
    // `use pkg::A::B;` imports nothing — mods are not values. One diag
    // naming the fix; the item still parses (recovery).
    let (ast, diags) = parse("use pkg::A::B;\n", Mode::Impl);
    assert_eq!(diags.len(), 1, "one diag, no cascade: {diags:?}");
    assert!(
        diags[0].msg.contains("imports nothing")
            && diags[0].msg.contains("spell a name at the leaf"),
        "the empty-leaf diag names the fix: {diags:?}"
    );
    let items = ast.root().items(&ast);
    match items[0] {
        ItemRef::Use(h) => {
            let u = h.get(&ast);
            let segs: Vec<&str> = u.path.iter().map(|&s| ast.name(s)).collect();
            assert_eq!(segs, vec!["pkg", "A", "B"], "the spelled path survives");
            assert!(u.names.is_empty(), "nothing was imported");
        }
        k => panic!("expected a use item, got {k:?}"),
    }
}

#[test]
fn pub_use_parses_with_the_reexport_vis() {
    // `pub use` — the re-export tool: the use grammar is unchanged
    // beneath the modifier (brace lists, paths, the single-name leaf),
    // the vis rides the node. Plain `use` stays `Vis::Self_`.
    for (src, want_vis) in [
        ("pub use pkg::{A, B};\n", Vis::Pub),
        ("pub use pkg::A::B::C;\n", Vis::Pub),
        ("pub(super) use pkg::{A};\n", Vis::Super),
        ("pub(pkg) use pkg::A;\n", Vis::Pkg),
        ("use pkg::{A};\n", Vis::Self_),
    ] {
        let (ast, diags) = parse(src, Mode::Impl);
        assert!(diags.is_empty(), "{src}: {diags:?}");
        let items = ast.root().items(&ast);
        match items[0] {
            ItemRef::Use(h) => {
                let u = h.get(&ast);
                assert_eq!(u.vis(), want_vis, "{src}: the re-export vis");
                // span law v3.1: the span starts at the item's FIRST
                // token — the `pub`/`pub(..)` modifier when one is
                // spelled, `use` otherwise
                let want_head = match want_vis {
                    Vis::Pub => "pub use",
                    Vis::Super => "pub(super) use",
                    Vis::Pkg => "pub(pkg) use",
                    Vis::Self_ => "use",
                };
                let text = u.span.text(src);
                assert!(
                    text.starts_with(want_head),
                    "{src}: the span starts at its first token: {text}"
                );
            }
            k => panic!("{src}: expected a use item, got {k:?}"),
        }
    }
    // Decl-mode files parse it too (a `.d.rut` surface may re-export)
    let (ast, diags) = parse("pub use core::{ opaque };\n", Mode::Decl);
    assert!(diags.is_empty(), "{diags:?}");
    match ast.root().items(&ast)[0] {
        ItemRef::Use(h) => assert_eq!(h.get(&ast).vis(), Vis::Pub),
        k => panic!("expected a use item, got {k:?}"),
    }
}

#[test]
fn inline_mod_blocks_are_gone() {
    // no inline `mod { }` blocks — the body lives in `NAME/mod.rut`.
    // One diag naming the file location; the declaration survives and
    // the tail keeps parsing.
    let src = "mod W { fn f() -> nil { } }\nfn g() -> nil { }\n";
    let (ast, diags) = parse(src, Mode::Impl);
    assert_eq!(diags.len(), 1, "one diag, no cascade: {diags:?}");
    assert!(
        diags[0].msg.contains("inline `mod { }` blocks are gone")
            && diags[0].msg.contains("NAME/mod.rut"),
        "the inline-block diag names the fix: {diags:?}"
    );
    let items = ast.root().items(&ast);
    assert!(matches!(items[0], ItemRef::ModDecl(_)), "the decl survives");
    assert!(matches!(items[1], ItemRef::Fn(_)), "the tail still parses");
}

#[test]
fn pub_mod_and_pub_self_are_gone_everywhere() {
    // the paren spellings diagnose at module scope too; the bare
    // default and the widened forms keep parsing
    let (ast, diags) = parse("pub(mod) type T = i64;\n", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("are gone")),
        "{diags:?}"
    );
    assert!(matches!(ast.root().items(&ast)[0], ItemRef::Alias(_)));
    let (_, diags) = parse("pub(self) mod m;", Mode::Impl);
    assert!(
        diags.iter().any(|d| d.msg.contains("are gone")),
        "{diags:?}"
    );
    // the surviving set: `pub(super)`, `pub(pkg)`, `pub`
    let (ast, diags) = parse(
        "pub(super) mod a;\npub(pkg) mod b;\npub mod c;\n",
        Mode::Impl,
    );
    assert!(diags.is_empty(), "{diags:?}");
    let items = ast.root().items(&ast);
    let vis = |i: usize| match items[i] {
        ItemRef::ModDecl(h) => h.get(&ast).vis(),
        _ => panic!("expected a mod decl"),
    };
    assert_eq!(vis(0), Vis::Super);
    assert_eq!(vis(1), Vis::Pkg);
    assert_eq!(vis(2), Vis::Pub);
}

#[test]
fn inf_is_the_infinity_float_literal() {
    // D2: ident `inf` in EXPRESSION position folds to the infinity
    // float literal (the `true`/`false` precedent — no lexer keyword),
    // f64 always (D1, the family end-to-end); `-inf` is unary
    // negation on it (folded at the const lowering), and `inf` as a
    // NAME stays a plain ident — only expression position folds
    use rut_ast::{ExprRef, Lit, StmtRef, UnOp};
    use rut_lexer::token::FloatSuffix;

    let (ast, diags) = parse("fn f() -> nil { let a = inf; let b = -inf; }", Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    let body = common::first_fn_body(&ast);
    let stmts = body.get(&ast).stmts.to_vec();
    let inf_lit = |e: ExprRef, what: &str| match e {
        ExprRef::Lit(lh) => match &lh.get(&ast).lit {
            Lit::Float(bits, Some(FloatSuffix::F64)) => {
                assert_eq!(*bits, f64::INFINITY.to_bits(), "{what} carries inf bits");
            }
            l => panic!("{what} is the f64 infinity literal, got {l:?}"),
        },
        e => panic!("{what} folds to a literal, got {e:?}"),
    };
    match stmts[0] {
        StmtRef::Let(h) => inf_lit(h.get(&ast).init, "`inf`"),
        s => panic!("expected a let, got {s:?}"),
    }
    match stmts[1] {
        StmtRef::Let(h) => match h.get(&ast).init {
            ExprRef::Unary(uh) => {
                let u = uh.get(&ast);
                assert_eq!(u.op, UnOp::Neg, "`-inf` is unary negation");
                inf_lit(u.expr, "`-inf`'s operand");
            }
            e => panic!("`-inf` is unary negation, got {e:?}"),
        },
        s => panic!("expected a let, got {s:?}"),
    }

    // `inf` as a NAME — a plain ident, zero diags (expression USE of
    // the name reads the literal; the binding itself is ordinary)
    let (ast, diags) = parse("fn f() -> nil { let inf = 1.0; }", Mode::Impl);
    assert!(diags.is_empty(), "{diags:?}");
    let body = common::first_fn_body(&ast);
    match body.get(&ast).stmts[0] {
        StmtRef::Let(h) => assert_eq!(ast.name(h.get(&ast).name), "inf"),
        s => panic!("expected a let named `inf`, got {s:?}"),
    }
}
