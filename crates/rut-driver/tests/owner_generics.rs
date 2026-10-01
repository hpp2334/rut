//! Owner-anchored generics: instantiation is owned by the declaring
//! package and consumers request. `Vec<i64>` is ONE row program-wide —
//! a linked library's `make() -> Vec<i64>` and the consumer's own
//! `Vec<i64>` unify at link (assignment and `is` both hold), the
//! duplicated instantiation bodies collapse to one compiled copy, and
//! a packaged generic's binary carries the ledger its consumers'
//! requests resolve against.

use std::path::{Path, PathBuf};

use rut_driver::{
    compile_graph, load_bundle_bytes, load_dir_session, mount_std, pack_dir, ModuleBody, Module,
    Session,
};

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-owner-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, text).unwrap();
}

fn manifest(name: &str, entry: &str, extra: &str) -> String {
    format!("name = \"{name}\"\nentry.lib = \"./{entry}\"\n{extra}")
}

/// One session, one in-memory module set (no filesystem): the specs'
/// sources registered by hand. Every module links (the `bool` arg is
/// the retired `inline` flag's seat, kept for the callers' shape).
fn session_of(modules: &[(&str, &str, bool)]) -> Session {
    let mut s = Session::new();
    mount_std(&mut s);
    for (spec, src, _inline_retired) in modules {
        s.register_module(
            spec,
            Module {
                body: ModuleBody::Source { text: src.to_string(), is_decl: false },
                ..Default::default()
            },
        )
        .unwrap();
    }
    s
}

fn run_entry<R: rut_vm::interp::Ret>(session: &Session, root: &str, entry: &str) -> R {
    let g = compile_graph(session, root);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let flat = rut_core::link::flatten(g.program.expect("linked program"));
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
    vm.call::<_, R>(entry, ()).expect("run")
}

/// The pinned identity world: `cells` (a generic dataclass pkg, linked —
/// no inline flag) and `maker` (a linked lib whose pub fn returns an
/// instantiation of the foreign generic). The consumer uses both plus
/// its own spelling of the same instantiation.
const CELLS_SRC: &str = "\
pub struct Cell<T> {
    v: T;
}
";

const MAKER_SRC: &str = "\
use cells::{Cell};

pub fn make() -> Cell<i64> {
    return Cell { v: 7 };
}
";

const CONSUMER_SRC: &str = "\
use cells::{Cell};
use maker::{make};

entry fn go() -> i64 {
    let made = make();
    let mut mine: Cell<i64> = Cell { v: 1 };
    // the identity law: the lib's `Cell<i64>` and the consumer's ARE one
    // type — assignment flows both ways and `is` agrees
    mine = made;
    if (mine is Cell<i64>) {
        return mine.v;
    }
    return -1;
}
";

#[test]
fn cross_module_type_identity_is_one_row() {
    let s = session_of(&[("cells", CELLS_SRC, false), ("maker", MAKER_SRC, false), ("app", CONSUMER_SRC, false)]);
    let got: i64 = run_entry(&s, "app", "go");
    assert_eq!(got, 7, "the lib's instantiation and the consumer's are one type");
}

