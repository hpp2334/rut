//! PHASE 0 SPIKES (temporary — deleted once green).
//!
//! Each test answers one language-level question the two-package store
//! redesign depends on:
//!
//!   (a) splice privacy — does a consumer leaf see another leaf's
//!       private members/fns (the t1-harness precedent says fields leak)?
//!   (b) closure → `opaque` → `downcast<fn(..)>` roundtrip + capture +
//!       call-through-value (the erased-program trick);
//!   (c) `?T` and tuples as GENERIC args (`Mutation<i64, ?Req>`,
//!       `Mutation<str, (str, str)>`);
//!   (d) generic trait impls on generic classes + trait-typed fn params
//!       (the `Readable<T>`/`Writable<A, R>` shape);
//!   (e) the mini-store end-to-end: pull-on-read, discovered edges,
//!       derived-on-derived, the cycle guard, the str content-skip —
//!       the whole `ui/store.rut` design in spike form.

use std::rc::Rc;

use rut_driver::{Module, Session};
use rut_vm::interp::{HostHooks, HostRegistry, Vm};
use rut_vm::OpaqueRef;

const POUCH_RUT: &str = include_str!("../../../rut/pouch/pouch.rut");
const NMAP_HOST_D_RUT: &str = include_str!("../../../rut/nmap_host/nmap.d.rut");
const NMAPSET_RUT: &str = include_str!("../../../rut/nmapset/nmapset.rut");

fn limits() -> rut_vm::interp::Limits {
    rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    }
}

/// A session with core + pouch + nmapset mounted (the mini-store's
/// closure), then `src` compiled as the root spec `spike`.
fn boot(src: &str) -> Vm {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    session
        .register_module("pouch", Module { source: Some(POUCH_RUT.to_string()), inline: true, ..Default::default() })
        .expect("pouch mounts");
    let nmap = rut_driver::lower_decl_module(NMAP_HOST_D_RUT, "nmap.d.rut").expect("nmap decl lowers");
    session.register_module("nmap_host", nmap).expect("nmap_host mounts");
    session
        .register_module("nmapset", Module { source: Some(NMAPSET_RUT.to_string()), inline: true, ..Default::default() })
        .expect("nmapset mounts");
    let out = rut_driver::compile_module_in(&mut session, src, rut_parser::Mode::Impl, "spike");
    assert!(
        out.diags.is_empty(),
        "the spike does not compile: {}",
        out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("; ")
    );
    let bin = out.binary.expect("the spike compiled to no binary");
    let prog = rut_core::binary::decode(&bin).expect("binary decode");
    rut_vm::verify::verify(&prog).expect("verify");
    let mut hosts = HostRegistry::new();
    rut_std::nmap::install_std_nmap(&mut hosts);
    Vm::new(Rc::new(prog), &limits(), HostHooks::default(), hosts).expect("the vm boots")
}

/// Two-leaf variant: `inner` mounted inline first, `src` compiled as the
/// root that uses it.
fn boot_two(inner: &str, src: &str) -> Result<Vm, String> {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    session
        .register_module("inner", Module { source: Some(inner.to_string()), inline: true, ..Default::default() })
        .map_err(|e| format!("{e:?}"))?;
    let out = rut_driver::compile_module_in(&mut session, src, rut_parser::Mode::Impl, "outer");
    if !out.diags.is_empty() {
        return Err(out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("; "));
    }
    let bin = out.binary.ok_or("the outer leaf compiled to no binary")?;
    let prog = rut_core::binary::decode(&bin).map_err(|e| format!("binary decode: {e}"))?;
    rut_vm::verify::verify(&prog).map_err(|e| format!("verify: {e}"))?;
    let v = Vm::new(Rc::new(prog), &limits(), HostHooks::default(), HostRegistry::new())
        .map_err(|e| format!("{e:?}"))?;
    Ok(v)
}

// ---- (b) closure → opaque → downcast roundtrip ----------------------

const SPIKE_B: &str = r#"
pub fn make() -> opaque {
    let k: i32 = 21;
    let f = fn (x: i32) -> i32 { return x * k; };   // captures k
    return opaque(f);
}

pub fn apply(o: opaque, v: i32) -> i32 {
    let g = opaque.downcast<fn(i32) -> i32>(o);     // the erased program, back
    return g(v);                                    // call through the value
}

entry fn run() -> i32 {
    return apply(make(), 2);
}
"#;

#[test]
fn spike_b_closure_erasure_roundtrip() {
    let mut v = boot(SPIKE_B);
    let r: i32 = v.call("run", ()).expect("run");
    assert_eq!(r, 42, "the erased closure runs with its capture");
}

