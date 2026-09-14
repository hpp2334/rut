//! End-to-end — the demo page's 8 cases (RFC 0041 §3 / demo/src/cases.ts)
//! compiled AND executed through the full pipeline: parse → check → LIR →
//! binary → decode → verify → interpret with budgets.

use std::cell::RefCell;
use std::rc::Rc;

fn run_case(src: &str, fuel: u64) -> (Vec<String>, Option<String>, u64) {
    // append the logger import so the original spans stay put
    let combined = format!("{src}\nimport {{ Logger }} from \"std:log\";\n");
    let out = rut_driver::compile_module(&combined, rut_parser::Mode::Impl, "main");
    assert!(
        out.diags.is_empty(),
        "unexpected diags:\n{}",
        rut_lexer::diag::render_diags(&combined, &out.diags)
    );
    let binary = out.binary.expect("binary");
    let prog = rut_core::binary::decode(&binary).expect("decode");
    rut_vm::verify::verify(&prog).expect("verify");
    let lines: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = lines.clone();
    let limits = rut_vm::interp::Limits {
        fuel: Some(fuel),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default()).expect("vm");
    rut_std::logger::install_std_log(&mut vm, move |msg| sink.borrow_mut().push(msg.to_string()));
    rut_std::math::install_std_math(&mut vm);
    let trap = match vm.call("main", &[]) {
        Ok(_) => None,
        Err(t) => Some(t.name()),
    };
    let lines = lines.borrow().clone();
    (lines, trap, vm.fuel_used)
}

/// Compile a snippet with the logger import appended (the original spans of
/// `src` are preserved).
fn compile(src: &str, module: &str) -> rut_driver::CompileOutput {
    let combined = format!("{src}\nimport {{ Logger }} from \"std:log\";\n");
    rut_driver::compile_module(&combined, rut_parser::Mode::Impl, module)
}

#[test]
fn case1_hello_format() {
    let src = r#"
enum Flavor { Sweet, Sour }
fn describe(f: Flavor) -> str {
    return when (f) {
        Flavor.Sweet -> "sweet",
        Flavor.Sour  -> "sour",
    };
}
pub fn main() -> unit {
    let name = "rut";
    let n = 41 + 1;
    Logger.new("app").info(f"hi {name}! n={n} tab:\t'c'={'c'}");
    Logger.new("app").info(describe(Flavor.Sour));
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["hi rut! n=42 tab:\t'c'=c", "sour"]);
}

