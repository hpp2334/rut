// the async/await landing: the legality diagnostics and the
// open-surface story. Every pinned message is the contract; the
// no-launcher mode proves ruling 8 (users may write their own
// launchers — the engine knows none of these names).






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

fn diags_of(src: &str) -> Vec<String> {
    let async_pkgs = rut_driver::std_async_pkgs().expect("the async pair walks");
    let compiled = rut_driver::RutRun::new()
        .pkgs(&rut_driver::Loaded { pkgs: async_pkgs, root: String::new() })
        .pkg(rut_driver::Pkg::source("app", src))
        .entrypoint("app")
        .compile()
        .unwrap();
    compiled.graph.diags.iter().map(|d| d.msg.clone()).collect()
}

fn one_diag(src: &str) -> String {
    let d = diags_of(src);
    assert_eq!(d.len(), 1, "expected exactly one diagnostic, got {d:?}");
    d[0].clone()
}

#[test]
fn await_outside_an_async_fn_diagnoses() {
    let msg = one_diag(r#"
use futures::sleep;

entry fn main() -> nil {
    await sleep(5);
}
"#);
    assert!(
        msg.contains("`await` outside an async fn"),
        "got: {msg}"
    );
}

#[test]
fn await_on_a_non_future_diagnoses() {
    let msg = one_diag(r#"
async fn work() -> nil {
    await 7;
}

entry fn main() -> nil {
    work();
}
"#);
    assert!(msg.contains("`await` needs a Future"), "got: {msg}");
    assert!(msg.contains("i32"), "got: {msg}");
}

#[test]
fn await_on_a_user_type_diagnoses() {
    // futures are engine-minted only: awaiting a user class value
    // diagnoses with the frame law — the record is a raw frame type
    // that cannot surface, and the awaitable is the async CALL's
    // `Future<T>`, never the argument
    let msg = one_diag(r#"
class NotWoven {
    n: i32 = 0;
}

async fn work(f: NotWoven) -> nil {
    await f;
}

entry fn main() -> nil {
    work(NotWoven { });
}
"#);
    assert!(
        msg.contains("`await` needs a `Future<..>`")
            && msg.contains("`NotWoven` is a raw frame type and cannot surface anymore")
            && msg.contains("the call's `Future<T>` is the awaitable"),
        "got: {msg}"
    );
}

#[test]
fn await_on_the_receipt_diagnoses_with_the_join_law() {
    let msg = one_diag(r#"
use futures::{ launch_future, LaunchedFutureHandle };

async fn work() -> nil { }

async fn awaiter(h: LaunchedFutureHandle<nil>) -> nil {
    await h;
}

entry fn main() -> nil {
    awaiter(launch_future(work()));
}
"#);
    assert!(
        msg.contains("cannot `await` a LaunchedFutureHandle") && msg.contains("cannot be re-launched"),
        "got: {msg}"
    );
}

#[test]
fn relaunching_the_receipt_is_a_type_error() {
    // ruling 6: `launch_future(launch_future(f))` — the receipt is not
    // a Future; the arrow fails to type, never a runtime check
    let msg = one_diag(r#"
use futures::launch_future;

async fn work() -> nil { }

entry fn main() -> nil {
    launch_future(launch_future(work()));
}
"#);
    assert!(
        msg.contains("cannot infer type parameter `T` of `launch_future`"),
        "got: {msg}"
    );
}

#[test]
fn an_async_fn_never_spells_the_cx_parameter() {
    // the resume context is INJECTED as `cx` — a spelled parameter of
    // that name diagnoses (the parameter list carries ordinary values)
    let msg = one_diag(r#"
async fn work(cx: i32) -> nil { }

entry fn main() -> nil {
    work(7);
}
"#);
    assert!(
        msg.contains("an async fn's parameters are ordinary values")
            && msg.contains("the resume context is injected as `cx`, never spelled"),
        "got: {msg}"
    );
}

#[test]
fn runcontext_has_no_other_members() {
    let msg = one_diag(r#"
use core::{ RunContext };

async fn work() -> nil {
    let x = cx.bogus();
}

entry fn main() -> nil {
    work();
}
"#);
    assert!(
        msg.contains("`RunContext` has no member `bogus`"),
        "got: {msg}"
    );
}

// ---- the open surface: users may write their own launchers ----

#[test]
fn a_user_launcher_over_the_same_future_surface() {
    // a minimal private launcher, typed by the SAME core surface: the
    // engine's standard set is untouched, the receipt is a user class,
    // and the driving API is the engine half the user's row binds to
    let engine = rut_driver::lower_decl_module(
        include_str!("../../../rut/async_host/engine.d.rut"),
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
        .pkg(engine)
        .pkg(rut_driver::Pkg::source("app", src))
        .entrypoint("app")
        .compile()
        .unwrap();
    assert!(
        compiled.graph.diags.is_empty(),
        "diags: {:?}",
        compiled.graph.diags
    );
}