// ---- (c) ?T and tuples as generic args ------------------------------

const SPIKE_C: &str = r#"
pub struct Item {
    id: i64;
}

pub class M<A, R> {
    marker: i64 = 0;
}

pub fn m1() -> M<i64, ?Item> { return M { marker: 1 }; }
pub fn m2() -> M<str, (str, str)> { return M { marker: 2 }; }

entry fn run() -> i64 {
    let a = m1();
    let b = m2();
    return a.marker + b.marker;
}
"#;

#[test]
fn spike_c_nullable_and_tuple_generic_args() {
    let mut v = boot(SPIKE_C);
    let r: i64 = v.call("run", ()).expect("run");
    assert_eq!(r, 3, "?T and (A, B) both inhabit generic positions");
}

// ---- (d) generic trait impls on generic classes ---------------------

const SPIKE_D: &str = r#"
trait Readable<T> {
    fn atom_id(self) -> u32;
}

pub class Source<T> {
    id: u32 = 0;
    seed: opaque;
}

pub class Derived<T> {
    id: u32 = 0;
}

impl Readable<T> for Source<T> {
    fn atom_id(self) -> u32 { return self.id; }
}

impl Readable<T> for Derived<T> {
    fn atom_id(self) -> u32 { return self.id; }
}

pub fn read_id<T>(a: Readable<T>) -> u32 {
    return a.atom_id();   // the fat-ref dispatch
}

entry fn run() -> i64 {
    let s = Source<str> { id: 7, seed: opaque("x") };
    let d = Derived<i64> { id: 9 };
    return (read_id(s) + read_id(d)) as i64;
}
"#;

#[test]
fn spike_d_generic_trait_impls_and_fat_refs() {
    let mut v = boot(SPIKE_D);
    let r: i64 = v.call("run", ()).expect("run");
    assert_eq!(r, 16, "generic trait impls dispatch through trait-typed params");
}

// ---- (a) splice privacy ----------------------------------------------

const SPIKE_A_INNER: &str = r#"
pub class C {
    open_field: i64 = 5;              // private by default (RFC 0002)
    fn secret(self) -> i64 { return 41; }
}

pub(self) fn hidden() -> i64 { return 1; }

pub fn open() -> i64 { return hidden() + 100; }   // same-leaf use: fine
"#;

/// The consumer reaches for what inner did not export. IF this
/// compiles, the splice flattened privacy (the t1-harness precedent)
/// and the law gate is the enforcement layer for members.
const SPIKE_A_OUTER_REACHING: &str = r#"
use inner::{ C };

entry fn run() -> i64 {
    let c = C { };
    return c.open_field + c.secret();
}
"#;

/// The consumer imports a pub(self) name. This must FAIL (RFC 0003 §2
/// checks at use).
const SPIKE_A_OUTER_USE_HIDDEN: &str = r#"
use inner::{ hidden };

entry fn run() -> i64 {
    return hidden();
}
"#;

#[test]
fn spike_a_use_of_private_name_fails() {
    let err = boot_two(SPIKE_A_INNER, SPIKE_A_OUTER_USE_HIDDEN)
        .err()
        .expect("a use of a pub(self) name must fail to compile");
    eprintln!("--- use-of-private diagnostic: {err}");
}

#[test]
fn spike_a_private_member_across_spliced_leaves() {
    match boot_two(SPIKE_A_INNER, SPIKE_A_OUTER_REACHING) {
        Ok(mut v) => {
            let r: i64 = v.call("run", ()).expect("run");
            eprintln!("--- SPIKE (a): the splice FLATTENED member privacy (run = {r}) — the law gate is the member-enforcement layer");
        }
        Err(e) => {
            eprintln!("--- SPIKE (a): member privacy HOLDS across spliced leaves: {e}");
        }
    }
}

// ---- (e) the mini-store, end to end ----------------------------------

const SPIKE_E: &str = include_str!("spike0_store.rut");

