//! 06-github-viewer-cli — the offline session gate (the async brain
//! over the REDESIGNED rut/http lane).
//!
//! ZERO network: the HTTP lane rides `http::pkg_with` (the
//! fixture lane) keyed on method+URL — the fixture map's keys ARE the
//! assertions (a wrong URL is a LOUD transport failure, never a
//! pass-through), and the recorded reply's CHUNK PLAN arrives on the
//! virtual clock: the send at tick 0, chunk i at tick i+1, the
//! terminal (EOF or the mid-read failure) one tick later — the
//! deterministic small chunks the streaming cases assert on. The
//! recorded payloads:
//!
//! - `tree.json` — data.jsdelivr.com's tree for jquery/jquery@3.7.1,
//!   verbatim (351 entries, 4 depths, base64 integrity hashes,
//!   directories without size/hash, a real 0-byte file);
//! - `tree-boundaries.json` — the same API shape, hand-built to seat
//!   the human-size boundaries (0/1023/1024/1048576/1048577/1 GiB)
//!   and a hash-less file;
//! - `text.txt` / `binary.jpg` — cdn.jsdelivr.net file bodies (the
//!   jpg is byte-verbatim; the streamed download must be binary-safe);
//! - `notfound.json` — a real 404 body.
//!
//! The `list` expectations over the real tree are derived Rust-side
//! from the same fixture (the 02-digest oracle pattern: the reference
//! walks the JSON independently), while the boundary tree pins the
//! format EXACTLY, string for string. The download cases ride the
//! streamed lane (chunk-per-await, append per chunk) and the
//! one-shot/sticky/degrade laws get their own probe. A live smoke
//! runs behind RGH_LIVE=1 only (the reqwest lane, real DNS).

use std::cell::RefCell;
use std::rc::Rc;

use rut_std::http::{ FixtureReply, HttpFixture };
use rut_vm::interp::Vm;
use rut_vm::Trap;

const SRC: &str = include_str!("../rgh.rut");
const TREE_JSON: &str = include_str!("fixtures/tree.json");
const BOUNDARY_JSON: &str = include_str!("fixtures/tree-boundaries.json");
const TEXT_BODY: &[u8] = include_bytes!("fixtures/text.txt");
const BINARY_BODY: &[u8] = include_bytes!("fixtures/binary.jpg");
const NOTFOUND_BODY: &[u8] = include_bytes!("fixtures/notfound.json");

// the exact URLs the brain must build (the fixture keys)
const LIST_URL: &str = "https://data.jsdelivr.com/v1/packages/gh/jquery/jquery@3.7.1";
const LIST_URL_404: &str = "https://data.jsdelivr.com/v1/packages/gh/jquery/jquery@no-such-ref-xyz";
const BOUNDARY_URL: &str = "https://data.jsdelivr.com/v1/packages/gh/acme/widgets@1.0.0";
const DL_TEXT_URL: &str = "https://cdn.jsdelivr.net/gh/jquery/jquery@3.7.1/test/data/text.txt";
const DL_BINARY_URL: &str = "https://cdn.jsdelivr.net/gh/jquery/jquery@3.7.1/test/data/1x1.jpg";
const DL_404_URL: &str = "https://cdn.jsdelivr.net/gh/jquery/jquery@3.7.1/no/such/file.txt";

// ------------------------------------------------------------- sinks ---

#[derive(Clone, Default)]
struct Sinks {
    /// every `out` line, in order
    out: Rc<RefCell<Vec<String>>>,
    /// every `eprint` line, in order
    err: Rc<RefCell<Vec<String>>>,
    /// every successful file row: ("W"/"A" for write/append, dest, bytes)
    files: Rc<RefCell<Vec<(char, String, Vec<u8>)>>>,
    /// when set, a write/append to this dest answers this io error text
    fail_write: Rc<RefCell<Option<(String, String)>>>,
    /// the `exit` row's code (the recording lane's one-way door)
    code: Rc<RefCell<Option<i32>>>,
    /// whether `exit` fired at all
    exited: Rc<RefCell<bool>>,
}

impl Sinks {
    fn out_lines(&self) -> Vec<String> {
        self.out.borrow().clone()
    }
    fn err_lines(&self) -> Vec<String> {
        self.err.borrow().clone()
    }
    fn code(&self) -> i32 {
        self.code.borrow().expect("the brain exited")
    }
    fn file_bytes(&self, dest: &str) -> Vec<u8> {
        // the write (truncate) + the appends, in order — the file's content
        let mut b = Vec::new();
        for (kind, d, data) in self.files.borrow().iter() {
            if d == dest {
                if *kind == 'W' {
                    b.clear();
                }
                b.extend_from_slice(data);
            }
        }
        b
    }
}

// -------------------------------------------------------------- boot ---