#[test]
fn one_instantiation_row_and_one_body_set_program_wide() {
    // two consumers of the same foreign generic, each spelling the same
    // instantiation and each calling its method: the ledger unifies the
    // mirrors and the link drops every body but the first claim's
    let consumer2 = "\
use cells::{Cell};

pub fn touch(c: Cell<i64>) -> i64 {
    let mut c2: Cell<i64> = c;
    return c2.v;
}

entry fn go2() -> i64 {
    return touch(Cell { v: 9 });
}
";
    let s = session_of(&[
        ("cells", CELLS_SRC, false),
        ("maker", MAKER_SRC, false),
        ("app", CONSUMER_SRC, false),
        ("app2", consumer2, false),
    ]);
    let g = compile_graph(&s, "app");
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let p = g.program.expect("linked");
    // ONE mirror row per (owner, decl, args): the merged ledger carries
    // every unit's row, and the link mapped them ALL onto one global row
    let mut tys: Vec<u32> = p
        .inst_types
        .iter()
        .filter(|r| {
            p.interner.name(r.owner) == "cells" && p.interner.name(r.decl) == "Cell"
        })
        .map(|r| r.ty)
        .collect();
    tys.sort();
    tys.dedup();
    assert!(!tys.is_empty(), "the ledger names the instantiation");
    assert_eq!(tys.len(), 1, "one program-wide Cell<i64> row: {tys:?}");
    // every cell-typed operand in the program points at that one row —
    // the dead mirrors the merge left in the table are unreferenced
    let live = tys[0];
    let mut mirror = Vec::new();
    for r in &p.inst_types {
        if p.interner.name(r.owner) == "cells" && p.interner.name(r.decl) == "Cell" && r.ty != live
        {
            mirror.push(r.ty);
        }
    }
    use rut_core::ops::Op;
    for f in &p.funcs {
        for op in &f.code {
            let ty_operands: Vec<u32> = match op {
                Op::NewCell { ty, .. }
                | Op::MakeRecord { ty, .. }
                | Op::Own { ty, .. }
                | Op::MakeOpt { ty, .. }
                | Op::WeakNew { ty, .. }
                | Op::ArrNew { ty, .. }
                | Op::ArrLit { ty, .. }
                | Op::EnumNew { ty, .. }
                | Op::IsType { want: ty, .. }
                | Op::Unbox { ty, .. }
                | Op::Box { ty, .. } => vec![*ty],
                _ => vec![],
            };
            for t in ty_operands {
                assert!(
                    !mirror.contains(&t),
                    "an op still names a dead mirror row: {op:?}"
                );
            }
        }
    }

    // ONE body set: every `Cell<i64>` instantiation fn appears once
    // across the whole program
    let flat = rut_core::link::flatten(p);
    let mut fns: Vec<String> = flat.funcs.iter().map(|f| flat.interner.name(f.name).to_string()).collect();
    fns.sort();
    let dupes: Vec<String> = fns.iter().duplicates().cloned().collect();
    assert!(dupes.is_empty(), "no duplicated instantiation bodies: {dupes:?}");
}

trait DuplicatesExt: Iterator {
    fn duplicates(self) -> Duplicates<Self>
    where
        Self: Sized,
        Self::Item: PartialEq + Clone,
    {
        Duplicates { inner: self, seen: Vec::new() }
    }
}

impl<I: Iterator> DuplicatesExt for I {}

struct Duplicates<I: Iterator> {
    inner: I,
    seen: Vec<I::Item>,
}

impl<I> Iterator for Duplicates<I>
where
    I: Iterator,
    I::Item: PartialEq + Clone,
{
    type Item = I::Item;
    fn next(&mut self) -> Option<I::Item> {
        while let Some(x) = self.inner.next() {
            if self.seen.contains(&x) {
                return Some(x);
            }
            self.seen.push(x);
        }
        None
    }
}

/// A generic dataclass pkg consumed through the REQUEST path: the
/// consumer instantiates against the linked owner's template, the owner
/// recompiles with the seeded instantiation, and both sides' rows unify
/// at link.
#[test]
fn consumer_requests_route_to_the_source_owner() {
    let pairz = "\
pub struct Pair<A, B> {
    fst: A;
    snd: B;
}
";
    let app = "\
use pairz::{Pair};

entry fn go() -> i64 {
    let p = Pair<i64, str> { fst: 40, snd: \"x\" };
    let q: Pair<i64, str> = p;
    if (q is Pair<i64, str>) {
        return q.fst + 2;
    }
    return -1;
}
";
    let s = session_of(&[("pairz", pairz, false), ("app", app, false)]);
    let got: i64 = run_entry(&s, "app", "go");
    assert_eq!(got, 42, "the request compiled in the owner; identity holds");
}

