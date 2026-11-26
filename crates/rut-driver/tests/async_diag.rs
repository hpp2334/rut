// the async/await landing: the legality diagnostics and the
// open-surface story. Every pinned message is the contract; the
// no-launcher mode proves ruling 8 (users may write their own
// launchers — the engine knows none of these names).

use std::collections::BTreeMap;

/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            .mount(rut_driver::Mounted::source(spec, src))
            .entrypoint(spec)
            .compile(),
    )
}

/// [`compiled`] with calc offered: core auto-rides, `calc` is an
/// ordinary pkg).
#[allow(dead_code)]
fn compiled_std(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            // the root DECLARES its calc use (the structural form; the
            // offer stays the walked tree pkg)
            .mount(
                rut_driver::Mounted::source(spec, src)
                    .manifest(rut_driver::Manifest::lib(spec, &["calc_host"])),
            )
            .mount(rut_native::tree_pkg("calc_host").expect("the toolchain tree's rut/calc_host"))
            .entrypoint(spec)
            .compile(),
    )
}

#[allow(dead_code)]
fn graph_of(c: Result<rut_driver::Compiled, rut_driver::RunError>) -> rut_driver::GraphOutput {
    match c {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(
                rut_lexer::span::Span::new(0, 0),
                e.msg,
            )],
            program: None,
        },
    }
}

fn graph_diags(src: &str) -> Vec<rut_lexer::diag::Diag> {
    let async_pkgs = rut_native::dir_pkgs(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/futures"),
    )
    .expect("the async pair walks")
    .pkgs;
    let compiled = rut_driver::Walk {
        pkgs: async_pkgs,
        root: String::new(),
    }
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
    .mount(
        rut_driver::Mounted::source("app", src)
            .manifest(rut_driver::Manifest::lib("app", &["futures"])),
    )
    .entrypoint("app")
    .compile()
    .unwrap();
    compiled.graph.diags
}

fn diags_of(src: &str) -> Vec<String> {
    graph_diags(src).iter().map(|d| d.msg.clone()).collect()
}

fn one_diag(src: &str) -> String {
    let d = diags_of(src);
    assert_eq!(d.len(), 1, "expected exactly one diagnostic, got {d:?}");
    d[0].clone()
}

#[test]
fn await_outside_an_async_fn_diagnoses() {
    let msg = one_diag(
        r#"
use futures::sleep;

entry fn main() -> nil {
    await sleep(5);
}
"#,
    );
    assert!(msg.contains("`await` outside an async fn"), "got: {msg}");
}

#[test]
fn await_on_a_non_future_diagnoses() {
    let msg = one_diag(
        r#"
async fn work() -> nil {
    await 7;
}

entry fn main() -> nil {
    work();
}
"#,
    );
    assert!(msg.contains("`await` needs a Future"), "got: {msg}");
    assert!(msg.contains("i32"), "got: {msg}");
}

#[test]
fn await_on_a_user_type_diagnoses() {
    // futures are engine-minted only: awaiting a user class value
    // diagnoses with the frame law — the record is a raw frame type
    // that cannot surface, and the awaitable is the async CALL's
    // `Future<T>`, never the argument
    let msg = one_diag(
        r#"
class NotWoven {
    n: i32 = 0;
}

async fn work(f: NotWoven) -> nil {
    await f;
}

entry fn main() -> nil {
    work(NotWoven { });
}
"#,
    );
    assert!(
        msg.contains("`await` needs a `Future<..>`")
            && msg.contains("`NotWoven` is a raw frame type and cannot surface anymore")
            && msg.contains("the call's `Future<T>` is the awaitable"),
        "got: {msg}"
    );
}

#[test]
fn await_on_the_receipt_diagnoses_with_the_join_law() {
    let msg = one_diag(
        r#"
use futures::{ launch_future, LaunchedFutureHandle };

async fn work() -> nil { }

async fn awaiter(h: LaunchedFutureHandle<nil>) -> nil {
    await h;
}

entry fn main() -> nil {
    awaiter(launch_future(work()));
}
"#,
    );
    assert!(
        msg.contains("cannot `await` a LaunchedFutureHandle")
            && msg.contains("cannot be re-launched"),
        "got: {msg}"
    );
}