#[test]
fn spike_e_pull_on_read() {
    let mut v = boot(SPIKE_E);
    let c: OpaqueRef = v.call("world_new", ()).expect("world_new");
    // the first get computes
    assert_eq!(v.call::<_, String>("counts", (c.clone(),)).unwrap(), "0 open | 0 done");
    assert_eq!(v.call::<_, i32>("recomputes", (c.clone(),)).unwrap(), 1);
    // the next get serves the cache
    assert_eq!(v.call::<_, i32>("recomputes", (c.clone(),)).unwrap(), 1);
    // a write stales; the next get recomputes exactly once
    v.call::<_, i64>("add", (c.clone(), 1_i64)).unwrap();
    assert_eq!(v.call::<_, String>("counts", (c.clone(),)).unwrap(), "1 open | 0 done");
    assert_eq!(v.call::<_, i32>("recomputes", (c.clone(),)).unwrap(), 2);
    assert_eq!(v.call::<_, i32>("recomputes", (c.clone(),)).unwrap(), 2);
    // two writes before the next get: still exactly one recompute
    v.call::<_, i64>("add", (c.clone(), 2_i64)).unwrap();
    v.call::<_, i64>("add", (c.clone(), 3_i64)).unwrap();
    assert_eq!(v.call::<_, String>("counts", (c.clone(),)).unwrap(), "3 open | 0 done");
    assert_eq!(v.call::<_, i32>("recomputes", (c.clone(),)).unwrap(), 3);
}

#[test]
fn spike_e_discovered_edges() {
    let mut v = boot(SPIKE_E);
    let c: OpaqueRef = v.call("world_new", ()).expect("world_new");
    // counts$ reads items$ and note$: the DAG grows through ctx.get
    assert_eq!(v.call::<_, String>("counts", (c.clone(),)).unwrap(), "0 open | 0 done");
    let deps = v.call::<_, String>("deps", (c.clone(),)).unwrap();
    eprintln!("--- counts$ discovered deps: {deps}");
    assert!(deps.split('|').count() == 2, "counts$ discovered exactly its reads: {deps}");
    // a write counts$ never read recomputes NOTHING
    v.call::<_, ()>("note", (c.clone(), "hello".to_string())).unwrap();
    assert_eq!(v.call::<_, String>("counts", (c.clone(),)).unwrap(), "0 open | 0 done");
    assert_eq!(v.call::<_, i32>("recomputes", (c.clone(),)).unwrap(), 1);
}

#[test]
fn spike_e_str_lane_content_skip() {
    let mut v = boot(SPIKE_E);
    let c: OpaqueRef = v.call("world_new", ()).expect("world_new");
    assert_eq!(v.call::<_, String>("counts", (c.clone(),)).unwrap(), "0 open | 0 done");
    // the same content marks nothing: note$ depends on nothing, but the
    // str write's no-op is observable through a derived READING note$
    v.call::<_, ()>("note", (c.clone(), "same".to_string())).unwrap();
    let base = v.call::<_, i32>("recomputes", (c.clone(),)).unwrap();
    v.call::<_, ()>("note", (c.clone(), "same".to_string())).unwrap();
    v.call::<_, ()>("note", (c.clone(), "same".to_string())).unwrap();
    assert_eq!(
        v.call::<_, i32>("recomputes", (c.clone(),)).unwrap(),
        base,
        "equal content writes mark nothing"
    );
    v.call::<_, ()>("note", (c.clone(), "different".to_string())).unwrap();
    // (a derived reading note$ would recompute; counts$ does not read it)
    assert_eq!(v.call::<_, String>("note_get", (c.clone(),)).unwrap(), "different");
}

#[test]
fn spike_e_derived_on_derived_sees_fresh_upstream() {
    let mut v = boot(SPIKE_E);
    let c: OpaqueRef = v.call("world_new", ()).expect("world_new");
    assert_eq!(v.call::<_, i64>("doubled", (c.clone(),)).unwrap(), 0);
    v.call::<_, i64>("add", (c.clone(), 1_i64)).unwrap();
    assert_eq!(v.call::<_, i64>("doubled", (c.clone(),)).unwrap(), 2, "downstream sees the FRESH upstream");
    v.call::<_, i64>("add", (c.clone(), 2_i64)).unwrap();
    v.call::<_, i64>("add", (c.clone(), 3_i64)).unwrap();
    assert_eq!(v.call::<_, i64>("doubled", (c.clone(),)).unwrap(), 6);
}

#[test]
fn spike_e_derived_is_not_writable() {
    let mut v = boot(SPIKE_E);
    let c: OpaqueRef = v.call("world_new", ()).expect("world_new");
    let err = v
        .call::<_, String>("set_derived", (c.clone(), "x".to_string()))
        .err()
        .expect("setting a derived must trap");
    eprintln!("--- derived-set trap: {err:?}");
}

#[test]
fn spike_e_cycle_guard() {
    let mut v = boot(SPIKE_E);
    let c: OpaqueRef = v.call("cycle_new", ()).expect("cycle_new");
    let err = v
        .call::<_, String>("cycle_get", (c.clone(),))
        .err()
        .expect("a self-reading derive must trap");
    eprintln!("--- cycle trap: {err:?}");
}
