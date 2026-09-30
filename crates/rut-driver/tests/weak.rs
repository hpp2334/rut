//! `Weak<T>` — the weak reference. The
//! surface is the `builtin class` row beside `StackTrace`,
//! constructed by the class method `Weak.new(v)` with one
//! member `upgrade() -> ?T`. The pins here:
//!
//! - alive round trip: `upgrade` retains the live referent (field reads
//!   through the answer);
//! - death: the referent's last strong handle released ⇒ `upgrade`
//!   answers nil, forever (repeat upgrades stable);
//! - the cache shape: weaks over retired cells answer nil, live cells
//!   still hit (`Weak<T>` in param position = the type-position arm);
//! - immortals: a weak over an enum-member singleton upgrades forever;
//! - `Weak<?U>`: legal, `upgrade` answers `??U` (MakeOpt wraps the box —
//!   the sticky-`?` law); the weak refers to the OPT BOX, so nil-ing the
//!   binding kills it;
//! - weak over an `opaque(v)` box and over a HOST box (D1): store-entry
//!   referents die on the entry path and their weaks go dead with them;
//! - the generic path: `Weak.new(v)` inside a generic fn instantiates per
//!   concrete `T`;
//! - box identity: two `Weak.new(v)` of one `v` are distinct boxes (`==` is
//!   the cell-identity law); aliasing one box is equality;
//! - admission + diagnostics: `Weak.new(prim)` and `Weak.new(fn)` diagnose
//!   ("weak needs a reference type"), `weak on nil` traps, arity and
//!   member errors name the one-member contract, `impl Weak` refuses;
//! - the retired type-call spelling: a bare call of the type name does
//!   not compile — the diagnostic names the class-method fix;
//! - the consuming op: `Weak.new(make())` watches a referent that dies
//!   with the argument's temporary;
//! - the heap comes back clean: usage after the run returns to the
//!   post-boot baseline (the weak lists leak nothing), and an OOM at the
//!   WeakNew mint surfaces as `OutOfMemory` before any registration.
//!
//! VERSION 13: the weak batch (TyKind::Weak + the two Weak ops — new
//! encoded vocabulary, the bump law).

use rut_driver::{Module, ModuleBody, Session};
use rut_vm::OpaqueRef;
use std::cell::RefCell;
use std::rc::Rc;