#[test]
fn case2_cells_and_own() {
    let src = r#"
dataclass Point { x: f32; y: f32 }
pub fn main() -> unit {
    let mut p = Point { x: 1, y: 2 };
    let q = p;
    p.x = 4;
    let mut r = own(p);
    r.x = 9;
    Logger.new("app").info(f"q.x={q.x} p.x={p.x} r.x={r.x}");
    Logger.new("app").info(f"q==p {q == p}, r==p {r == p}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["q.x=4 p.x=4 r.x=9", "q==p true, r==p false"]);
}

#[test]
fn case3_opaque() {
    let src = r#"
dataclass Point { x: f32; y: f32 }
pub fn main() -> unit {
    let box1 = Opaque.new(Point { x: 1, y: 2 });
    let box2 = Opaque.new("hello");
    Logger.new("app").info(f"box1 is Point: {box1 is Point}");
    Logger.new("app").info(f"box2 is Point: {box2 is Point}");
    let p = downcast<Point>(box1);
    if (p.is_some()) {
        Logger.new("app").info(f"recovered {p.value.x} {p.value.y}");
    }
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["box1 is Point: true", "box2 is Point: false", "recovered 1 2"]);
}

#[test]
fn case4_sieve() {
    let src = r#"
import { Vec } from "std:collection";
fn sieve(limit: i32) -> Vec<i32> {
    let mut marks = Vec<u8>.zeroed(limit + 1);
    let primes: Vec<i32> = Vec.new();
    for (let i = 2; i <= limit; i += 1) {
        if (marks[i] == 0) {
            primes.push(i);
            let mut m = i * i;
            while (m <= limit) {
                marks[m] = 1;
                m += i;
            }
        }
    }
    return primes;
}
pub fn main() -> unit {
    let primes = sieve(100);
    Logger.new("app").info(f"{primes.len()} primes up to 100, last={primes[primes.len() - 1]}");
}
"#;
    let (lines, trap, _) = run_case(src, 10_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["25 primes up to 100, last=97"]);
}

#[test]
fn case5_when_exhaustive() {
    let src = r#"
enum Color { Red, Green, Blue }
fn mix(a: Color, b: Color) -> str {
    return when (a) {
        Color.Red -> when (b) {
            Color.Red   -> "red+red",
            Color.Green -> "yellow",
            Color.Blue  -> "magenta",
        },
        Color.Green -> "greenish",
        Color.Blue  -> "blueish",
    };
}
pub fn main() -> unit {
    Logger.new("app").info(mix(Color.Red, Color.Green));
    Logger.new("app").info(mix(Color.Blue, Color.Blue));
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["yellow", "blueish"]);
}

#[test]
fn case6_dyn_dispatch() {
    let src = r#"
import { Vec } from "std:collection";
interface Shape {
    fn area(self) -> f32;
    fn name(self) -> str;
}
dataclass Circle { r: f32; }
impl Shape for Circle {
    fn area(self) -> f32 { return 3.14159265f32 * self.r * self.r; }
    fn name(self) -> str { return "circle"; }
}
dataclass Square { s: f32; }
impl Shape for Square {
    fn area(self) -> f32 { return self.s * self.s; }
    fn name(self) -> str { return "square"; }
}
pub fn main() -> unit {
    let shapes: Vec<Shape> = Vec.from([
        Circle { r: 1 },
        Square { s: 2 },
    ]);
    for (let s of shapes) {
        Logger.new("app").info(f"{s.name()} area={s.area()}");
    }
    let c = Circle { r: 1 };
    Logger.new("app").info(f"c is Shape: {c is Shape}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(
        lines,
        vec!["circle area=3.1415927", "square area=4", "c is Shape: true"]
    );
}

#[test]
fn case7_closures_generics() {
    let src = r#"
import { Vec } from "std:collection";
fn map<T, U>(v: Vec<T>, f: fn(T) -> U) -> Vec<U> {
    let out: Vec<U> = Vec<U>.new();
    for (let x of v) { out.push(f(x)); }
    return out;
}
pub fn main() -> unit {
    let xs = Vec<i32>.from([1, 2, 3, 4]);
    let k = 10;
    let ys = map<i32, i32>(xs, (x) => x * k);
    Logger.new("app").info(f"{ys[0]} {ys[1]} {ys[2]} {ys[3]}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["10 20 30 40"]);
}

#[test]
fn case8_fuel_traps_and_parks() {
    let src = r#"
pub fn main() -> unit {
    let mut i = 0;
    while (true) {
        i += 1;
        if (i % 1000000 == 0) {
            Logger.new("app").info(f"tick {i}");
        }
    }
}
"#;
    // 10M fuel: bounded, resumable (RFC 0040 §2) — the frame parks
    let (lines, trap, fuel_used) = run_case(src, 10_000_000);
    assert_eq!(trap.as_deref(), Some("OutOfFuel"));
    assert!(fuel_used >= 9_999_000, "fuel consumed: {fuel_used}");
    // whether a tick prints depends on ops/iteration; the CONTRACT is the
    // clean trap with the frame parked (no hang, no crash)
    assert!(lines.len() <= 10);
}

#[test]
fn overflow_traps_and_wrapping_escapes() {
    let src = r#"
import { Math } from "std:math";
pub fn main() -> unit {
    let mut x = 2147483647;
    x = Math.wrapping_add(x, 1);   // wrapping: fine (RFC 0004 §3)
    Logger.new("app").info(f"x={x}");
    let mut s = 1073741824;  // 1 << 30
    s = Math.wrapping_shl(s, 1);   // wrapping shl: bits shifted out are gone
    Logger.new("app").info(f"s={s}");
    s = Math.wrapping_shl(s, 1);   // same law
    Logger.new("app").info(f"s={s}");
    let u: u32 = 3221225472u32; // 0xC000_0000
    Logger.new("app").info(f"u={Math.wrapping_shl(u, 1)}");
    let y = 2147483647 + 1;  // trapping: overflow
    Logger.new("app").info(f"y={y}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(lines, vec!["x=-2147483648", "s=-2147483648", "s=0", "u=2147483648"]);
    assert_eq!(trap.as_deref(), Some("Overflow"));
}

#[test]
fn plain_shl_traps_when_bits_leave_the_width() {
    // RFC 0004 §3: `<<` traps; only `Math.wrapping_shl` wraps. This was silently a
    // RIGHT shift before BitOp::WrapShl existed.
    let src = r#"
pub fn main() -> unit {
    Logger.new("app").info("before");
    let s = 1073741824 << 1;   // 1 << 31 does not fit i32
    Logger.new("app").info(f"after {s}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(lines, vec!["before"]);
    assert_eq!(trap.as_deref(), Some("Overflow"));
}

#[test]
fn shr_follows_signedness() {
    // RFC 0004 §1: `>>` is logical on unsigned (no sign-extension of the
    // top bit — u64's bit 63 is magnitude, not sign), arithmetic on signed.
    let src = r#"
pub fn main() -> unit {
    let u: u64 = 0xFFFFFFFFFFFFFFFFu64;
    Logger.new("app").info(f"u={u >> 1}");
    let v: u32 = 0x80000000u32;
    Logger.new("app").info(f"v={v >> 31}");
    let s = -8 as i64;
    Logger.new("app").info(f"s={s >> 1}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(lines, vec!["u=9223372036854775807", "v=1", "s=-4"]);
    assert_eq!(trap, None);
}

#[test]
fn ne_on_primitives_is_not_eq() {
    // `!=` on numbers used to compile to `CmpOp::Eq` — silently the
    // opposite comparison. It flowed everywhere: `while (len % 64 != 56)`
    // never entered its body.
    let src = r#"
import { Vec } from "std:collection";
pub fn main() -> unit {
    Logger.new("app").info(f"a={1 != 56}");
    Logger.new("app").info(f"b={1 == 56}");
    Logger.new("app").info(f"c={2 != 2}");
    if (3 != 4) { Logger.new("app").info("differs"); } else { Logger.new("app").info("equal"); }
    let mut m: Vec<u8> = Vec.new();
    m.push(0x80);
    while (m.len() % 64 != 56) { m.push(0); }
    Logger.new("app").info(f"padded={m.len()}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(lines, vec!["a=true", "b=false", "c=false", "differs", "padded=56"]);
    assert_eq!(trap, None);
}

#[test]
fn bare_vec_with_length_allocates_zeroed_elements() {
    // RFC 0005 §3: `Vec<f32>(1024)` — n zeroed elements. The bare form
    // (element from the annotation) used to emit ArrNew with a hardcoded
    // ZERO length — the argument was silently dropped, and the first
    // index-assign on it trapped out of bounds.
    let src = r#"
import { Vec } from "std:collection";
pub fn main() -> unit {
    let mut k: Vec<u32> = Vec.zeroed(4);
    Logger.new("app").info(f"a len={k.len()} k3={k[3]}");
    k[0] = 7;
    k[3] = 9;
    Logger.new("app").info(f"b k0={k[0]} k3={k[3]} len={k.len()}");
    let mut j: Vec<u8> = Vec.zeroed(3);
    j.push(1);
    Logger.new("app").info(f"c len={j.len()} j0={j[0]} j3={j[3]}");
    let e: Vec<u32> = Vec.zeroed(0);
    let b: Vec<u32> = Vec.new();
    Logger.new("app").info(f"d {e.len()} {b.len()}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(lines, vec!["a len=4 k3=0", "b k0=7 k3=9 len=4", "c len=4 j0=0 j3=1", "d 0 0"]);
    assert_eq!(trap, None);
}

#[test]
fn generic_static_receiver_resolves() {
    // `Vec<u32>.from(..)` parses as a Method over Path[Vec<u32>] — the
    // static-receiver route used to demand an EMPTY generics list, so the
    // callee fell through to expression-compile and died as `unknown name
    // Vec — module paths`, order-dependently. The explicit type argument
    // now reaches the static: it plays the annotation's role.
    let src = r#"
import { Vec } from "std:collection";
pub fn main() -> unit {
    let a: Vec<u32> = Vec<u32>.from([1, 2, 3]);
    let b = Vec<u32>.from([4, 5]);
    let c = Vec<u8>.from([250, 251]);
    let d: Vec<u32> = Vec.from([6, 7]);   // bare form, annotation-driven
    Logger.new("app").info(f"a={a[0] + a[1] + a[2]} b={b[0]} c={c[1]} d={d[1]} n={d.len()}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(lines, vec!["a=6 b=4 c=251 d=7 n=2"]);
    assert_eq!(trap, None);
    // everywhere else, explicit generics on a static head stay a clear error
    // (the prelude import is present, so the static route engages — RFC 0028)
    let bad = "import { Option } from \"std:core\";\npub fn main() -> unit { let x = Option<i32>.some(5); Logger.new(\"app\").info(f\"{x.value}\"); }";
    let out = rut_driver::compile_module(bad, rut_parser::Mode::Impl, "main");
    assert!(out.diags.iter().any(|d| d.msg.contains("not supported in this build")));
    // and without the import the name is simply not in scope (RFC 0028:
    // the prelude is imported, never ambient)
    let unimported = "pub fn main() -> unit { let x = Option.some(5); Logger.new(\"app\").info(f\"{x.value}\"); }";
    let out = rut_driver::compile_module(unimported, rut_parser::Mode::Impl, "main");
    assert!(out
        .diags
        .iter()
        .any(|d| d.msg.contains("`Option` is not in scope") && d.msg.contains("std:core")));
}

#[test]
fn heap_budget_traps_before_the_write() {
    // RFC 0040 §1: OutOfMemory leaves the heap byte-identical — a tiny
    // budget fails the Vec allocation cleanly. `Vec<u8>` packs one byte
    // per element, so it takes 5M elements to exceed the 4 MB budget.
    let src = r#"
import { Vec } from "std:collection";
pub fn main() -> unit {
    let v = Vec<u8>.zeroed(5000000);
    Logger.new("app").info("allocated");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(lines, Vec::<String>::new());
    assert_eq!(trap.as_deref(), Some("OutOfMemory"));
}

#[test]
fn mut_binding_law_is_enforced() {
    let src = r#"
dataclass P { x: i32 }
pub fn main() -> unit {
    let p = P { x: 1 };
    p.x = 2;
}
"#;
    let out = compile(src, "main");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("let mut")),
        "want the mut-binding law diag: {:?}",
        out.diags
    );
    assert!(out.binary.is_none());
}

#[test]
fn when_exhaustiveness_is_enforced() {
    let src = r#"
enum Color { Red, Green, Blue }
pub fn main() -> unit {
    Logger.new("app").info(when (Color.Red) { Color.Red -> "r" });
}
"#;
    let out = compile(src, "main");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("exhaustive")),
        "{:?}",
        out.diags
    );
}

#[test]
fn option_eq_is_a_compile_error() {
    let src = r#"
pub fn main() -> unit {
    let a = Option.some(1);
    Logger.new("app").info(f"{a == a}");
}
"#;
    let out = compile(src, "main");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("Option")),
        "{:?}",
        out.diags
    );
}

#[test]
fn dump_is_labeled_and_spanned() {
    // the CLI text dump and the demo JSON tree render from one mapping:
    // labeled `field: value` lines, `- item` bullets, spans on every node,
    // and no display strings on the JSON wire
    let src = r#"enum Flavor { Sweet, Sour = 5 }
pub fn main() -> unit { Logger.new("app").info(f"{1 + 1}"); }
"#;
    let out = compile(src, "main");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let text = &out.ast_dump;
    assert!(text.contains("@0 Enum Flavor [0,35)"), "header: {text}");
    assert!(text.contains("vis: pub(self)"), "vis label: {text}");
    assert!(text.contains("- Sour = 5"), "member bullet: {text}");
    let json = &out.ast_json;
    let root = serde_hint_parse(json);
    assert_eq!(root.kind, "Module");
    assert!(json.contains("\"span\":[0,35]"), "span on the wire: {json}");
    for banned in ["\"text\"", "\"label\"", "\"fields\"", "\"summary\""] {
        assert!(!json.contains(banned), "banned key {banned} on the wire");
    }
}

/// tiny structural probe: the root object's `kind` (no serde in this crate)
fn serde_hint_parse(json: &str) -> KindProbe {
    let i = json.find("\"kind\":\"").expect("kind key") + 8;
    let rest = &json[i..];
    let end = rest.find('"').unwrap();
    KindProbe { kind: rest[..end].to_string() }
}
struct KindProbe {
    kind: String,
}

#[test]
fn if_else_chains_execute_both_arms() {
    // else-blocks and else-if chains were a stubbed statement ("expected a
    // statement") — the corpus never compiled them
    let src = r#"
fn classify(n: i32) -> str {
    if (n == 0) {
        return "zero";
    } else if (n < 0) {
        return "negative";
    } else {
        return "positive";
    }
}
pub fn main() -> unit {
    Logger.new("app").info(classify(0));
    Logger.new("app").info(classify(-3));
    Logger.new("app").info(classify(7));
    let mut late = 0;
    for (let i = 0; i < 4; i += 1) {
        if (i % 2 == 0) { late += 1; } else { late += 10; }
    }
    Logger.new("app").info(f"late={late}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["zero", "negative", "positive", "late=22"]);
}

#[test]
fn shortcircuit_truth_table() {
    // && and || both miscompiled: the rhs value never reached the result
    // register (&& was always false; f||t was false too)
    let src = r#"
pub fn main() -> unit {
    let t = true;
    let f = false;
    Logger.new("app").info(f"and: {t && t} {t && f} {f && t} {f && f}");
    Logger.new("app").info(f"or:  {t || t} {t || f} {f || t} {f || f}");
    let n = 6;
    if (n > 0 && n % 2 == 0) { Logger.new("app").info("even positive"); }
    if (n < 0 || n % 3 == 0) { Logger.new("app").info("div by 3 or negative"); }
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(
        lines,
        vec![
            "and: true false false false",
            "or:  true true true false",
            "even positive",
            "div by 3 or negative",
        ]
    );
}

#[test]
fn zero_param_class_constructor_and_self_ty() {
    // zero-param class methods panicked on params[0]; `Self` in signature
    // position was rejected at the call site
    let src = r#"
class Counter {
    n: i32;
    fn new() -> Self { return Self { n: 0 }; }
    fn bump(mut self) -> unit { self.n += 1; }
    fn count(self) -> i32 { return self.n; }
}
class Wrapped {
    inner: Counter;
    fn new() -> Self { return Self { inner: Counter.new() }; }
    fn bump(mut self) -> unit { self.inner.bump(); }
    fn count(self) -> i32 { return self.inner.count(); }
}
pub fn main() -> unit {
    let mut c = Counter.new();
    c.bump();
    c.bump();
    Logger.new("app").info(f"count={c.count()}");
    let mut w = Wrapped.new();
    w.bump();
    Logger.new("app").info(f"wrapped={w.count()}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["count=2", "wrapped=1"]);
}

#[test]
fn when_statement_block_arms() {
    // `when` as a statement with block bodies was rejected ("unsupported
    // expression in this build") — RFC 0008's statement form
    let src = r#"
enum Light { Green, Yellow, Red }
pub fn main() -> unit {
    let mut dropped = 0;
    let mut kept = 0;
    for (let i = 0; i < 6; i += 1) {
        when (i % 3) {
            0       -> { dropped += 1; },
            1, 2    -> { kept += 1; },
            else    -> { kept += 100; },
        }
    }
    Logger.new("app").info(f"dropped={dropped} kept={kept}");
    when (Light.Red) {
        Light.Green  -> { Logger.new("app").info("go"); },
        Light.Yellow -> { Logger.new("app").info("brake"); },
        Light.Red    -> { Logger.new("app").info("stop"); },
    }
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["dropped=2 kept=4", "stop"]);
}

#[test]
fn enum_singletons_survive_their_use_sites() {
    // EnumNew stored the immortal singleton slot WITHOUT taking an owned
    // reference — every frame teardown over-released, freeing the "immortal"
    // cell while the singleton map still pointed at it (heap corruption;
    // detected by case1's thread, locked here with many use sites)
    let src = r#"
enum Flavor { Sweet, Sour, Salty }
fn describe(f: Flavor) -> str {
    return when (f) {
        Flavor.Sweet  -> "sweet",
        Flavor.Sour   -> "sour",
        Flavor.Salty  -> "salty",
    };
}
pub fn main() -> unit {
    let hits = [describe(Flavor.Sweet), describe(Flavor.Sour), describe(Flavor.Salty)];
    Logger.new("app").info(hits[0]);
    Logger.new("app").info(hits[1]);
    Logger.new("app").info(hits[2]);
    for (let i = 0; i < 50; i += 1) {
        Logger.new("app").info(describe(when (i % 2) { 0 -> Flavor.Sweet, else -> Flavor.Salty }));
    }
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines[0], "sweet");
    assert_eq!(lines[1], "sour");
    assert_eq!(lines[2], "salty");
    assert_eq!(lines.len(), 53);
    assert!(lines[3..].iter().all(|l| l == "sweet" || l == "salty"));
}

// ---- the entry surface (RFC 0035 §3): host-callable fns, no main ----

fn entry_vm(src: &str) -> rut_vm::interp::Vm {
    let out = compile(src, "m");
    assert!(
        out.diags.is_empty(),
        "unexpected diags:\n{}",
        rut_lexer::diag::render_diags(src, &out.diags)
    );
    let prog = rut_core::binary::decode(out.binary.as_deref().expect("binary")).expect("decode");
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    rut_vm::interp::Vm::new(std::rc::Rc::new(prog), &limits, rut_vm::interp::HostHooks::default()).expect("vm")
}

#[test]
fn entry_fns_compile_without_main_and_cross_values() {
    // a module with entries and NO main compiles (entries are roots) and
    // the host drives it: Opaque container in/out, primitives, bytes,
    // Option and Result in both arms
    let src = r#"
import { Vec } from "std:collection";
dataclass Row { id: i32; }
dataclass Box { rows: Vec<Row>; }

entry fn make() -> Opaque { return Opaque.new(Box { rows: Vec.new() }); }
entry fn put(c: Opaque) -> u32 {
    let b = downcast<Box>(c).value;
    b.rows.push(Row { id: 1 });
    return b.rows.len() as u32;
}
entry fn echo_bytes(v: bytes) -> bytes { return v; }
entry fn maybe(v: i32) -> Option<i32> {
    return when (v > 0) { true -> Option.some(v), else -> Option.none() };
}
entry fn checked(v: i32) -> Result<i32, str> {
    return when (v >= 0) { true -> Result.ok(v), else -> Result.err("negative") };
}
"#;
    let mut vm = entry_vm(src);
    use rut_vm::heap::Value;
    let Value::Opaque(c) = vm.call("make", &[]).unwrap() else { unreachable!() };
    assert_eq!(vm.call("put", &[Value::Opaque(c.clone())]).unwrap(), Value::I64(1));
    assert_eq!(vm.call("put", &[Value::Opaque(c.clone())]).unwrap(), Value::I64(2));
    assert_eq!(
        vm.call("echo_bytes", &[Value::Bytes(vec![1, 2, 250])]).unwrap(),
        Value::Bytes(vec![1, 2, 250])
    );
    assert_eq!(
        vm.call("maybe", &[Value::I64(7)]).unwrap(),
        Value::Opt(Some(Box::new(Value::I64(7))))
    );
    assert_eq!(vm.call("maybe", &[Value::I64(-1)]).unwrap(), Value::Opt(None));
    assert_eq!(
        vm.call("checked", &[Value::I64(3)]).unwrap(),
        Value::Res(Ok(Box::new(Value::I64(3))))
    );
    assert_eq!(
        vm.call("checked", &[Value::I64(-3)]).unwrap(),
        Value::Res(Err(Box::new(Value::Str("negative".into()))))
    );
    // embedder mistakes are named traps, never silent zeros
    let err = vm.call("maybe", &[Value::Str("x".into())]).unwrap_err();
    assert!(err.msg.contains("argument"), "{}", err.msg);
}

#[test]
fn entry_crossing_rule_is_compile_time() {
    // rut cells never cross: dataclasses, Vec<T> of cells, generics —
    // each is a source diagnostic naming the offending signature
    let src = r#"
dataclass Row { id: i32; }
entry fn bad_param(r: Row) -> unit { }
"#;
    let out = compile(src, "m");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("parameter `r` is `Row`")),
        "{:?}",
        out.diags
    );

    let src = r#"
import { Vec } from "std:collection";
dataclass Row { id: i32; }
entry fn bad_ret() -> Vec<Row> { return Vec.new(); }
"#;
    let out = compile(src, "m");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("returns `Vec<Row>`")),
        "{:?}",
        out.diags
    );

    // `Vec<u8>` is a mutable builder, not the binary type: it no longer
    // crosses — `bytes` is what does (RFC 0004, RFC 0023 §2)
    let src = r#"
import { Vec } from "std:collection";
entry fn old_buffer(v: Vec<u8>) -> Vec<u8> { return v; }
"#;
    let out = compile(src, "m");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("parameter `v` is `Vec<u8>`")),
        "{:?}",
        out.diags
    );

    let src = r#"
entry fn generic<T>(v: T) -> T { return v; }
"#;
    let out = compile(src, "m");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("cannot be generic")),
        "{:?}",
        out.diags
    );
}

#[test]
fn bytes_are_an_immutable_primitive() {
    // RFC 0004: bytes is the immutable binary primitive. Construction via
    // bytes(n)/bytes.from(..), a Vec<u8> builder freezes into one, and ==
    // compares content. `Vec<u8>` is a mutable builder, not the binary type.
    let src = r#"
import { Vec } from "std:collection";
pub fn main() -> unit {
    let z = bytes(3);
    Logger.new("app").info(f"z={bytes_len(z)}");
    let a = bytes_from([1, 2, 3]);
    let b = bytes_from([1, 2, 3]);
    Logger.new("app").info(f"a={bytes_len(a)} eq={a == b} ne={a != z}");
    let mut buf: Vec<u8> = Vec.new();
    buf.push(9);
    buf.push(8);
    let f = buf.freeze();
    Logger.new("app").info(f"f={bytes_len(f)} f0={f[0]} f1={f[1]}");
    let enc = string_encode("rut");
    Logger.new("app").info(f"enc={bytes_len(enc)} dec={bytes_decode(enc)}");
    let mut sum: u8 = 0u8;
    for (let x of a) { sum = sum + x; }
    Logger.new("app").info(f"sum={sum}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(
        lines,
        vec!["z=3", "a=3 eq=true ne=true", "f=2 f0=9 f1=8", "enc=3 dec=rut", "sum=6"]
    );
}

#[test]
fn bytes_index_out_of_bounds_traps() {
    let src = r#"
pub fn main() -> unit {
    let a = bytes_from([1]);
    Logger.new("app").info(f"{a[5]}");
}
"#;
    let (_, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap.as_deref(), Some("IndexOutOfBounds"));
}

#[test]
fn str_index_ascii_and_utf8_and_for_of_array() {
    // `str` is a `char` sequence: the ASCII fast path makes `s[i]`/`for..of`
    // O(1), while a non-ASCII string still decodes the UTF-8 prefix. The
    // `for..of` over the fixed `Array` also exercises the hoisted length.
    let src = r#"
pub fn main() -> unit {
    let a = "ACGT";
    let u = "héllo";
    let mut k = 0;
    for (let c of u) { k += 1; }
    Logger.new("app").info(f"{a[0]}{a[3]}{u[1]} {k}");
    let xs = [10, 20, 30];
    let mut s = 0;
    for (let x of xs) { s += x; }
    Logger.new("app").info(f"s={s}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["ATé 5", "s=60"]);
}

#[test]
fn str_index_out_of_bounds_traps() {
    let src = r#"
pub fn main() -> unit {
    let a = "ACGT";
    Logger.new("app").info(f"{a[9]}");
}
"#;
    let (_, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap.as_deref(), Some("IndexOutOfBounds"));
}

#[test]
fn plain_pub_stays_unrestricted() {
    // `pub` is import-visibility for rut modules (RFC 0003 §2), NOT
    // the host surface: a Stack crosses fine between rut fns
    let src = r#"
import { Vec } from "std:collection";
dataclass Row { id: i32; }
pub class Stack {
    items: Vec<Row>;                // unannotated member = module-private
    fn new() -> Self { return Self { items: Vec.new() }; }
    fn push(mut self, id: i32) -> unit { self.items.push(Row { id: id }); }
    pub fn len(self) -> i32 { return self.items.len(); }
}
pub fn drain(s: Stack) -> i32 { return s.len(); }
pub fn main() -> unit {
    let mut st = Stack.new();
    st.push(1);
    Logger.new("app").info(f"drained {drain(st)}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["drained 1"]);
}

#[test]
fn member_pub_scopes_compile_and_run() {
    // RFC 0003 §2 / 0010 §2: the pub scopes apply to class members —
    // parsing, the AST dump label, and the compiled call path
    let src = r#"
pub class Gauge {
    n: i32;                        // unannotated = module-private
    pub w: i32;                    // pub field

    pub fn new() -> Self { return Self { n: 0, w: 3 }; }
    pub(mod) fn bump(mut self) -> unit { self.n += self.w; }
    pub(self) fn raw(self) -> i32 { return self.n; }
    fn secret(self) -> i32 { return self.n * 100; }
}
pub fn main() -> unit {
    let mut g = Gauge.new();
    g.bump();
    g.bump();
    Logger.new("app").info(f"gauge={g.raw()} secret={g.secret()} w={g.w}");
}
"#;
    let out = compile(src, "main");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    // the dump labels member visibility (annotated members only)
    assert!(out.ast_dump.contains("vis: pub(mod)"), "member vis label: {}", out.ast_dump);
    assert!(!out.ast_dump.contains("vis: pub\n      name: n"), "unannotated stays quiet: {}", out.ast_dump);
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["gauge=6 secret=600 w=3"]);
}

#[test]
fn recursive_dataclass_tree_runs() {
    // RFC 0009 §"Representation": recursive shapes like `left: Option<Node>`
    // are legal. They used to fail at resolve (`unknown type Node`) because
    // the record was registered only after its fields resolved, and the
    // layout pass inlined field payloads (infinite recursion). Fields are
    // now registered first and laid out as handle slots.
    let src = r#"
dataclass Node {
    value: i32,
    left: Option<Node>,
    right: Option<Node>,
}
fn make(depth: i32, v: i32) -> Node {
    if (depth <= 0) {
        return Node { value: v, left: Option.none(), right: Option.none() };
    }
    let l = make(depth - 1, v * 2);
    let r = make(depth - 1, v * 2 + 1);
    return Node { value: v, left: Option.some(l), right: Option.some(r) };
}
fn count(n: Node) -> i32 {
    let mut c = 1;
    if (n.left.is_some()) { c += count(n.left.value); }
    if (n.right.is_some()) { c += count(n.right.value); }
    return c;
}
pub fn main() -> unit {
    Logger.new("app").info(f"CHECKSUM {count(make(10, 1))}");
}
"#;
    let (lines, trap, _) = run_case(src, 5_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["CHECKSUM 2047"]);
}

#[test]
fn forward_and_mutually_recursive_types_resolve() {
    // `Early` names `Later` before it is declared; `Later` names `Early`
    // back. Declaration order must not matter.
    let src = r#"
dataclass Early {
    later: Later,
    tag: i32,
}
dataclass Later {
    back: Option<Early>,
    x: i32,
}
fn make_early() -> Early {
    return Early { later: Later { back: Option.none(), x: 7 }, tag: 1 };
}
pub fn main() -> unit {
    let e = make_early();
    Logger.new("app").info(f"tag={e.tag} x={e.later.x}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["tag=1 x=7"]);
}

#[test]
fn recursive_class_field_resolves_and_runs() {
    // The class form of a self-referential field (a handle slot, so
    // pointer-sized) — `demo/src/examples/node-cycle.rut`'s shape.
    let src = r#"
class Node {
    v: i32 = 0;
    next: Option<Node> = Option.none;
    fn new() -> Self { return Self {}; }
    fn val(self) -> i32 { return self.v; }
}
pub fn main() -> unit {
    let a = Node.new();
    Logger.new("app").info(f"v={a.val()} has_next={a.next.is_some()}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["v=0 has_next=false"]);
}

#[test]
fn generic_vec_over_array_runs() {
    // std:collection's Vec<T> shape: a generic class over the non-growable
    // heap array primitive, monomorphized for i32 (RFC 0013 / RFC 0005).
    let src = r#"
class Vec<T> {
    buf: Array<T>;
    len: i32;
    fn new() -> Self { return Vec.with_capacity(0); }
    fn with_capacity(cap: i32) -> Self { return Self { buf: Array<T>(cap), len: 0 }; }
    fn len(self) -> i32 { return self.len; }
    fn push(mut self, v: T) -> unit {
        if (self.len == self.buf.len()) {
            let mut cap = self.buf.len() * 2;
            if (cap == 0) {
                cap = 4;
            }
            let mut next = Array<T>(cap);
            for (let i = 0; i < self.len; i += 1) {
                next[i] = self.buf[i];
            }
            self.buf = next;
        }
        self.buf[self.len] = v;
        self.len += 1;
    }
    fn get(self, i: i32) -> T { return self.buf[i]; }
    fn pop(mut self) -> Option<T> {
        if (self.len == 0) {
            return Option.none();
        }
        self.len -= 1;
        return Option.some(self.buf[self.len]);
    }
}
pub fn main() -> unit {
    let mut v: Vec<i32> = Vec.new();
    v.push(10);
    v.push(20);
    v.push(30);
    v.push(40);
    v.push(50);
    Logger.new("app").info(f"{v.len()} {v.get(0)} {v.get(4)} {v.get(2)}");
    let p = v.pop();
    Logger.new("app").info(f"{p.value}");
}
"#;
    let (lines, trap, _) = run_case(src, 2_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["5 10 50 30", "50"]);
}

#[test]
fn vec_class_slice_syntax_runs() {
    // `Vec` is rut std-lib code; `v[i]`, `v[i] = x`, and `for (x of v)`
    // lower through its `impl Slice<T> for Vec<T>` (RFC 0005)
    let src = r#"
import { Vec } from "std:collection";
pub fn main() -> unit {
    let mut v: Vec<i32> = Vec.new();
    v.push(1); v.push(2); v.push(3);
    v[0] = 10;
    let mut sum = 0;
    for (let x of v) { sum += x; }
    let w: Vec<i32> = Vec.from([7, 8]);
    let z: Vec<u8> = Vec.zeroed(3);
    Logger.new("app").info(f"{v[0]} {v.len()} {sum} {w[1]} {z.len()}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["10 3 15 8 3"]);
}

#[test]
fn std_collection_vec_via_module_loader_runs() {
    // the real rut/std-collection source, mounted as a module and imported
    // by a consumer; generic Vec is inlined and monomorphized, then linked
    // and executed
    let coll = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../rut/std-collection/entry.rut");
    let coll_src = rut_driver::expand_module_source(&coll).expect("expand");
    let mut s = rut_driver::Session::new();
    // std:core first: the collection source imports its prelude names
    rut_driver::mount_std_core(&mut s);
    s.register_module(
        "std:collection",
        rut_driver::Module { source: Some(coll_src), ..Default::default() },
    )
    .unwrap();
    rut_driver::mount_std_log(&mut s);
    s.register_module(
        "app:main",
        rut_driver::Module {
            source: Some(
                r#"
import { Vec } from "std:collection";
import { Logger } from "std:log";
pub fn main() -> unit {
    let mut v: Vec<i32> = Vec.new();
    v.push(10);
    v.push(20);
    v.push(30);
    let top = v.pop();
    Logger.new("app").info(f"{v.len()} {top.value}");
}
"#
                .into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();

    let out = rut_driver::compile_graph(&s, "app:main");
    assert!(
        out.diags.is_empty(),
        "diags: {:?}",
        out.diags.iter().map(|d| &d.msg).collect::<Vec<_>>()
    );
    let prog = out.program.expect("program");
    rut_vm::verify::verify(&prog).expect("verify");

    let lines: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = lines.clone();
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default()).expect("vm");
    rut_std::logger::install_std_log(&mut vm, move |msg| sink.borrow_mut().push(msg.to_string()));
    rut_std::math::install_std_math(&mut vm);
    let trap = vm.call("main", &[]).err().map(|t| t.name());
    assert_eq!(trap, None);
    assert_eq!(*lines.borrow(), vec!["2 30"]);
}

#[test]
fn std_collection_via_module_loader_runs() {
    // the real rut/std-collection source, mounted and imported by a
    // consumer: `Vec<T>` is rut source over the engine's `Array<T>` cell,
    // so this exercises the rut-source module loader end to end
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std(&mut s);
    s.register_module(
        "app:main",
        rut_driver::Module {
            source: Some(
                r#"
import { Vec } from "std:collection";
import { Logger } from "std:log";
pub fn main() -> unit {
    let mut v: Vec<str> = Vec.new();
    v.push("hello");
    v.push(" ");
    v.push("world");
    let s = string_join(v.as_array());
    Logger.new("app").info(f"{string_len(s)} {s}");
}
"#
                .into(),
            ),
            ..Default::default()
        },
    )
    .unwrap();

    let out = rut_driver::compile_graph(&s, "app:main");
    assert!(
        out.diags.is_empty(),
        "diags: {:?}",
        out.diags.iter().map(|d| &d.msg).collect::<Vec<_>>()
    );
    let prog = out.program.expect("program");
    rut_vm::verify::verify(&prog).expect("verify");

    let lines: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = lines.clone();
    let limits = rut_vm::interp::Limits {
        fuel: Some(4_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default()).expect("vm");
    rut_std::logger::install_std_log(&mut vm, move |msg| sink.borrow_mut().push(msg.to_string()));
    rut_std::math::install_std_math(&mut vm);
    let trap = vm.call("main", &[]).err().map(|t| t.name());
    assert_eq!(trap, None);
    assert_eq!(*lines.borrow(), vec!["11 hello world"]);
}

#[test]
fn fstring_single_part_needs_no_concat() {
    // `f"{s}"` is the identity on a string and `f"{c}"` a char render — the
    // LIR now elides the one-argument `Concat` these used to emit
    // (RFC 0007 §2); the observable result is unchanged.
    let src = r#"
pub fn main() -> unit {
    let s = "abc";
    let t = f"{s}";
    let c = 'x';
    let u = f"{c}";
    Logger.new("app").info(f"{t} {u} {string_len(t)}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["abc x 3"]);
}

#[test]
fn fstring_accumulator_appends_in_place() {
    // `s = f"{s}..."` lowers to `Concat([s, ..]) -> s` (dst == args[0]), so
    // the VM appends into the uniquely-owned cell — amortized growth, not a
    // copy of the whole prefix each step. Correctness must hold for the
    // bail cases (alias in a later part, different target) too.
    let src = r#"
pub fn main() -> unit {
    let mut s = "";
    let mut i = 0;
    while (i < 1000) { s = f"{s}ab"; i += 1; }
    let mut a = "a";
    a = f"{a}{a}";
    let base = "x";
    let b = f"{base}y";
    let mut n = "";
    let mut k = 0;
    while (k < 5) { n = f"{n}{k},"; k += 1; }
    Logger.new("app").info(f"{string_len(s)} {a} {b} {base} {n}");
}
"#;
    let (lines, trap, _) = run_case(src, 4_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["2000 aa xy x 0,1,2,3,4,"]);
}

#[test]
fn string_join_and_vec_as_array_run() {
    // the builtin `string_join(Array<str>)` and the `Vec<T>.as_array()` bridge
    let src = r#"
import { Vec } from "std:collection";
pub fn main() -> unit {
    let mut v: Vec<str> = Vec.new();
    v.push("x"); v.push("y"); v.push("z");
    let e: Vec<str> = Vec.new();
    Logger.new("app").info(f"{string_join(v.as_array())} {string_len(string_join(e.as_array()))}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["xyz 0"]);
}

#[test]
fn as_casts_truncate_like_c_and_rust() {
    // `expr as T` (RFC 0007 §1): conversions never trap. int→int keeps
    // the target's low bits (signed targets sign-extend), float→int
    // truncates toward zero and saturates at the bounds (NaN -> 0). The
    // cast binds tighter than `*` (Rust placement) and chains left.
    let src = r#"
import { Logger } from "std:log";
pub fn main() -> unit {
    let a = 300 as u8;
    let b = -1 as u8;
    let c = 4294967295u32 as i32;
    let d = 2.9 as i32;
    let e = 1.0e300f64 as i32;
    let f = (0.0 / 0.0) as i32;
    let g = -300 as u8;
    let h = 2u8 * 300 as u8;
    let i = 300 as u8 as i32;
    let j = -2.9 as i32;
    let k = 1.0e300f64 as u8;
    Logger.new("app").info(f"{a} {b} {c} {d} {e} {f} {g} {h} {i} {j} {k}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["44 255 -1 2 2147483647 0 212 88 44 -2 255"]);

    // the cast RHS is restricted to the numeric primitives at the parse
    // level — anything else leaves `as` for the select-arm bind
    // (`fut as name`, RFC 0019 §3), so `1 as str` is a syntax error
    let bad = "pub fn main() -> unit { let x = 1 as str; }";
    let out = rut_driver::compile_module(bad, rut_parser::Mode::Impl, "main");
    assert!(out.diags.iter().any(|d| d.msg.contains("expected")));
}

#[test]
fn unsuffixed_int_literals_must_fit_i32() {
    // RFC 0007 §1: bidirectional inference absorbs an unsuffixed literal
    // only while it fits the `i32` default — past it the literal must
    // declare its width, even in a `u64` position, so a dropped or
    // doubled digit can't masquerade as a constant
    let ok = "pub fn main() -> unit { let x: u64 = 5; let y = 2147483647; }";
    let out = rut_driver::compile_module(ok, rut_parser::Mode::Impl, "main");
    assert!(
        out.diags.is_empty(),
        "diags: {:?}",
        out.diags.iter().map(|d| &d.msg).collect::<Vec<_>>()
    );

    let in_u64_ctx = "pub fn main() -> unit { let x: u64 = 13503953896175478587; }";
    let out = rut_driver::compile_module(in_u64_ctx, rut_parser::Mode::Impl, "main");
    assert!(out
        .diags
        .iter()
        .any(|d| d.msg.contains("exceeds the `i32` default") && d.msg.contains("u64")));

    let no_ctx = "pub fn main() -> unit { let x = 99999999999; }";
    let out = rut_driver::compile_module(no_ctx, rut_parser::Mode::Impl, "main");
    assert!(out.diags.iter().any(|d| d.msg.contains("exceeds the `i32` default")));

    let fixed = "pub fn main() -> unit { let x: u64 = 13503953896175478587u64; }";
    let out = rut_driver::compile_module(fixed, rut_parser::Mode::Impl, "main");
    assert!(
        out.diags.is_empty(),
        "diags: {:?}",
        out.diags.iter().map(|d| &d.msg).collect::<Vec<_>>()
    );

    // floats: magnitude is the trigger, precision is not
    let float_ok = "pub fn main() -> unit { let x: f64 = 0.1; }";
    let out = rut_driver::compile_module(float_ok, rut_parser::Mode::Impl, "main");
    assert!(out.diags.is_empty());

    let float_bad = "pub fn main() -> unit { let x: f64 = 1.0e300; }";
    let out = rut_driver::compile_module(float_bad, rut_parser::Mode::Impl, "main");
    assert!(out.diags.iter().any(|d| d.msg.contains("exceeds the `f32` default")));

    let float_fixed = "pub fn main() -> unit { let x: f64 = 1.0e300f64; }";
    let out = rut_driver::compile_module(float_fixed, rut_parser::Mode::Impl, "main");
    assert!(out.diags.is_empty());
}

#[test]
fn math_namespace_intrinsics_run() {
    // `std:math`'s `Math` namespace: wrapping/saturating/checked integer
    // arithmetic (compiler-lowered) plus abs/min/max/signum and the f64
    // host functions/constants (RFC 0004 §3, RFC 0028).
    let src = r#"
import { Math } from "std:math";
pub fn main() -> unit {
    let log = Logger.new("app");
    // i32 wrapping
    log.info(f"{Math.wrapping_add(2147483647, 1)} {Math.wrapping_sub(-2147483647 - 1, 1)} {Math.wrapping_mul(65536, 65536)}");
    // i32 saturating
    log.info(f"{Math.saturating_add(2147483647, 1)} {Math.saturating_sub(-2147483647 - 1, 1)} {Math.saturating_mul(65536, 65536)} {Math.saturating_mul(-65536, 65536)}");
    // i32 checked -> Option
    let a = Math.checked_add(2147483647, 1);
    let b = Math.checked_add(1, 2);
    let c = Math.checked_mul(65536, 65536);
    let d = Math.checked_mul(-65536, 65536);
    log.info(f"{a.is_some()} {b.is_some()} {b.value} {c.is_some()} {d.is_some()}");
    log.info(f"{a.unwrap_or(-1)} {c.unwrap_or(-1)}");
    // u8 edges
    let u: u8 = 255;
    let z: u8 = 0;
    log.info(f"{Math.wrapping_add(u, 1u8)} {Math.saturating_add(u, 1u8)} {Math.checked_add(u, 1u8).is_some()}");
    log.info(f"{Math.wrapping_sub(z, 1u8)} {Math.saturating_sub(z, 1u8)} {Math.checked_sub(z, 1u8).is_some()}");
    // i8 edges
    let lo: i8 = -127 - 1;
    let hi: i8 = 127;
    log.info(f"{Math.wrapping_add(hi, 1i8)} {Math.saturating_add(hi, 1i8)} {Math.wrapping_sub(lo, 1i8)} {Math.saturating_sub(lo, 1i8)}");
    // u64 edge
    let w: u64 = 18446744073709551615u64;
    log.info(f"{Math.wrapping_add(w, 1u64)} {Math.saturating_add(w, 1u64)} {Math.checked_add(w, 1u64).is_some()}");
    // wrapping shift + int helpers
    log.info(f"{Math.wrapping_shl(1073741824, 1)}");
    log.info(f"{Math.abs(-5)} {Math.abs(-2147483647 - 1)} {Math.min(3, 9)} {Math.max(3, 9)} {Math.signum(-7)} {Math.signum(0)} {Math.signum(7)}");
    // float helpers + constants
    log.info(f"{Math.abs(-2.5)} {Math.min(1.0, 2.0)} {Math.max(1.0, 2.0)} {Math.signum(-3.5)} {Math.signum(0.0)} {Math.signum(4.5)}");
    log.info(f"{Math.sqrt(2.0)} {Math.PI}");
}
"#;
    let (lines, trap, _) = run_case(src, 2_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec![
        "-2147483648 2147483647 0",
        "2147483647 -2147483648 2147483647 -2147483648",
        "false true 3 false false",
        "-1 -1",
        "0 255 false",
        "255 0 false",
        "-128 127 127 -128",
        "0 18446744073709551615 false",
        "-2147483648",
        "5 -2147483648 3 9 -1 0 1",
        "2.5 1 2 -1 0 1",
        "1.4142135623730951 3.141592653589793",
    ]);
}

#[test]
fn generic_interface_dispatch() {
    // `Wrap<i32>` — one interface id per type-argument list
    let src = r#"
interface Wrap<T> {
    fn get(self) -> T;
}
class B {
    v: i32;
    fn new(v: i32) -> Self { return Self { v: v }; }
}
impl Wrap<i32> for B {
    fn get(self) -> i32 { return self.v; }
}
pub fn main() -> unit {
    let b = B.new(7);
    let w: Wrap<i32> = b;
    Logger.new("app").info(f"{w.get()}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["7"]);
}

#[test]
fn user_index_contract() {
    // a user interface implements `Index<i32>`; `r[i]`, `r.len()`, and
    // `for (x of r)` all lower through it (the element is the type argument)
    let src = r#"
interface Index<T> {
    fn len(self) -> i32;
    fn get(self, i: i32) -> T;
}
class Range {
    n: i32;
    fn new(n: i32) -> Self { return Self { n: n }; }
}
impl Index<i32> for Range {
    fn len(self) -> i32 { return self.n; }
    fn get(self, i: i32) -> i32 { return i * 2; }
}
pub fn main() -> unit {
    let r = Range.new(3);
    let mut sum = 0;
    for (let i = 0; i < r.len(); i += 1) { sum += r[i]; }
    for (let x of r) { sum += x; }
    Logger.new("app").info(f"{sum} {r[2]}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["12 4"]);
}

#[test]
fn next_based_iterator_contract() {
    // the builtin `Iterator<i32>` contract: `impl Iterator<i32> for X`
    // + `for..of` lowers to `next`
    let src = r#"
class Countdown {
    n: i32;
    fn new(n: i32) -> Self { return Self { n: n }; }
}
impl Iterator<i32> for Countdown {
    fn next(mut self) -> Option<i32> {
        if (self.n <= 0) { return Option.none(); }
        let v = self.n;
        self.n -= 1;
        return Option.some(v);
    }
}
pub fn main() -> unit {
    let c = Countdown.new(3);
    let mut sum = 0;
    for (let x of c) { sum += x; }
    Logger.new("app").info(f"{sum}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["6"]);
}

#[test]
fn vec_iter_cursor() {
    // `Vec` is rut code and ships its own cursor: `for (x of v.iter())`
    // inlines `VecIter<T>.next` (a generic-target impl, no vtable needed)
    let src = r#"
import { Vec } from "std:collection";
pub fn main() -> unit {
    let v = Vec<i32>.from([10, 20, 30]);
    let mut sum = 0;
    for (let x of v.iter()) { sum += x; }
    Logger.new("app").info(f"{sum} {v.len()}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["60 3"]);
}

#[test]
fn float_literal_defaults_to_f32() {
    // an uncontextualized float literal is `f32` (RFC 0007 §1); the slot
    // carries f32 precision, so a literal and a computed f32 agree.
    let src = r#"
pub fn main() -> unit {
    let a = 0.1;            // default: f32
    let b: f64 = 0.1;       // annotation: f64
    let c = 0.1 + 0.0;      // computed f32
    Logger.new("app").info(f"{a} {b} {c}");
    Logger.new("app").info(f"{a == c}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["0.1 0.1 0.1", "true"]);
}

#[test]
fn float_literal_default_does_not_implicitly_widen() {
    // the default is f32, and there are no implicit numeric conversions:
    // feeding it to `f64` is a width error (RFC 0007 §1)
    let src = r#"
import { Vec } from "std:collection";
pub fn main() -> unit {
    let a = 0.1;                 // f32 by default
    let v: Vec<f64> = Vec.new();
    v.push(a);                   // must not widen to f64
}
"#;
    let out = compile(src, "main");
    let msg = out
        .diags
        .iter()
        .map(|d| d.msg.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(msg.contains("f32") && msg.contains("f64"), "expected a width diagnostic, got:\n{msg}");
}

#[test]
fn literals_adapt_to_expected_float_in_unary_and_binary() {
    // `-2.0` as an `f64` argument and the lhs of `1.0 / 4.0` under an
    // `f64` annotation both take the expected type (RFC 0007 §1)
    let src = r#"
fn offset(x: f64) -> f64 { return x + 0.5; }
pub fn main() -> unit {
    let r = offset(-2.0);
    let v: f64 = 1.0 / 4.0;
    Logger.new("app").info(f"{r} {v}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["-1.5 0.25"]);
}

// ---- A1 surface (RFC 0005/0007/0013/0016): nil + *T, make_ptr, on_drop,
// tuples, anonymous closures, zero-value field defaults ----

#[test]
fn a1_make_ptr_deref_and_nil() {
    let src = r#"
import { make_ptr } from "std:core";
dataclass P { x: i32 = 0; }
pub fn main() -> unit {
    let p = make_ptr(P { x: 5 });
    Logger.new("t").info(f"x={p.x} nil={p == nil}");
    let n: *P = nil;
    Logger.new("t").info(f"n nil={n == nil}");
}
"#;
    let (lines, trap, _) = run_case(src, 100_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["x=5 nil=false", "n nil=true"]);
}

#[test]
fn a1_nil_deref_traps() {
    let src = r#"
dataclass P { x: i32 = 0; }
pub fn main() -> unit {
    let n: *P = nil;
    let _ = n.x;
}
"#;
    let (_, trap, _) = run_case(src, 100_000);
    assert_eq!(trap.as_deref(), Some("NilDeref"));
}

#[test]
fn a1_on_drop_runs_at_refcount_zero() {
    let src = r#"
import { make_ptr, on_drop } from "std:core";
dataclass P { x: i32 = 0; }
pub fn main() -> unit {
    let p = make_ptr(P { x: 9 });
    on_drop(p, fn (p: *P) { Logger.new("t").info(f"dropped {p.x}"); });
    Logger.new("t").info("body done");
}
"#;
    let (lines, trap, _) = run_case(src, 100_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["body done", "dropped 9"]);
}

#[test]
fn a1_tuples_multi_return() {
    let src = r#"
fn divmod(a: i32, b: i32) -> (i32, i32) {
    return (a / b, a % b);
}
pub fn main() -> unit {
    let (q, r) = divmod(7, 2);
    Logger.new("t").info(f"q={q} r={r}");
    Logger.new("t").info(f"t1={divmod(9, 4).1}");
}
"#;
    let (lines, trap, _) = run_case(src, 100_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["q=3 r=1", "t1=1"]);
}

#[test]
fn a1_anonymous_fn_and_fn_typed_param() {
    let src = r#"
fn apply(f: fn(i32) -> i32, v: i32) -> i32 {
    return f(v);
}
pub fn main() -> unit {
    let r = apply(fn (x: i32) -> i32 { return x + 1; }, 5);
    Logger.new("t").info(f"r={r}");
}
"#;
    let (lines, trap, _) = run_case(src, 100_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["r=6"]);
}

#[test]
fn a1_zero_value_field_defaults() {
    let src = r#"
dataclass P3 { x: i32; s: str; }
pub fn main() -> unit {
    let p = P3 { };
    Logger.new("t").info(f"x={p.x} s=[{p.s}]");
}
"#;
    let (lines, trap, _) = run_case(src, 100_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["x=0 s=[]"]);
}