#[test]
fn relaunching_the_receipt_is_a_type_error() {
    // ruling 6: `launch_future(launch_future(f))` — the receipt is not
    // a Future; the arrow fails to type, never a runtime check
    let msg = one_diag(
        r#"
use futures::launch_future;

async fn work() -> nil { }

entry fn main() -> nil {
    launch_future(launch_future(work()));
}
"#,
    );
    assert!(
        msg.contains("cannot infer type parameter `T` of `launch_future`"),
        "got: {msg}"
    );
}

#[test]
fn an_async_fn_never_spells_the_cx_parameter() {
    // the resume context is INJECTED as `cx` — a spelled parameter of
    // that name diagnoses (the parameter list carries ordinary values);
    // the refusal is a declaration-level law, so it sits at the
    // declaration's span, never at the call that reached it
    let src = r#"
async fn work(cx: i32) -> nil { }

entry fn main() -> nil {
    work(7);
}
"#;
    let ds = graph_diags(src);
    assert_eq!(ds.len(), 1, "expected exactly one diagnostic, got {ds:?}");
    assert!(
        ds[0]
            .msg
            .contains("an async fn's parameters are ordinary values")
            && ds[0]
                .msg
                .contains("the resume context is injected as `cx`, never spelled"),
        "got: {ds:?}"
    );
    // at the declaration's span — the parameter rides the decl
    let needle = "async fn work(cx: i32) -> nil { }";
    let lo = src.find(needle).expect("the declaration") as u32;
    let hi = lo + needle.len() as u32;
    assert!(
        ds[0].span.lo >= lo && ds[0].span.hi <= hi,
        "the refusal must sit at the declaration: {:?} vs {lo}..{hi}",
        ds[0].span
    );
}

#[test]
fn runcontext_has_no_other_members() {
    let msg = one_diag(
        r#"
use core::{ RunContext };

async fn work() -> nil {
    let x = cx.bogus();
}

entry fn main() -> nil {
    work();
}
"#,
    );
    assert!(
        msg.contains("`RunContext` has no member `bogus`"),
        "got: {msg}"
    );
}

// ---- law 2: a declaration-level refusal fires at the declaration,
// call or no call. The two weave refusals sit at fn registration now,
// and an async METHOD is refused dead like a called one.

#[test]
fn a_generic_async_fn_refuses_at_its_declaration() {
    // the weave's generic refusal lives at registration now: a generic
    // async fn refuses whether or not anything ever calls it
    let src = "async fn g<T>(x: T) -> T { return x; }\n\
               entry fn main() -> nil { }\n";
    let ds = graph_diags(src);
    assert_eq!(ds.len(), 1, "expected exactly one diagnostic, got {ds:?}");
    assert!(
        ds[0]
            .msg
            .contains("generic async fns are not woven in this build"),
        "got: {ds:?}"
    );
    // at the declaration's span
    let needle = "async fn g<T>(x: T) -> T { return x; }";
    let lo = src.find(needle).expect("the declaration") as u32;
    let hi = lo + needle.len() as u32;
    assert!(
        ds[0].span.lo >= lo && ds[0].span.hi <= hi,
        "the refusal must sit at the declaration: {:?} vs {lo}..{hi}",
        ds[0].span
    );
}

#[test]
fn a_spelled_cx_parameter_refuses_at_its_declaration() {
    // the spelled-`cx` law needs no call site: the fn below is
    // call-site-shaped (a trait-object parameter — no root reaches it
    // uncalled) and refused all the same, at registration
    let src = "interface Num { fn m(self) -> i32; }\n\
               class WI(i32);\n\
               impl WI { pub fn m(self) -> i32 { return self.inner; } }\n\
               async fn work(cx: Num) -> nil { }\n\
               entry fn main() { }\n";
    let ds = graph_diags(src);
    assert_eq!(ds.len(), 1, "expected exactly one diagnostic, got {ds:?}");
    assert!(
        ds[0]
            .msg
            .contains("an async fn's parameters are ordinary values")
            && ds[0]
                .msg
                .contains("the resume context is injected as `cx`, never spelled"),
        "got: {ds:?}"
    );
    // at the declaration's span — the parameter rides the decl
    let needle = "async fn work(cx: Num) -> nil { }";
    let lo = src.find(needle).expect("the declaration") as u32;
    let hi = lo + needle.len() as u32;
    assert!(
        ds[0].span.lo >= lo && ds[0].span.hi <= hi,
        "the refusal must sit at the declaration: {:?} vs {lo}..{hi}",
        ds[0].span
    );
}

