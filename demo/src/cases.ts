/**
 * Prepared cases (RFC 0041 §3). Each is self-contained — the host
 * provides the bundled packages (`ink`, `core`, `calc`, `rt`, `pouch`),
 * whose `info` lines stream back as output.
 *
 * `expected` is the case's SIDECAR: after every real run the page diffs
 * the engine's actual output against it (the sidecar flip, survey D2)
 * and the smoke gate does the same headlessly. Sources reflect TODAY's
 * surface — RFC 0044: bindings share by reference (copy-by-value and
 * `own` are gone; `bytes.clone()` is the one copy), `==` is identity
 * for cells, the pointer shape is the nullable `?T` (prefix-only;
 * `*T`/`&v` diagnose) — plus no `dataclass` (spell it `struct`), no
 * `char` literals (1-codepoint `str`), no arrows (RFC 0013 block
 * bodies).
 */

export interface RutCase {
  id: string;
  name: string;
  blurb: string;
  /** RFC the case demonstrates */
  rfcs: string;
  source: string;
  expected: string[];
}

export const CASES: RutCase[] = [
  {
    id: "hello-format",
    name: "hello, format",
    blurb: "f-strings, escapes, when on enums",
    rfcs: "0007 §2, 0008",
    source: [
      "use ink::{Logger};",
      "",
      "enum Flavor { Sweet, Sour }",
      "",
      "fn describe(f: Flavor) -> str {",
      "    return when (f) {",
      "        Flavor.Sweet -> \"sweet\",",
      "        Flavor.Sour  -> \"sour\",",
      "    };",
      "}",
      "",
      "pub fn main() {",
      "    let log = Logger.new(\"case\");",
      "    let name = \"rut\";",
      "    let n = 41 + 1;",
      "    log.info(f\"hi {name}! n={n} tab:\\t\");",
      "    log.info(describe(Flavor.Sour));",
      "}",
    ].join("\n"),
    expected: [
      "hi rut! n=42 tab:\t",
      "sour",
    ],
  },
  {
    id: "values-and-pointers",
    name: "values & pointers",
    blurb: "bindings share by reference — identity == for cells, absence as ?T",
    rfcs: "0044, 0016 §1, 0005 §8",
    source: [
      "use ink::{Logger};",
      "",
      "struct Point { x: f32; y: f32 }",
      "",
      "pub fn main() {",
      "    let log = Logger.new(\"case\");",
      "    let mut p = Point { x: 1, y: 2 };",
      "    let q = p;                  // share: q and p name ONE cell (RFC 0044)",
      "    p.x = 4;                    // q.x is 4 now — sharing is the law",
      "    let same = Point { x: 1, y: 2 };",
      "    log.info(f\"q.x={q.x} p.x={p.x}\");",
      "    log.info(f\"q==same {q == same}, p==same {p == same}\"); // cell identity ==",
      "    let mut rp: ?Point = p;     // the pointer shape today: the nullable box",
      "    rp.x = 9;                   // writes through the box (auto-deref) — shared",
      "    log.info(f\"rp.x={rp.x} p.x={p.x}\");",
      "    let rq: ?Point = same;",
      "    log.info(f\"rp==rp {rp == rp}, rp==rq {rp == rq}\"); // box identity",
      "}",
    ].join("\n"),
    expected: [
      "q.x=4 p.x=4",
      "q==same false, p==same false",
      "rp.x=9 p.x=9",
      "rp==rp true, rp==rq false",
    ],
  },
  {
    id: "opaque",
    name: "opaque + downcast",
    blurb: "explicit erasure with checked recovery — downcast yields ?T (nil on a miss)",
    rfcs: "0014",
    source: [
      "use ink::{Logger};",
      "",
      "struct Point { x: f32; y: f32; }",
      "",
      "pub fn main() {",
      "    let log = Logger.new(\"case\");",
      "    let box1 = opaque(Point { x: 1, y: 2 });",
      "    let box2 = opaque(\"hello\");",
      "    log.info(f\"box1 is Point: {box1 is Point}\");",
      "    log.info(f\"box2 is Point: {box2 is Point}\");",
      "    let p = opaque.downcast<Point>(box1);      // ?Point",
      "    when (p != nil) {",
      "        true -> { log.info(f\"recovered {p.x} {p.y}\"); },",
      "        else  -> { log.info(\"nil\"); },",
      "    }",
      "}",
    ].join("\n"),
    expected: [
      "box1 is Point: true",
      "box2 is Point: false",
      "recovered 1 2",
    ],
  },
  {
    id: "sieve",
    name: "sieve",
    blurb: "flat Vec<u8>/Vec<i32> primitive buffers, for loops",
    rfcs: "0016 §4",
    source: [
      "use pouch::{Vec};",
      "use ink::{Logger};",
      "",
      "fn sieve(limit: i32) -> Vec<i32> {",
      "    let mut marks = Vec<u8>.filled(0, limit + 1);   // 0 = candidate, 1 = crossed",
      "    let primes: Vec<i32> = Vec.new();",
      "    for (let i = 2; i <= limit; i += 1) {",
      "        if (marks[i] == 0) {",
      "            primes.push(i);",
      "            let mut m = i * i;",
      "            while (m <= limit) {",
      "                marks[m] = 1;",
      "                m += i;",
      "            }",
      "        }",
      "    }",
      "    return primes;",
      "}",
      "",
      "pub fn main() {",
      "    let log = Logger.new(\"case\");",
      "    let primes = sieve(100);",
      "    log.info(f\"{primes.len()} primes up to 100, last={primes[primes.len() - 1]}\");",
      "}",
    ].join("\n"),
    expected: [
      "25 primes up to 100, last=97",
    ],
  },
  {
    id: "when-exhaustive",
    name: "when & enums",
    blurb: "exhaustive pattern expressions over simple enums",
    rfcs: "0006, 0008",
    source: [
      "use ink::{Logger};",
      "",
      "enum Color { Red, Green, Blue }",
      "",
      "fn mix(a: Color, b: Color) -> str {",
      "    return when (a) {",
      "        Color.Red -> when (b) {",
      "            Color.Red   -> \"red+red\",",
      "            Color.Green -> \"yellow\",",
      "            Color.Blue  -> \"magenta\",",
      "        },",
      "        Color.Green -> \"greenish\",",
      "        Color.Blue  -> \"blueish\",",
      "    };",
      "}",
      "",
      "pub fn main() {",
      "    let log = Logger.new(\"case\");",
      "    log.info(mix(Color.Red, Color.Green));",
      "    log.info(mix(Color.Blue, Color.Blue));",
      "}",
    ].join("\n"),
    expected: [
      "yellow",
      "blueish",
    ],
  },
  {
    id: "tuple-errors",
    name: "errors as tuples",
    blurb: "the (T, err) convention — an empty err string is success",
    rfcs: "0004",
    source: [
      "use ink::{Logger};",
      "",
      "// v1.1 error convention: `(T, err)` — an empty err string is success.",
      "fn parse_u8(s: str) -> (i32, str) {",
      "    let n = str_len(s);",
      "    if (n == 0) { return (0, \"empty input\"); }",
      "    let mut v = 0;",
      "    for (let c of s) {",
      "        let d = digit(f\"{c}\");",
      "        if (d < 0) { return (0, f\"not a digit: {c}\"); }",
      "        v = v * 10 + d;",
      "        if (v > 255) { return (0, \"out of range\"); }",
      "    }",
      "    return (v, \"\");",
      "}",
      "",
      "fn str_len(s: str) -> i32 {",
      "    let mut n = 0;",
      "    for (let c of s) { n = n + 1; }",
      "    return n;",
      "}",
      "",
      "fn digit(c: str) -> i32 {",
      "    let digits = \"0123456789\";",
      "    let mut i = 0;",
      "    for (let t of digits) {",
      "        if (f\"{t}\" == c) { return i; }",
      "        i = i + 1;",
      "    }",
      "    return -1;",
      "}",
      "",
      "pub fn main() {",
      "    let log = Logger.new(\"case\");",
      "    let ok = parse_u8(\"247\");",
      "    log.info(f\"ok=({ok.0}, \\\"{ok.1}\\\")\");",
      "    let bad = parse_u8(\"9x2\");",
      "    log.info(f\"bad=({bad.0}, \\\"{bad.1}\\\")\");",
      "    let big = parse_u8(\"300\");",
      "    log.info(f\"big=({big.0}, \\\"{big.1}\\\")\");",
      "}",
    ].join("\n"),
    expected: [
      "ok=(247, \"\")",
      "bad=(0, \"not a digit: x\")",
      "big=(0, \"out of range\")",
    ],
  },
  {
    id: "closures-generics",
    name: "closures & generics",
    blurb: "monomorphized generics, anonymous fns, capture",
    rfcs: "0013",
    source: [
      "use pouch::{Vec};",
      "use ink::{Logger};",
      "",
      "fn map<T, U>(v: Vec<T>, f: fn(T) -> U) -> Vec<U> {",
      "    let out: Vec<U> = Vec.new();",
      "    for (let x of v) { out.push(f(x)); }",
      "    return out;",
      "}",
      "",
      "pub fn main() {",
      "    let log = Logger.new(\"case\");",
      "    let xs = Vec<i32>.from([1, 2, 3, 4]);",
      "    let k = 10;",
      "    let ys = map<i32, i32>(xs, fn (x: i32) -> i32 { return x * k; });  // captures k",
      "    log.info(f\"{ys[0]} {ys[1]} {ys[2]} {ys[3]}\");",
      "}",
    ].join("\n"),
    expected: [
      "10 20 30 40",
    ],
  },
  {
    id: "fuel-demo",
    name: "fuel demo (infinite loop)",
    blurb: "budgets bite: while(true) parks on Trap::OutOfFuel — Resume continues the frame",
    rfcs: "0040 §2, 0034 §4",
    source: [
      "use ink::{Logger};",
      "",
      "pub fn main() {",
      "    let log = Logger.new(\"case\");",
      "    let mut i = 0;",
      "    while (true) {",
      "        i += 1;",
      "        if (i % 1000000 == 0) {",
      "            log.info(f\"tick {i}\");",
      "        }",
      "    }",
      "}",
    ].join("\n"),
    // the HONEST expected at the pinned DEFAULT budget (10M / 4 MiB):
    // ~10 ops per iteration means zero tick lines fit — the run parks
    // immediately. The old sidecar (tick 1000000/2000000/3000000, "…",
    // a resume hint) described a ~40M-fuel magnitude and a preview-era
    // fabrication; both died (survey §2.3/D2). The trap line
    // participates: the pane renders `Trap::<name>` last. At a raised
    // budget (or after Resume, which accumulates) the chip shows a
    // diff BY DESIGN — this sidecar pins the default.
    expected: [
      "Trap::OutOfFuel",
    ],
  },
];

export const DEFAULT_BUDGET = { fuel: 10_000_000, heapBytes: 4 * 1024 * 1024 };
