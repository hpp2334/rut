//! End-to-end — the demo page's 8 cases (RFC 0041 §3 / demo/src/cases.ts)
//! compiled AND executed through the full pipeline: parse → check → LIR →
//! binary → decode → verify → interpret with budgets.

use std::cell::RefCell;
use std::rc::Rc;

fn run_case(src: &str, fuel: u64) -> (Vec<String>, Option<String>, u64) {
    let out = rut_driver::compile_module(src, rut_parser::Mode::Impl, "main");
    assert!(
        out.diags.is_empty(),
        "unexpected diags:\n{}",
        rut_lexer::diag::render_diags(src, &out.diags)
    );
    let binary = out.binary.expect("binary");
    let prog = rut_core::binary::decode(&binary).expect("decode");
    rut_vm::verify::verify(&prog).expect("verify");
    let lines: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = lines.clone();
    let hooks = rut_vm::interp::HostHooks {
        print: Some(Rc::new(RefCell::new(move |s: &str| {
            sink.borrow_mut().push(s.to_string());
        }))),
    };
    let limits = rut_vm::interp::Limits {
        fuel: Some(fuel),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, hooks).expect("vm");
    let trap = match vm.call("main", &[]) {
        Ok(_) => None,
        Err(t) => Some(t.name()),
    };
    let lines = lines.borrow().clone();
    (lines, trap, vm.fuel_used)
}

