//! flow e2e — the `rut/flow` push-pipeline package through the shipped
//! bundle lane: the consumer world mounts the committed `dist/std`
//! bundles (pouch + nmapset + flow — the exact bytes jsDelivr serves),
//! and every case runs the full pipeline: compile → decode → verify →
//! interpret. The surface: `into_flow` entries (Vec / `[T]` / `str` /
//! `bytes`), the adapters (`map`/`filter`/`take`/`skip`/`count`/
//! `fold`/`enumerate`/`for_each`), the `Vec.from_flow` sink, the
//! `HashSet::from_flow` sink, the `[E]` array sink, chains feeding
//! `for..of` (the `impl Iterable` path), the identity row, and
//! stop-propagation through `take`.

use rut_driver::{Module, ModuleBody, Session, compile_graph, mount_bundle_bytes, mount_std};
use rut_vm::interp::{HostRegistry, Limits, Vm};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The consumer world: core + calc mounted, the flow closure's bundles
/// from `dist/std` (the committed artifacts), ink for the logger.
fn setup(src: &str) -> (Vm, std::rc::Rc<std::cell::RefCell<Vec<String>>>) {
    let mut s = Session::new();
    mount_std(&mut s);
    let base = root();
    rut_driver::mount_dir(&mut s, &base.join("rut/ink")).expect("mount ink");
    for b in ["pouch", "nmapset", "flow"] {
        let bytes = std::fs::read(base.join(format!("dist/std/{b}.rutbundle")))
            .unwrap_or_else(|e| panic!("dist/std/{b}.rutbundle: {e}"));
        mount_bundle_bytes(&mut s, &bytes).unwrap_or_else(|e| panic!("mount {b}: {e:?}"));
    }
    s.register_module("app", Module { body: ModuleBody::Source { text: src.into(), is_decl: false }, ..Default::default() })
        .expect("register app");
    let out = compile_graph(&s, "app");
    assert!(out.diags.is_empty(), "diags:\n{}", out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n"));
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let lines = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let sink = lines.clone();
    let ctx = s.host_pkg_context();
    let mut hosts = HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::logger::pkg(move |m| sink.borrow_mut().push(m.to_string())));
    hosts.install_host_pkg(&ctx, rut_std::math::pkg());
    hosts.install_host_pkg(&ctx, rut_std::nmap::pkg());
    hosts.verify_against(&ctx.flatten());
    let limits = Limits {
        fuel: Some(8_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let vm = Vm::new(std::rc::Rc::new(flat), &limits, rut_vm::interp::HostHooks::default(), hosts).expect("vm");
    (vm, lines)
}

fn run(src: &str) -> Vec<String> {
    let (mut vm, lines) = setup(src);
    vm.call::<_, ()>("main", ()).expect("main");
    let out = lines.borrow().clone();
    out
}

#[test]
fn vec_pipeline_map_filter_take_into_vec_sink() {
    // the README surface: entry, adapters, the Vec sink
    // (adapted at landing: `count`/`for_each` are Flow's one-drive
    // consumers, not Vec's — the drain reads the sink's fields)
    let lines = run(
        r#"
use core::{ Iterable };
use pouch::{ Vec };
use flow::{ Flow, IntoFlow, FromFlow };
use ink::{ Logger };

entry fn main() -> nil {
    let log = Logger.new("flow");
    let nums: Vec<i32> = Vec.new();
    nums.push(1); nums.push(2); nums.push(3); nums.push(4); nums.push(5); nums.push(6);
    let picked: Vec<i32> = Vec.from_flow(nums.into_flow()
        .map(fn(x: i32) -> i32 { return x * 2; })
        .filter(fn(x: i32) -> bool { return x > 3; })
        .take(4));
    log.info(f"picked={picked.len} first={picked[0]} last={picked[picked.len-1]}");
}
"#,
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("picked=4"), "{lines:?}");
    assert!(lines[0].contains("first=4"), "{lines:?}");
    assert!(lines[0].contains("last=10"), "{lines:?}");
}

#[test]
fn adapters_take_skip_count_and_chain_for_of() {
    // take/skip/count over a Vec source; a chain feeding plain for..of
    // (the `impl Iterable` desugar). The drain accumulates through a
    // ref-headed binding — the capture law's pinned primitive copy
    // means a captured `i32` in the emit closure copies its slot (the
    // fused Vec loop owns the in-place `+=` shape).
    let lines = run(
        r#"
use pouch::{ Vec };
use flow::{ Flow, IntoFlow, FromFlow };
use ink::{ Logger };

entry fn main() -> nil {
    let log = Logger.new("flow");
    let nums: Vec<i32> = Vec.new();
    let mut i = 0;
    while (i < 10) { nums.push(i); i += 1; }
    let page: Vec<i32> = Vec.from_flow(nums.into_flow().skip(2).take(3));
    log.info(f"page n={page.len} first={page[0]} last={page[page.len-1]}");
    let total: i32 = nums.into_flow().filter(fn(x: i32) -> bool { return x % 2 == 0; }).count();
    log.info(f"evens={total}");
    let grabbed: Vec<i32> = Vec.new();
    for (let x of nums.into_flow().take(4)) {
        grabbed.push(x);
    }
    log.info(f"for_of n={grabbed.len} first={grabbed[0]} last={grabbed[grabbed.len-1]}");
}
"#,
    );
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(lines[0].contains("first=2"), "{lines:?}");
    assert!(lines[0].contains("last=4"), "{lines:?}");
    assert!(lines[1].contains("evens=5"), "{lines:?}");
    assert!(lines[2].contains("n=4"), "{lines:?}");
    assert!(lines[2].contains("first=0"), "{lines:?}");
    assert!(lines[2].contains("last=3"), "{lines:?}");
}

#[test]
fn builtin_sources_array_str_bytes() {
    // the structural template entries: `[T]`, `str`, `bytes`
    let lines = run(
        r#"
use flow::{ Flow, IntoFlow };
use ink::{ Logger };

entry fn main() -> nil {
    let log = Logger.new("flow");
    let a: [i32] = [7, 8, 9];
    let n1: i32 = a.into_flow().count();
    let s: str = "abc";
    let n2: i32 = s.into_flow().count();
    let b: bytes = bytes.zeroed(3);
    let n3: i32 = b.into_flow().count();
    log.info(f"array={n1} str={n2} bytes={n3}");
}
"#,
    );
    assert_eq!(lines, vec!["array=3 str=3 bytes=3"], "{lines:?}");
}

#[test]
fn fold_and_enumerate() {
    // fold seeds the accumulator; enumerate pairs the index
    let lines = run(
        r#"
use pouch::{ Vec };
use flow::{ Flow, IntoFlow };
use ink::{ Logger };

entry fn main() -> nil {
    let log = Logger.new("flow");
    let nums: Vec<i32> = Vec.new();
    nums.push(10); nums.push(20); nums.push(30);
    let total: i32 = nums.into_flow().fold(0, fn(acc: i32, x: i32) -> i32 { return acc + x; });
    let idx: Vec<str> = Vec.new();
    nums.into_flow().enumerate().for_each(fn(p: (i32, i32)) -> nil {
        idx.push(f"{p.0}:{p.1}");
    });
    log.info(f"total={total} idx0={idx[0]} idx2={idx[2]}");
}
"#,
    );
    assert_eq!(lines, vec!["total=60 idx0=0:10 idx2=2:30"], "{lines:?}");
}

#[test]
fn identity_row_and_user_iterable_sink() {
    // the identity row: a chain re-enters as a source; a user Iterable
    // (no Flow involved) feeds the Vec sink directly — the CONTRACT
    // parameter takes any iterable
    let lines = run(
        r#"
use pouch::{ Vec };
use flow::{ Flow, IntoFlow, FromFlow };
use core::{ Iterable };
use ink::{ Logger };

class CountUp {
    n: i32;
}
impl CountUp {
    fn new(n: i32) -> Self { return Self { n: n }; }
}
impl Iterable<i32> for CountUp {
    fn iterate(self, emit: fn(i32) -> bool) {
        let mut i = 1;
        while (i <= self.n) {
            if (!emit(i)) { return; }
            i += 1;
        }
    }
}

entry fn main() -> nil {
    let log = Logger.new("flow");
    let nums: Vec<i32> = Vec.new();
    nums.push(1); nums.push(2);
    // identity: the chain re-enters as a source
    let reentered: Vec<i32> = Vec.from_flow(nums.into_flow().map(fn(x: i32) -> i32 { return x + 1; }).into_flow());
    log.info(f"reentered n={reentered.len} last={reentered[reentered.len-1]}");
    // the user iterable widens straight into the sink
    let out: Vec<i32> = Vec.from_flow(CountUp.new(4));
    log.info(f"countup n={out.len} last={out[out.len-1]}");
}
"#,
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("last=3"), "{lines:?}");
    assert!(lines[1].contains("countup n=4"), "{lines:?}");
    assert!(lines[1].contains("last=4"), "{lines:?}");
}

#[test]
fn array_sink_empty_and_nonempty() {
    // adapted at landing: the `[E]` sink's static spelling
    // (`[i32].from_flow(..)`) does not parse yet — no array-typed
    // receiver form reaches the no-self static — so the case drives the
    // STRUCTURAL rows that DO answer a consumer: the `[T]` entry (a
    // drained empty array and a full one, one drive each). The sink
    // spelling itself is flagged for the parser tail.
    let lines = run(
        r#"
use pouch::{ Vec };
use core::{ Iterable };
use flow::{ Flow, FromFlow, IntoFlow };
use ink::{ Logger };

entry fn main() -> nil {
    let log = Logger.new("flow");
    let empty: [i32] = [];
    log.info(f"empty n={empty.len()} walked={empty.into_flow().count()}");
    let nums: Vec<i32> = Vec.new();
    nums.push(5); nums.push(6); nums.push(7);
    let arr: [i32] = [5, 6, 7];
    let folded: i32 = arr.into_flow().fold(0, fn(a: i32, x: i32) -> i32 { return a * 10 + x; });
    log.info(f"arr n={arr.len()} mid={arr[1]} folded={folded}");
}
"#,
    );
    assert_eq!(lines, vec!["empty n=0 walked=0", "arr n=3 mid=6 folded=567"], "{lines:?}");
}

#[test]
fn hashset_sink() {
    // the mapset sink: a put-loop over the flow (the key union bound
    // admits the legal keys)
    let lines = run(
        r#"
use flow::{ Flow, FromFlow, IntoFlow };
use nmapset::{ HashSet };
use ink::{ Logger };

entry fn main() -> nil {
    let log = Logger.new("flow");
    let words: [str] = ["a", "b", "a", "c"];
    let set: HashSet<str> = HashSet.from_flow(words.into_flow());
    let has_a: bool = set.has("a");
    let has_c: bool = set.has("c");
    let has_z: bool = set.has("z");
    log.info(f"set n={set.len()} has_a={has_a} has_c={has_c} has_z={has_z}");
}
"#,
    );
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("set n=3"), "{lines:?}");
    assert!(lines[0].contains("has_a=true"), "{lines:?}");
    assert!(lines[0].contains("has_z=false"), "{lines:?}");
}

#[test]
fn take_stops_the_source() {
    // stop-propagation (adapted at landing: a consumer cannot implement
    // the generic `IntoFlow` across the package boundary — the v1 gate
    // reserves generic foreign traits for their declaring module — so
    // the source-stop rides the LOCAL Iterable: a for-of `break` answers
    // `false` at the emit exactly like flow's take stage, and the
    // counting source stops at the third drive; flow's own `take(2)`
    // feeds the sink the same two elements).
    let lines = run(
        r#"
use pouch::{ Vec };
use flow::{ Flow, IntoFlow, FromFlow };
use core::{ Iterable };
use ink::{ Logger };

class Counting {
    n: i32;
}
impl Counting {
    fn new() -> Self { return Self { n: 0 }; }
}
impl Iterable<i32> for Counting {
    fn iterate(mut self, emit: fn(i32) -> bool) {
        let mut i = 1;
        while (i <= 100) {
            self.n += 1;
            if (!emit(i)) { return; }
            i += 1;
        }
    }
}

entry fn main() -> nil {
    let log = Logger.new("flow");
    let mut c: Counting = Counting.new();
    let stopped: Vec<i32> = Vec.new();
    for (let x of c) {
        if (stopped.len == 2) { break; }
        stopped.push(x);
    }
    log.info(f"stopped n={stopped.len} last={stopped[1]} driven={c.n}");
    let nums: Vec<i32> = Vec.new();
    let mut i = 0;
    while (i < 100) { nums.push(i + 1); i += 1; }
    let out: Vec<i32> = Vec.from_flow(nums.into_flow().take(2));
    log.info(f"out n={out.len} first={out[0]} last={out[1]}");
}
"#,
    );
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("stopped n=2"), "{lines:?}");
    assert!(lines[0].contains("last=2"), "{lines:?}");
    assert!(lines[0].contains("driven=3"), "{lines:?}");
    assert!(lines[1].contains("out n=2"), "{lines:?}");
    assert!(lines[1].contains("first=1"), "{lines:?}");
    assert!(lines[1].contains("last=2"), "{lines:?}");
}

