//! 02-digest — a Rust app embedding rut.
//!
//! `digest.rut` is the application: byte-level encodings (hex, base64),
//! crypto digests (MD5, SHA-1, SHA-256, SHA-512), hashmap hash keys
//! (CRC-32, FNV-1a 32/64, djb2, sdbm), and a JSON codec — everything
//! flowing over `bytes`/`str`/`Opaque`, the shapes RFC 0023 §2
//! lets cross the host boundary.
//!
//! This file is the embedder AND the oracle: every rut result below is
//! checked against independent Rust — the RustCrypto hash crates,
//! `base64`, `crc32fast`, and `serde_json` (plus three-line references
//! for FNV/djb2/sdbm, which have no canonical crate) — and the demo
//! prints the verdict per row. Nothing in `digest.rut` knows about the
//! crates; only the host compares.

use std::rc::Rc;

use base64::Engine as _;
use md5::Digest as _;
use rut_vm::heap::Value;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn md5_hex(b: &[u8]) -> String {
    hex(&md5::Md5::digest(b))
}
fn sha1_hex(b: &[u8]) -> String {
    hex(&sha1::Sha1::digest(b))
}
fn sha256_hex(b: &[u8]) -> String {
    hex(&sha2::Sha256::digest(b))
}
fn sha512_hex(b: &[u8]) -> String {
    hex(&sha2::Sha512::digest(b))
}

