//! Cross-instantiation and cross-scope identity laws for the unit-local
//! closure families and the re-shipped interface rows — the two
//! duplicate-boot-scope type-interning defects pinned here:
//!
//! 1. **The lambda-per-instantiation law**: a lambda (and an `async { }`
//!    block, and a `for..of` emit closure) nested inside a generic fn
//!    compiles ONCE PER enclosing substitution. The unit-local Inst key
//!    carries the creation site's substitution, and the sig maps key by
//!    the instantiation's fid — a node-only key collapses two
//!    instantiations onto one body, the LAST writer's substitution
//!    silently mistyping every other instantiation's closure (the
//!    `unopaque<A>` inside a shared wrapper unboxes the WRONG type).
//!
//! 2. **The re-shipped iface-id remap law**: a surface that re-ships a
//!    dependency's type rows carries interface ids in ITS OWN private
//!    numbering (its own binding already remapped the original
//!    exporter's ids). Every referenced id must re-declare on the
//!    surface — with its true generics count — and the consumer must
//!    register it (namelessly when the name was never used) so
//!    `use_types`'s trait map has a key for it. An unregistered local
//!    leaves the row's id stale, naming whatever interface happens to
//!    sit at that index in the consumer's table — the import-shape
//!    sensitive mistyping (a generic fn's return reading as a DIFFERENT
//!    interface).

fn graph(modules: &[(&str, &str)]) -> rut_driver::GraphOutput {
    let mut chain = rut_driver::RutRun::new();
    for (spec, src) in modules {
        let heads: Vec<&str> =
            modules.iter().map(|(s, _)| *s).filter(|s| *s != *spec).collect();
        chain = chain
            .pkg(rut_driver::Pkg::source(spec, *src).manifest(rut_driver::Manifest::lib(spec, &heads)));
    }
    let (root, _) = modules.last().expect("root");
    match chain.entrypoint(root).compile() {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(rut_lexer::span::Span::new(0, 0), e.msg)],
            program: None,
        },
    }
}

fn run_main(modules: &[(&str, &str)]) -> i32 {
    let g = graph(modules);
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let prog = rut_core::link::flatten(g.program.expect("program"));
    rut_vm::verify::verify(&prog).expect("verify");
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(prog))
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    vm.call("main", ()).expect("the entry runs")
}

// ---------------------------------------------------------------------------
// 1 — the lambda-per-instantiation law
// ---------------------------------------------------------------------------

/// One generic fn, one lambda node, TWO instantiations in ONE unit: the
/// boxed wrapper's `unopaque<A>` must recover each instantiation's own
/// argument. Under the node-only key both `MakeClosure`s pointed at one
/// body — the u64 instantiation unboxed i32 (BadUnbox at runtime) and
/// the i32 read garbage.
const TAG_RUT: &str = "\
pub fn tagged<A>(v: A) -> u64 {
    let b = opaque(v);
    let wrapper = fn (x: opaque) -> A { return unopaque<A>(x); };
    return wrapper(b) as u64;
}
entry fn main() -> i32 {
    let a = tagged<i32>(7);
    let b = tagged<u64>(3000000000u64);
    if (a != 7) { return 1; }
    if (b != 3000000000u64) { return 2; }
    return 0;
}
";

#[test]
fn a_lambda_compiles_once_per_enclosing_substitution() {
    assert_eq!(
        run_main(&[("app", TAG_RUT)]),
        0,
        "each instantiation's wrapper unboxes its own argument type"
    );
}

/// The same law through the CROSS-PKG reseed lane (the tur kit's
/// `mutate<A>` shape): the middle pkg re-exports the generic fn and the
/// consumer instantiates it twice — the owner reseeds with both seeds,
/// and the wrapper lambda must still be per-instantiation.
const TAG_KIT: &str = "\
pub fn tagged<A>(v: A) -> u64 {
    let b = opaque(v);
    let wrapper = fn (x: opaque) -> A { return unopaque<A>(x); };
    return wrapper(b) as u64;
}
";

const TAG_MID: &str = "\
use kit::{ tagged };
pub fn via_i32(v: i32) -> u64 { return tagged<i32>(v); }
";

