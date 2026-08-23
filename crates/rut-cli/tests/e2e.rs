//! End-to-end — the demo page's 8 cases (RFC 0041 §3 / demo/src/cases.ts)
//! compiled AND executed through the full pipeline: parse → check → LIR →
//! binary → decode → verify → interpret with budgets.

use std::cell::RefCell;
use std::rc::Rc;

fn run_case(src: &str, fuel: u64) -> (Vec<String>, Option<String>, u64) {
    let out = rutc::compile_module(src, rutc::Mode::Impl, "main");
    assert!(
        out.diags.is_empty(),
        "unexpected diags:\n{}",
        rutc::diag::render_diags(src, &out.diags)
    );
    let binary = out.binary.expect("binary");
    let prog = rut_core::binary::decode(&binary).expect("decode");
    rut_core::verify::verify(&prog).expect("verify");
    let lines: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink = lines.clone();
    let hooks = rut_core::interp::HostHooks {
        print: Some(Rc::new(RefCell::new(move |s: &str| {
            sink.borrow_mut().push(s.to_string());
        }))),
    };
    let limits = rut_core::interp::Limits {
        fuel: Some(fuel),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_core::interp::Vm::new(Rc::new(prog), &limits, hooks).expect("vm");
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
fn describe(f: Flavor): string {
    return when (f) {
        Flavor.Sweet -> "sweet",
        Flavor.Sour  -> "sour",
    };
}
export fn main(): void {
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
export fn main(): void {
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
export fn main(): void {
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
fn sieve(limit: i32): Vec<i32> {
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
export fn main(): void {
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
fn mix(a: Color, b: Color): string {
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
export fn main(): void {
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
    fn area(self): f32;
    fn name(self): string;
}
dataclass Circle { r: f32; }
impl Shape for Circle {
    fn area(self): f32 { return 3.14159265f32 * self.r * self.r; }
    fn name(self): string { return "circle"; }
}
dataclass Square { s: f32; }
impl Shape for Square {
    fn area(self): f32 { return self.s * self.s; }
    fn name(self): string { return "square"; }
}
export fn main(): void {
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
fn map<T, U>(v: Vec<T>, f: fn(T): U): Vec<U> {
    let out: Vec<U> = Vec();
    for (let x of v) { out.push(f(x)); }
    return out;
}
export fn main(): void {
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
export fn main(): void {
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
export fn main(): void {
    let mut x = 2147483647;
    x = x &+ 1;              // wrapping: fine (RFC 0004 §3)
    print(f"x={x}");
    let y = 2147483647 + 1;  // trapping: overflow
    print(f"y={y}");
}
"#;
    let (lines, trap, _) = run_case(src, 1_000_000);
    assert_eq!(lines, vec!["x=-2147483648"]);
    assert_eq!(trap.as_deref(), Some("Overflow"));
}

#[test]
fn heap_budget_traps_before_the_write() {
    // RFC 0040 §1: OutOfMemory leaves the heap byte-identical — a tiny
    // budget fails the Vec allocation cleanly
    let src = r#"
export fn main(): void {
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
export fn main(): void {
    let p = P { x: 1 };
    p.x = 2;
}
"#;
    let out = rutc::compile_module(src, rutc::Mode::Impl, "main");
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
export fn main(): void {
    print(when (Color.Red) { Color.Red -> "r" });
}
"#;
    let out = rutc::compile_module(src, rutc::Mode::Impl, "main");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("exhaustive")),
        "{:?}",
        out.diags
    );
}

#[test]
fn option_eq_is_a_compile_error() {
    let src = r#"
export fn main(): void {
    let a = Option.some(1);
    print(f"{a == a}");
}
"#;
    let out = rutc::compile_module(src, rutc::Mode::Impl, "main");
    assert!(
        out.diags.iter().any(|d| d.msg.contains("Option")),
        "{:?}",
        out.diags
    );
}
