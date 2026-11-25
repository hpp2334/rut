//! The compiled-dependency lane at std scale: every package of the
//! template lock (`crates/rut-cli/template/rut.lock`) — `calc flow
//! futures http ink json nmapset pouch strbuild` — is a compiled path
//! dependency of a consumer that exercises it the way a user does, and
//! the loaded result runs. The lane: `rut install` compiles the dep
//! tree into a scratch store, `rut run --no-cache` mounts the deps'
//! compiled binaries inside the closure archive and drives them — the
//! toolchain reads what it writes, for the whole swappable set. The
//! futures row is this file's phase-1 pin (the book's
//! `launch_future(async { .. })` shape), grown to the package's full
//! launcher surface.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-cli-compileddep-{tag}-{}", std::process::id()));
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

fn rut() -> &'static str {
    env!("CARGO_BIN_EXE_rut")
}

/// `rut <args>` with a fresh scratch rut home — the run cache and the
/// dep store both live under it; nothing ever touches `~/.rut`.
fn rut_run(home: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(rut())
        .env("RUT_HOME", home)
        .args(args)
        .output()
        .expect("run the rut binary");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The repo's `rut/<pkg>` sources, as seen from this test's crate.
fn repo_pkg(pkg: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../rut")
        .join(pkg)
        .to_string_lossy()
        .into_owned()
}

/// The dependency lane, end to end: a consumer app whose manifest
/// declares `deps` as path rows on the repo's `rut/<pkg>` sources,
/// `rut install`ed into a scratch store (the deps compile there), then
/// `rut run --no-cache` (the compiled dep binaries mount inside the
/// closure archive and drive).
fn dep_lane_run(tag: &str, deps: &[&str], src: &str) -> (i32, String, String) {
    let root = scratch(tag);
    let dir = root.join("app");
    let rows: Vec<String> = deps
        .iter()
        .map(|d| format!(r#""{d}": {{"path": "{}"}}"#, repo_pkg(d)))
        .collect();
    write(
        &dir,
        "rut.jsonc",
        &format!(r#"{{"name": "app", "deps": {{{}}}}}"#, rows.join(", ")),
    );
    write(&dir, "src/mod.rut", src);
    let home = root.join("home");
    let (code, _out, err) = rut_run(&home, &["install", dir.to_str().unwrap()]);
    assert_eq!(code, 0, "install: {err}");
    let res = rut_run(&home, &["run", "--no-cache", dir.to_str().unwrap()]);
    let _ = std::fs::remove_dir_all(&root);
    res
}

/// The consumer's stdout equals these lines, exit 0 — the logger
/// prints each `info` raw, so the answer rides stdout verbatim.
fn expect_lines(tag: &str, deps: &[&str], src: &str, want: &[&str]) {
    let (code, out, err) = dep_lane_run(tag, deps, src);
    assert_eq!(code, 0, "run: {err}");
    let got: Vec<&str> = out.lines().collect();
    assert_eq!(got, want, "the consumer's answer rides stdout");
}

// ---------------------------------------------------------------------
// calc — the float math lib: the surface lives one module down
// (`calc::math`), every member a closed-union generic whose width
// falls out of the argument (`docs/src/reference/stdlib.md`, the calc
// page). The consumer calls the book's shape and reads the SAME
// generic (`math::sqrt`) at two DISTINCT type arguments — the f64 row
// and the f32 row — plus the binary/rounding members.
// ---------------------------------------------------------------------

const CALC_CONSUMER: &str = r#"
use calc::{ math };
use ink::{ Logger };

entry fn main() {
    let log = Logger("t");
    let x = 3.0f64;
    let y = 4.0f64;
    let d = math::sqrt(x * x + y * y);
    let a = math::abs(x);
    let hf = math::sqrt(2.0);
    log.info(f"{d} {a} {hf}");
    log.info(f"{math::min(1.0f64, 2.0f64)} {math::max(1.0f64, 2.0f64)} {math::pow(2.0f64, 10.0f64)}");
    log.info(f"{math::floor(2.7f64)} {math::ceil(2.1f64)} {math::round(2.5f64)}");
}
"#;

#[test]
fn a_calc_consumer_runs_through_the_dependency_lane() {
    expect_lines(
        "calc",
        &["calc", "ink"],
        CALC_CONSUMER,
        &["5 3 1.4142135", "1 2 1024", "2 3 3"],
    );
}

// ---------------------------------------------------------------------
// flow — the push pipeline (`docs/src/reference/stdlib.md`, the flow
// page): the spelled adapters at every entry, `map` at two DISTINCT
// type arguments (R = i32 and R = str), `fold` likewise, the
// `VecFlow::from_flow` + `collected()` and `SetFlow::from_flow` +
// `items()` sinks, a chain feeding `for..of`, and stop through `take`.
// ---------------------------------------------------------------------

const FLOW_CONSUMER: &str = r#"
use pouch::{ Vec };
use flow::{ Flow, IntoFlow, FromFlow, VecFlow, SetFlow, ArrFlow, StrFlow };
use nmapset::{ HashSet };
use ink::{ Logger };

entry fn main() {
    let log = Logger("flow");
    let nums: Vec<i32> = Vec::new();
    nums.push(1);
    nums.push(2);
    nums.push(3);
    nums.push(4);
    let src: VecFlow<i32> = VecFlow(nums);
    let picked: Flow<i32> = src.into_flow()
        .map(fn(x: i32) -> i32 { return x * 10; })
        .filter(fn(x: i32) -> bool { return x > 15; });
    log.info(f"picked={picked.count()}");
    let words: Vec<str> = Vec::from(["ab", "cde"]);
    let lens: Flow<i32> = VecFlow(words).into_flow()
        .map(fn(s: str) -> i32 { return s.len(); });
    let total = lens.fold(0, fn(acc: i32, n: i32) -> i32 { return acc + n; });
    log.info(f"total={total}");
    let out = VecFlow<i32>::from_flow(
        ArrFlow([5, 6, 7]).into_flow()
            .map(fn(x: i32) -> i32 { return x + 1; })
            .take(2),
    );
    let got: Vec<i32> = out.collected();
    log.info(f"got={got[0]} {got[1]} {got.len()}");
    let joined = StrFlow("hey").into_flow().fold("", fn(acc: str, c: str) -> str { return f"{acc}{c}"; });
    log.info(f"joined={joined}");
    let names: Flow<str> = VecFlow(words).into_flow().map(fn(s: str) -> str { return f"<{s}>"; });
    for (let w of names) {
        log.info(f"w={w}");
    }
    let set: SetFlow<i32> = SetFlow<i32>::from_flow(picked);
    let items: HashSet<i32> = set.items();
    log.info(f"setlen={items.len()}");
}
"#;

#[test]
fn a_flow_consumer_runs_through_the_dependency_lane() {
    expect_lines(
        "flow",
        &["flow", "pouch", "nmapset", "ink"],
        FLOW_CONSUMER,
        &[
            "picked=3",
            "total=5",
            "got=6 7 2",
            "joined=hey",
            "w=<ab>",
            "w=<cde>",
            "setlen=3",
        ],
    );
}

// ---------------------------------------------------------------------
// futures — the launcher surface (`docs/src/reference/
// launched-futures.md`). The row is phase 1's pin (the book's
// `launch_future(async { .. })` shape), grown to the book's completer
// tier and a `select2` race at two DISTINCT type arguments (T = nil
// from the sleep, U = str from the producer): the call mints,
// `launch_future` drives it, the fast side wins, the loser is
// cancelled.
//
// Recorded beside this row (probed before the row was written): BINDING
// the race's answer is the shape the engine cannot serve yet —
//
//   trap: Invalid — no impl for interface slot 3 on #frame@select2<Either2<nil, str>> — `is` would have said false
//
// (exit 1; T = U = nil traps identically, `Either2<nil, nil>` — the
// phase-1 review's finding, both revisions). The row exercises the
// book's own green shapes and asserts their exact output.
// ---------------------------------------------------------------------

const FUTURES_CONSUMER: &str = r#"
use core::{ Future };
use ink::{ Logger };
use futures::{ launch_future, sleep, select2, completer, Completer };

struct P { a: i32 }

async fn fut() -> P {
    return P { a: 7 };
}

// the completer tier: a PLAIN fn returning a Future — the mint plus
// the resolution right, settled from a second frame
fn fetch_like(tag: str) -> Future<str> {
    let (f, done) = completer<str>();
    launch_future(settle_later(done, tag));
    return f;
}

async fn settle_later(done: Completer<str>, tag: str) -> nil {
    await sleep(10);
    done.resolve(tag);
}

// the race's other side — a Future<str>, so select2 mints at
// T = nil, U = str (two distinct type arguments)
async fn word() -> str {
    return "data";
}

entry fn main() {
    let log = Logger("t");
    launch_future(async {
        let p = await fut();
        let x = p.a;
        log.info(f"{x}");
        let v = await fetch_like("tag!");
        log.info(v);
        launch_future(select2(sleep(1), word()));
        log.info("raced");
    });
}
"#;

#[test]
fn a_futures_consumer_runs_through_the_dependency_lane() {
    expect_lines(
        "futures",
        &["futures", "ink"],
        FUTURES_CONSUMER,
        &["7", "tag!", "raced"],
    );
}

// ---------------------------------------------------------------------
// http — the std HTTP lane's rut face. The package embeds a host (the
// run chain installs `rut_std::http::pkg()`, the real reqwest lane),
// so the consumer is the embedding shape the package's own tests use
// (`crates/rut-std/tests/http.rs`,
// `a_worker_thread_streams_through_the_reqwest_lane`): a local TCP
// server in the test, the client's async chain driven by the CLI's
// driving loop, `send` resolving at headers, the drain taking the body.
// ---------------------------------------------------------------------

const HTTP_CONSUMER_TEMPLATE: &str = r#"
use http::{ HttpClient };
use futures::launch_future;
use ink::{ Logger };

async fn probe(log: Logger, url: str) -> nil {
    let client = HttpClient();
    let resp = await client.get(url).build().send();
    log.info(f"status={resp.status()}");
    let b = await resp.body();
    log.info(f"body={b.len()}");
    log.info(f"text={b.decode()}");
}

entry fn main() {
    let log = Logger("t");
    launch_future(probe(log, "http://127.0.0.1:@PORT@/hello"));
}
"#;

/// One request, one fixed response: the headers land with the whole
/// 9-octet body, then the connection closes — the drain reads the
/// buffered octets past the close.
fn serve_one_hello() -> (u16, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let handle = std::thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut buf = [0u8; 4096];
        let _ = sock.read(&mut buf);
        let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\nhello rut");
        let _ = sock.flush();
    });
    (port, handle)
}

#[test]
fn a_http_consumer_runs_through_the_dependency_lane() {
    let (port, server) = serve_one_hello();
    let src = HTTP_CONSUMER_TEMPLATE.replace("@PORT@", &port.to_string());
    expect_lines(
        "http",
        &["http", "futures", "ink"],
        &src,
        &["status=200", "body=9", "text=hello rut"],
    );
    server.join().expect("the server thread");
}

// ---------------------------------------------------------------------
// ink — the logger (`docs/src/reference/stdlib.md`, the ink page): the
// `Logger` class over `ink_host`, every level. There is no global
// output builtin — the logger IS the package's whole surface; it has
// neither a generic nor an async member to spell.
// ---------------------------------------------------------------------

const INK_CONSUMER: &str = r#"
use ink::{ Logger };

entry fn main() {
    let log = Logger("app");
    log.info("started");
    log.debug("detail");
    log.warn("watch");
    log.error("boom");
    log.log("via-log");
}
"#;

#[test]
fn a_ink_consumer_runs_through_the_dependency_lane() {
    expect_lines(
        "ink",
        &["ink"],
        INK_CONSUMER,
        &["started", "detail", "watch", "boom", "via-log"],
    );
}

// ---------------------------------------------------------------------
// json — the serde package (`docs/src/reference/stdlib.md`, the json
// page): the generic entries `encodeJson` / `decodeJson` at two
// DISTINCT type arguments each (the `JsonI64` and `JsonStr` wrappers —
// the wrapper law: the spelled, per-call manufacture), plus the pair
// law's failure arms on a broken lexeme and an out-of-range integer.
//
// Recorded beside this row (probed before the row was written): a
// CONSUMER type cannot ride the pkg's templates through this lane —
//
//   error: `json@36c839daa1a52dc7`'s template calls `Row.decode`, but no surface carries that member — corrupt or doctored, re-pack the directory
//
// (exit 1 — the owner unit cannot see the consumer's `Row`; the book's
// 06-github-viewer example spells the remedy: the user type rides a
// LOCAL generic fn in the consumer's own unit). That lane's container
// wrappers over foreign element types verify-refuse on a load below —
// `verify: function with_capacity (#16): op @1: arrnew repr does not
// match the element type` — identically on this phase's base revision
// (7c7bf592), so it is not this plan's.
// ---------------------------------------------------------------------

const JSON_CONSUMER: &str = r#"
use json::{ JsonI64, JsonStr, encodeJson, decodeJson };
use ink::{ Logger };

entry fn main() {
    let log = Logger("t");
    let (num, en) = decodeJson<JsonI64>("64");
    if (en == nil) {
        let iv: JsonI64 = num;
        log.info(f"i={iv.get()}");
    }
    let (word, es) = decodeJson<JsonStr>("\"hi\"");
    if (es == nil) {
        let sv: JsonStr = word;
        log.info(f"s={sv.get()}");
    }
    let (j, ej) = encodeJson(JsonI64(7));
    if (ej == nil) {
        let t: str = j;
        log.info(f"enc={t}");
    }
    let (j2, ej2) = encodeJson(JsonStr("out"));
    if (ej2 == nil) {
        let t2: str = j2;
        log.info(f"enc2={t2}");
    }
    let (bad, eb) = decodeJson<JsonI64>("{oops");
    if (eb != nil) {
        log.info("bad rejected");
    }
    let (over, eo) = decodeJson<JsonI64>("99999999999999999999");
    if (eo != nil) {
        log.info("overflow rejected");
    }
}
"#;

#[test]
fn a_json_consumer_runs_through_the_dependency_lane() {
    expect_lines(
        "json",
        &["json", "ink"],
        JSON_CONSUMER,
        &[
            "i=64",
            "s=hi",
            "enc=7",
            "enc2=\"out\"",
            "bad rejected",
            "overflow rejected",
        ],
    );
}

// ---------------------------------------------------------------------
// nmapset — the native-key map/set (`docs/src/reference/stdlib.md`,
// the nmapset page): the generic classes at three distinct
// instantiations (`HashMap<str, i32>`, `HashMap<i32, str>`,
// `HashSet<i64>`), put's insert-or-replace answer, the `get -> ?V`
// nil-check law, has/remove/len, and `for..of` walking the keys in
// birth order.
// ---------------------------------------------------------------------

const NMAPSET_CONSUMER: &str = r#"
use nmapset::{ HashMap, HashSet };
use ink::{ Logger };

entry fn main() {
    let log = Logger("t");
    let m: HashMap<str, i32> = HashMap();
    let first = m.put("a", 1);
    m.put("b", 2);
    let second = m.put("a", 3);
    let g = m.get("a");
    if (g != nil) {
        let n: i32 = g;
        log.info(f"a={n} new1={first} new2={second}");
    }
    log.info(f"hasc={m.has("c")} len={m.len()}");
    let gone = m.remove("b");
    log.info(f"gone={gone} len={m.len()}");
    for (let k of m) {
        log.info(f"key={k}");
    }
    let s: HashSet<i64> = HashSet();
    let s1 = s.put(7);
    s.put(8);
    let s2 = s.put(7);
    log.info(f"setlen={s.len()} s1={s1} s2={s2} has={s.has(8)}");
    let mi: HashMap<i32, str> = HashMap();
    mi.put(1, "one");
    mi.put(2, "two");
    let gv = mi.get(2);
    if (gv != nil) {
        let t: str = gv;
        log.info(f"2={t} milen={mi.len()}");
    }
}
"#;

#[test]
fn a_nmapset_consumer_runs_through_the_dependency_lane() {
    expect_lines(
        "nmapset",
        &["nmapset", "ink"],
        NMAPSET_CONSUMER,
        &[
            "a=3 new1=true new2=false",
            "hasc=false len=2",
            "gone=true len=1",
            "key=a",
            "setlen=2 s1=true s2=false has=true",
            "2=two milen=2",
        ],
    );
}

// ---------------------------------------------------------------------
// pouch — the growable sequence (`docs/src/reference/stdlib.md`, the
// pouch page): the generic class at four distinct instantiations
// (`Vec<i32>`, `Vec<str>`, `Vec<u8>`, `Vec<i64>`), push/pop/index/
// from/filled/with_capacity/as_array, the fused `for..of`, and the
// module-level `freeze_bytes` bridge.
// ---------------------------------------------------------------------

const POUCH_CONSUMER: &str = r#"
use pouch::{ Vec, freeze_bytes };
use ink::{ Logger };

entry fn main() {
    let log = Logger("t");
    let v: Vec<i32> = Vec::new();
    v.push(1);
    v.push(2);
    v.push(3);
    let w: Vec<str> = Vec::from(["a", "b"]);
    w.push("c");
    let last = v.pop();
    log.info(f"{v.len()} {last} {v[1]} {w[2]}");
    let total: i32 = 0;
    for (let x of v) {
        total += x;
    }
    log.info(f"sum={total}");
    let bs: Vec<u8> = Vec::filled(65u8, 3);
    bs.push(66u8);
    let b = freeze_bytes(bs);
    log.info(f"{b.len()} {b.decode()}");
    let fixed = v.as_array();
    log.info(f"arr={fixed.len()}");
    let big: Vec<i64> = Vec::with_capacity(8);
    big.push(9000000000i64);
    log.info(f"big={big[0]}");
}
"#;

#[test]
fn a_pouch_consumer_runs_through_the_dependency_lane() {
    expect_lines(
        "pouch",
        &["pouch", "ink"],
        POUCH_CONSUMER,
        &["2 3 2 c", "sum=3", "4 AAAB", "arr=2", "big=9000000000"],
    );
}

// ---------------------------------------------------------------------
// strbuild — the builder (`docs/src/reference/stdlib.md`, the
// strbuild page): the book's exact program, the codepoint append, the
// len law, and the finish-keeps-buffer law (the built str is immune to
// later appends; the builder keeps its buffer).
// ---------------------------------------------------------------------

const STRBUILD_CONSUMER: &str = r#"
use ink::{ Logger };
use strbuild::{ StringBuilder };

entry fn main() {
    let log = Logger("t");
    let k = "name";
    let b = StringBuilder::with_cap(1024);
    b.append(f"{k}=");
    b.append_code(0x21);
    let s = b.build();
    log.info(s);
    log.info(f"len={b.len()}");
    b.append("MORE");
    log.info(f"immune={s} kept={b.build()}");
    let b2 = StringBuilder();
    b2.append("again");
    b2.append_code(0x3E9);
    log.info(b2.build());
    log.info(f"len2={b2.len()}");
}
"#;

#[test]
fn a_strbuild_consumer_runs_through_the_dependency_lane() {
    expect_lines(
        "strbuild",
        &["strbuild", "ink"],
        STRBUILD_CONSUMER,
        &[
            "name=!",
            "len=6",
            "immune=name=! kept=name=!MORE",
            "againϩ",
            "len2=6",
        ],
    );
}