/// Compile, flatten, verify, and run `main` — the i32 checksum.
fn run_main(src: &str) -> i32 {
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module("app_main", Module { body: ModuleBody::Source { text: src.into(), is_decl: false }, ..Default::default() }).unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

/// Run `main`, answering the trap name (None = clean run).
fn run_trap(src: &str) -> Option<String> {
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module("app_main", Module { body: ModuleBody::Source { text: src.into(), is_decl: false }, ..Default::default() }).unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    match vm.call::<_, i32>("main", ()) {
        Ok(_) => None,
        Err(t) => Some(t.name()),
    }
}

/// Compile only — the rendered diagnostics (negative pins).
fn compile_diags(src: &str) -> Vec<String> {
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    s.register_module("app_main", Module { body: ModuleBody::Source { text: src.into(), is_decl: false }, ..Default::default() }).unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    out.diags
        .iter()
        .map(|d| format!("{d:?}"))
        .collect()
}

/// The logged lane (the e2e harness shape): ink's Logger captures lines
/// — the observation channel for `dispose` bodies, which run at the
/// root call's drain (or a drive's, on the async lanes).
fn run_logged(src: &str) -> (Vec<String>, Option<String>) {
    let combined = format!("{src}\nuse ink::{{Logger}};\n");
    let mut s = Session::new();
    rut_driver::mount_std(&mut s);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    rut_driver::mount_dir(&mut s, &root.join("rut/ink")).expect("mount ink (+rt)");
    s.register_module("app_main", Module { body: ModuleBody::Source { text: combined.clone().into(), is_decl: false }, ..Default::default() }).unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    assert!(
        out.diags.is_empty(),
        "unexpected diags:\n{}",
        rut_lexer::diag::render_diags(&combined, &out.diags)
    );
    let prog = out.program.expect("linked program");
    rut_vm::verify::verify(&prog).expect("verify");
    let lines: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = lines.clone();
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_std::logger::install_std_log(&mut hosts, move |msg| {
        sink.borrow_mut().push(msg.to_string());
    });
    rut_std::math::install_std_math(&mut hosts);
    hosts.verify_against(&s.expected_host_fns());
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(std::rc::Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts).expect("vm");
    let trap = match vm.call::<_, ()>("main", ()) {
        Ok(_) => None,
        Err(t) => Some(t.name()),
    };
    let out_lines = lines.borrow().clone();
    (out_lines, trap)
}

const TILE: &str = r#"
class Tile { n: i32; }
impl Tile {
    fn new(n: i32) -> Self { return Self { n: n }; }
}
"#;

// ---- the alive round trip ------------------------------------------------

#[test]
fn upgrade_round_trips_while_alive() {
    let src = format!(
        r#"{TILE}
pub fn main() -> i32 {{
    let t = Tile.new(41);
    let w = Weak.new(t);
    let b = w.upgrade();
    if (b == nil) {{ return 0; }}   // the referent is alive: the answer is not nil
    return b.n + 1;                 // the SAME cell: field reads see the value
}}
"#
    );
    assert_eq!(run_main(&src), 42);
}

// ---- death ----------------------------------------------------------------

#[test]
fn death_answers_nil_forever() {
    let src = format!(
        r#"{TILE}
pub fn main() -> i32 {{
    let mut t = Tile.new(7);
    let w = Weak.new(t);
    t = Tile.new(99);       // the old cell loses its last strong handle: it dies NOW
    let a = w.upgrade();
    if (a != nil) {{ return 1; }}   // the weak did not keep it alive
    let b = w.upgrade();
    if (b != nil) {{ return 2; }}   // repeat upgrades stay nil — stable, not resurrecting
    return 42;
}}
"#
    );
    assert_eq!(run_main(&src), 42);
}

// ---- the cache shape (Weak<T> in type position) ---------------------------

#[test]
fn the_cache_shape_probes_live_and_dead_entries() {
    let src = format!(
        r#"{TILE}
fn alive_n(w: Weak<Tile>) -> i32 {{
    let b = w.upgrade();
    if (b == nil) {{ return 0; }}
    return b.n;
}}
pub fn main() -> i32 {{
    let a = Tile.new(10);
    let mut retired = Tile.new(20);
    let mut fresh = Tile.new(30);
    let wa = Weak.new(a);
    let wd = Weak.new(retired);
    let wf = Weak.new(fresh);
    retired = Tile.new(21);   // the old cell retires: wd's referent dies
    fresh = Tile.new(31);     // wf's referent dies too
    return alive_n(wa) + alive_n(wd) + alive_n(wf);   // 10 + 0 + 0
}}
"#
    );
    assert_eq!(run_main(&src), 10);
}

// ---- the cycle law (the logged lane — dispose is the observer) ------------

#[test]
fn weak_back_pointer_breaks_the_cycle_and_dispose_sees_nil() {
    // The pair is BUILT in a callee (its temps die at the ret), the
    // parent handle comes back fused (the sole reference), and the sever
    // is `parent = nil` in main: parent dies → its dispose runs → the
    // field release kills the child → the child's dispose runs (the
    // drain loops). The child's dispose UPGRADES its weak back-pointer
    // from INSIDE dispose: the parent's weak list was nulled before any
    // user code ran, so the answer is nil — the deterministic-ordering
    // pin (no resurrection, the shape frees to the last cell).
    let src = r#"
use core::{ Disposal, DisposalContext };
class Node {
    name: str;
    other: ?Node = nil;            // the STRONG edge (parent -> child)
    back: ?Weak<?Node> = nil;      // the WEAK edge (child -> parent): the cycle cannot close
}
impl Node {
    fn new(name: str) -> Self { return Self { name: name, other: nil, back: nil }; }
}
impl Disposal for Node {
    fn dispose(mut self, cx: DisposalContext) {
        if (self.back != nil) {
            let b = self.back.upgrade();   // INSIDE dispose: the parent's box died first —
            if (b != nil) { panic("cycle resurrected"); }   // the nulling preceded user code
        }
        Logger.new("w").info(f"{self.name} gone");
    }
}
fn build() -> ?Node {
    let mut p: ?Node = Node.new("p");
    let mut c: ?Node = Node.new("c");
    let oc: ?Node = c;
    p.other = oc;                  // the STRONG edge
    let wp: ?Weak<?Node> = Weak.new(p);
    c.back = wp;                   // the WEAK edge — the cycle cannot close
    return p;                      // the parent handle crosses; the child rides p.other
}
pub fn main() -> nil {
    let mut parent = build();      // fused: parent is the ONLY reference
    parent = nil;                  // sever: the whole shape must free, in order
}
"#;
    let (lines, trap) = run_logged(src);
    assert_eq!(trap, None, "dispose ran without trapping (no resurrection)");
    assert_eq!(
        lines,
        vec!["p gone", "c gone"],
        "both nodes freed — the weak back-pointer closed no cycle"
    );
}

// ---- immortals --------------------------------------------------------------


#[test]
fn weak_over_immortal_enum_member_upgrades_forever() {
    let src = r#"
enum E { A, B }
pub fn main() -> i32 {
    let e = E.A;              // the immortal singleton cell
    let w = Weak.new(e);          // never dies: the list is never nulled
    let b = w.upgrade();
    if (b == nil) { return 0; }
    let b2 = w.upgrade();
    if (b2 == nil) { return 1; }
    return 42;
}
"#;
    assert_eq!(run_main(src), 42);
}

// ---- Weak<?U>: the sticky-? law ---------------------------------------------

#[test]
fn weak_over_nullable_answers_double_optional_and_dies_with_the_box() {
    // the referent of `Weak.new(ot)` is the OPT BOX itself; `upgrade`
    // answers `??Tile` — the ??U spelling pinned as probe_ot's return
    // type. The kill+probe split across a call boundary: in-frame
    // temporaries would lawfully pin the box past the kill.
    let src = format!(
        r#"{TILE}
fn probe_ot(w: Weak<?Tile>) -> ??Tile {{
    return w.upgrade();        // MakeOpt wraps the box — the sticky-? law
}}
pub fn main() -> i32 {{
    let mut ot: ?Tile = Tile.new(5);   // the OPT BOX is the referent
    let w = Weak.new(ot);              // Weak<?Tile>: legal (D2)
    ot = nil;                          // the box loses its only strong handle: it dies
    let b = probe_ot(w);
    if (b == nil) {{ return 42; }}
    return 0;
}}
"#
    );
    assert_eq!(run_main(&src), 42);
}

// ---- store-entry referents (D1) ----------------------------------------------

#[test]
fn weak_over_user_opaque_box_dies_with_the_entry() {
    // the D1 mechanism at rut level: the referent is a Rut STORE ENTRY
    // (the tagged word); the kill rides the entry path (release_entry).
    // The kill+probe live across a CALL BOUNDARY: in-frame temporaries
    // (the call-result temps, the if-condition reads) lawfully pin cells
    // until the frame ends, so a decisive mid-frame death test severs in
    // a callee whose temps died at its own ret.
    let src = format!(
        r#"{TILE}
fn probe_o(w: Weak<opaque>) -> i32 {{
    let b = w.upgrade();
    if (b == nil) {{ return 42; }}
    return 0;
}}
pub fn main() -> i32 {{
    let t = Tile.new(8);
    let mut o = opaque(t);      // a Rut store entry — the TAGGED-word referent
    let w = Weak.new(o);
    o = opaque(t);              // the first entry is released: it dies on the entry path
    return probe_o(w);
}}
"#
    );
    assert_eq!(run_main(&src), 42);
}

#[test]
fn weak_over_host_box_dies_with_the_entry() {
    // the D1 host leg: a `Host` store entry (alloc_host_box) weak-
    // referenced from script — the entry arm is shape-blind (Host and
    // Rut entries route through the same release).
    use rut_vm::interp::{HostHooks, HostRegistry};
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    // the test's own host pkg (the host_boxes pattern): the decl surface
    // here, the body bound below
    rut_driver::mount_dir(&mut s, &root.join("crates/rut-driver/tests/data/weak_host")).expect("mount weak_host");
    let src = r#"
use weak_host::{ make_box };
fn probe_o(w: Weak<opaque>) -> i32 {
    let b = w.upgrade();
    if (b == nil) { return 42; }
    return 0;
}
pub fn main() -> i32 {
    let mut b = make_box(41);
    let w = Weak.new(b);
    b = make_box(42);      // the first host box is released: the weak goes dead
    return probe_o(w);
}
"#;
    s.register_module("app_main", Module { spec: "app_main".into(), body: ModuleBody::Source { text: src.into(), is_decl: false }, ..Default::default() }).unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = out.program.expect("linked program");
    rut_vm::verify::verify(&prog).expect("verify");
    let expected = s.expected_host_fns();
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut hosts = HostRegistry::new();
    struct Stamp(i64);
    rut_vm::register!(hosts, "weak_host::make_box", (i64) -> OpaqueRef, |vm: &mut rut_vm::interp::Vm, n: i64| {
        let b = rut_vm::Opaque::alloc(vm, Stamp(n))?;
        Ok(b.handle().clone())
    });
    hosts.verify_against(&expected);
    let mut vm = rut_vm::interp::Vm::new(std::rc::Rc::new(prog), &limits, HostHooks::default(), hosts).expect("vm");
    let r = vm.call::<_, i32>("main", ()).expect("run");
    assert_eq!(r, 42);
}

// ---- the generic path ----------------------------------------------------------

#[test]
fn weak_inside_a_generic_fn_instantiates_per_concrete_t() {
    let src = format!(
        r#"{TILE}
fn watch<T>(v: T) -> Weak<T> {{
    return Weak.new(v);
}}
pub fn main() -> i32 {{
    let t = Tile.new(3);
    let w = watch(t);          // Weak<Tile> at this instantiation
    let b = w.upgrade();
    if (b == nil) {{ return 0; }}
    return b.n;
}}
"#
    );
    assert_eq!(run_main(&src), 3);
}

// ---- box identity -------------------------------------------------------------

#[test]
fn weak_boxes_compare_by_cell_identity() {
    let src = format!(
        r#"{TILE}
pub fn main() -> i32 {{
    let t = Tile.new(1);
    let w1 = Weak.new(t);
    let w2 = Weak.new(t);      // a SECOND box over the same referent
    if (w1 == w2) {{ return 1; }}   // distinct cells: never equal
    let alias = w1;
    if (alias != w1) {{ return 2; }} // one handle's alias: equal (the identity law)
    return 42;
}}
"#
    );
    assert_eq!(run_main(&src), 42);
}

// ---- admission + runtime diagnostics --------------------------------------------

#[test]
fn weak_of_a_primitive_diagnoses_at_the_instantiation() {
    let diags = compile_diags(
        r#"pub fn main() -> i32 {
    let w = Weak.new(5);
    return 0;
}
"#,
    );
    assert!(
        diags.iter().any(|d| d.contains("weak needs a reference type")),
        "{diags:?}"
    );
}

