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
fn rut_hex_enc(vm: &mut rut_vm::interp::Vm, data: &[u8]) -> String {
    vm.call_typed("hex_enc", (data.to_vec(),)).unwrap()
}

fn rut_b64_enc(vm: &mut rut_vm::interp::Vm, data: &[u8], url: bool) -> String {
    vm.call_typed("b64_enc", (data.to_vec(), url)).unwrap()
}

fn rut_crc32(vm: &mut rut_vm::interp::Vm, data: &[u8]) -> u32 {
    vm.call_typed("crc32", (data.to_vec(),)).unwrap()
}

fn rut_fnv1a32(vm: &mut rut_vm::interp::Vm, data: &[u8]) -> u32 {
    vm.call_typed("fnv1a32", (data.to_vec(),)).unwrap()
}

fn rut_fnv1a64(vm: &mut rut_vm::interp::Vm, data: &[u8]) -> u64 {
    vm.call_typed("fnv1a64", (data.to_vec(),)).unwrap()
}

fn rut_djb2(vm: &mut rut_vm::interp::Vm, data: &[u8]) -> u64 {
    vm.call_typed("djb2", (data.to_vec(),)).unwrap()
}

fn rut_sdbm(vm: &mut rut_vm::interp::Vm, data: &[u8]) -> u64 {
    vm.call_typed("sdbm", (data.to_vec(),)).unwrap()
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
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_std::math::install_std_math(&mut hosts);
    hosts.verify_against(&session.expected_host_fns()); // calc: .d.rut ↔ bodies
    let mut vm = rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts).unwrap();

    // ---- hex + base64 read-back -------------------------------------
    let msg = b"rut!";
    println!("hex({msg:?}) = {}", rut_hex_enc(&mut vm, msg));
    for (data, url) in [(b"foobar" as &[u8], false), (b"foobar" as &[u8], true), (b"ru" as &[u8], false)] {
        let rut = rut_b64_enc(&mut vm, data, url);
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
            // v1.1 error convention: `(T, err)` — the tuple crosses typed
            let (got, _err): (Vec<u8>, String) = vm.call_typed("digest", (algo, data.clone())).unwrap();
            let ok = hex(&got) == want;
            all_ok &= ok;
            println!("  {algo:<7} {label:<13} fuel {:>7}  {}  {}", vm.fuel_used - before, hex(&got), verdict(ok));
        }
    }

    // ---- hash keys over real data: a JSON document ------------------
    let doc: rut_vm::OpaqueRef = vm.call_typed("sample_doc", ()).unwrap();
    let json_text: String = vm.call_typed("json_enc", (doc,)).unwrap();
    println!("sample_doc -> {json_text}");
    println!("serde_json parses it: {}", verdict(serde_json::from_str::<serde_json::Value>(&json_text).is_ok()));
    let data = json_text.as_bytes();
    println!("hash keys over that JSON text:");
    let rows: Vec<(&str, u64, u64)> = vec![
        ("crc32", u64::from(rut_crc32(&mut vm, data)), crc32fast::hash(data) as u64),
        ("fnv1a32", u64::from(rut_fnv1a32(&mut vm, data)), fnv1a32(data) as u64),
        ("fnv1a64", rut_fnv1a64(&mut vm, data), fnv1a64(data)),
        ("djb2", rut_djb2(&mut vm, data), djb2(data)),
        ("sdbm", rut_sdbm(&mut vm, data), sdbm(data)),
    ];
    let wide = rows.iter().any(|(n, _, _)| *n != "crc32" && *n != "fnv1a32");
    for (name, got, want) in rows {
        all_ok &= got == want;
        let fmt = |x: u64| if wide && name != "crc32" && name != "fnv1a32" { format!("{x:016x}") } else { format!("{x:08x}") };
        println!("  {name:<7} {}  {}", fmt(got), verdict(got == want));
    }

    // ---- JSON round-trip --------------------------------------------
    let tricky = r#"{"a":[1,2.5,-3e2],"s":"quote \" back \\ nl \n end","n":null,"t":true,"f":false,"o":{"x":[]}}"#;
    let (rt, _err): (String, String) = vm.call_typed("json_roundtrip", (tricky,)).unwrap();
    let equal = serde_json::from_str::<serde_json::Value>(&rt).unwrap() == serde_json::from_str::<serde_json::Value>(tricky).unwrap();
    all_ok &= equal;
    println!("json_roundtrip(tricky) = {rt}");
    println!("  semantically equal to serde_json: {}", verdict(equal));

    // ---- error paths are values, not traps --------------------------
    let (_v, err): (Vec<u8>, String) = vm.call_typed("hex_dec", ("zz",)).unwrap();
    println!("hex_dec(\"zz\")   = {err:?}");
    let (_v, err): (Vec<u8>, String) = vm.call_typed("b64_dec", ("!*", false)).unwrap();
    println!("b64_dec(\"!*\")   = {err:?}");
    let (_o, err): (rut_vm::OpaqueRef, String) = vm.call_typed("json_dec", ("{,}",)).unwrap();
    println!("json_dec(\"{{,}}\") = {err:?}");

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
