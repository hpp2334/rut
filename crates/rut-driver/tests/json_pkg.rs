//! The json pkg's phase-1 gate (the rut-json batch; docs/rut-json-survey.md
//! is the contract). DOM-free and pure — the `store.rs` precedent: every
//! test drives the pkg through its entry surface on a plain VM and
//! asserts canonical strings, no DOM, no clock, no host surface.
//!
//! The fixtures: `tests/data/jsonpkg` (json — the base surface end to
//! end) and `tests/data/jsonlight` (json alone — the same base, a
//! minimal entry set).
//!
//! Entry answer shape:
//!   successes  ->  `OK:<payload>`
//!   failures   ->  `ERR:<Kind>@<at>:<got>:<expected>`   (decode)
//!                  `ERR:<Kind>@<at>:<path>`             (encode)

use std::path::Path;
use std::rc::Rc;

use rut_vm::interp::{HostHooks, Vm};

const PKG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/jsonpkg");
const LIGHT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/jsonlight");




/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(rut_driver::RutRun::new().pkg(rut_driver::Pkg::source(spec, src)).entrypoint(spec).compile())
}

/// [`compiled`] with calc offered (the old `mount_std` shape: core
/// auto-rides, `calc` is an ordinary pkg).
#[allow(dead_code)]
fn compiled_std(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            // the root DECLARES its calc use (the structural form; the
            // offer stays the walked tree pkg)
            .pkg(
                rut_driver::Pkg::source(spec, src)
                    .manifest(rut_driver::Manifest::lib(spec, &["calc"])),
            )
            .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
            .entrypoint(spec)
            .compile(),
    )
}

#[allow(dead_code)]
fn graph_of(c: Result<rut_driver::Compiled, rut_driver::RunError>) -> rut_driver::GraphOutput {
    match c {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(rut_lexer::span::Span::new(0, 0), e.msg)],
            program: None,
        },
    }
}