#[test]
fn weak_of_a_fn_value_diagnoses() {
    // "everything except primitives and fn values is a heap
    // cell" — fn values are the one non-prim non-cell, so the admission
    // refuses them with the same law.
    let diags = compile_diags(
        r#"pub fn main() -> i32 {
    let f: fn(i32) -> i32 = fn (x: i32) -> i32 { return x + 1; };
    let w = Weak.new(f);
    return 0;
}
"#,
    );
    assert!(
        diags.iter().any(|d| d.contains("weak needs a reference type")),
        "{diags:?}"
    );
}

#[test]
fn weak_on_nil_traps() {
    let src = format!(
        r#"{TILE}
pub fn main() -> i32 {{
    let t: ?Tile = nil;
    let w = Weak.new(t);      // no cell to point at: the loud trap
    return 0;
}}
"#
    );
    assert_eq!(run_trap(&src), Some("NilDeref".into()));
}

#[test]
fn upgrade_on_a_non_weak_traps() {
    // a verifier-shaped lie cannot reach the engine (the register is
    // checker-typed), so this pins the ENGINE guard directly: run the
    // op through a hand-built program is overkill — the type system
    // makes it unreachable from rut; the trap text is the contract.
    // (Pinned at the unit level instead: see the heap tests.)
}

#[test]
fn construction_and_type_position_arity_diagnose() {
    let diags = compile_diags(
        r#"pub fn main() -> i32 {
    let a = 1;
    let w = Weak.new(a, a);
    return 0;
}
"#,
    );
    assert!(
        diags.iter().any(|d| d.contains("exactly one argument")),
        "{diags:?}"
    );
    let diags = compile_diags(
        r#"pub fn main() -> i32 {
    let w = Weak.new();
    return 0;
}
"#,
    );
    assert!(
        diags.iter().any(|d| d.contains("exactly one argument")),
        "{diags:?}"
    );
    let diags = compile_diags(
        r#"pub fn main() -> i32 {
    let w: Weak<i32, i32> = Weak.new(5);
    return 0;
}
"#,
    );
    assert!(
        diags.iter().any(|d| d.contains("exactly one type parameter")),
        "{diags:?}"
    );
}

