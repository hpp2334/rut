//! Inlay hints' gate — TYPE hints on unannotated bindings and
//! PARAMETER hints at exact-arity call sites, through the `inlay`
//! envelope (full LSP values: position, label, kind, tooltip). Pins:
//! the hover-consistency law (one source), the never-wrong guards
//! (arity mismatch, ambiguity, own-surface members), and the viewport
//! range filter.

mod common;

use common::*;
use serde_json::Value;

/// the hint's label string
fn label(h: &Value) -> &str {
    h["label"].as_str().expect("a string label")
}

/// the hint's tooltip markdown
fn tooltip(h: &Value) -> &str {
    h["tooltip"]["value"].as_str().expect("a markdown tooltip")
}

fn labels(hs: &[Value]) -> Vec<&str> {
    hs.iter().map(label).collect()
}

/// (line, character) where the `occ`-th needle's ident ENDS — where a
/// TYPE hint hangs
fn end_of(src: &str, needle: &str, occ: usize) -> Value {
    let (l, c) = pos_after(src, needle, occ);
    serde_json::json!({ "line": l, "character": c })
}

fn pos(h: &Value) -> Value {
    h["position"].clone()
}

#[test]
fn unannotated_let_gets_a_type_hint() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nfn go() -> f64 {\n    let c = Circle::new(1.0);\n    return c.r;\n}\n";
    let uri = "file:///t/main.rut";
    let hs = s.inlay_all(uri, src);
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(label(&hs[0]), ": Circle");
    assert_eq!(hs[0]["kind"], 1, "TYPE kind: {}", hs[0]);
    // right after the declaring ident: `let c⌊: Circle⌋ = ..`
    assert_eq!(pos(&hs[0]), end_of(src, "let c", 1), "{}", hs[0]);
    // the tooltip IS the hover markdown
    assert!(
        tooltip(&hs[0]).contains("let c: Circle"),
        "{:?}",
        tooltip(&hs[0])
    );
    assert!(
        tooltip(&hs[0]).contains("type inferred"),
        "{:?}",
        tooltip(&hs[0])
    );
}

#[test]
fn annotated_lets_are_never_restated() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nfn go() -> nil {\n    let c: Circle = Circle::new(1.0);\n}\n";
    assert!(s.inlay_all("file:///t/main.rut", src).is_empty());
}

#[test]
fn inference_miss_stays_silent() {
    let mut s = S::boot();
    let src = "entry fn main() -> nil {\n    let x = mystery();\n}\n";
    assert!(s.inlay_all("file:///t/main.rut", src).is_empty());
}

#[test]
fn for_of_and_for_c_vars_get_hints() {
    let mut s = S::boot();
    let src = "struct Point {\n    x: f64;\n}\nfn sum(points: [Point]) -> f64 {\n    let total = 0.0;\n    for (let p of points) {\n        total += p.x;\n    }\n    for (let i = 0; i < 3; i += 1) {\n        total += total;\n    }\n    return total;\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    // the float default types `total`, the [Point] element types `p`,
    // the literal default types `i`
    assert_eq!(labels(&hs), [": f32", ": Point", ": i32"], "{hs:?}");
    assert_eq!(pos(&hs[1]), end_of(src, "for (let p", 1), "{}", hs[1]);
    assert_eq!(hs[1]["kind"], 1, "TYPE kind: {}", hs[1]);
    assert!(
        tooltip(&hs[1]).contains("p: Point"),
        "{:?}",
        tooltip(&hs[1])
    );
    assert!(
        tooltip(&hs[1]).contains("loop variable"),
        "{:?}",
        tooltip(&hs[1])
    );
}

#[test]
fn hint_and_hover_agree_on_the_type_text() {
    // the consistency law: the hint's type is the hover's type — one
    // source (the binding pass), defaults included
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nfn go() -> f64 {\n    let c = Circle::new(1.0);\n    return c.r;\n}\n";
    let uri = "file:///t/main.rut";
    let hs = s.inlay_all(uri, src);
    assert_eq!(hs.len(), 1);
    let md = markdown(s.hover_in(uri, src, "let c", 1, 4));
    assert!(md.contains("let c: Circle"), "{md}");
    let hover_ty = md
        .split("let c: ")
        .nth(1)
        .unwrap()
        .split('\n')
        .next()
        .unwrap();
    assert_eq!(label(&hs[0]), format!(": {hover_ty}"));
}