fn vm_at(dir: &str) -> Vm {
    let loaded = rut_native::load_dir(Path::new(dir)).expect("mount");
    let compiled = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
        // json's writer rides the strbuild pkg — its `strbuild_host` rows
        // are in this closure's declared set, so the bodies install
        // through the same chain
        .host_pkg(rut_std::nmap::pkg())
        .host_pkg(rut_std::strbuild::pkg())
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile the walk");
    assert!(
        compiled.graph.diags.is_empty(),
        "diags: {}",
        compiled.graph.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let flat = rut_core::link::flatten(compiled.graph.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(400_000_000),
        heap_limit_bytes: Some(256 * 1024 * 1024),
        interrupt_every: 1024,
    };
    rut_vm::interp:: Vm::builder().program(Rc::new(flat)).limits(limits).hooks(HostHooks::default()).hosts(compiled.hosts).build().expect("vm")
}

fn dec(entry: &str, doc: &str) -> String {
    let mut vm = vm_at(PKG);
    vm.call(entry, (doc.to_string(),)).unwrap_or_else(|e| panic!("{entry}({doc:?}): {e:?}"))
}

fn dec_expect(entry: &str, doc: &str, want: &str) {
    let out = dec(entry, doc);
    assert_eq!(out, want, "{entry}({doc:?})");
}

// ---- the exactly-one-nil invariant (survey §2.8, §3 matrix 1) ----

#[test]
fn exactly_one_nil_decode() {
    for doc in ["42", "", "1 2", "1.5", "-", "[1,2", "true", "{}"] {
        let mut vm = vm_at(PKG);
        let out: String = vm.call("pair_dec_i64", (doc.to_string(),)).unwrap();
        assert!(
            out == "OK" || out == "ERR",
            "pair_dec_i64({doc:?}) must answer exactly one non-nil: {out}"
        );
    }
}

#[test]
fn exactly_one_nil_encode() {
    let mut vm = vm_at(PKG);
    let out: String = vm.call("pair_enc_i64", (42i64,)).unwrap();
    assert_eq!(out, "OK");
}

// ---- i64: the §2.4 table, row by row ----

#[test]
fn i64_decode_edges() {
    let cases = [
        ("0", "OK:0"),
        ("-0", "OK:0"),
        ("42", "OK:42"),
        ("-17", "OK:-17"),
        (" 42 ", "OK:42"),
        // i64::MIN / MAX exact — never a silent wrap
        ("-9223372036854775808", "OK:-9223372036854775808"),
        ("9223372036854775807", "OK:9223372036854775807"),
        // the first overflow, either sign: WrongType naming the lexeme
        ("9223372036854775808", "ERR:WrongType@0:9223372036854775808:an i64"),
        ("-9223372036854775809", "ERR:WrongType@0:-9223372036854775809:an i64"),
        // a float LEXEME into i64: WrongType, the whole lexeme in `got`
        ("1.5", "ERR:WrongType@0:1.5:an i64"),
        ("1e3", "ERR:WrongType@0:1e3:an i64"),
        ("-2.5e-3", "ERR:WrongType@0:-2.5e-3:an i64"),
        // leading zeros are invalid JSON (the integer part)
        ("01", "ERR:Unexpected@0:01:a JSON number"),
        // empty and non-number docs
        ("", "ERR:Truncated@0::an i64"),
        ("-", "ERR:Truncated@1::an i64"),
        ("true", "ERR:WrongType@0:t:an i64"),
        ("t", "ERR:WrongType@0:t:an i64"),
    ];
    for (doc, want) in cases {
        dec_expect("dec_i64", doc, want);
    }
}

#[test]
fn i64_encode_decimal() {
    let mut vm = vm_at(PKG);
    for (v, want) in [
        (0i64, "0"),
        (-17i64, "-17"),
        (9223372036854775807, "9223372036854775807"),
        (-9223372036854775808, "-9223372036854775808"),
    ] {
        let out: String = vm.call("enc_i64", (v,)).unwrap();
        assert_eq!(out, want);
    }
}

// ---- f64: tier 1 exact, the round-trip law (survey §2.4) ----

#[test]
fn f64_decode_and_shortest_encode() {
    let cases = [
        ("0.1", "OK:0.1"),
        ("1.5", "OK:1.5"),
        ("2.5", "OK:2.5"),
        ("-0.25", "OK:-0.25"),
        ("3", "OK:3"),
        // -0.0: the sign survives decode and re-encode
        ("-0.0", "OK:-0"),
        ("1e22", "OK:10000000000000000000000"),
        ("1e-22", "OK:0.0000000000000000000001"),
        ("9007199254740992", "OK:9007199254740992"),
        ("3.141592653589793", "OK:3.141592653589793"),
    ];
    for (doc, want) in cases {
        dec_expect("dec_f64", doc, want);
    }
}

#[test]
fn f64_round_trip_stability() {
    // survey §2.4's round-trip law. Tier-1-safe samples are bit-exact;
    // tier-2 shapes (huge exponents, >18-digit mantissas) are disclosed
    // ±ulp — they assert STABILITY (decode∘encode∘decode fixes), never
    // bit-exactness against the literal parse.
    let docs = [
        "0.1", "1.5", "2.5", "-0.25", "1e22", "-1e22", "3", "1e-22",
        "9007199254740992", "3.141592653589793", "-2.5e-21", "123.456e-20",
    ];
    for doc in docs {
        let mut vm = vm_at(PKG);
        let out: String = vm.call("f64_rt", (doc.to_string(),)).unwrap();
        assert_eq!(out, "STABLE", "f64_rt({doc:?})");
    }
}

#[test]
fn f64_tier2_edges_stable() {
    // MAX, MIN_POSITIVE: Display's full expansion re-decodes to the SAME
    // value — stability, the survey's disclosed tier-2 honesty
    for doc in ["1.7976931348623157e308", "2.2250738585072014e-308", "-2.5e-22"] {
        let mut vm = vm_at(PKG);
        let out: String = vm.call("f64_rt", (doc.to_string(),)).unwrap();
        assert_eq!(out, "STABLE", "f64_rt({doc:?})");
    }
}

// ---- bool: literals only; `1`/`0` are WrongType ----

#[test]
fn bool_laws() {
    let cases = [
        ("true", "OK:true"),
        ("false", "OK:false"),
        (" true ", "OK:true"),
        ("1", "ERR:WrongType@0:1:a bool"),
        ("0", "ERR:WrongType@0:0:a bool"),
        ("tru", "ERR:Unexpected@0:tru:true or false"),
        ("trueX", "ERR:Trailing@4:X:end of input"),
        ("", "ERR:Truncated@0::a bool"),
    ];
    for (doc, want) in cases {
        dec_expect("dec_bool", doc, want);
    }
    let mut vm = vm_at(PKG);
    let t: String = vm.call("enc_bool", (true,)).unwrap();
    assert_eq!(t, "true");
    let f: String = vm.call("enc_bool", (false,)).unwrap();
    assert_eq!(f, "false");
}

// ---- str: the RFC 8259 escape set, both directions ----

#[test]
fn str_decode_escapes() {
    let cases = [
        // the shorthands
        ("\"a\\nb\"", "OK:a\nb"),
        ("\"a\\tb\"", "OK:a\tb"),
        ("\"\\b\\f\\r\"", "OK:\u{8}\u{c}\r"),
        ("\"\\\\\"", "OK:\\"),
        ("\"\\\"\"", "OK:\""),
        ("\"\\/\"", "OK:/"),
        // \\u escapes, surrogate pairs
        ("\"\\u4E16\"", "OK:世"),
        ("\"\\ud83d\\ude00\"", "OK:\u{1f600}"),
        // an unpaired surrogate is invalid JSON rut cannot represent —
        // Unexpected, never a silent replacement char
        ("\"\\uD800\"", "ERR:Unexpected@8:\\uD800:a surrogate pair"),
        ("\"\\uDEAD\"", "ERR:Unexpected@8:\\uDEAD:a codepoint"),
        // a broken \\u is a grammar miss
        ("\"\\uZZ\"", "ERR:Unexpected@6:Z:4 hex digits"),
        // raw control characters are invalid JSON
        ("\"a\nb\"", "ERR:Unexpected@2:\n:a string"),
        // non-ASCII passes through verbatim
        ("\"世\"", "OK:世"),
        // unterminated
        ("\"abc", "ERR:Truncated@4::a string"),
        // not a string
        ("42", "ERR:WrongType@0:4:a string"),
    ];
    for (doc, want) in cases {
        dec_expect("dec_str", doc, want);
    }
}

#[test]
fn str_encode_escapes() {
    let cases = [
        // the escape set: quote, backslash, C0 shorthands, \u00xx;
        // DEL and non-ASCII pass through verbatim
        ("plain", "\"plain\""),
        ("a\"b", "\"a\\\"b\""),
        ("a\\b", "\"a\\\\b\""),
        ("a\nb", "\"a\\nb\""),
        ("a\tb", "\"a\\tb\""),
        ("\u{1}", "\"\\u0001\""),
        ("\u{1f}", "\"\\u001f\""),
        ("世", "\"世\""),
        ("\u{7f}", "\"\u{7f}\""),
    ];
    for (arg, want) in cases {
        let mut vm = vm_at(PKG);
        let out: String = vm.call("enc_str", (arg.to_string(),)).unwrap();
        assert_eq!(out, want, "enc_str({arg:?})");
    }
}

#[test]
fn str_escape_round_trips() {
    // decode(encode(s)) == s, bit-exact, across the whole escape set
    let samples = [
        "plain",
        "",
        "a\"b",
        "a\\b",
        "a\nb\tc\rd",
        "\u{8}\u{c}",
        "\u{1}\u{1f}",
        "世界",
        "\u{7f}",
        "mixed \"q\" \\ / end\n",
    ];
    for s in samples {
        let mut vm = vm_at(PKG);
        let out: String = vm.call("str_rt", (s.to_string(),)).unwrap();
        assert_eq!(out, "RT", "str_rt({s:?})");
    }
}

// ---- [T]: the base's structural row ----

#[test]
fn arr_decodes() {
    let cases = [
        ("[]", "OK:len=0"),
        ("[7]", "OK:len=1:7"),
        ("[7,8,9]", "OK:len=3:7:8:9"),
        ("[1, 2 , 3]", "OK:len=3:1:2:3"),
        // nested arrays ride the element's own impl
        ("[[1,2],[3]]", "ERR:WrongType@1:[:an i64"),
    ];
    for (doc, want) in cases {
        dec_expect("dec_arr", doc, want);
    }
}

#[test]
fn arr_errors() {
    let cases = [
        ("[1,2", "ERR:Truncated@4::the container's closer"),
        ("[1,]", "ERR:Unexpected@3:]:an element or member"),
    ];
    for (doc, want) in cases {
        dec_expect("dec_arr", doc, want);
    }
}

#[test]
fn arr_encodes() {
    let mut vm = vm_at(PKG);
    let empty: String = vm.call("enc_arr_empty", ()).unwrap();
    assert_eq!(empty, "[]");
    let three: String = vm.call("enc_arr_three", ()).unwrap();
    assert_eq!(three, "[7,8,9]");
    let nested: String = vm.call("enc_arr_nested", ()).unwrap();
    assert_eq!(nested, "OK:[[1,2],[3]]");
}

// ---- ?T: null <-> nil (survey §2.6's optional row) ----

#[test]
fn opt_mapping() {
    let cases = [
        // the null case: (nil, nil) — successful absence, the pair law's
        // ?T reading
        ("null", "OK:nil"),
        ("55", "OK:55"),
        ("\"x\"", "ERR:WrongType@0:\":an i64"),
    ];
    for (doc, want) in cases {
        dec_expect("dec_q_i64", doc, want);
    }
    let mut vm = vm_at(PKG);
    let nil_doc: String = vm.call("enc_q_nil", ()).unwrap();
    assert_eq!(nil_doc, "null");
    let val_doc: String = vm.call("enc_q_val", (55i64,)).unwrap();
    assert_eq!(val_doc, "55");
}

// ---- the depth limit: 128, both directions, recoverable ----

#[test]
fn depth_decode() {
    // the reader's own container ops enforce the cap directly: 200 open
    // brackets answer Depth at the 129th (a DIRECT schema like
    // `Vec<i64>` answers WrongType at the first type mismatch long
    // before doc nesting accrues — the type, not the doc, is the law)
    let mut vm = vm_at(PKG);
    let out: String = vm.call("reader_depth", ()).unwrap();
    assert!(out.starts_with("ERR:Depth@"), "deep decode: {out}");

    let mut okdeep = String::new();
    for _ in 0..100 {
        okdeep.push('[');
    }
    okdeep.push('1');
    for _ in 0..100 {
        okdeep.push(']');
    }
    // 100 levels: inside the cap, the reader stays clean
    let mut vm = vm_at(PKG);
    let okout: String = vm.call("reader_depth_ok", ()).unwrap();
    assert_eq!(okout, "NOERR");
}

#[test]
fn depth_encode() {
    let mut vm = vm_at(PKG);
    let deep: String = vm.call("enc_deep", (200i32,)).unwrap();
    assert!(deep.starts_with("ERR:Depth@"), "deep encode: {deep}");
    let mut vm = vm_at(PKG);
    let okdeep: String = vm.call("enc_deep", (100i32,)).unwrap();
    assert_eq!(okdeep, "NOERR");
}

// ---- trailing garbage, truncation ----

#[test]
fn trailing_and_truncated() {
    let cases = [
        ("dec_i64", "1 2", "ERR:Trailing@2:2:end of input"),
        ("dec_i64", "1 x", "ERR:Trailing@2:x:end of input"),
        ("dec_i64", "{}", "ERR:WrongType@0:{:an i64"),
        ("dec_arr", "[1] ]", "ERR:Trailing@4:]:end of input"),
    ];
    for (entry, doc, want) in cases {
        dec_expect(entry, doc, want);
    }
}

// ---- the writer's public surface (the serde model) ----

#[test]
fn writer_surface() {
    let mut vm = vm_at(PKG);
    let out: String = vm.call("writer_demo", ()).unwrap();
    assert_eq!(
        out,
        "OK:{\"name\":\"rut\",\"n\":-7,\"ok\":true,\"x\":1.5,\"rows\":[1,2]}"
    );
}

#[test]
fn writer_not_finite() {
    // inf/nan have NO JSON spelling — NotFinite, at the output offset,
    // with the lazily-built path
    let mut vm = vm_at(PKG);
    let out: String = vm.call("writer_not_finite", (f64::INFINITY,)).unwrap();
    assert_eq!(out, "ERR:NotFinite@7:$.bad");
}

// ---- the reader's public surface + skip_value ----

#[test]
fn reader_surface() {
    let cases = [
        (
            "{\"name\": \"rut\", \"age\": 9}",
            "OK:rut:9",
        ),
        (
            // unknown keys skip grammatically (containers included)
            "{\"name\": \"a\", \"age\": 1, \"unknown\": {\"deep\": [true, null]}}",
            "OK:a:1",
        ),
    ];
    for (doc, want) in cases {
        let mut vm = vm_at(PKG);
        let out: String = vm.call("reader_walk", (doc.to_string(),)).unwrap();
        assert_eq!(out, want, "reader_walk({doc:?})");
    }
}

// ---- decodeJsonBytes: STRICT UTF-8, never the lossy shortcut ----

#[test]
fn bytes_entry() {
    let mut vm = vm_at(PKG);
    let clean: String = vm.call("dec_bytes", (b"17".to_vec(),)).unwrap();
    assert_eq!(clean, "OK:17");

    // 0xFF in the middle: InvalidUtf8 at the octet, hex `got`
    let mut vm = vm_at(PKG);
    let bad: String = vm.call("dec_bytes", (vec![b'1', 0xFF, b'2'],)).unwrap();
    assert_eq!(bad, "ERR:InvalidUtf8@1:0xff:UTF-8");

    // a CJK lead octet with the continuation missing
    let mut vm = vm_at(PKG);
    let trunc: String = vm.call("dec_bytes", (vec![0xE3],)).unwrap();
    assert_eq!(trunc, "ERR:InvalidUtf8@1:0x00:UTF-8");

    // C0 AF — the overlong '/', classically nasty — the lead octet C0
    // itself is rejected (an overlong lead can never start a sequence)
    let mut vm = vm_at(PKG);
    let overlong: String = vm.call("dec_bytes", (vec![0xC0, 0xAF],)).unwrap();
    assert_eq!(overlong, "ERR:InvalidUtf8@0:0xc0:UTF-8");

    // an ED A0 continuation (surrogate range) — rejected
    let mut vm = vm_at(PKG);
    let surrogate: String = vm.call("dec_bytes", (vec![b'"', 0xED, 0xA0, 0x80, b'"'],)).unwrap();
    assert!(surrogate.starts_with("ERR:InvalidUtf8"), "ed-a0: {surrogate}");

    // F4 90 (beyond U+10FFFF) — rejected
    let mut vm = vm_at(PKG);
    let beyond: String = vm.call("dec_bytes", (vec![0xF4, 0x90, 0x80, 0x80],)).unwrap();
    assert!(beyond.starts_with("ERR:InvalidUtf8"), "f4-90: {beyond}");

    // clean multi-byte content rides the same str cursor
    let mut vm = vm_at(PKG);
    let world: String = vm.call("dec_bytes", ("-5".as_bytes().to_vec(),)).unwrap();
    assert_eq!(world, "OK:-5");
    let mut vm = vm_at(PKG);
    let cjk: String = vm.call("dec_bytes_str", ("\"\u{4e16}\"".as_bytes().to_vec(),)).unwrap();
    assert_eq!(cjk, "OK:世");
}

// ---- every error kind reachable (survey §3 matrix 1) ----

#[test]
fn every_decode_kind_reachable() {
    let mut vm = vm_at(PKG);
    let wt: String = vm.call("dec_i64", ("true".to_string(),)).unwrap();
    assert!(wt.contains("WrongType"), "{wt}");
    let mut vm = vm_at(PKG);
    let tr: String = vm.call("dec_i64", ("".to_string(),)).unwrap();
    assert!(tr.contains("Truncated"), "{tr}");
    let mut vm = vm_at(PKG);
    let un: String = vm.call("dec_str", ("\"\\uZZ\"".to_string(),)).unwrap();
    assert!(un.contains("Unexpected"), "{un}");
    let mut vm = vm_at(PKG);
    let dp: String = vm.call("reader_depth", ()).unwrap();
    assert!(dp.contains("Depth"), "{dp}");
    let mut vm = vm_at(PKG);
    let tl: String = vm.call("dec_i64", ("1 x".to_string(),)).unwrap();
    assert!(tl.contains("Trailing"), "{tl}");
    let mut vm = vm_at(PKG);
    let iu: String = vm.call("dec_bytes", (vec![0xFFu8],)).unwrap();
    assert!(iu.contains("InvalidUtf8"), "{iu}");
}

#[test]
fn every_encode_kind_reachable() {
    // Depth: the cyclic writer (the expected failure — data,
    // never a trap); NotFinite: inf. KeyUnsupported is DECLARED in the
    // finalized error set (survey §2.3) but DORMANT, on record: its
    // only sanctioned fire site would be the map ENCODE rows (the
    // map wrappers rode the retired `[peer-deps]` groups).
    let mut vm = vm_at(PKG);
    let deep: String = vm.call("enc_deep", (200i32,)).unwrap();
    assert!(deep.starts_with("ERR:Depth"), "{deep}");
    let mut vm = vm_at(PKG);
    let inf: String = vm.call("writer_not_finite", (f64::INFINITY,)).unwrap();
    assert_eq!(inf, "ERR:NotFinite@7:$.bad");
}

// ---- the LIGHT consumer: json mounts base-only ----

#[test]
fn light_consumer_base_rows() {
    let mut vm = vm_at(LIGHT);
    let enc_i: String = vm.call("enc_i", (42i64,)).unwrap();
    assert_eq!(enc_i, "42");
    let mut vm = vm_at(LIGHT);
    let dec_i: String = vm.call("dec_i", ("-17".to_string(),)).unwrap();
    assert_eq!(dec_i, "OK:-17");
    let mut vm = vm_at(LIGHT);
    let dec_o: String = vm.call("dec_o", ("\"x\"".to_string(),)).unwrap();
    assert_eq!(dec_o, "OK:x");
    let mut vm = vm_at(LIGHT);
    let dec_b: String = vm.call("dec_b", ("true".to_string(),)).unwrap();
    assert_eq!(dec_b, "OK:true");
    let mut vm = vm_at(LIGHT);
    let dec_f: String = vm.call("dec_f", ("0.1".to_string(),)).unwrap();
    assert_eq!(dec_f, "OK:0.1");
    let mut vm = vm_at(LIGHT);
    let dec_a: String = vm.call("dec_a", ("[3,4]".to_string(),)).unwrap();
    assert_eq!(dec_a, "OK:len=2:3:4");
    let mut vm = vm_at(LIGHT);
    let dec_q: String = vm.call("dec_q", ("null".to_string(),)).unwrap();
    assert_eq!(dec_q, "nil");
    let mut vm = vm_at(LIGHT);
    // enc_a's own return is the nullable ?str (the entry surface's
    // Option crossing); read it as an Option
    let arr: Option<String> = vm.call("enc_a", ()).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(arr, Some("[1,2]".to_string()));
}

// ---- the user-type lane: the fixture's DocRow satisfies both
// interfaces on its own inherent impl (the structural fork's whole
// point); the boundary check admits it at the entries ----

#[test]
fn user_type_satisfies_structurally() {
    let mut vm = vm_at(PKG);
    let dec: String = vm.call("dec_row", (r#"{"id":7,"name":"seven"}"#.to_string(),)).unwrap();
    assert_eq!(dec, "OK:7:seven");
}

// ---- the json-perf batch phase 1: the reader's per-byte classification
// and the writer's chunk-buffer shape (unit pins through the public
// surface; the Rust side computes the expectations independently) ----

/// the expected per-codepoint class for `classify_one`, computed HERE.
/// The rut ladder is: digit probe, ws probe, in-string probe — so the
/// ws trio answers 'w' even though it is ALSO a raw control in-string
/// (the dual classification the LUT experiment's bit 1|16 had to
/// carry; the probe pins the observable behavior, not the mechanism):
///   d digit (0-9), w whitespace, q string terminator,
///   c raw control (in-string), e the escape char, . content
fn expect_class(cp: u32) -> char {
    if (0x30..=0x39).contains(&cp) {
        return 'd';
    }
    match cp {
        // the quote and the backslash are in-string classes first
        0x22 => return 'q',
        0x5C => return 'e',
        // whitespace (the ws probe fires before the in-string one)
        0x09 | 0x0A | 0x0D | 0x20 => return 'w',
        _ => {}
    }
    if cp < 0x20 {
        return 'c';
    }
    '.'
}

#[test]
fn reader_byte_classification_total() {
    let mut want = String::new();
    for cp in 0..=255u32 {
        want.push(expect_class(cp));
    }
    let mut vm = vm_at(PKG);
    let out: String = vm.call("classify_report", ()).unwrap();
    assert_eq!(out.len(), 256, "one class char per codepoint");
    assert_eq!(out, want, "total byte classification");
}

#[test]
fn reader_byte_classification_more() {
    // the array-context classification: the pair survives the byte
    // (comma, whitespace, and bytes the element grammar merges: digits
    // and `-`), `]` closes cleanly, everything else is rejected — with
    // the recorded honesty that a wrong closer and element-missed
    // content share the rejection bucket (the element decoder's
    // opinion follows more()'s "other" class on the public surface)
    let mut want = String::new();
    for cp in 0..=255u32 {
        want.push(match cp {
            0x09 | 0x0A | 0x0D | 0x20 | 0x2C | 0x2D | 0x30..=0x39 => ',',
            0x5D => ']',
            _ => '.',
        });
    }
    let mut vm = vm_at(PKG);
    let out: String = vm.call("classify_more_report", ()).unwrap();
    assert_eq!(out.len(), 256, "one class char per codepoint");
    assert_eq!(out, want, "total comma/closer classification");
}

#[test]
fn writer_chunk_shapes_output_identical() {
    // the oracle is built HERE from the serde model, independently of
    // the writer's internal chunking: `{"big":[0,..,399],"nest":{..}}`
    let mut big = String::from("[");
    for i in 0..400 {
        if i > 0 {
            big.push(',');
        }
        big.push_str(&i.to_string());
    }
    big.push(']');
    let want = format!(
        "OK:{{\"big\":{},\"nest\":{{\"a\":[[\"x\\\"y\",null],{{}}],\"s\":\"tab\\there\\n\"}},\"empty\":{{}}}}",
        big
    );
    let mut vm = vm_at(PKG);
    let out: String = vm.call("writer_chunk_shapes", ()).unwrap();
    assert_eq!(out, want, "chunked writer output byte-identical");
}

#[test]
fn writer_chunk_flat_stream_and_dangling() {
    // a containerless stream crosses the cap too; the top-level shape
    // concatenates values with no separators (the caller's contract)
    let mut flat = String::new();
    for i in 0..300 {
        flat.push_str(&i.to_string());
    }
    let mut vm = vm_at(PKG);
    let out: String = vm.call("writer_flat_stream", ()).unwrap();
    assert_eq!(out, format!("OK:{flat}"));

    // the unbalanced clamp: a dangling key survives in the chunk
    let mut vm = vm_at(PKG);
    let out: String = vm.call("writer_finish_dangling", ()).unwrap();
    assert_eq!(out, "RAW:{\"dangling\":");
}

#[test]
fn writer_chunk_error_offsets() {
    // NotFinite after a field run: at counts the drained chunk too —
    // `{"a":1,"bad":` is 13 bytes
    let mut vm = vm_at(PKG);
    let out: String = vm.call("writer_not_finite_after_run", (f64::INFINITY,)).unwrap();
    assert_eq!(out, "ERR:NotFinite@13:$.bad");

    // Depth with a pending chunk: the outer object counts toward the
    // cap, so the 128th bracket fails the sticky model and the pending
    // key never emits: `{"x":` + 127 brackets = 132
    let mut vm = vm_at(PKG);
    let out: String = vm.call("writer_depth_key_offset", ()).unwrap();
    assert!(
        out.starts_with("ERR:Depth@132:$.x[1]"),
        "depth offset through the chunk: {out}"
    );
}
