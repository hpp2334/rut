// RFC 0018 — the async/await landing: the legality diagnostics and the
// open-surface story. Every pinned message is the contract; the
// no-launcher mode proves ruling 8 (users may write their own
// launchers — the engine knows none of these names).

use rut_driver::{Module, ModuleBody, Session, compile_graph, mount_std_async, mount_std_core};

fn diags_of(src: &str) -> Vec<String> {
    let mut s = Session::new();
    mount_std_core(&mut s);
    mount_std_async(&mut s);
    s.register_module("app", Module { body: ModuleBody::Source { text: src.into(), is_decl: false }, ..Default::default() })
        .expect("register app");
    compile_graph(&s, "app").diags.iter().map(|d| d.msg.clone()).collect()
}

fn one_diag(src: &str) -> String {
    let d = diags_of(src);
    assert_eq!(d.len(), 1, "expected exactly one diagnostic, got {d:?}");
    d[0].clone()
}

#[test]
fn await_outside_an_async_fn_diagnoses() {
    let msg = one_diag(r#"
use async_host::sleep;

pub fn main() -> nil {
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
async fn work(cx: RunContext) -> nil {
    await 7;
}

pub fn main() -> nil {
    work();
}
"#);
    assert!(msg.contains("`await` needs a Future"), "got: {msg}");
    assert!(msg.contains("i32"), "got: {msg}");
}

#[test]
fn await_on_a_user_impl_future_diagnoses_in_v1() {
    let msg = one_diag(r#"
class NotWoven {
    n: i32 = 0;
}

async fn work(cx: RunContext, f: NotWoven) -> nil {
    await f;
}

pub fn main() -> nil {
    work(NotWoven { });
}
"#);
    assert!(
        msg.contains("engine-woven futures") && msg.contains("launchers"),
        "got: {msg}"
    );
}

#[test]
fn await_on_the_receipt_diagnoses_with_the_join_law() {
    let msg = one_diag(r#"
use async_host::{ launch_future, LaunchedFutureHandle };

async fn work(cx: RunContext) -> nil { }

async fn awaiter(cx: RunContext, h: LaunchedFutureHandle<nil>) -> nil {
    await h;
}

pub fn main() -> nil {
    awaiter(launch_future(work()));
}
"#);
    assert!(
        msg.contains("cannot `await` a LaunchedFutureHandle") && msg.contains("RFC 0019"),
        "got: {msg}"
    );
}

#[test]
fn relaunching_the_receipt_is_a_type_error() {
    // ruling 6: `launch_future(launch_future(f))` — the receipt is not
    // a Future; the arrow fails to type, never a runtime check
    let msg = one_diag(r#"
use async_host::launch_future;

async fn work(cx: RunContext) -> nil { }

pub fn main() -> nil {
    launch_future(launch_future(work()));
}
"#);
    assert!(
        msg.contains("cannot infer type parameter `T` of `launch_future`"),
        "got: {msg}"
    );
}

#[test]
fn await_select_stays_parse_only() {
    let msg = one_diag(r#"
use async_host::sleep;

async fn work(cx: RunContext) -> nil {
    let n = await select {
        sleep(5) -> 1,
    };
}

pub fn main() -> nil {
    work();
}
"#);
    assert!(msg.contains("`await select`") && msg.contains("RFC 0019"), "got: {msg}");
}

#[test]
fn an_async_fn_demands_the_cx_first_parameter() {
    let msg = one_diag(r#"
async fn work(n: u32) -> nil { }

pub fn main() -> nil {
    work(7);
}
"#);
    assert!(msg.contains("`cx: RunContext`"), "got: {msg}");
}

#[test]
fn runcontext_has_no_other_members() {
    let msg = one_diag(r#"
async fn work(cx: RunContext) -> nil {
    let x = cx.bogus();
}

pub fn main() -> nil {
    work();
}
"#);
    assert!(
        msg.contains("`RunContext` has no member `bogus`"),
        "got: {msg}"
    );
}

// ---- the open surface (RFC 0012 §7): users may write their own launchers ----

#[test]
fn a_user_launcher_over_the_same_future_surface() {
    // a minimal private launcher, typed by the SAME core surface: the
    // engine's standard set is untouched, the receipt is a user class,
    // and the driving API is the engine half the user's row binds to
    let mut s = Session::new();
    mount_std_core(&mut s);
    let engine = rut_driver::lower_decl_module(
        include_str!("../../../rut/async_engine/engine.d.rut"),
        "engine.d.rut",
    )
    .expect("engine surface");
    s.register_module("my_engine", engine).expect("mount");
    let src = r#"
use my_engine::{ __launch };

class MyHandle {
    f: ?Future<nil>;
}

fn launch(f: Future<nil>) -> MyHandle {
    __launch(opaque(f));
    return MyHandle { f: f };
}

async fn tick(cx: RunContext) -> nil { }

pub fn main() -> nil {
    launch(tick());
}
"#;
    s.register_module("app", Module { body: ModuleBody::Source { text: src.into(), is_decl: false }, ..Default::default() })
        .expect("register app");
    let out = compile_graph(&s, "app");
    assert!(out.diags.is_empty(), "diags: {:?}", out.diags);
}