/// Boot the brain over the fixture lane. Mounts what the embedder
/// mounts (std + the brain's libs + the example's own host pkg), runs
/// the peer gate, compiles `rgh.rut` in Impl mode, verifies, binds
/// math + nmap + the async engine + the fixture HTTP lane + the
/// rgh_host rows over the sinks (exit RECORDS — the one-way door is
/// the embedder's), and checks the decl ↔ bodies contract pre-boot
fn boot(fix: impl Fn(&str, &str, &str, &[u8]) -> Result<FixtureReply, String> + 'static) -> (Vm, Sinks, HttpFixture) {
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std(&mut s);
    // the async pair (the launcher set `boot` drives — the embedder's
    // own mount list carries it too)
    rut_driver::mount_std_async(&mut s);
    let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut");
    for d in ["pouch", "nmapset", "json", "http_host", "http"] {
        rut_driver::mount_dir(&mut s, &tree.join(d)).expect("mount tree pkg");
    }
    rut_driver::mount_dir(&mut s, &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rgh_host"))
        .expect("mount rgh_host");
    rut_driver::assemble_peers(&mut s).expect("assemble peer groups");

    let out = rut_driver::compile_module_in(&mut s, SRC, rut_parser::Mode::Impl, "rgh");
    assert!(
        out.diags.is_empty(),
        "{}",
        out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let prog = rut_core::binary::decode(out.binary.as_deref().unwrap()).unwrap();
    rut_vm::verify::verify(&prog).unwrap();

    let limits = rut_vm::interp::Limits {
        fuel: Some(250_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let sinks = Sinks::default();
    let ctx = s.host_pkg_context();
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::math::pkg());
    hosts.install_host_pkg(&ctx, rut_std::nmap::pkg());
    hosts.install_host_pkg(&ctx, rut_std::async_host::pkg());
    // json's writer rides the strbuild pkg — the `strbuild_host` rows are
    // in this closure's declared set, so the bodies install here too
    hosts.install_host_pkg(&ctx, rut_std::strbuild::pkg());
    let (http_pkg, fx) = rut_std::http::pkg_with(fix);
    hosts.install_host_pkg(&ctx, http_pkg);
    let out_sink = sinks.out.clone();
    rut_vm::register!(hosts, "rgh_host::out", (&str,) -> (),
        move |_vm: &mut Vm, line: &str| -> Result<(), Trap> {
            out_sink.borrow_mut().push(line.to_string());
            Ok(())
        });
    let err_sink = sinks.err.clone();
    rut_vm::register!(hosts, "rgh_host::eprint", (&str,) -> (),
        move |_vm: &mut Vm, line: &str| -> Result<(), Trap> {
            err_sink.borrow_mut().push(line.to_string());
            Ok(())
        });
    let files_sink = sinks.files.clone();
    let fail_sink = sinks.fail_write.clone();
    rut_vm::register!(hosts, "rgh_host::write_file", (&str, Vec<u8>) -> Option<String>,
        move |_vm: &mut Vm, dest: &str, data: Vec<u8>| -> Result<Option<String>, Trap> {
            let fail = fail_sink.borrow();
            if let Some((path, text)) = fail.as_ref() {
                if path == dest {
                    return Ok(Some(text.clone()));
                }
            }
            drop(fail);
            files_sink.borrow_mut().push(('W', dest.to_string(), data));
            Ok(None)
        });
    let files_sink = sinks.files.clone();
    let fail_sink = sinks.fail_write.clone();
    rut_vm::register!(hosts, "rgh_host::append_file", (&str, Vec<u8>) -> Option<String>,
        move |_vm: &mut Vm, dest: &str, data: Vec<u8>| -> Result<Option<String>, Trap> {
            let fail = fail_sink.borrow();
            if let Some((path, text)) = fail.as_ref() {
                if path == dest {
                    return Ok(Some(text.clone()));
                }
            }
            drop(fail);
            files_sink.borrow_mut().push(('A', dest.to_string(), data));
            Ok(None)
        });
    let code_sink = sinks.code.clone();
    let exited_sink = sinks.exited.clone();
    rut_vm::register!(hosts, "rgh_host::exit", (i32,) -> (),
        move |_vm: &mut Vm, code: i32| -> Result<(), Trap> {
            *code_sink.borrow_mut() = Some(code);
            *exited_sink.borrow_mut() = true;
            Ok(()) // the recording lane: the brain retires, the loop idles
        });
    hosts.verify_against(&ctx.flatten()); // the decl ↔ the bodies
    let vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts)
        .unwrap();
    (vm, sinks, fx)
}

/// launch + pump: `boot` launches the brain, the driving loop runs to
/// idle over the virtual clock (settle the fixture dues, advance to
/// the next due/deadline — the async_host_fns shape), the exit code
/// reads out of the recording lane
fn run(vm: &mut Vm, fx: &HttpFixture, sinks: &Sinks, args: &[&str]) -> i32 {
    vm.call::<_, ()>("boot", (args.join("\n"),)).unwrap();
    for _ in 0..500 {
        vm.run_ready().expect("run_ready");
        if vm.pending_tasks() == 0 && !fx.has_pending() {
            break;
        }
        if fx.settle(vm.now_ms()) {
            continue;
        }
        let next = [fx.next_due(), vm.next_deadline()]
            .into_iter()
            .flatten()
            .filter(|&d| d > vm.now_ms())
            .min();
        match next {
            Some(d) => vm.set_now(d),
            None => vm.set_now(vm.now_ms() + 1),
        }
    }
    assert!(
        *sinks.exited.borrow(),
        "the brain exited: {:?} / {:?}",
        sinks.out_lines(),
        sinks.err_lines()
    );
    sinks.code()
}

/// a fixture lane keyed on (method, url): a GET the map knows answers
/// its recorded reply; anything else is a LOUD transport failure — the
/// fixture map's keys ARE the assertions
fn fixmap(entries: &[(&str, FixtureReply)]) -> impl Fn(&str, &str, &str, &[u8]) -> Result<FixtureReply, String> {
    let map: std::collections::HashMap<String, FixtureReply> = entries
        .iter()
        .map(|(u, r)| (u.to_string(), r.clone()))
        .collect();
    move |method, url, _headers, body| {
        if method != "GET" {
            return Err(format!("rgh-test: unexpected method {method} for {url}"));
        }
        if !headers_empty_case(_headers) {
            return Err(format!("rgh-test: unexpected headers for {url}"));
        }
        if !body.is_empty() {
            return Err(format!("rgh-test: unexpected request body for {url}"));
        }
        match map.get(url) {
            Some(r) => Ok(r.clone()),
            None => Err(format!("rgh-test: unexpected url: {url}")),
        }
    }
}

fn headers_empty_case(_h: &str) -> bool {
    true // rgh sends no headers today; the builder test in rut-std pins the format
}

// ------------------------------------------------- the Rust reference ---

/// the human-size ladder, independently spelled (the oracle's twin of
/// the brain's `human_size`): `N B` under 1024, then KiB/MiB/GiB, one
/// decimal (floor) only when the remainder is nonzero
fn human(b: i64) -> String {
    if b < 1024 {
        return format!("{b} B");
    }
    let (unit, div): (&str, i64) = if b < 1048576 {
        ("KiB", 1024)
    } else if b < 1073741824 {
        ("MiB", 1048576)
    } else {
        ("GiB", 1073741824)
    };
    let (whole, rem) = (b / div, b % div);
    if rem == 0 {
        format!("{whole} {unit}")
    } else {
        format!("{whole}.{} {unit}", rem * 10 / div)
    }
}

/// depth-first flatten in API order — the reference walk of a tree
/// document (dirs walked, never printed; files one tab-separated line)
fn flatten(files: &serde_json::Value, prefix: &str, out: &mut Vec<String>) {
    for f in files.as_array().expect("files array") {
        let name = f["name"].as_str().expect("entry name");
        let path = if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") };
        match f.get("files").and_then(|k| k.as_array()) {
            Some(kids) => flatten(&serde_json::Value::Array(kids.clone()), &path, out),
            None => {
                let size = f["size"].as_i64().unwrap_or(0);
                let h8 = match f["hash"].as_str() {
                    Some(h) => h.chars().take(8).collect(),
                    None => "-".to_string(),
                };
                out.push(format!("{path}\t{}\t{h8}", human(size)));
            }
        }
    }
}

fn expected_lines(tree_json: &str) -> Vec<String> {
    let doc: serde_json::Value = serde_json::from_str(tree_json).unwrap();
    let mut out = Vec::new();
    flatten(&doc["files"], "", &mut out);
    out
}

// -------------------------------------------------------------- tests --

#[test]
fn usage_errors_exit_two_with_text() {
    let cases: &[(&[&str], &str)] = &[
        // no flags at all
        (&[], "missing --repo=owner/name"),
        // repo + ref but no subcommand
        (&["--repo=o/r", "--ref=v1"], "missing a subcommand (list | download)"),
        // each flag missing
        (&["--ref=v1", "list"], "missing --repo=owner/name"),
        (&["--repo=o/r", "list"], "missing --ref=<tag|branch|sha>"),
        // bad repo shape: no slash, owner empty, name empty, two slashes
        (&["--repo=plain", "--ref=v1", "list"], "--repo=plain is not owner/name (exactly one '/', both sides nonempty)"),
        (&["--repo=/lead", "--ref=v1", "list"], "--repo=/lead is not owner/name (exactly one '/', both sides nonempty)"),
        (&["--repo=trail/", "--ref=v1", "list"], "--repo=trail/ is not owner/name (exactly one '/', both sides nonempty)"),
        (&["--repo=a/b/c", "--ref=v1", "list"], "--repo=a/b/c is not owner/name (exactly one '/', both sides nonempty)"),
        // empty ref value
        (&["--repo=o/r", "--ref=", "list"], "--ref wants a nonempty value"),
        // unknown subcommand
        (&["--repo=o/r", "--ref=v1", "fetch"], "unknown subcommand: fetch (want list | download)"),
        // unknown flag, before and after the subcommand slot
        (&["--repo=o/r", "--ref=v1", "--wat", "list"], "unknown flag: --wat"),
        (&["--repo=o/r", "--wat", "--ref=v1", "list"], "unknown flag: --wat"),
        // flags repeat
        (&["--repo=o/r", "--repo=p/q", "--ref=v1", "list"], "--repo given twice"),
        (&["--repo=o/r", "--ref=v1", "--ref=v2", "list"], "--ref given twice"),
        // list takes nothing else
        (&["--repo=o/r", "--ref=v1", "list", "extra"], "list takes no arguments (got extra)"),
        // download needs a path; a directory target is refused
        (&["--repo=o/r", "--ref=v1", "download"], "download needs a repo path"),
        (&["--repo=o/r", "--ref=v1", "download", "dir/"], "download: dir/ is a directory target — rgh downloads one file"),
        (&["--repo=o/r", "--ref=v1", "download", "/"], "download: / is a directory target — rgh downloads one file"),
        // download takes at most a dest
        (&["--repo=o/r", "--ref=v1", "download", "x.txt", "a", "b"], "download takes a repo path and at most one dest"),
    ];
    for (args, why) in cases {
        let (mut vm, sinks, fx) = boot(fixmap(&[]));
        let code = run(&mut vm, &fx, &sinks, args);
        assert_eq!(code, 2, "{args:?}: usage exits 2");
        let err = sinks.err_lines();
        assert_eq!(err.first().map(String::as_str), Some(format!("rgh: {why}").as_str()), "{args:?}: the reason leads stderr");
        assert_eq!(err.len(), 3, "{args:?}: reason + the two usage lines");
        assert!(err[1].starts_with("usage: rgh --repo=owner/name"), "{args:?}");
        assert!(sinks.out_lines().is_empty(), "{args:?}: nothing on stdout");
    }
}

#[test]
fn list_boundary_tree_exact() {
    let (mut vm, sinks, fx) = boot(fixmap(&[(
        BOUNDARY_URL,
        FixtureReply::ok(200, BOUNDARY_JSON.as_bytes()),
    )]));
    let code = run(&mut vm, &fx, &sinks, &["--repo=acme/widgets", "--ref=1.0.0", "list"]);
    assert_eq!(code, 0);
    assert!(sinks.err_lines().is_empty());
    // the whole output, string for string: the human-size boundaries
    // (0 / 1023 / 1024 / 1536 / 1 MiB / the 1048577 "1.0 MiB" edge /
    // 1 GiB / 1.5 GiB), hash8 truncation to 8 chars, the hash-less
    // file's "-", depth-first flatten in API order, top-level files
    // after the directories
    let want = [
        "a/one.txt\t0 B\tAAAA0AAA",
        "a/deep/two.bin\t1023 B\tBBBB1023",
        "a/deep/gate.bin\t1 MiB\tCCCC1MIB",
        "a/deep/gate1.bin\t1.0 MiB\tDDDD1M0B",
        "b/three.md\t1 KiB\tEEEE1KIB",
        "b/nohash.dat\t1.5 KiB\t-",
        "c/gib.txt\t1 GiB\tFFFF1GIB",
        "c/gibhalf.txt\t1.5 GiB\tGGGG1G5B",
        "top.txt\t7 B\tHHHH7B7B",
    ];
    assert_eq!(sinks.out_lines(), want);
}

#[test]
fn list_real_tree_matches_the_rust_oracle() {
    let (mut vm, sinks, fx) = boot(fixmap(&[(
        LIST_URL,
        FixtureReply::ok(200, TREE_JSON.as_bytes()),
    )]));
    let code = run(&mut vm, &fx, &sinks, &["--repo=jquery/jquery", "--ref=3.7.1", "list"]);
    assert_eq!(code, 0);
    assert!(sinks.err_lines().is_empty());
    let got = sinks.out_lines();
    let want = expected_lines(TREE_JSON);
    assert_eq!(got.len(), want.len(), "one line per file entry");
    assert_eq!(got, want, "the brain's flatten/human/hash8 vs the Rust reference");
    // hand-checked anchors from the recorded payload (the tree's own
    // rows, byte-spelled): the 0-byte log file, the text file the
    // download tests fetch, the binary jpg
    let find = |p: &str| got.iter().find(|l| l.starts_with(p)).expect("anchor row");
    assert_eq!(find("test/data/support/csp.log\t"), "test/data/support/csp.log\t0 B\t47DEQpj8");
    assert_eq!(find("test/data/text.txt\t"), "test/data/text.txt\t287 B\t5KyZ6UVn");
    assert_eq!(find("test/data/1x1.jpg\t"), "test/data/1x1.jpg\t693 B\tyg5WQThG");
}

#[test]
fn download_streams_binary_verbatim_in_deterministic_chunks() {
    // the jpg as small chunks — the streamed lane must reassemble it
    // VERBATIM through truncate + appends
    let (a, b) = BINARY_BODY.split_at(BINARY_BODY.len() / 2);
    let (mut vm, sinks, fx) = boot(fixmap(&[(
        DL_BINARY_URL,
        FixtureReply::chunked(200, vec![a.to_vec(), b.to_vec()]),
    )]));
    let code = run(&mut vm, &fx, &sinks, &["--repo=jquery/jquery", "--ref=3.7.1", "download", "test/data/1x1.jpg"]);
    assert_eq!(code, 0);
    assert!(sinks.err_lines().is_empty());
    // the default dest is the basename after the final '/'; the write
    // pair is one truncate + the appends, and the reassembly is exact
    assert_eq!(sinks.file_bytes("1x1.jpg"), BINARY_BODY, "the bytes land VERBATIM — binary-safe");
    let files = sinks.files.borrow();
    let kinds: Vec<char> = files.iter().map(|(k, _, _)| *k).collect();
    assert_eq!(kinds.first().copied(), Some('W'), "truncate first");
    assert!(kinds[1..].iter().all(|k| *k == 'A'), "then appends only");
    assert_eq!(sinks.out_lines(), ["downloaded test/data/1x1.jpg (693 bytes) -> 1x1.jpg"]);
}

#[test]
fn download_streams_text_to_explicit_dest_in_small_chunks() {
    // the 287-byte text as five deterministic small chunks
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let step = TEXT_BODY.len() / 5 + 1;
    let mut at = 0;
    while at < TEXT_BODY.len() {
        let end = (at + step).min(TEXT_BODY.len());
        chunks.push(TEXT_BODY[at..end].to_vec());
        at = end;
    }
    let nchunks = chunks.len();
    let (mut vm, sinks, fx) = boot(fixmap(&[(
        DL_TEXT_URL,
        FixtureReply::chunked(200, chunks),
    )]));
    let code = run(
        &mut vm,
        &fx,
        &sinks,
        &["--repo=jquery/jquery", "--ref=3.7.1", "download", "test/data/text.txt", "/tmp/rgh-text.txt"],
    );
    assert_eq!(code, 0);
    assert!(sinks.err_lines().is_empty());
    assert_eq!(sinks.file_bytes("/tmp/rgh-text.txt"), TEXT_BODY);
    let files = sinks.files.borrow();
    let appends = files.iter().filter(|(k, d, _)| *k == 'A' && d == "/tmp/rgh-text.txt").count();
    assert_eq!(appends, nchunks, "one append row per streamed chunk");
    assert_eq!(
        sinks.out_lines(),
        [format!("downloaded test/data/text.txt (287 bytes) -> /tmp/rgh-text.txt")]
    );
}

#[test]
fn download_write_failure_is_a_message_not_a_trap() {
    let (mut vm, sinks, fx) = boot(fixmap(&[(
        DL_TEXT_URL,
        FixtureReply::ok(200, TEXT_BODY),
    )]));
    *sinks.fail_write.borrow_mut() =
        Some(("/tmp/rgh-text.txt".to_string(), "Permission denied (os error 13)".to_string()));
    let code = run(
        &mut vm,
        &fx,
        &sinks,
        &["--repo=jquery/jquery", "--ref=3.7.1", "download", "test/data/text.txt", "/tmp/rgh-text.txt"],
    );
    assert_eq!(code, 1, "a failed write is exit 1");
    assert_eq!(sinks.err_lines(), ["rgh: /tmp/rgh-text.txt: Permission denied (os error 13)"]);
    assert!(sinks.out_lines().is_empty(), "no confirmation line on failure");
}

#[test]
fn download_append_failure_mid_stream_is_a_message_not_a_trap() {
    let (mut vm, sinks, fx) = boot(fixmap(&[(
        DL_TEXT_URL,
        FixtureReply::chunked(200, vec![b"abc".to_vec(), b"def".to_vec()]),
    )]));
    *sinks.fail_write.borrow_mut() =
        Some(("/tmp/rgh-text.txt".to_string(), "No space left on device (os error 28)".to_string()));
    let code = run(
        &mut vm,
        &fx,
        &sinks,
        &["--repo=jquery/jquery", "--ref=3.7.1", "download", "test/data/text.txt", "/tmp/rgh-text.txt"],
    );
    assert_eq!(code, 1);
    assert_eq!(sinks.err_lines(), ["rgh: /tmp/rgh-text.txt: No space left on device (os error 28)"]);
    assert!(sinks.out_lines().is_empty(), "no confirmation line on failure");
}

#[test]
fn a_mid_read_death_is_data_exit_one_with_the_sticky_text() {
    let (mut vm, sinks, fx) = boot(fixmap(&[(
        DL_TEXT_URL,
        FixtureReply::failing(200, vec![b"ab".to_vec(), b"cd".to_vec(), b"ef".to_vec()], 2, "connection reset by peer"),
    )]));
    let code = run(
        &mut vm,
        &fx,
        &sinks,
        &["--repo=jquery/jquery", "--ref=3.7.1", "download", "test/data/text.txt", "/tmp/rgh-text.txt"],
    );
    assert_eq!(code, 1, "a mid-read wire death is exit 1");
    assert_eq!(sinks.err_lines(), ["rgh: /tmp/rgh-text.txt: connection reset by peer"]);
    assert!(sinks.out_lines().is_empty(), "no confirmation line on failure");
    // the arrived prefix DID land (the degrade law: the short drain is
    // the stream's past reads) — the dest holds the two chunks that
    // made it, never a trap
    assert_eq!(sinks.file_bytes("/tmp/rgh-text.txt"), b"abcd".to_vec());
}

#[test]
fn not_found_maps_for_both_subcommands() {
    // list: the repo-or-ref message names the data host
    let (mut vm, sinks, fx) = boot(fixmap(&[(
        LIST_URL_404,
        FixtureReply::ok(404, NOTFOUND_BODY),
    )]));
    let code = run(&mut vm, &fx, &sinks, &["--repo=jquery/jquery", "--ref=no-such-ref-xyz", "list"]);
    assert_eq!(code, 1);
    assert_eq!(sinks.err_lines(), ["rgh: no such repo or ref (checked the CDN: data.jsdelivr.com)"]);
    assert!(sinks.out_lines().is_empty());

    // download: the 404 also covers "no such file", naming the cdn host
    let (mut vm, sinks, fx) = boot(fixmap(&[(
        DL_404_URL,
        FixtureReply::ok(404, NOTFOUND_BODY),
    )]));
    let code = run(&mut vm, &fx, &sinks, &["--repo=jquery/jquery", "--ref=3.7.1", "download", "no/such/file.txt"]);
    assert_eq!(code, 1);
    assert_eq!(sinks.err_lines(), ["rgh: no such file (or repo/ref) (checked the CDN: cdn.jsdelivr.net)"]);
    assert!(sinks.out_lines().is_empty());
}

#[test]
fn transport_failure_maps_to_the_network_message() {
    let (mut vm, sinks, fx) = boot(move |_m, url, _h, _b| {
        if url == LIST_URL {
            Err("dns: lookup data.jsdelivr.com: no such host".to_string())
        } else {
            Err(format!("rgh-test: unexpected url: {url}"))
        }
    });
    let code = run(&mut vm, &fx, &sinks, &["--repo=jquery/jquery", "--ref=3.7.1", "list"]);
    assert_eq!(code, 1);
    // status 0 is the RESERVED transport verdict; its text surfaces
    assert_eq!(sinks.err_lines(), ["rgh: network error: dns: lookup data.jsdelivr.com: no such host"]);
    assert!(sinks.out_lines().is_empty());
}

#[test]
fn rate_limit_and_other_statuses_map() {
    // 403 — the CDN's rate-limit shape
    let (mut vm, sinks, fx) = boot(fixmap(&[(LIST_URL, FixtureReply::ok(403, b""))]));
    let code = run(&mut vm, &fx, &sinks, &["--repo=jquery/jquery", "--ref=3.7.1", "list"]);
    assert_eq!(code, 1);
    assert_eq!(sinks.err_lines()[0], "rgh: rate limited by the CDN (HTTP 403) — wait and retry");

    // any other non-2xx is the bare status
    let (mut vm, sinks, fx) = boot(fixmap(&[(LIST_URL, FixtureReply::ok(500, b"boom"))]));
    let code = run(&mut vm, &fx, &sinks, &["--repo=jquery/jquery", "--ref=3.7.1", "list"]);
    assert_eq!(code, 1);
    assert_eq!(sinks.err_lines(), ["rgh: CDN 500"]);
}

// ------------------------------------------- the one-shot law probe ----

/// the degraded second-taker case: the brain never double-takes, so a
/// tiny probe rides the same mounts and exercises BOTH orders — body
/// then stream (the mint degrades to a dead reader), stream then body
/// (the drain degrades to empty) — through the SAME fixture lane.
const ONESHOT_SRC: &str = r#"
use http::HttpClient;
use async_host::launch_future;
use rgh_host::{ out };

async fn body_first(url: str) -> nil {
    let client = HttpClient.new();
    let resp = await client.get(url).build().send();
    let b = await resp.body();
    out(f"body={b.len()}");
    let s = resp.byte_stream();
    let c = await s.next();
    if (c == nil) {
        out("late-next=eof");
    } else {
        out("late-next=data");
    }
}

async fn stream_first(url: str) -> nil {
    let client = HttpClient.new();
    let resp = await client.get(url).build().send();
    let s = resp.byte_stream();
    let c = await s.next();
    if (c == nil) {
        out("first-next=eof");
    } else {
        let v: bytes = c;
        out(f"first-next={v.len()}");
    }
    let b = await resp.body();
    out(f"late-body={b.len()}");
}

entry fn boot_oneshot(url: str) -> nil {
    launch_future(body_first(url));
    launch_future(stream_first(url));
}
"#;

#[test]
fn the_one_shot_law_degrades_second_takers() {
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std(&mut s);
    rut_driver::mount_std_async(&mut s);
    let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut");
    for d in ["pouch", "nmapset", "json", "http_host", "http"] {
        rut_driver::mount_dir(&mut s, &tree.join(d)).expect("mount tree pkg");
    }
    rut_driver::mount_dir(&mut s, &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rgh_host"))
        .expect("mount rgh_host");
    rut_driver::assemble_peers(&mut s).expect("assemble peer groups");
    let out = rut_driver::compile_module_in(&mut s, ONESHOT_SRC, rut_parser::Mode::Impl, "rgh");
    assert!(
        out.diags.is_empty(),
        "{}",
        out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let prog = rut_core::binary::decode(out.binary.as_deref().unwrap()).unwrap();
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(4_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let sinks = Sinks::default();
    let ctx = s.host_pkg_context();
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::math::pkg());
    hosts.install_host_pkg(&ctx, rut_std::nmap::pkg());
    hosts.install_host_pkg(&ctx, rut_std::async_host::pkg());
    // json's writer rides the strbuild pkg — the `strbuild_host` rows are
    // in this closure's declared set, so the bodies install here too
    hosts.install_host_pkg(&ctx, rut_std::strbuild::pkg());
    let (http_pkg, fx) = rut_std::http::pkg_with(|_m, _u, _h, _b| {
        Ok(FixtureReply::chunked(200, vec![b"ab".to_vec(), b"cd".to_vec()]))
    });
    hosts.install_host_pkg(&ctx, http_pkg);
    let out_sink = sinks.out.clone();
    rut_vm::register!(hosts, "rgh_host::out", (&str,) -> (),
        move |_vm: &mut Vm, line: &str| -> Result<(), Trap> {
            out_sink.borrow_mut().push(line.to_string());
            Ok(())
        });
    let err_sink = sinks.err.clone();
    rut_vm::register!(hosts, "rgh_host::eprint", (&str,) -> (),
        move |_vm: &mut Vm, line: &str| -> Result<(), Trap> {
            err_sink.borrow_mut().push(line.to_string());
            Ok(())
        });
    // the probe never touches the files or the exit row, but the decl
    // contract is TOTAL — every mounted row binds something
    let files_sink = sinks.files.clone();
    rut_vm::register!(hosts, "rgh_host::write_file", (&str, Vec<u8>) -> Option<String>,
        move |_vm: &mut Vm, dest: &str, data: Vec<u8>| -> Result<Option<String>, Trap> {
            files_sink.borrow_mut().push(('W', dest.to_string(), data));
            Ok(None)
        });
    let files_sink = sinks.files.clone();
    rut_vm::register!(hosts, "rgh_host::append_file", (&str, Vec<u8>) -> Option<String>,
        move |_vm: &mut Vm, dest: &str, data: Vec<u8>| -> Result<Option<String>, Trap> {
            files_sink.borrow_mut().push(('A', dest.to_string(), data));
            Ok(None)
        });
    let code_sink = sinks.code.clone();
    rut_vm::register!(hosts, "rgh_host::exit", (i32,) -> (),
        move |_vm: &mut Vm, code: i32| -> Result<(), Trap> {
            *code_sink.borrow_mut() = Some(code);
            Ok(())
        });
    hosts.verify_against(&ctx.flatten());
    let mut vm =
        rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts).unwrap();
    vm.call::<_, ()>("boot_oneshot", ("fixture://oneshot",)).unwrap();
    for _ in 0..500 {
        vm.run_ready().unwrap();
        if vm.pending_tasks() == 0 && !fx.has_pending() {
            break;
        }
        if fx.settle(vm.now_ms()) {
            continue;
        }
        let next = [fx.next_due(), vm.next_deadline()].into_iter().flatten().filter(|&d| d > vm.now_ms()).min();
        match next {
            Some(d) => vm.set_now(d),
            None => vm.set_now(vm.now_ms() + 1),
        }
    }
    // the two futures interleave — the LAWS are per-response, compare
    // as a set
    let mut got = sinks.out_lines();
    got.sort();
    assert_eq!(
        got,
        vec![
            "body=4",           // the drain took the whole body
            "first-next=2",     // the stream lane took chunk one
            "late-body=0",      // the late DRAIN degrades to empty
            "late-next=eof",    // the late MINT is a dead reader
        ],
        "body xor stream: whichever lane took first wins, the second degrades — never a trap"
    );
    assert!(sinks.err_lines().is_empty());
}

// ------------------------------------------------- the embedder budget -

#[test]
fn the_real_tree_stays_inside_the_embedder_budget() {
    // the real 351-entry tree over the full boot: the fuel spend the
    // native embedder's budget (250M) must cover with room to spare
    let (mut vm, sinks, fx) = boot(fixmap(&[(
        LIST_URL,
        FixtureReply::ok(200, TREE_JSON.as_bytes()),
    )]));
    let code = run(&mut vm, &fx, &sinks, &["--repo=jquery/jquery", "--ref=3.7.1", "list"]);
    assert_eq!(code, 0);
    assert_eq!(sinks.out_lines().len(), 296);
    assert!(vm.fuel_used < 250_000_000, "fuel {}", vm.fuel_used);
    assert!(vm.fuel_used < 25_000_000, "an order of magnitude of headroom (fuel {})", vm.fuel_used);
    println!("list of the real 351-entry tree: {} fuel", vm.fuel_used);
}

// ------------------------------------------------- the live smoke ------

#[test]
fn live_smoke_over_the_real_cdn() {
    if std::env::var("RGH_LIVE").ok().as_deref() != Some("1") {
        eprintln!("live_smoke_over_the_real_cdn: skipped (set RGH_LIVE=1 to run against data.jsdelivr.com)");
        return;
    }
    // the embedder's own boot, but the observations ride the sinks and
    // `exit` records: reqwest lane (http::pkg()), the real DNS,
    // the real CDN, the wall-clock pump
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std(&mut s);
    rut_driver::mount_std_async(&mut s);
    let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut");
    for d in ["pouch", "nmapset", "json", "http_host", "http"] {
        rut_driver::mount_dir(&mut s, &tree.join(d)).expect("mount tree pkg");
    }
    rut_driver::mount_dir(&mut s, &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rgh_host"))
        .expect("mount rgh_host");
    rut_driver::assemble_peers(&mut s).expect("assemble peer groups");
    let out = rut_driver::compile_module_in(&mut s, SRC, rut_parser::Mode::Impl, "rgh");
    assert!(out.diags.is_empty());
    let prog = rut_core::binary::decode(out.binary.as_deref().unwrap()).unwrap();
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(250_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let sinks = Sinks::default();
    let ctx = s.host_pkg_context();
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::math::pkg());
    hosts.install_host_pkg(&ctx, rut_std::nmap::pkg());
    hosts.install_host_pkg(&ctx, rut_std::async_host::pkg());
    hosts.install_host_pkg(&ctx, rut_std::http::pkg()); // the reqwest lane
    let out_sink = sinks.out.clone();
    rut_vm::register!(hosts, "rgh_host::out", (&str,) -> (),
        move |_vm: &mut Vm, line: &str| -> Result<(), Trap> {
            out_sink.borrow_mut().push(line.to_string());
            Ok(())
        });
    let err_sink = sinks.err.clone();
    rut_vm::register!(hosts, "rgh_host::eprint", (&str,) -> (),
        move |_vm: &mut Vm, line: &str| -> Result<(), Trap> {
            err_sink.borrow_mut().push(line.to_string());
            Ok(())
        });
    // the list smoke never touches the files, but the decl contract is
    // total
    let files_sink = sinks.files.clone();
    rut_vm::register!(hosts, "rgh_host::write_file", (&str, Vec<u8>) -> Option<String>,
        move |_vm: &mut Vm, dest: &str, data: Vec<u8>| -> Result<Option<String>, Trap> {
            files_sink.borrow_mut().push(('W', dest.to_string(), data));
            Ok(None)
        });
    let files_sink = sinks.files.clone();
    rut_vm::register!(hosts, "rgh_host::append_file", (&str, Vec<u8>) -> Option<String>,
        move |_vm: &mut Vm, dest: &str, data: Vec<u8>| -> Result<Option<String>, Trap> {
            files_sink.borrow_mut().push(('A', dest.to_string(), data));
            Ok(None)
        });
    let code_sink = sinks.code.clone();
    rut_vm::register!(hosts, "rgh_host::exit", (i32,) -> (),
        move |_vm: &mut Vm, code: i32| -> Result<(), Trap> {
            *code_sink.borrow_mut() = Some(code);
            Ok(())
        });
    hosts.verify_against(&ctx.flatten());
    let mut vm =
        rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts).unwrap();
    vm.call::<_, ()>("boot", ("--repo=jquery/jquery\n--ref=3.7.1\nlist".to_string(),)).unwrap();
    // the wall-clock pump: real workers settle the completers
    let mut ok = false;
    for _ in 0..20_000 {
        vm.run_ready().unwrap();
        if sinks.code.borrow().is_some() {
            ok = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(ok, "the brain exited: {:?}", sinks.err_lines());
    assert_eq!(sinks.code(), 0, "stderr: {:?}", sinks.err_lines());
    assert!(!sinks.out_lines().is_empty(), "the real tree lists files");
}