#[test]
fn module_let_gets_a_hint_at_its_decl() {
    let mut s = S::boot();
    let src = "let TOTAL = 41;\nfn bump() -> i32 {\n    return TOTAL + 1;\n}\n";
    let uri = "file:///t/main.rut";
    let hs = s.inlay_all(uri, src);
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(label(&hs[0]), ": i32");
    assert_eq!(pos(&hs[0]), end_of(src, "let TOTAL", 1));
    assert!(
        tooltip(&hs[0]).contains("let TOTAL = 41;"),
        "{:?}",
        tooltip(&hs[0])
    );
    // an annotated module let restates nothing
    let src = "let TOTAL: i32 = 41;\nfn bump() -> i32 {\n    return TOTAL + 1;\n}\n";
    assert!(s.inlay_all(uri, src).is_empty());
}

#[test]
fn destructured_literal_tuple_hints_each_name() {
    let mut s = S::boot();
    let src = "fn go() -> f64 {\n    let (a, b) = (1, 2.0);\n    return b;\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    assert_eq!(labels(&hs), [": i32", ": f32"], "{hs:?}");
    assert_eq!(pos(&hs[0]), end_of(src, "(a", 1));
    // the second hint hangs at the end of the `b` ident
    let (bl, bc) = pos_of(src, "b)", 1);
    assert_eq!(
        pos(&hs[1]),
        serde_json::json!({ "line": bl, "character": bc + 1 })
    );
}

#[test]
fn tuple_types_render_no_empty_hints() {
    // a tuple ret used to render empty (`ty_src` lacked the arm),
    // which leaked a bare `: ` hint — now the type renders as
    // written, in the hint AND the hover (one source)
    let mut s = S::boot();
    let src = "fn parse(s: str) -> (opaque, str) {\n    return (opaque(s), \"\");\n}\nentry fn main() -> str {\n    let v = parse(\"x\");\n    return \"\";\n}\n";
    let uri = "file:///t/main.rut";
    let hs = s.inlay_all(uri, src);
    // the type hint renders the tuple as written, and the call site's
    // param hint rides along
    assert_eq!(labels(&hs), [": (opaque, str)", "s:"], "{hs:?}");
    // ...and the hover shows the same text (the consistency law)
    let md = markdown(s.hover_in(uri, src, "let v", 1, 4));
    assert!(md.contains("let v: (opaque, str)"), "{md}");
}

// ---- generic args render verbatim ----

#[test]
fn generic_constructor_hints_the_full_type() {
    // the args are the OWNING doc's source text (the span join): the
    // hint AND the hover binding line carry `Pair<i32, i32>` whole
    let mut s = S::boot();
    let src = "class Pair<A, B> {\n    head: A;\n}\nfn go() -> i32 {\n    let p = Pair<i32, i32>();\n    return p.head;\n}\n";
    let uri = "file:///t/main.rut";
    let hs = s.inlay_all(uri, src);
    assert_eq!(labels(&hs), [": Pair<i32, i32>"], "{hs:?}");
    // the hover binding line rides the same `binding_ty` — one source
    let md = markdown(s.hover_in(uri, src, "let p", 1, 4));
    assert!(md.contains("let p: Pair<i32, i32>"), "{md}");
}

#[test]
fn nested_generic_args_render_verbatim() {
    // `Pair<Vec<str>, i32>()` — the arg's span covers the whole
    // `Vec<str>` slice, so nesting renders by construction (no
    // normalization, no stringifier)
    let mut s = S::boot();
    let src = "class Pair<A, B> {\n    head: A;\n}\nfn go() -> i32 {\n    let p = Pair<Vec<str>, i32>();\n    return 1;\n}\n";
    let uri = "file:///t/main.rut";
    let hs = s.inlay_all(uri, src);
    assert_eq!(labels(&hs), [": Pair<Vec<str>, i32>"], "{hs:?}");
}

#[test]
fn generic_new_call_renders_its_args() {
    // `Map<i32, str>::new(..)` — the parser's PathGen staging records
    // the args on the HEAD seg; the two-seg arm renders them
    let mut s = S::boot();
    let src = "class Map<K, V> {\n    n: K;\n}\nfn go() -> i32 {\n    let m = Map<i32, str>::new(1);\n    return m.n;\n}\n";
    let uri = "file:///t/main.rut";
    let hs = s.inlay_all(uri, src);
    assert_eq!(labels(&hs), [": Map<i32, str>"], "{hs:?}");
    let md = markdown(s.hover_in(uri, src, "let m", 1, 4));
    assert!(md.contains("let m: Map<i32, str>"), "{md}");
}

