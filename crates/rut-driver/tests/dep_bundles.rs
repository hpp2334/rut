//! Bundle v3 (RFC 0038 §4 + RFC 0045 §3, phase 2): peer groups ride the
//! archive. The packer emits `format_version = 4` and packs each
//! package's `[peer-deps]` `lib` files beside its entry; the v3 loader
//! resolves groups by name exactly like a directory world, then runs
//! the peer gate over the archive — peer present → the group file is
//! read from the zip and appended (a declared group the archive lacks
//! is a load error: refuse, never guess); optional peer absent →
//! inert; required peer absent → the loud D1 error.
//!
//! T12 pins the round trip on the hermetic peers world
//! (`tests/data/peers/`): the T2 world (one group) and the T3 world
//! (both groups) pack, load, compile BIT-IDENTICALLY to their
//! directory forms, and dispatch through the VM — plus the refusal
//! gates: a v2-era loader refuses a v3 bundle by the same unknown-
//! version gate pinned here, a peer-deps manifest under `format_version
//! = 2` is refused (the v2 layout has no group entries), and an old v2
//! bundle still loads.

use std::path::Path;

use rut_driver::{load_bundle_bytes, load_dir_session, pack_dir};

const PEERS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/peers");

const POUCH_GROUP: &str = "impl JsonSerialize for Vec<T>";
const NMAPSET_GROUP: &str = "impl JsonSerialize for Map<K, V>";

fn pack(rel: &str) -> Vec<u8> {
    pack_dir(Path::new(&format!("{PEERS}/{rel}"))).expect("pack")
}

fn load_bytes(bytes: &[u8]) -> Result<(rut_driver::Session, String), String> {
    load_bundle_bytes(bytes, Path::new("mem"))
}

fn linked_binary(session: &rut_driver::Session, root: &str) -> Vec<u8> {
    let g = rut_driver::compile_graph(session, root);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    rut_core::binary::encode(&g.program.expect("linked program"))
}

