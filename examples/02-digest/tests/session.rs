//! 02-digest — the session gate.
//!
//! The rut side is verified two independent ways: canonical test vectors
//! (RFC 1321, FIPS 180-4, RFC 4648, the CRC/FNV catalogues) hardcoded
//! below, and cross-checks against the md-5/sha1/sha2/base64/
//! crc32fast/serde_json crates on deterministic pseudo-random inputs —
//! including every padding-edge length the block digests have.

use std::rc::Rc;

use base64::Engine as _;
use md5::Digest as _;
use rut_vm::OpaqueRef;

const SRC: &str = include_str!("../digest.rut");

fn session(fuel: u64, heap: u64) -> rut_vm::interp::Vm {
    let mut s = rut_driver::Session::new();
    rut_driver::mount_std(&mut s);
    rut_driver::mount_dir(
        &mut s,
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/pouch"),
    )
    .expect("mount pouch");
    // json after pouch (the std order). Mounted LIGHT (the base only —
    // rut/json/rut.toml's own comment): the encode path uses json's
    // traits + writer directly and needs no peer group, so
    // `assemble_peers` is deliberately not called. (This DOM's children
    // are `Vec<opaque>`; the peer-gated `impl JsonSerialize for Vec<T>`
    // instantiated at T = opaque miscompiles — its element
    // `x.encode(w)` binds the `?T` row's uncompiled concrete twin and
    // the binary fails load-time verify. An engine-side
    // devirtualization gap, disclosed in the batch commit.)
    rut_driver::mount_dir(
        &mut s,
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/json"),
    )
    .expect("mount json");
    let out = rut_driver::compile_module_in(&mut s, SRC, rut_parser::Mode::Impl, "digests");
    assert!(out.diags.is_empty(), "{}", out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n"));
    let prog = rut_core::binary::decode(out.binary.as_deref().unwrap()).unwrap();
    rut_vm::verify::verify(&prog).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(fuel),
        heap_limit_bytes: Some(heap),
        interrupt_every: 1024,
    };
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_std::math::install_std_math(&mut hosts);
    hosts.verify_against(&s.expected_host_fns()); // calc: .d.rut ↔ bodies
    {
        let mut vm = rut_vm::interp::Vm::new(
            Rc::new(prog),
            &limits,
            rut_vm::interp::HostHooks::default(),
            hosts,
        )
        .unwrap();
        vm
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn lcg(n: usize, seed: u32) -> Vec<u8> {
    let mut x = seed | 1;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        x = x.wrapping_mul(1664525).wrapping_add(1013904223);
        out.push((x >> 16) as u8);
    }
    out
}

// entry shorthands — typed crossings (RFC 0023 revised)
fn rut_digest(vm: &mut rut_vm::interp::Vm, algo: &str, data: &[u8]) -> String {
    let (v, _e): (Vec<u8>, String) = vm.call("digest", (algo, data.to_vec())).unwrap();
    hex(&v)
}
fn rut_b64_enc(vm: &mut rut_vm::interp::Vm, data: &[u8], url: bool) -> String {
    vm.call("b64_enc", (data.to_vec(), url)).unwrap()
}
fn rut_b64_dec(vm: &mut rut_vm::interp::Vm, s: &str, url: bool) -> Vec<u8> {
    let (v, _e): (Vec<u8>, String) = vm.call("b64_dec", (s, url)).unwrap();
    v
}
fn rut_hex_enc(vm: &mut rut_vm::interp::Vm, data: &[u8]) -> String {
    vm.call("hex_enc", (data.to_vec(),)).unwrap()
}
fn rut_hex_dec(vm: &mut rut_vm::interp::Vm, s: &str) -> Vec<u8> {
    let (v, _e): (Vec<u8>, String) = vm.call("hex_dec", (s,)).unwrap();
    v
}
fn rut_crc32(vm: &mut rut_vm::interp::Vm, data: &[u8]) -> u32 {
    vm.call("crc32", (data.to_vec(),)).unwrap()
}
fn rut_fnv1a32(vm: &mut rut_vm::interp::Vm, data: &[u8]) -> u32 {
    vm.call("fnv1a32", (data.to_vec(),)).unwrap()
}
fn rut_fnv1a64(vm: &mut rut_vm::interp::Vm, data: &[u8]) -> u64 {
    vm.call("fnv1a64", (data.to_vec(),)).unwrap()
}
fn rut_djb2(vm: &mut rut_vm::interp::Vm, data: &[u8]) -> u64 {
    vm.call("djb2", (data.to_vec(),)).unwrap()
}
fn rut_sdbm(vm: &mut rut_vm::interp::Vm, data: &[u8]) -> u64 {
    vm.call("sdbm", (data.to_vec(),)).unwrap()
}
fn rut_json_roundtrip(vm: &mut rut_vm::interp::Vm, s: &str) -> Result<String, String> {
    // v1.1 error convention: `(T, err)` — position 1 carries the message
    let (out, e): (String, String) = vm.call("json_roundtrip", (s,)).unwrap();
    if e.is_empty() { Ok(out) } else { Err(e) }
}
fn rut_sample_doc(vm: &mut rut_vm::interp::Vm) -> OpaqueRef {
    vm.call("sample_doc", ()).unwrap()
}
fn rut_json_enc(vm: &mut rut_vm::interp::Vm, o: OpaqueRef) -> String {
    vm.call("json_enc", (o,)).unwrap()
}
fn rut_direct(vm: &mut rut_vm::interp::Vm, algo: &str, data: &[u8]) -> String {
    let v: Vec<u8> = vm.call(algo, (data.to_vec(),)).unwrap();
    hex(&v)
}

// crate references
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

#[test]
fn official_vectors() {
    let mut vm = session(20_000_000, 64 * 1024 * 1024);
    // RFC 1321 appendix A.5 — the canonical MD5 suite
    for (msg, want) in [
        ("", "d41d8cd98f00b204e9800998ecf8427e"),
        ("a", "0cc175b9c0f1b6a831c399e269772661"),
        ("abc", "900150983cd24fb0d6963f7d28e17f72"),
        ("message digest", "f96b697d7cb7938d525a2f31aaf161d0"),
        ("abcdefghijklmnopqrstuvwxyz", "c3fcd3d76192e4007dfb496cca67e13b"),
        ("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789", "d174ab98d277d9f5a5611c2c9f419d9f"),
        ("12345678901234567890123456789012345678901234567890123456789012345678901234567890", "57edf4a22be3c955ac49da2e2107b67a"),
    ] {
        assert_eq!(rut_digest(&mut vm, "md5", msg.as_bytes()), want, "md5({msg:?})");
    }
    // FIPS 180-4 examples: empty, "abc", and the classic two-block message
    let two_block = "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
    let sha512_two_block = "abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu";
    assert_eq!(rut_digest(&mut vm, "sha1", b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(rut_digest(&mut vm, "sha256", b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(rut_digest(&mut vm, "sha512", b""), "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e");
    assert_eq!(rut_digest(&mut vm, "sha1", b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    assert_eq!(rut_digest(&mut vm, "sha256", b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(rut_digest(&mut vm, "sha512", b"abc"), "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f");
    assert_eq!(rut_digest(&mut vm, "sha1", two_block.as_bytes()), "84983e441c3bd26ebaae4aa1f95129e5e54670f1");
    assert_eq!(rut_digest(&mut vm, "sha256", two_block.as_bytes()), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
    assert_eq!(rut_digest(&mut vm, "sha512", sha512_two_block.as_bytes()), "8e959b75dae313da8cf4f72814fc143f8f7779c6eb9f7fa17299aeadb6889018501d289e4900f7e4331b99dec4b5433ac7d329eeb6dd26545e96e55b874be909");
}

#[test]
fn crate_cross_check_on_padding_edges() {
    let mut vm = session(100_000_000, 64 * 1024 * 1024);
    // every length class around the block/padding boundaries, plus randoms
    let lens: Vec<usize> = vec![0, 1, 2, 3, 54, 55, 56, 57, 63, 64, 65, 111, 112, 113, 119, 120, 127, 128, 129, 200];
    for (i, &n) in lens.iter().enumerate() {
        let data = lcg(n, 7 + i as u32);
        assert_eq!(rut_digest(&mut vm, "md5", &data), md5_hex(&data), "md5 len {n}");
        assert_eq!(rut_digest(&mut vm, "sha1", &data), sha1_hex(&data), "sha1 len {n}");
        assert_eq!(rut_digest(&mut vm, "sha256", &data), sha256_hex(&data), "sha256 len {n}");
        assert_eq!(rut_digest(&mut vm, "sha512", &data), sha512_hex(&data), "sha512 len {n}");
        // base64 both directions, both alphabets
        for url in [false, true] {
            let enc = rut_b64_enc(&mut vm, &data, url);
            let want = if url {
                base64::engine::general_purpose::URL_SAFE.encode(&data)
            } else {
                base64::engine::general_purpose::STANDARD.encode(&data)
            };
            assert_eq!(enc, want, "b64 enc url={url} len {n}");
            let dec = rut_b64_dec(&mut vm, &enc, url);
            assert_eq!(dec, data, "b64 dec url={url} len {n}");
        }
    }
}

#[test]
fn base64_rfc4648_and_errors() {
    let mut vm = session(1_000_000, 8 * 1024 * 1024);
    for (data, want) in [
        (&b""[..], ""),
        (b"f", "Zg=="),
        (b"fo", "Zm8="),
        (b"foo", "Zm9v"),
        (b"foob", "Zm9vYg=="),
        (b"fooba", "Zm9vYmE="),
        (b"foobar", "Zm9vYmFy"),
    ] {
        let got = rut_b64_enc(&mut vm, data, false);
        assert_eq!(got, want);
        let dec = rut_b64_dec(&mut vm, want, false);
        assert_eq!(dec, data);
    }
    // URL-safe alphabet differs exactly where +/ appear
    let hi = [0xFBu8, 0xFC, 0xFD, 0xFE, 0xFF];
    let std = rut_b64_enc(&mut vm, &hi, false);
    let url = rut_b64_enc(&mut vm, &hi, true);
    assert_eq!(std, base64::engine::general_purpose::STANDARD.encode(hi));
    assert_eq!(url, base64::engine::general_purpose::URL_SAFE.encode(hi));
    assert_ne!(std, url);
    // errors
    let (_v, e): (Vec<u8>, String) = vm.call("b64_dec", ("!*", false)).unwrap();
    assert!(e.contains("invalid"));
        let (_v, e): (Vec<u8>, String) = vm.call("b64_dec", ("Zg==Zg==", false)).unwrap();
    assert!(e.contains("after padding"));
}

#[test]
fn hex_roundtrip_and_errors() {
    let mut vm = session(1_000_000, 8 * 1024 * 1024);
    let data = lcg(257, 3);
    let enc = rut_hex_enc(&mut vm, &data);
    assert_eq!(enc, hex(&data));
    let dec = rut_hex_dec(&mut vm, &enc);
    assert_eq!(dec, data);
    // uppercase is accepted
    let up = rut_hex_enc(&mut vm, b"\xde\xad\xbe\xef");
    assert_eq!(up, "deadbeef");
    let dec = rut_hex_dec(&mut vm, "DEADBEEF");
    assert_eq!(dec, b"\xde\xad\xbe\xef".to_vec());
        let (_v, e): (Vec<u8>, String) = vm.call("hex_dec", ("zz",)).unwrap();
    assert!(e.contains("invalid"));
        let (_v, e): (Vec<u8>, String) = vm.call("hex_dec", ("abc",)).unwrap();
    assert!(e.contains("odd"));
}

#[test]
fn hash_key_vectors_and_cross_check() {
    let mut vm = session(5_000_000, 32 * 1024 * 1024);
    // catalogue anchors
    assert_eq!(rut_crc32(&mut vm, &[]), 0);
    assert_eq!(rut_crc32(&mut vm, b"123456789"), 0xCBF43926);
    assert_eq!(rut_crc32(&mut vm, b"The quick brown fox jumps over the lazy dog"), 0x414FA339);
    assert_eq!(rut_fnv1a32(&mut vm, &[]), 0x811C9DC5);
    assert_eq!(rut_fnv1a32(&mut vm, b"a"), 0xE40C292C);
    assert_eq!(rut_fnv1a32(&mut vm, b"foobar"), 0xBF9CF968);
    assert_eq!(rut_fnv1a64(&mut vm, &[]), 0xCBF29CE484222325);
    assert_eq!(rut_fnv1a64(&mut vm, b"a"), 0xAF63DC4C8601EC8C);
    assert_eq!(rut_fnv1a64(&mut vm, b"foobar"), 0x85944171F73967E8);
    assert_eq!(rut_djb2(&mut vm, &[]), 5381);
    assert_eq!(rut_djb2(&mut vm, b"a"), 5381 * 33 + 97);
    assert_eq!(rut_sdbm(&mut vm, b"a"), 97);
    // cross-check on pseudo-random blobs (crc32fast + local refs)
    let fnv32_ref = |b: &[u8]| {
        let mut h = 0x811c9dc5u32;
        for &x in b {
            h ^= x as u32;
            h = h.wrapping_mul(16777619);
        }
        h
    };
    let fnv64_ref = |b: &[u8]| {
        let mut h = 0xcbf29ce484222325u64;
        for &x in b {
            h ^= x as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    };
    let djb2_ref = |b: &[u8]| {
        let mut h = 5381u64;
        for &x in b {
            h = h.wrapping_mul(33).wrapping_add(x as u64);
        }
        h
    };
    let sdbm_ref = |b: &[u8]| {
        let mut h = 0u64;
        for &x in b {
            h = (x as u64).wrapping_add(h.wrapping_mul(65599));
        }
        h
    };
    for i in 0..20usize {
        let data = lcg(50 * i, 100 + i as u32);
        assert_eq!(rut_crc32(&mut vm, &data), crc32fast::hash(&data), "crc32 #{i}");
        assert_eq!(rut_fnv1a32(&mut vm, &data), fnv32_ref(&data), "fnv1a32 #{i}");
        assert_eq!(rut_fnv1a64(&mut vm, &data), fnv64_ref(&data), "fnv1a64 #{i}");
        assert_eq!(rut_djb2(&mut vm, &data), djb2_ref(&data), "djb2 #{i}");
        assert_eq!(rut_sdbm(&mut vm, &data), sdbm_ref(&data), "sdbm #{i}");
    }
}

#[test]
fn json_codec_against_serde() {
    let mut vm = session(200_000_000, 32 * 1024 * 1024);
    let roundtrip = |vm: &mut rut_vm::interp::Vm, s: &str| -> Result<String, String> {
        let (out, e): (String, String) = vm.call("json_roundtrip", (s,)).unwrap();
        if e.is_empty() { Ok(out) } else { Err(e) }
    };
    let docs = [
        r#"{}"#,
        r#"[]"#,
        r#"null"#,
        r#"true"#,
        r#"-42"#,
        r#"2.5"#,
        r#"-3e2"#,
        r#"1e-9"#,
        r#"0.0000000000000000000001"#,
        r#"{"a":1}"#,
        r#"{"a":[1,2,3],"b":{"c":null},"d":false}"#,
        r#"[[],[[]],[[[1]]]]"#,
        r#"  {  "spaced"  :  [  1 , 2 ]  }  "#,
        r#"{"s":"quote \" back \\ slash \/ solidus"}"#,
        r#"{"esc":"tab\there\nnl\rcr"}"#,
        r#"{"ctl":"bs\bfh\fdel"}"#,
        r#"{"uni":"héllo wörld — ✓"}"#,
        r#"{"nums":[0,-0,1e10,-2.5E-3]}"#,
        r#"{"deep":{"a":{"b":{"c":{"d":[1,{"e":2}]}}}}}"#,
    ];
    for doc in docs {
        let out = roundtrip(&mut vm, doc).unwrap_or_else(|e| panic!("{doc:?}: {e}"));
        let rut_v = serde_json::from_str::<serde_json::Value>(&out).unwrap_or_else(|e| panic!("{doc:?} -> {out:?}: {e}"));
        let ref_v = serde_json::from_str::<serde_json::Value>(doc).unwrap();
        assert_eq!(rut_v, ref_v, "{doc:?}");
        // canonical form is idempotent
        let again = roundtrip(&mut vm, &out).unwrap();
        assert_eq!(again, out, "idempotence for {doc:?}");
    }
    // canonical shape: compact, insertion order, minimal escapes
    assert_eq!(roundtrip(&mut vm, r#"{"b":1,"a":2}"#).unwrap(), r#"{"b":1,"a":2}"#);
    assert_eq!(roundtrip(&mut vm, " [ 1 , 2 ] ").unwrap(), "[1,2]");
    assert_eq!(roundtrip(&mut vm, r#""""#).unwrap(), r#""""#);
    // sample_doc encodes to valid JSON
    let doc: OpaqueRef = vm.call("sample_doc", ()).unwrap();
    let enc: String = vm.call("json_enc", (doc,)).unwrap();
    assert_eq!(enc, r#"{"name":"rut","version":0.2,"tags":["tiny","fast","verified"],"meta":{"ok":true,"lines":607}}"#);
    serde_json::from_str::<serde_json::Value>(&enc).unwrap();
    // error paths
    for (bad, needle) in [
        ("", "end of input"),
        ("{,}", "key string"),
        (r#"{"a":1} x"#, "trailing"),
        (r#"{"a""#, "expected"),
        (r#"{"a" 1}"#, "expected"),
        (r#"[1,2,]"#, "unexpected"),
        (r#"{"a":"\q"}"#, "escape"),
        (r#"{"a":"\u0041"}"#, "not supported"),
        (r#"{"a":+1}"#, "unexpected"),
        (r#"{"a":1.}"#, "malformed number"),
        (r#"{"a":12e}"#, "malformed number"),
        (r#"nul"#, "literal"),
        (r#"truex"#, "trailing"),
    ] {
        let err = roundtrip(&mut vm, bad).unwrap_err();
        assert!(err.contains(needle), "{bad:?}: {err}");
    }
    // nesting cap at 64
    let mut deep = String::new();
    for _ in 0..70 {
        deep.push('[');
    }
    assert!(roundtrip(&mut vm, &deep).unwrap_err().contains("nesting"));
}

#[test]
fn dispatcher_matches_direct_entries() {
    let mut vm = session(20_000_000, 32 * 1024 * 1024);
    let data = lcg(150, 11);
    for algo in ["md5", "sha1", "sha256", "sha512"] {
        let via_disp = rut_digest(&mut vm, algo, &data);
        let v: Vec<u8> = vm.call(algo, (data.clone(),)).unwrap();
        assert_eq!(via_disp, hex(&v), "{algo}");
    }
    let (_v, err): (Vec<u8>, String) = vm.call("digest", ("md4", b"x".to_vec())).unwrap();
    assert_eq!(err, "unknown algorithm: md4");
}

#[test]
fn stress_8k_under_raised_budgets() {
    // RFC 0040: budgets are the host's call — a real workload gets real
    // numbers, and the session still cannot exceed them. (Sized for the
    // debug-build interpreter; release is ~30x faster.)
    let mut vm = session(500_000_000, 256 * 1024 * 1024);
    let data = lcg(8 * 1024, 5);
    assert_eq!(rut_digest(&mut vm, "md5", &data), md5_hex(&data));
    assert_eq!(rut_digest(&mut vm, "sha256", &data), sha256_hex(&data));
    assert_eq!(rut_digest(&mut vm, "sha512", &data), sha512_hex(&data));
    assert!(vm.fuel_used < 500_000_000);

}

#[test]
fn wrong_kind_arguments_trap_cleanly() {
    let mut vm = session(1_000_000, 8 * 1024 * 1024);
    let err = vm.call::<_, u32>("crc32", ("not bytes",)).unwrap_err();
    assert!(err.msg.contains("argument"), "{err:?}");
    let err = vm.call::<_, (Vec<u8>, String)>("hex_dec", (vec![1u8],)).unwrap_err();
    assert!(err.msg.contains("argument"), "{err:?}");
}