/// The packaged owner: a v5 bundle whose generic lib rides COMPILED.
/// The pack walk seeds the consumer's instantiation into the lib's
/// binary; a later consumer session resolves its request against the
/// binary's ledger. A request the binary does not carry refuses loudly.
#[test]
fn packaged_generic_owner_serves_consumer_requests() {
    let root = scratch("pkg");
    let pairz = root.join("pairz");
    write(
        &pairz,
        "rut.toml",
        &format!(
            "format = \"rutbundle\"\nformat_version = 5\n{}",
            manifest("pairz", "pairz.rut", "")
        ),
    );
    write(
        &pairz,
        "pairz.rut",
        "pub struct Pair<A, B> {\n\
         \x20   fst: A;\n\
         \x20   snd: B;\n\
         }\n",
    );
    let app = root.join("app");
    write(
        &app,
        "rut.toml",
        &format!(
            "format = \"rutbundle\"\nformat_version = 5\n{}",
            manifest("app", "app.rut", "[deps]\npairz = { path = \"../pairz\" }\n")
        ),
    );
    write(
        &app,
        "app.rut",
        "use pairz::{ Pair };\n\n\
         pub fn make() -> Pair<i64, str> {\n\
         \x20   return Pair { fst: 5, snd: \"x\" };\n\
         }\n\n\
         entry fn go() -> i64 {\n\
         \x20   return make().fst;\n\
         }\n",
    );

    let bytes = pack_dir(&app).expect("pack");
    // the generic lib rode compiled — and, the generic-source riding
    // law, its source rides beside the binary so consumer-spelled
    // shapes stay servable at load
    let names: Vec<String> =
        rut_driver::bundle::parse_bundle(&bytes).unwrap().into_iter().map(|(n, _)| n).collect();
    assert!(names.contains(&"pairz/pairz.rutc".to_string()), "{names:?}");
    assert!(names.contains(&"pairz/pairz.rut".to_string()), "{names:?}");
    // and the pack-time closure's instantiation is IN the binary's ledger
    {
        let group = rut_driver::bundle::parse_bundle(&bytes)
            .unwrap()
            .into_iter()
            .find(|(n, _)| n == "pairz/pairz.rutc")
            .unwrap()
            .1;
        let prog = rut_core::binary::decode(&group).expect("decode");
        assert!(
            prog.inst_types.iter().any(|r| prog.interner.name(r.decl) == "Pair"),
            "the seeded instantiation rides the binary: {:?}",
            prog.inst_types.iter().map(|r| prog.interner.name(r.decl)).collect::<Vec<_>>()
        );
    }

    // a consumer session over the bundle: its request resolves against
    // the ledger — assignment and `is` hold across the package boundary
    let (mut session, app_root) = load_bundle_bytes(&bytes, Path::new("mem")).expect("load");
    mount_std(&mut session);
    session
        .register_module(
            "consumer",
            Module {
                body: ModuleBody::Source {
                    text: "use pairz::{ Pair };\nuse app::{ make };\n\n\
                           entry fn go2() -> i64 {\n\
                           \x20   let mine: Pair<i64, str> = make();\n\
                           \x20   if (mine is Pair<i64, str>) {\n\
                           \x20       return mine.fst + 1;\n\
                           \x20   }\n\
                           \x20   return -1;\n\
                           }\n"
                        .into(),
                    is_decl: false,
                },
                ..Default::default()
            },
        )
        .unwrap();
    let got: i64 = run_entry(&session, "consumer", "go2");
    assert_eq!(got, 6, "the packaged instantiation served the consumer");
    let _ = app_root;

    // a request the binary does not carry used to refuse — now the
    // generic-source riding law answers it: the lib packed ALONE rides
    // its source, so the consumer-spelled shape compiles at the link
    // and runs
    let standalone = pack_dir(&pairz).expect("the lib packs alone");
    let (mut lone, _pairz_root) = load_bundle_bytes(&standalone, Path::new("lone")).expect("load");
    mount_std(&mut lone);
    lone.register_module(
        "late",
        Module {
            body: ModuleBody::Source {
                text: "use pairz::{ Pair };\n\n\
                       entry fn go3() -> i64 {\n\
                       \x20   let p = Pair<i64, str> { fst: 1, snd: \"x\" };\n\
                       \x20   return p.fst;\n\
                       }\n"
                    .into(),
                is_decl: false,
            },
            ..Default::default()
        },
    )
    .unwrap();
    let g = compile_graph(&lone, "late");
    assert!(
        g.diags.is_empty(),
        "the consumer-spelled shape compiles from the ridden source: {:?}",
        g.diags
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The pack walk seeds a generic dep from its consumers: the same
/// directory packs byte-identically (the canonical, sorted request
/// order), and the ledger rows ride sorted.
#[test]
fn request_order_is_canonical_and_the_pack_is_byte_deterministic() {
    let root = scratch("det");
    let pairz = root.join("pairz");
    write(&pairz, "rut.toml", &manifest("pairz", "pairz.rut", ""));
    write(
        &pairz,
        "pairz.rut",
        "pub struct Pair<A, B> {\n\
         \x20   fst: A;\n\
         \x20   snd: B;\n\
         }\n",
    );
    let app = root.join("app");
    write(
        &app,
        "rut.toml",
        &format!(
            "format = \"rutbundle\"\nformat_version = 5\n{}",
            manifest("app", "app.rut", "[deps]\npairz = { path = \"../pairz\" }\n")
        ),
    );
    write(
        &app,
        "app.rut",
        "use pairz::{ Pair };\n\n\
         entry fn go() -> i64 {\n\
         \x20   let b = Pair<str, i64> { fst: \"x\", snd: 1 };\n\
         \x20   let a = Pair<i64, str> { fst: 1, snd: \"x\" };\n\
         \x20   return a.fst + b.snd;\n\
         }\n",
    );
    let a = pack_dir(&app).expect("pack");
    let b = pack_dir(&app).expect("pack again");
    assert_eq!(a, b, "same dir => byte-identical bundle (canonical request order)");
    let _ = std::fs::remove_dir_all(&root);
}

/// The directory world and the packaged world still link identically —
/// now with the generic lib compiled on both sides.
#[test]
fn compiled_bundle_matches_the_directory_with_a_generic_lib() {
    let root = scratch("equiv");
    let pairz = root.join("pairz");
    write(&pairz, "rut.toml", &manifest("pairz", "pairz.rut", ""));
    write(
        &pairz,
        "pairz.rut",
        "pub struct Pair<A, B> {\n\
         \x20   fst: A;\n\
         \x20   snd: B;\n\
         }\n",
    );
    let app = root.join("app");
    write(
        &app,
        "rut.toml",
        &format!(
            "format = \"rutbundle\"\nformat_version = 5\n{}",
            manifest("app", "app.rut", "[deps]\npairz = { path = \"../pairz\" }\n")
        ),
    );
    write(
        &app,
        "app.rut",
        "use pairz::{ Pair };\n\n\
         entry fn go() -> i64 {\n\
         \x20   let p = Pair<i64, str> { fst: 21, snd: \"x\" };\n\
         \x20   return p.fst * 2;\n\
         }\n",
    );
    let (dir_session, dir_root) = load_dir_session(&app).expect("dir load");
    let from_dir = {
        let g = compile_graph(&dir_session, &dir_root);
        assert!(g.diags.is_empty(), "{:?}", g.diags);
        rut_core::binary::encode(&g.program.expect("linked"))
    };
    let bytes = pack_dir(&app).expect("pack");
    let (session, app_root) = load_bundle_bytes(&bytes, Path::new("mem")).expect("load");
    let g = compile_graph(&session, &app_root);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let from_bundle = rut_core::binary::encode(&g.program.expect("linked"));
    assert_eq!(from_dir, from_bundle, "dir and bundle compile identically");
    let _ = std::fs::remove_dir_all(&root);
}