#[test]
fn bare_constructor_stays_bare() {
    // the empty-generics path is untouched: `Logger("hello")` still
    // types the binding `Logger`
    let mut s = S::boot();
    let src = "class Logger {\n    n: i32;\n}\nfn go() -> i32 {\n    let log = Logger(\"hello\");\n    return log.n;\n}\n";
    let uri = "file:///t/main.rut";
    let hs = s.inlay_all(uri, src);
    assert_eq!(labels(&hs), [": Logger"], "{hs:?}");
}

// ---- the Future family ----

#[test]
fn await_unwraps_a_future_method_ret() {
    // the repro: `const b = await resp.body();` — the impl method
    // answers `Future<bytes>`, the await unwraps to `bytes` (one
    // source: the hint's label is the hover's binding line)
    let mut s = S::boot();
    let src = "class Client {\nn: i32;\n}\nimpl Client {\npub fn send(self) -> Future<Response> { return nil; }\n}\n\
               class Response {\nn: i32;\n}\nimpl Response {\npub fn body(self) -> Future<bytes> { return nil; }\n}\n\
               async fn work(r: Client) -> bytes {\n    const resp: Response = await r.send();\n    const b = await resp.body();\n    return b;\n}\n";
    let uri = "file:///t/main.rut";
    let hs = s.inlay_all(uri, src);
    assert_eq!(labels(&hs), [": bytes"], "{hs:?}");
    assert_eq!(pos(&hs[0]), end_of(src, "const b", 1), "{}", hs[0]);
    assert!(
        tooltip(&hs[0]).contains("const b: bytes"),
        "{:?}",
        tooltip(&hs[0])
    );
    assert!(
        tooltip(&hs[0]).contains("type inferred"),
        "{:?}",
        tooltip(&hs[0])
    );
}

#[test]
fn nested_future_generics_unwrap_to_the_inner_text() {
    // `Future<Vec<str>>` → `Vec<str>` — the depth-aware first-argument
    // cut keeps nesting whole
    let mut s = S::boot();
    let src = "class Buf {\nn: i32;\n}\nimpl Buf {\npub fn lines(self) -> Future<Vec<str>> { return nil; }\n}\n\
               fn go(b: Buf) -> nil {\n    const ls = await b.lines();\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    assert_eq!(labels(&hs), [": Vec<str>"], "{hs:?}");
}

#[test]
fn non_future_await_stays_silent() {
    // the unwrap is strict: `await` on a non-Future is a miss, never
    // a wrong type
    let mut s = S::boot();
    let src =
        "fn n() -> i32 { return 1; }\nentry fn main() -> nil {\n    const x = await n();\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    assert!(hs.is_empty(), "{:?}", labels(&hs));
}

#[test]
fn awaiting_an_optional_future_stays_silent() {
    // strictness: the head is read WITHOUT the `?` peel — the engine
    // rejects awaiting an optional future, so `?Future<..>` misses
    let mut s = S::boot();
    let src = "fn f() -> ?Future<i32> { return nil; }\nentry fn main() -> nil {\n    const g = f();\n    const x = await g;\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    // the operand itself still types; only the await stays silent
    assert_eq!(labels(&hs), [": ?Future<i32>"], "{hs:?}");
}

#[test]
fn an_async_fn_call_types_future_and_awaits_to_the_ret() {
    // the engine's sugar law: a call to an `async` fn answers
    // `Future<R>` even unawaited (the engine value is the frame)
    let mut s = S::boot();
    let src = "async fn get() -> i32 { return 1; }\nentry fn main() -> nil {\n    const v = await get();\n    const f = get();\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    assert_eq!(labels(&hs), [": i32", ": Future<i32>"], "{hs:?}");
}

#[test]
fn unwritten_async_ret_answers_future_nil() {
    // the engine's ret default: an unwritten async ret means `nil`,
    // so the call answers `Future<nil>` (the bare written ret used to
    // leak `: nil`)
    let mut s = S::boot();
    let src = "async fn job() { }\nentry fn main() -> nil {\n    const f = job();\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    assert_eq!(labels(&hs), [": Future<nil>"], "{hs:?}");
}