// the hash-key trio with no canonical crate: three-line references
fn fnv1a32(b: &[u8]) -> u32 {
    let mut h = 0x811c9dc5u32;
    for &x in b {
        h ^= x as u32;
        h = h.wrapping_mul(16777619);
    }
    h
}
fn fnv1a64(b: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
fn djb2(b: &[u8]) -> u64 {
    let mut h = 5381u64;
    for &x in b {
        h = h.wrapping_mul(33).wrapping_add(x as u64);
    }
    h
}
fn sdbm(b: &[u8]) -> u64 {
    let mut h = 0u64;
    for &x in b {
        h = (x as u64).wrapping_add(h.wrapping_mul(65599));
    }
    h
}

// entry shorthands (the session pattern from 00-todolist / 01-sort)
fn as_str(v: Value) -> String {
    let Value::Str(s) = v else { unreachable!("{v:?}") };
    s
}
fn as_err(v: Value) -> String {
    // v1.1 error convention: `(T, err)` — position 1 carries the message
    let Value::Tuple(parts) = v else { unreachable!("{v:?}") };
    match &parts[1] {
        Value::Str(s) => s.clone(),
        v => unreachable!("{v:?}"),
    }
}
fn as_u64(v: Value) -> u64 {
    let Value::I64(x) = v else { unreachable!("{v:?}") };
    x as u64
}

fn main() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/digest.rut")).unwrap();
    // the app's libs: core+calc (engine) and `pouch` — a third-party pkg
    // the app declares, mounted from the toolchain tree
    let mut session = rut_driver::Session::new();
    rut_driver::mount_std(&mut session);
    rut_driver::mount_dir(
        &mut session,
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/pouch")),
    )
    .expect("mount pouch");
    let out = rut_driver::compile_module_in(&mut session, &src, rut_parser::Mode::Impl, "digests");
    assert!(out.diags.is_empty());
    let prog = rut_core::binary::decode(out.binary.as_deref().unwrap()).unwrap();
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(50_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default()).unwrap();
    rut_std::math::install_std_math(&mut vm);
    vm.verify_host_fns(&session.expected_host_fns()); // calc: .d.rut ↔ bodies
    let call = |vm: &mut rut_vm::interp::Vm, name: &str, args: &[Value]| vm.call(name, args).unwrap();

    // ---- hex + base64 read-back -------------------------------------
    let msg = b"rut!";
    println!("hex({msg:?}) = {}", as_str(call(&mut vm, "hex_enc", &[Value::Bytes(msg.to_vec())])));
    for (data, url) in [(b"foobar" as &[u8], false), (b"foobar" as &[u8], true), (b"ru" as &[u8], false)] {
        let rut = as_str(call(&mut vm, "b64_enc", &[Value::Bytes(data.to_vec()), Value::Bool(url)]));
        let want = if url {
            base64::engine::general_purpose::URL_SAFE.encode(data)
        } else {
            base64::engine::general_purpose::STANDARD.encode(data)
        };
        println!("b64({data:?}, url={url}) = {rut}  {}", verdict(rut == want));
    }

    // ---- the oracle table: rut vs the crates ------------------------
    let blob = lcg(1024, 42);
    let inputs: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("\"abc\"", b"abc".to_vec()),
        ("1 KiB lcg(42)", blob),
    ];
    println!("digests, rut vs crates:");
    let mut all_ok = true;
    for (label, data) in &inputs {
        for (algo, want) in [
            ("md5", md5_hex(data)),
            ("sha1", sha1_hex(data)),
            ("sha256", sha256_hex(data)),
            ("sha512", sha512_hex(data)),
        ] {
            let before = vm.fuel_used;
            let got = match call(&mut vm, "digest", &[Value::Str(algo.into()), Value::Bytes(data.clone())]) {
                Value::Tuple(parts) => match &parts[0] {
                    Value::Bytes(v) => v.clone(),
                    v => unreachable!("{v:?}"),
                },
                v => unreachable!("{v:?}"),
            };
            let ok = hex(&got) == want;
            all_ok &= ok;
            println!("  {algo:<7} {label:<13} fuel {:>7}  {}  {}", vm.fuel_used - before, hex(&got), verdict(ok));
        }
    }

    // ---- hash keys over real data: a JSON document ------------------
    let doc = match call(&mut vm, "sample_doc", &[]) {
        Value::Opaque(c) => c,
        v => unreachable!("{v:?}"),
    };
    let json_text = as_str(call(&mut vm, "json_enc", &[Value::Opaque(doc)]));
    println!("sample_doc -> {json_text}");
    println!("serde_json parses it: {}", verdict(serde_json::from_str::<serde_json::Value>(&json_text).is_ok()));
    let data = json_text.as_bytes();
    println!("hash keys over that JSON text:");
    let rows: Vec<(&str, u64, u64)> = vec![
        ("crc32", as_u64(call(&mut vm, "crc32", &[Value::Bytes(data.to_vec())])) & 0xFFFFFFFF, crc32fast::hash(data) as u64),
        ("fnv1a32", as_u64(call(&mut vm, "fnv1a32", &[Value::Bytes(data.to_vec())])), fnv1a32(data) as u64),
        ("fnv1a64", as_u64(call(&mut vm, "fnv1a64", &[Value::Bytes(data.to_vec())])), fnv1a64(data)),
        ("djb2", as_u64(call(&mut vm, "djb2", &[Value::Bytes(data.to_vec())])), djb2(data)),
        ("sdbm", as_u64(call(&mut vm, "sdbm", &[Value::Bytes(data.to_vec())])), sdbm(data)),
    ];
    let wide = rows.iter().any(|(n, _, _)| *n != "crc32" && *n != "fnv1a32");
    for (name, got, want) in rows {
        all_ok &= got == want;
        let fmt = |x: u64| if wide && name != "crc32" && name != "fnv1a32" { format!("{x:016x}") } else { format!("{x:08x}") };
        println!("  {name:<7} {}  {}", fmt(got), verdict(got == want));
    }

    // ---- JSON round-trip --------------------------------------------
    let tricky = r#"{"a":[1,2.5,-3e2],"s":"quote \" back \\ nl \n end","n":null,"t":true,"f":false,"o":{"x":[]}}"#;
    let rt = match call(&mut vm, "json_roundtrip", &[Value::Str(tricky.into())]) {
        Value::Tuple(parts) => match &parts[0] {
            Value::Str(s) => s.clone(),
            v => unreachable!("{v:?}"),
        },
        v => unreachable!("{v:?}"),
    };
    let equal = serde_json::from_str::<serde_json::Value>(&rt).unwrap() == serde_json::from_str::<serde_json::Value>(tricky).unwrap();
    all_ok &= equal;
    println!("json_roundtrip(tricky) = {rt}");
    println!("  semantically equal to serde_json: {}", verdict(equal));

    // ---- error paths are values, not traps --------------------------
    println!("hex_dec(\"zz\")   = {:?}", as_err(call(&mut vm, "hex_dec", &[Value::Str("zz".into())])));
    println!("b64_dec(\"!*\")   = {:?}", as_err(call(&mut vm, "b64_dec", &[Value::Str("!*".into()), Value::Bool(false)])));
    println!("json_dec(\"{{,}}\") = {:?}", as_err(call(&mut vm, "json_dec", &[Value::Str("{,}".into())])));
    println!("digest(\"md4\")   = {:?}", as_err(call(&mut vm, "digest", &[Value::Str("md4".into()), Value::Bytes(b"x".to_vec())])));

    println!("every row agrees: {}", verdict(all_ok));
    println!("fuel used: {} of {:?}", vm.fuel_used, limits.fuel);
}

fn verdict(ok: bool) -> &'static str {
    if ok { "OK" } else { "MISMATCH" }
}

/// deterministic LCG bytes — the same generator `fill` uses in 01-sort
fn lcg(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed | 1;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        x = x.wrapping_mul(1664525).wrapping_add(1013904223);
        out.push((x >> 16) as u8);
    }
    out
}