#[test]
fn a_dead_async_method_refuses_at_its_declaration() {
    // law 3: an async METHOD is refused, dead or called — the private
    // method below is no root-skipped body anymore, so the compile
    // lane's own refusal fires without a call site
    let src = "class Wob { n: i32 }\n\
               impl Wob {\n\
               \x20   async fn tick(self) -> nil { }\n\
               }\n\
               entry fn main() -> nil { }\n";
    let ds = graph_diags(src);
    assert_eq!(ds.len(), 1, "expected exactly one diagnostic, got {ds:?}");
    assert!(
        ds[0]
            .msg
            .contains("async methods are not woven in this build"),
        "got: {ds:?}"
    );
    // at the declaration's span
    let needle = "async fn tick(self) -> nil { }";
    let lo = src.find(needle).expect("the declaration") as u32;
    let hi = lo + needle.len() as u32;
    assert!(
        ds[0].span.lo >= lo && ds[0].span.hi <= hi,
        "the refusal must sit at the declaration: {:?} vs {lo}..{hi}",
        ds[0].span
    );
}

#[test]
fn a_dead_pub_async_method_refuses_at_its_declaration() {
    // the pub half: an exported async method roots eagerly — for the
    // refusal only (the surface rows still carry no async method)
    let src = "class Wob { n: i32 }\n\
               impl Wob {\n\
               \x20   pub async fn tick(self) -> nil { }\n\
               }\n\
               entry fn main() -> nil { }\n";
    let ds = graph_diags(src);
    assert_eq!(ds.len(), 1, "expected exactly one diagnostic, got {ds:?}");
    assert!(
        ds[0]
            .msg
            .contains("async methods are not woven in this build"),
        "got: {ds:?}"
    );
    // at the declaration's span
    let needle = "pub async fn tick(self) -> nil { }";
    let lo = src.find(needle).expect("the declaration") as u32;
    let hi = lo + needle.len() as u32;
    assert!(
        ds[0].span.lo >= lo && ds[0].span.hi <= hi,
        "the refusal must sit at the declaration: {:?} vs {lo}..{hi}",
        ds[0].span
    );
}

// ---- the open surface: users may write their own launchers ----

#[test]
fn a_user_launcher_over_the_same_future_surface() {
    // a minimal private launcher, typed by the SAME core surface: the
    // engine's standard set is untouched, the receipt is a user class,
    // and the driving API is the engine half the user's row binds to
    let engine = rut_driver::lower_decl_module(
        include_str!("../../../rut/async_host/src/engine.d.rut"),
        &BTreeMap::new(),
        "engine.d.rut",
    )
    .expect("engine surface")
    .named("my_engine");
    let src = r#"
use core::{ Future };
use my_engine::{ __launch };

class MyHandle {
    f: ?Future<nil>;
}

fn launch(f: Future<nil>) -> MyHandle {
    __launch(opaque(f));
    return MyHandle { f: f };
}

async fn tick() -> nil { }

entry fn main() -> nil {
    launch(tick());
}
"#;
    let compiled = rut_driver::RutRun::new()
        .mount(engine)
        .mount(
            rut_driver::Mounted::source("app", src)
                .manifest(rut_driver::Manifest::lib("app", &["my_engine"])),
        )
        .entrypoint("app")
        .compile()
        .unwrap();
    assert!(
        compiled.graph.diags.is_empty(),
        "diags: {:?}",
        compiled.graph.diags
    );
}