#[test]
fn an_async_block_types_its_returns_and_the_awaits_chain() {
    // the weave's founds law: the answer is the body's valued return
    // (`return 5;` — no tail), wrapped `Future<i32>`; the chained
    // await unwraps it
    let mut s = S::boot();
    let src =
        "entry fn main() -> nil {\n    const f = async { return 5; };\n    const b = await f;\n}\n";
    let uri = "file:///t/main.rut";
    let hs = s.inlay_all(uri, src);
    assert_eq!(labels(&hs), [": Future<i32>", ": i32"], "{hs:?}");
    // the hover counterpart rides the same binding_ty — one source
    let md = markdown(s.hover_in(uri, src, "const b = await", 1, 6));
    assert!(md.contains("const b: i32"), "{md}");
}

#[test]
fn await_through_a_local_unwraps() {
    let mut s = S::boot();
    let src = "class Response {\nn: i32;\n}\nimpl Response {\npub fn body(self) -> Future<bytes> { return nil; }\n}\n\
               fn go(r: Response) -> nil {\n    const fut = r.body();\n    const b = await fut;\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    assert_eq!(labels(&hs), [": Future<bytes>", ": bytes"], "{hs:?}");
}

#[test]
fn spelled_call_site_generics_substitute() {
    // `make<i32>()` names `T := i32` — the ret prices under the map,
    // the async wrap rides after (`Future<i32>` → await → `i32`)
    let mut s = S::boot();
    let src = "async fn make<T>() -> T { return nil; }\nentry fn main() -> nil {\n    const v = await make<i32>();\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    assert_eq!(labels(&hs), [": i32"], "{hs:?}");
}

#[test]
fn argument_driven_substitution_matches_structurally() {
    // `sleep_like()` infers `Future<str>`; the declared `Future<T>`
    // param matches it — heads equal, generic arity equal — binding
    // `T := str`; the ret prices to the arg's element text
    let mut s = S::boot();
    let src = "async fn sleep_like() -> str { return \"\"; }\n\
               async fn drive<T>(f: Future<T>) -> T { return await f; }\n\
               entry fn main() -> nil {\n    const s = await drive(sleep_like());\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    // the substituted ret (`: str`) plus the call's own param hint
    // (`f:`) — the arity match rides along
    assert_eq!(labels(&hs), [": str", "f:"], "{hs:?}");
}

#[test]
fn residual_type_vars_stay_silent() {
    // the never-wrong law, enforced: an unsolvable generic ret is
    // silence — the old leak rendered `: T`
    let mut s = S::boot();
    let src =
        "fn lazy<T>() -> T { return nil; }\nentry fn main() -> nil {\n    const v = lazy();\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    assert!(hs.is_empty(), "{:?}", labels(&hs));
}

#[test]
fn host_surface_async_fn_types_future_and_awaits() {
    // the reversed v1 ruling: a surface fn's written ret IS the
    // crossing's contract — `probe(x)` answers `Future<bytes>`,
    // `await probe(x)` answers `bytes`
    let mut s = S::boot();
    s.index_file(
        "file:///t/host.d.rut",
        "pub host async fn probe(r: opaque) -> bytes;\n",
    );
    let src = "fn go() -> nil {\n    const p = probe(opaque(0));\n    const b = await probe(opaque(0));\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    // the type texts, plus each call's own `r:` param hint (the
    // surface fn records its params; the arity match rides along)
    assert_eq!(
        labels(&hs),
        [": Future<bytes>", "r:", ": bytes", "r:"],
        "{hs:?}"
    );
}

// ---- PARAMETER hints ----

#[test]
fn param_hints_at_a_free_call() {
    let mut s = S::boot();
    let src = "fn hex_val(c: str, k: i32) -> i32 {\n    return k;\n}\nentry fn main() -> i32 {\n    return hex_val(\"a\", 2);\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    assert_eq!(labels(&hs), ["c:", "k:"], "{hs:?}");
    assert_eq!(hs[0]["kind"], 2, "PARAMETER kind: {}", hs[0]);
    // each hint sits at the arg's first byte
    assert_eq!(pos(&hs[0]), pos_json(src, "\"a\"", 1), "{}", hs[0]);
    assert_eq!(pos(&hs[1]), pos_json(src, "2);", 1), "{}", hs[1]);
}

/// (line, character) as a JSON position — the arg-start asserts
fn pos_json(src: &str, needle: &str, occ: usize) -> Value {
    let (l, c) = pos_of(src, needle, occ);
    serde_json::json!({ "line": l, "character": c })
}