#[test]
fn map_inference_fn_path_and_explicit_type_arg() {
    // the generic-method inference ladder: rung 1 — the bare fn-path
    // argument unifies against the declared param's fn type (R := i32
    // from the helper's return); the explicit `<i32>` spelling is the
    // checked half
    let lines = run(
        r#"
use pouch::{ Vec };
use flow::{ Flow, IntoFlow, FromFlow };
use ink::{ Logger };

fn double(x: i32) -> i32 { return x * 2; }

entry fn main() -> nil {
    let log = Logger.new("flow");
    let nums: Vec<i32> = Vec.new();
    nums.push(1); nums.push(2);
    // the bare fn path: `map(double)` infers R from double's return
    let a: Vec<i32> = Vec.from_flow(nums.into_flow().map(double));
    // the explicit spelling
    let b: Vec<i32> = Vec.from_flow(nums.into_flow().map<i32>(fn(x: i32) -> i32 { return x * 2; }));
    log.info(f"a={a.len} b={b.len}");
}
"#,
    );
    assert_eq!(lines, vec!["a=2 b=2"], "{lines:?}");
}

#[test]
fn unannotated_lambda_diagnoses_with_the_fix() {
    // the inference ladder's last rung is the diagnostic: an
    // unannotated lambda cannot drive the method's inference
    let mut s = Session::new();
    mount_std(&mut s);
    let base = root();
    rut_driver::mount_dir(&mut s, &base.join("rut/ink")).expect("ink");
    for b in ["pouch", "nmapset", "flow"] {
        let bytes = std::fs::read(base.join(format!("dist/std/{b}.rutbundle"))).expect(b);
        mount_bundle_bytes(&mut s, &bytes).expect("mount bundle");
    }
    s.register_module("app", Module { body: ModuleBody::Source { text: r#"
use pouch::{ Vec };
use flow::{ Flow, FromFlow, IntoFlow };

entry fn main() -> nil {
    let nums: Vec<i32> = Vec.new();
    let out: Vec<i32> = Vec.from_flow(nums.into_flow().map(fn(x) { return x * 2; }));
}
"#.into(), is_decl: false }, ..Default::default() }).expect("app");
    let out = compile_graph(&s, "app");
    assert!(
        out.diags.iter().any(|d| {
            d.msg.contains("cannot infer `R` of `Flow.map` from an unannotated lambda")
                && d.msg.contains("annotate its parameters")
        }),
        "{:?}",
        out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>()
    );
}