#[test]
fn the_retired_type_call_spell_names_the_class_method_fix() {
    // The retired type-call spelling stopped compiling when construction
    // became the class method: the diagnostic is loud and actionable,
    // naming the fix.
    let diags = compile_diags(
        r#"pub fn main() -> i32 {
    let s = "x";
    let w = Weak(s);
    return 0;
}
"#,
    );
    assert!(
        diags
            .iter()
            .any(|d| d.contains("`Weak(v)` is not a function")
                && d.contains("construction is a class method: `Weak.new(v)`")),
        "{diags:?}"
    );
}

#[test]
fn weak_new_consumes_the_argument_temporary() {
    // `Weak.new(v)` consumes the argument's temporary and nulls its
    // register (the engine's one consuming op): the weak observes the
    // BINDING's lifetime, never a temporary's — no +1 of the temporary's
    // own survives the mint. `make()`'s result is bound first (the
    // documented shape); when the binding is rebound, nothing pins the
    // old referent — an unconsumed temporary would keep `upgrade()`
    // answering a handle past the rebind.
    let src = r#"
class Gad { n: i32; }
impl Gad {
    fn make(n: i32) -> Self { return Self { n: n }; }
}
pub fn main() -> i32 {
    let mut v = Gad.make(9);
    let w = Weak.new(v);            // the mint consumes the temporary
    v = Gad.make(10);               // the binding dies: the old referent with it
    let b1 = w.upgrade();
    if (b1 != nil) { return 1; }    // dead — the temporary did not pin it
    let w2 = Weak.new(v);           // a fresh weak over the new binding
    let b2 = w2.upgrade();
    if (b2 == nil) { return 2; }    // alive — the new binding keeps it
    return b2.n + 33;               // 10 + 33
}
"#
    .to_string();
    assert_eq!(run_main(&src), 43);
}