#[test]
fn param_hints_at_a_method_call_skip_self() {
    let mut s = S::boot();
    let src = "class Circle {\n    r: f64;\n}\nimpl Circle {\n    fn grown(self, k: f64) -> Circle { return self; }\n}\nfn go() -> Circle {\n    let c = Circle::new(1.0);\n    return c.grown(2.0);\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    // the let's TYPE hint + the call's PARAMETER hint (`self` never
    // surfaces — `new` has no recorded impl fn here, `grown` does)
    assert_eq!(labels(&hs), [": Circle", "k:"], "{hs:?}");
    assert_eq!(pos(&hs[1]), pos_json(src, "2.0)", 1), "{}", hs[1]);
}

#[test]
fn arity_mismatch_means_no_hints() {
    // the lets are annotated (type hints would otherwise fire —
    // inference doesn't arity-check); only the PARAMETER hints are
    // under test here: mismatch → none, never wrong names
    let mut s = S::boot();
    let src = "fn f(a: i32, b: i32) -> i32 {\n    return a;\n}\nentry fn main() -> i32 {\n    let x: i32 = f(1);\n    let y: i32 = f(1, 2, 3);\n    let z: i32 = f();\n    return x + y + z;\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    assert!(hs.is_empty(), "{:?}", labels(&hs));
}

#[test]
fn ambiguous_callee_stays_silent() {
    // the doc's `f` plus a same-named free fn in another index: no
    // unique match, no hints (the call_ty law)
    let mut s = S::boot();
    s.index_file(
        "file:///ws/other/lib.rut",
        "fn f(b: str) -> str {\n    return b;\n}\n",
    );
    let src = "fn f(a: i32) -> i32 {\n    return a;\n}\nfn main() -> i32 {\n    let x = f(1);\n    return x;\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    assert!(hs.is_empty(), "{:?}", labels(&hs));
}

#[test]
fn own_surface_method_beats_a_same_named_impl_fn() {
    // an interface-body `m` is the receiver's own surface: the FINAL
    // answer carries no recorded params, so no hints — the impl fn's
    // params must not leak through (never wrong hints)
    let mut s = S::boot();
    let src = "struct P {\n    x: i32;\n}\ninterface Draw {\n    fn m(self, k: i32) -> i32;\n}\nimpl P {\n    pub fn m(self, k: i32) -> i32 { return k; }\n}\nfn go(d: Draw) -> i32 {\n    return d.m(1);\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    assert!(hs.is_empty(), "{:?}", labels(&hs));
}

#[test]
fn nested_calls_hint_at_their_own_args() {
    let mut s = S::boot();
    let src = "fn inner(a: i32) -> i32 {\n    return a;\n}\nfn outer(b: i32, c: i32) -> i32 {\n    return b + c;\n}\nentry fn main() -> i32 {\n    return outer(inner(1), 2);\n}\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    // by position: outer's first arg STARTS at `inner`, inner's own
    // arg sits inside it
    assert_eq!(labels(&hs), ["b:", "a:", "c:"], "{hs:?}");
    assert_eq!(pos(&hs[0]), pos_json(src, "inner(1)", 1));
    assert_eq!(pos(&hs[1]), pos_json(src, "1)", 1));
    assert_eq!(pos(&hs[2]), pos_json(src, "2);", 1));
}

#[test]
fn module_let_initializer_calls_get_param_hints() {
    let mut s = S::boot();
    let src = "fn mk(n: i32) -> i32 {\n    return n;\n}\nlet N = mk(3);\n";
    let hs = s.inlay_all("file:///t/main.rut", src);
    assert_eq!(labels(&hs), [": i32", "n:"], "{hs:?}");
}

// ---- the range filter ----

#[test]
fn range_filters_hints() {
    let mut s = S::boot();
    let src = "let A = 1;\nlet B = 2;\n";
    let uri = "file:///t/main.rut";
    let all = s.inlay_all(uri, src);
    assert_eq!(labels(&all), [": i32", ": i32"]);
    // only the second line's hint stays inside [line 1, end]
    let hs = s.inlay_from(uri, src, "let B", 1);
    assert_eq!(hs.len(), 1, "{hs:?}");
    assert_eq!(pos(&hs[0])["line"], 1);
    // a range before everything is empty, never wrong
    let empty = take(s.pool.run_until(s.lsp.inlay(uri.to_string(), 0, 0, 0, 0)));
    assert!(empty.as_array().unwrap().is_empty(), "{empty}");
}
