//! The t1 twin-gate support: boot the FRAMEWORK HARNESS
//! (`tests/t1_harness.rut` over the mounted project) on the fake-DOM
//! twin, and the read helpers every t1 test shares — snapshots, live
//! handles as twin node ids, listener ids found through the host's own
//! listener rows (§6's law: hooks become the tests' vocabulary, numeric
//! ids stay plumbing).

use std::cell::RefCell;
use std::rc::Rc;

use rut_vm::interp::{ HostHooks, HostRegistry, Vm };
use rut_vm::{ OpaqueBox, OpaqueRef };

use todolist_web::fake_dom::{ FakeDom, Snapshot };
use todolist_web::host::WebHost;
use todolist_web::{ hosts, mount, state, WebState };

pub const HARNESS: &str = include_str!("../t1_harness.rut");

/// The booted twin host the tests pass around.
pub type Host = WebHost<FakeDom>;

/// Boot the harness on the twin: mount the MANIFEST (the rut/ project
/// — t1 mounts `inline` per its own manifest, exactly the shape the
/// probes need), compile the harness as its own root over the closure
/// — the inline splice puts the framework's source in the harness
/// unit, so the t1p_ probes read `root.els/regs/prev` same-unit —
/// bind BOTH body sets, `verify_against`, seed `#app`, run the boot
/// turn. (The old second hand-registration of t1 is gone: the
/// manifest's `inline = true` IS that statement now.)
pub fn make_host() -> (WebHost<FakeDom>, OpaqueRef) {
    let (mut session, _root) = mount::load_project_session().expect("the rut/ project mounts");
    let expected = session.expected_host_fns();
    let prog = mount::compile_root(&mut session, HARNESS, "t1_harness")
        .expect("the harness compiles");

    let (slot, sink) = state::weak_sink_slot::<FakeDom>();
    let shared = Rc::new(RefCell::new(WebState::new(FakeDom::new(sink))));
    state::bind_weak_sink(&slot, &shared);
    shared.borrow_mut().dom.seed_page("div", "app"); // the twin's index.html

    let mut hosts = HostRegistry::new();
    hosts::install_web_hosts(&mut hosts, &shared);
    rut_std::nmap::install_std_nmap(&mut hosts);
    hosts.verify_against(&expected);

    let vm = Vm::new(Rc::new(prog), &mount::limits(), HostHooks::default(), hosts)
        .expect("the vm boots");
    let mut host = WebHost::new(shared, vm);
    let app = host.boot().expect("the boot turn runs");
    (host, app)
}

/// One element's shape, read by the hook the lowering emitted as `id`.
pub fn snap(host: &WebHost<FakeDom>, hook: &str) -> Snapshot {
    host.with_dom(|d| {
        d.snapshot_by_id(hook).unwrap_or_else(|| panic!("no element '#{hook}' in the twin's tree"))
    })
}

/// Run one scenario render probe (`t1p_*`).
pub fn render(host: &mut WebHost<FakeDom>, app: &OpaqueRef, probe: &str) {
    host
        .call::<_, ()>(probe, (app.clone(),))
        .unwrap_or_else(|e| panic!("{probe} failed: {}", e.msg));
}

/// The twin's node id behind a live framework handle — ELEMENT
/// IDENTITY through the fake twin's handles: a rebuild mints fresh
/// ids, so equal ids across renders = the node survived.
pub fn node_id(host: &mut WebHost<FakeDom>, app: &OpaqueRef, path: &str) -> u32 {
    let h: OpaqueRef = host
        .call("t1p_els_handle", (app.clone(), path.to_string()))
        .unwrap_or_else(|e| panic!("t1p_els_handle('{path}') failed: {}", e.msg));
    el_id(&h)
}

fn el_id(h: &OpaqueRef) -> u32 {
    OpaqueBox::<todolist_web::fake_dom::El>::from_handle(h)
        .expect("an element handle")
        .with(|el| el.id)
        .expect("the element payload borrows")
}

/// The host listener row serving `path`'s element (§6: the tests find
/// ids through the hook, never by predicting the counter).
pub fn listener_of(host: &mut WebHost<FakeDom>, app: &OpaqueRef, path: &str) -> i64 {
    let nid = node_id(host, app, path);
    host
        .state
        .borrow()
        .listeners
        .iter()
        .find(|(_, row)| row.el.id == nid)
        .map(|(id, _)| *id)
        .unwrap_or_else(|| panic!("no listener registered for {path}"))
}

/// The framework registry's live subject rows.
pub fn regs_len(host: &mut WebHost<FakeDom>, app: &OpaqueRef) -> i32 {
    host.call("t1p_regs_len", (app.clone(),)).unwrap()
}

/// The subject one firing id answers, "" when absent (retired).
pub fn regs_subject(host: &mut WebHost<FakeDom>, app: &OpaqueRef, id: i64) -> String {
    host.call("t1p_regs_subject", (app.clone(), id.to_string())).unwrap()
}

/// The framework's live handle rows (path -> element).
pub fn els_len(host: &mut WebHost<FakeDom>, app: &OpaqueRef) -> i32 {
    host.call("t1p_els_len", (app.clone(),)).unwrap()
}

pub fn els_has(host: &mut WebHost<FakeDom>, app: &OpaqueRef, path: &str) -> bool {
    host.call("t1p_els_has", (app.clone(), path.to_string())).unwrap()
}

/// The delivered subjects, "|"-joined in arrival order.
pub fn log_of(host: &mut WebHost<FakeDom>, app: &OpaqueRef) -> String {
    host.call::<_, String>("t1p_log", (app.clone(),)).unwrap()
}

/// How many listener rows the HOST holds (ids are minted once per
/// element lifetime and the host never retires them — the twin's
/// documented no-unlisten limit; rut's registry is what retires).
pub fn host_listener_len(host: &WebHost<FakeDom>) -> usize {
    host.state.borrow().listeners.len()
}