#[test]
fn weak_takes_no_impl_blocks() {
    let diags = compile_diags(
        r#"impl Weak {
}
pub fn main() -> i32 {
    return 0;
}
"#,
    );
    assert!(
        diags.iter().any(|d| d.contains("takes no impl blocks")),
        "{diags:?}"
    );
}

#[test]
fn weak_has_no_other_members() {
    let src = format!(
        r#"{TILE}
pub fn main() -> i32 {{
    let t = Tile.new(1);
    let w = Weak.new(t);
    let b = w.sniff();
    if (b == nil) {{ return 1; }}
    return 0;
}}
"#
    );
    let diags = compile_diags(&src);
    assert!(
        diags.iter().any(|d| d.contains("its member is `upgrade()`")),
        "{diags:?}"
    );
}

// ---- the heap comes back clean ---------------------------------------------------

#[test]
fn heap_usage_returns_to_baseline_after_a_weak_churn() {
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    let src = format!(
        r#"{TILE}
pub fn main() -> i32 {{
    let mut live = 0;
    for (let i = 0; i < 50; i += 1) {{
        let t = Tile.new(i);
        let w = Weak.new(t);
        let b = w.upgrade();
        if (b != nil) {{ live += 1; }}
        // t, b and the box all leave scope each round: the referent dies,
        // the box unregisters, every charge refunds
    }}
    return live;
}}
"#
    );
    s.register_module("app_main", Module { body: ModuleBody::Source { text: src.into(), is_decl: false }, ..Default::default() }).unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    assert_eq!(vm.call::<_, i32>("main", ()).expect("run"), 50);
    // every cell, box and opt box released; the weak lists are empty;
    // the budget is back where boot left it
    assert_eq!(vm.heap_usage(), 0, "the weak machinery leaks nothing");
}

#[test]
fn oom_at_the_weak_mint_traps_before_registration() {
    let mut s = Session::new();
    rut_driver::mount_std_core(&mut s);
    let src = format!(
        r#"{TILE}
pub fn main() -> i32 {{
    let t = Tile.new(1);
    let w = Weak.new(t);
    return 0;
}}
"#
    );
    s.register_module("app_main", Module { body: ModuleBody::Source { text: src.into(), is_decl: false }, ..Default::default() }).unwrap();
    let out = rut_driver::compile_graph(&s, "app_main");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    // a budget the boot + Tile cannot both fit beside the WeakNew mint
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(1),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    let err = vm.call::<_, i32>("main", ()).expect_err("budget trip");
    assert_eq!(err.name(), "OutOfMemory", "{err:?}");
}