#[test]
fn case1_hello_format() {
    let src = r#"
enum Flavor { Sweet, Sour }
fn describe(f: Flavor) -> string {
    return when (f) {
        Flavor.Sweet -> "sweet",
        Flavor.Sour  -> "sour",
    };
}
pub fn main() -> unit {
    let name = "rut";
    let n = 41 + 1;
    print(f"hi {name}! n={n} tab:\t'c'={'c'}");
    print(describe(Flavor.Sour));
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
    print(f"q.x={q.x} p.x={p.x} r.x={r.x}");
    print(f"q==p {q == p}, r==p {r == p}");
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
    print(f"box1 is Point: {box1 is Point}");
    print(f"box2 is Point: {box2 is Point}");
    let p = downcast<Point>(box1);
    if (p.is_some()) {
        print(f"recovered {p.value.x} {p.value.y}");
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
fn sieve(limit: i32) -> Vec<i32> {
    let mut marks = Vec<u8>(limit + 1);
    let primes: Vec<i32> = Vec();
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
    print(f"{primes.len()} primes up to 100, last={primes[primes.len() - 1]}");
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
fn mix(a: Color, b: Color) -> string {
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
    print(mix(Color.Red, Color.Green));
    print(mix(Color.Blue, Color.Blue));
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["yellow", "blueish"]);
}

#[test]
fn case6_dyn_dispatch() {
    let src = r#"
trait Shape {
    fn area(self) -> f32;
    fn name(self) -> string;
}
dataclass Circle { r: f32; }
impl Shape for Circle {
    fn area(self) -> f32 { return 3.14159265f32 * self.r * self.r; }
    fn name(self) -> string { return "circle"; }
}
dataclass Square { s: f32; }
impl Shape for Square {
    fn area(self) -> f32 { return self.s * self.s; }
    fn name(self) -> string { return "square"; }
}
pub fn main() -> unit {
    let shapes: Vec<dyn Shape> = Vec.from([
        Circle { r: 1 },
        Square { s: 2 },
    ]);
    for (let s of shapes) {
        print(f"{s.name()} area={s.area()}");
    }
    let c = Circle { r: 1 };
    print(f"c is Shape: {c is Shape}");
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
fn map<T, U>(v: Vec<T>, f: fn(T) -> U) -> Vec<U> {
    let out: Vec<U> = Vec();
    for (let x of v) { out.push(f(x)); }
    return out;
}
pub fn main() -> unit {
    let xs = Vec.from([1, 2, 3, 4]);
    let k = 10;
    let ys = map<i32, i32>(xs, (x) => x * k);
    print(f"{ys[0]} {ys[1]} {ys[2]} {ys[3]}");
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
            print(f"tick {i}");
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
pub fn main() -> unit {
    let mut x = 2147483647;
    x = x &+ 1;              // wrapping: fine (RFC 0004 §3)
    print(f"x={x}");
    let mut s = 1073741824;  // 1 << 30
    s = s &<< 1;             // wrapping shl: bits shifted out are gone
    print(f"s={s}");
    s &<<= 1;                // compound form, same law
    print(f"s={s}");
    let u: u32 = 3221225472; // 0xC000_0000
    print(f"u={u &<< 1}");
    let y = 2147483647 + 1;  // trapping: overflow
    print(f"y={y}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(lines, vec!["x=-2147483648", "s=-2147483648", "s=0", "u=2147483648"]);
    assert_eq!(trap.as_deref(), Some("Overflow"));
}

#[test]
fn plain_shl_traps_when_bits_leave_the_width() {
    // RFC 0004 §3: `<<` traps; only `&<<` wraps. This was silently a
    // RIGHT shift before BitOp::WrapShl existed.
    let src = r#"
pub fn main() -> unit {
    print("before");
    let s = 1073741824 << 1;   // 1 << 31 does not fit i32
    print(f"after {s}");
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
    let u: u64 = 0xFFFFFFFFFFFFFFFF;
    print(f"u={u >> 1}");
    let v: u32 = 0x80000000;
    print(f"v={v >> 31}");
    let s = i64(-8);
    print(f"s={s >> 1}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(lines, vec!["u=9223372036854775807", "v=1", "s=-4"]);
    assert_eq!(trap, None);
}

#[test]
fn heap_budget_traps_before_the_write() {
    // RFC 0040 §1: OutOfMemory leaves the heap byte-identical — a tiny
    // budget fails the Vec allocation cleanly
    let src = r#"
pub fn main() -> unit {
    let v = Vec<u8>(1000000);
    print("allocated");
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
    let out = rut_driver::compile_module(src, rut_parser::Mode::Impl, "main");
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
    print(when (Color.Red) { Color.Red -> "r" });
}
"#;
    let out = rut_driver::compile_module(src, rut_parser::Mode::Impl, "main");
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
    print(f"{a == a}");
}
"#;
    let out = rut_driver::compile_module(src, rut_parser::Mode::Impl, "main");
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
pub fn main() -> unit { print(f"{1 + 1}"); }
"#;
    let out = rut_driver::compile_module(src, rut_parser::Mode::Impl, "main");
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
fn classify(n: i32) -> string {
    if (n == 0) {
        return "zero";
    } else if (n < 0) {
        return "negative";
    } else {
        return "positive";
    }
}
pub fn main() -> unit {
    print(classify(0));
    print(classify(-3));
    print(classify(7));
    let mut late = 0;
    for (let i = 0; i < 4; i += 1) {
        if (i % 2 == 0) { late += 1; } else { late += 10; }
    }
    print(f"late={late}");
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
    print(f"and: {t && t} {t && f} {f && t} {f && f}");
    print(f"or:  {t || t} {t || f} {f || t} {f || f}");
    let n = 6;
    if (n > 0 && n % 2 == 0) { print("even positive"); }
    if (n < 0 || n % 3 == 0) { print("div by 3 or negative"); }
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
    print(f"count={c.count()}");
    let mut w = Wrapped.new();
    w.bump();
    print(f"wrapped={w.count()}");
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
    print(f"dropped={dropped} kept={kept}");
    when (Light.Red) {
        Light.Green  -> { print("go"); },
        Light.Yellow -> { print("brake"); },
        Light.Red    -> { print("stop"); },
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
fn describe(f: Flavor) -> string {
    return when (f) {
        Flavor.Sweet  -> "sweet",
        Flavor.Sour   -> "sour",
        Flavor.Salty  -> "salty",
    };
}
pub fn main() -> unit {
    let hits = [describe(Flavor.Sweet), describe(Flavor.Sour), describe(Flavor.Salty)];
    print(hits[0]);
    print(hits[1]);
    print(hits[2]);
    for (let i = 0; i < 50; i += 1) {
        print(describe(when (i % 2) { 0 -> Flavor.Sweet, else -> Flavor.Salty }));
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
    let out = rut_driver::compile_module(src, rut_parser::Mode::Impl, "m");
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
    rut_vm::interp::Vm::new(std::rc::Rc::new(prog), &limits, rut_vm::interp::HostHooks { print: None }).expect("vm")
}

#[test]
fn entry_fns_compile_without_main_and_cross_values() {
    // a module with entries and NO main compiles (entries are roots) and
    // the host drives it: Opaque container in/out, primitives, Vec<u8>,
    // Option and Result in both arms
    let src = r#"
dataclass Row { id: i32; }
dataclass Box { rows: Vec<Row>; }

entry fn make() -> Opaque { return Opaque.new(Box { rows: Vec() }); }
entry fn put(c: Opaque) -> u32 {
    let b = downcast<Box>(c).value;
    b.rows.push(Row { id: 1 });
    return u32(b.rows.len());
}
entry fn echo_bytes(v: Vec<u8>) -> Vec<u8> { return v; }
entry fn maybe(v: i32) -> Option<i32> {
    return when (v > 0) { true -> Option.some(v), else -> Option.none() };
}
entry fn checked(v: i32) -> Result<i32, string> {
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
    let out = rut_driver::compile_module(src, rut_parser::Mode::Impl, "m");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("parameter `r` is `Row`")),
        "{:?}",
        out.diags
    );

    let src = r#"
dataclass Row { id: i32; }
entry fn bad_ret() -> Vec<Row> { return Vec(); }
"#;
    let out = rut_driver::compile_module(src, rut_parser::Mode::Impl, "m");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("returns `Vec<Row>`")),
        "{:?}",
        out.diags
    );

    let src = r#"
entry fn generic<T>(v: T) -> T { return v; }
"#;
    let out = rut_driver::compile_module(src, rut_parser::Mode::Impl, "m");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("cannot be generic")),
        "{:?}",
        out.diags
    );
}

#[test]
fn plain_pub_stays_unrestricted() {
    // `pub` is import-visibility for rut modules (RFC 0003 §2), NOT
    // the host surface: a Stack crosses fine between rut fns
    let src = r#"
dataclass Row { id: i32; }
pub class Stack {
    items: Vec<Row>;                // unannotated member = module-private
    fn new() -> Self { return Self { items: Vec() }; }
    fn push(mut self, id: i32) -> unit { self.items.push(Row { id: id }); }
    pub fn len(self) -> i32 { return self.items.len(); }
}
pub fn drain(s: Stack) -> i32 { return s.len(); }
pub fn main() -> unit {
    let mut st = Stack.new();
    st.push(1);
    print(f"drained {drain(st)}");
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
    pub(mod) fn bump(mut self) -> unit { self.n &+= self.w; }
    pub(self) fn raw(self) -> i32 { return self.n; }
    fn secret(self) -> i32 { return self.n * 100; }
}
pub fn main() -> unit {
    let mut g = Gauge.new();
    g.bump();
    g.bump();
    print(f"gauge={g.raw()} secret={g.secret()} w={g.w}");
}
"#;
    let out = rut_driver::compile_module(src, rut_parser::Mode::Impl, "main");
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    // the dump labels member visibility (annotated members only)
    assert!(out.ast_dump.contains("vis: pub(mod)"), "member vis label: {}", out.ast_dump);
    assert!(!out.ast_dump.contains("vis: pub\n      name: n"), "unannotated stays quiet: {}", out.ast_dump);
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(trap, None);
    assert_eq!(lines, vec!["gauge=6 secret=600 w=3"]);
}