const TAG_APP: &str = "\
use kit::{ tagged };
use mid::{ via_i32 };
entry fn main() -> i32 {
    let a = via_i32(7);
    let b = tagged<u64>(3000000000u64);
    if (a != 7) { return 1; }
    if (b != 3000000000u64) { return 2; }
    return 0;
}
";

#[test]
fn a_lambda_in_a_reseeded_owner_keeps_each_seeds_body() {
    assert_eq!(
        run_main(&[("kit", TAG_KIT), ("mid", TAG_MID), ("app", TAG_APP)]),
        0,
        "the reseeded owner's wrapper lambda is per-seed, not one shared body"
    );
}

// ---------------------------------------------------------------------------
// 2 — the re-shipped iface-id remap law
// ---------------------------------------------------------------------------

/// The re-ship topology of the tur failure: `base` declares two generic
/// interfaces side by side plus a generic mint over the second; `mid`
/// uses only `Mutation` namefully (so its own binding remaps base's
/// iface ids into ITS numbering — the rows it re-ships carry them) and
/// exports a row over `Mutation`; `app` binds base's surface FIRST (the
/// correct remap) and THEN mid's — the later use_block re-registers
/// base-scope rows whose iface ids are now mid-private, and the app's
/// `mk` call's return row must still decode as `Mutation<i32>`, never
/// as the adjacent `Readable<..>` position.
const RE_BASE: &str = "\
pub interface Readable<T> { fn get(self) -> T; }
pub interface Mutation<A> { fn emit(self, a: A) -> u64; }
pub class Mof<A> { f: opaque; }
impl<A> Mof<A> {
    [constructor] pub fn of(f: opaque) -> Self { return Self { f: f }; }
    pub fn emit(self, _a: A) -> u64 { return 1; }
}
pub fn mk<A>(f: fn(A)) -> Mutation<A> {
    return Mof<A>::of(opaque(f));
}
pub fn rd<T>(s: Readable<T>) -> T { return s.get(); }
";

const RE_MID: &str = "\
use base::{ Mutation, mk };
pub fn drive() -> u64 {
    let m: Mutation<i32> = mk<i32>(fn (_a: i32) { return; });
    return m.emit(2);
}
";

const RE_AUX: &str = "pub interface Marker { fn tag(self) -> u64; }\n";

/// The app binds `aux` FIRST (one extra interface registration, the
/// tur app's import-set sensitivity) and names `Readable` before
/// `Mutation` — a stale mid-private id (its `Mutation`) then lands on
/// the app's `Readable` row: the initializer of the annotated let
/// misreads as the WRONG interface and the origin law refuses it.
const RE_APP: &str = "\
use aux::{ Marker };
use base::{ Readable, Mutation, mk };
use mid::{ drive };
entry fn main() -> i32 {
    let m: Mutation<i32> = mk<i32>(fn (_a: i32) { return; });
    return m.emit(3) as i32 + drive() as i32;
}
";

#[test]
fn a_transitive_surface_rereads_its_reshipped_iface_ids() {
    // the spelled annotation (`let m: Mutation<i32>`) is the trap: under
    // the stale-id misbind the initializer read as `[interface]
    // Readable<#T>` and the origin law refused the let
    assert_eq!(
        run_main(&[("aux", RE_AUX), ("base", RE_BASE), ("mid", RE_MID), ("app", RE_APP)]),
        2,
        "the mint through the re-shipping surface keeps its own interface identity"
    );
}

/// The transitive-only shape: the consumer never binds the declaring
/// surface at all — every iface id the re-shipped rows carry must
/// decode through the re-export alone (the belt-and-suspenders side of
/// the re-declare law; the pinned misbind itself is the direct-shape
/// test above).
const RE_APP_TRANSITIVE: &str = "\
use aux::{ Marker };
use mid::{ drive };
entry fn main() -> i32 {
    return drive() as i32;
}
";

#[test]
fn a_transitive_only_consumer_decodes_the_reshipped_rows() {
    assert_eq!(
        run_main(&[
            ("aux", RE_AUX),
            ("base", RE_BASE),
            ("mid", RE_MID),
            ("app", RE_APP_TRANSITIVE)
        ]),
        1,
        "the interface dispatch through the re-shipped row binds the member"
    );
}