/// Flatten, verify, run an entry fn — its rut `str` result.
fn run_entry(session: rut_driver::Session, root: &str, entry: &str) -> String {
    let g = rut_driver::compile_graph(&session, root);
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let flat = rut_core::link::flatten(g.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    vm.call::<_, String>(entry, ()).expect("run")
}

#[test]
fn t12_pack_load_run_t2_world() {
    // matrix row 4 through the bundle path: the consumer supplies
    // pouch; the packed v3 archive carries json's group file; the
    // loader mounts the group by presence; the impl dispatches.
    let bytes = pack("cons_pouch");
    // determinism: same dir ⇒ byte-identical bundle (RFC 0038 §3)
    assert_eq!(bytes, pack("cons_pouch"), "same dir => byte-identical bundle");

    // the archive carries the group files beside the entry, per group
    let names: Vec<String> =
        rut_driver::parse_bundle(&bytes).unwrap().into_iter().map(|(n, _)| n).collect();
    for key in
        ["rut.toml", "cons.rut", "json/rut.toml", "json/json.rut", "json/serde_pouch.rut",
         "json/serde_nmapset.rut", "pouch/rut.toml", "pouch/pouch.rut", "base/rut.toml",
         "base/base.rut"]
    {
        assert!(names.contains(&key.to_string()), "the bundle must carry `{key}`: {names:?}");
    }

    // the packed form mounts the group by presence — and only that one
    let (session, root) = load_bytes(&bytes).expect("load v3 bundle");
    assert_eq!(root, "cons_pouch");
    let json_src = session.resolve("json").expect("json mounted").source.clone().expect("source");
    assert!(json_src.contains(POUCH_GROUP), "the pouch group must mount from the archive");
    assert!(!json_src.contains(NMAPSET_GROUP), "nmapset is absent — inert");

    // the group text came from the archive, byte-for-byte the fixture's
    let group = std::fs::read_to_string(Path::new(&format!("{PEERS}/json/serde_pouch.rut")))
        .expect("fixture group");
    assert!(json_src.contains(group.trim_end()), "the archive's group text rides verbatim");

    // the packed world compiles BIT-IDENTICALLY to the directory world
    let (dir_session, dir_root) = load_dir_session(Path::new(&format!("{PEERS}/cons_pouch")))
        .expect("dir load");
    assert_eq!(
        linked_binary(&dir_session, &dir_root),
        linked_binary(&session, &root),
        "dir and bundle compile identically"
    );

    // and the dispatch runs: the group's impl answers through the trait
    assert_eq!(run_entry(session, &root, "go"), "[vec]");
}

#[test]
fn t12_pack_load_run_t3_world() {
    // both peers present: BOTH groups ride the archive and mount
    // through the bundle path, in peer-name order after the base; both
    // impl sets dispatch.
    let bytes = pack("cons_both");
    let (session, root) = load_bytes(&bytes).expect("load v3 bundle");
    assert_eq!(root, "cons_both");
    let json_src = session.resolve("json").expect("json mounted").source.clone().expect("source");
    let a = json_src.find(NMAPSET_GROUP).expect("nmapset group mounted");
    let b = json_src.find(POUCH_GROUP).expect("pouch group mounted");
    assert!(a < b, "groups mount in peer-name (BTreeMap) order");

    let (dir_session, dir_root) =
        load_dir_session(Path::new(&format!("{PEERS}/cons_both"))).expect("dir load");
    assert_eq!(
        linked_binary(&dir_session, &dir_root),
        linked_binary(&session, &root),
        "dir and bundle compile identically"
    );
    assert_eq!(run_entry(session.clone(), &root, "go_vec"), "[vec]");
    assert_eq!(run_entry(session, &root, "go_map"), "[map]");
}

#[test]
fn t12_refuse_never_guess() {
    // --- the v2-era refusal, proven by its mechanism: an older loader
    // refuses a v3 bundle through the UNKNOWN-VERSION gate — the same
    // line this loader applies to a version it does not know, before
    // reading anything else (RFC 0038 §4). ---
    let entries: Vec<(String, Vec<u8>)> = vec![
        (
            "rut.toml".into(),
            "format = \"rutbundle\"\nformat_version = 5\nname = \"x\"\nentry.lib = \"./x.rut\"\n"
                .as_bytes().to_vec(),
        ),
        ("x.rut".into(), b"fn main() -> i32 { return 7; }\n".to_vec()),
    ];
    let err = load_bytes(&rut_driver::write_bundle(&entries).unwrap()).unwrap_err();
    assert!(err.contains("format_version"), "{err}");
    assert!(err.contains("1, 2, 3 and 4"), "{err}");

    // --- a peer-deps manifest under format_version = 2 is refused: the
    // v2 layout has no group entries, so loading it would silently
    // mount base-only — semantically wrong. ---
    let v2_with_peers: Vec<(String, Vec<u8>)> = vec![
        (
            "rut.toml".into(),
            "format = \"rutbundle\"\nformat_version = 2\nname = \"j\"\nentry.lib = \"./j.rut\"\n\n[peer-deps]\npouch = { path = \"../pouch\", optional = true, lib = \"./serde_pouch.rut\" }\n"
                .as_bytes().to_vec(),
        ),
        ("j.rut".into(), b"pub trait T { fn t(self) -> str; }\n".to_vec()),
        ("serde_pouch.rut".into(), b"".to_vec()),
    ];
    let err = load_bytes(&rut_driver::write_bundle(&v2_with_peers).unwrap()).unwrap_err();
    assert!(err.contains("needs bundle format_version 3"), "{err}");

    // --- back-compat half of the ledger: an honest v2 bundle (no peer
    // tables) still loads. ---
    let v2: Vec<(String, Vec<u8>)> = vec![
        (
            "rut.toml".into(),
            "format = \"rutbundle\"\nformat_version = 2\nname = \"m\"\nentry.lib = \"./m.rut\"\n[deps]\nd = { path = \"../d\" }\n"
                .as_bytes().to_vec(),
        ),
        ("m.rut".into(), b"pub fn seven() -> i32 { return 7; }\n".to_vec()),
        ("d/rut.toml".into(), b"name = \"d\"\nentry.lib = \"./d.rut\"\n".to_vec()),
        ("d/d.rut".into(), b"pub fn four() -> i32 { return 4; }\n".to_vec()),
    ];
    let (session, root) = load_bytes(&rut_driver::write_bundle(&v2).unwrap()).expect("v2 loads");
    assert_eq!(root, "m");
    assert!(session.resolve("d").is_ok());

    // --- the §4-consistency row: a declared group the archive does not
    // carry is a load error, never a silent base-only mount. ---
    let bytes = pack("cons_pouch");
    let stripped: Vec<(String, Vec<u8>)> = rut_driver::parse_bundle(&bytes)
        .unwrap()
        .into_iter()
        .filter(|(n, _)| n != "json/serde_pouch.rut")
        .collect();
    let err = load_bytes(&rut_driver::write_bundle(&stripped).unwrap()).unwrap_err();
    assert!(err.contains("json/serde_pouch.rut"), "{err}");

    // --- D1 through the bundle: a REQUIRED peer absent from the
    // archive's groups is the loud mount error naming pkg + peer + fix. ---
    let req = pack("cons_req");
    let err = load_bytes(&req).unwrap_err();
    assert!(
        err.contains("pkg `json_required` requires the peer `nmapset`"),
        "{err}"
    );
    assert!(err.contains("peers are not pulled transitively"), "{err}");
    assert!(err.contains("(RFC 0045 §3)"), "{err}");
}
