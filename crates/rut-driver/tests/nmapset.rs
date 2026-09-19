//! The `nmapset` package (the mapset-host plan, H3): generic wrapper
//! classes over the `nmap` host table — the host-backed twin of the
//! pure-rut `mapset`. The mapset scenarios port over with the SAME
//! checksums (the wrapper's `Hashable` uses mapset's exact mix64/FNV
//! constants, so slot behavior and value-derived checksums match);
//! adapted where the surface differs (`with_capacity` sizes through
//! the host's `map_cap`). PLUS the two host-experiment specifics
//! (phase 3): a user-defined key type is refused AT COMPILE TIME with
//! the union-bound diagnostic (the class bound is the visible contract
//! — escape hatches: encode to `bytes`, or use `mapset`), and `get`
//! parity — the same staleness
//! scenario run against BOTH packages must answer identically (the
//! native keys never change the `*V` aliasing law).

use std::rc::Rc;

use rut_driver::{Module, Session};

const NMAPSET_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/nmapset");
const MAPSET_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/mapset");

/// Mount `pkg` ("mapset" or "nmapset" — the latter pulls `nmap` +
/// `mapset` through its `[deps]`) and register `app_src` as the root.
fn session_with(pkg: &str, app_src: &str) -> Session {
    let mut session = Session::new();
    rut_driver::mount_std_core(&mut session);
    let dir = match pkg {
        "mapset" => MAPSET_DIR,
        "nmapset" => NMAPSET_DIR,
        other => panic!("unknown pkg {other}"),
    };
    rut_driver::mount_dir(&mut session, std::path::Path::new(dir))
        .expect("mount pkg");
    session
        .register_module(
            "app_main",
            Module { spec: "app_main".into(), source: Some(app_src.into()), ..Default::default() },
        )
        .unwrap();
    session
}

fn compile_pkg_app(pkg: &str, app_src: &str) -> rut_driver::GraphOutput {
    rut_driver::compile_graph(&session_with(pkg, app_src), "app_main")
}

fn diags_of(pkg: &str, app_src: &str) -> Vec<String> {
    compile_pkg_app(pkg, app_src)
        .diags
        .iter()
        .map(|d| d.msg.clone())
        .collect()
}

/// Compile, verify, and build the Vm with the host bindings the graph
/// declares — `install_std_nmap` for the nmapset side, checked against
/// the mounted `rut/nmap` surface (the RFC 0025 contract).
fn vm_for(pkg: &str, app_src: &str) -> rut_vm::interp::Vm {
    let session = session_with(pkg, app_src);
    let expected = session.expected_host_fns();
    if pkg == "nmapset" {
        for f in ["map_new", "map_entry", "map_find", "map_remove", "map_needs_grow",
                  "map_grow", "map_take_reloc", "map_cap", "map_len"]
        {
            assert!(
                expected.contains_key(&format!("nmap::{f}")),
                "the nmap surface must cross through the [deps] mount: {expected:?}"
            );
        }
    }
    let out = rut_driver::compile_graph(&session, "app_main");
    assert!(
        out.diags.is_empty(),
        "{}",
        out.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let prog = out.program.expect("linked program");
    rut_vm::verify::verify(&prog).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(20_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut hosts = rut_vm::interp::HostRegistry::new();
    if pkg == "nmapset" {
        rut_std::nmap::install_std_nmap(&mut hosts);
        hosts.verify_against(&expected); // rut/nmap/nmap.d.rut ↔ the bodies
    }
    rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts)
        .expect("vm")
}

/// Compile the app, flatten, verify, and run `main` — the i32 checksum.
fn run_main(pkg: &str, app_src: &str) -> i32 {
    vm_for(pkg, app_src).call::<_, i32>("main", ()).expect("run")
}

// ---- the ported mapset scenarios (checksums must match mapset's) ----

/// mapset's i32-key scenario verbatim (minus its unused Pt prelude):
/// 64 puts across the grow (cap 8 → 128), gets, replace, remove,
/// DEAD-slot reuse, a negative key. `vals` must survive every
/// relocation the native grow drives.
#[test]
fn hashmap_i32_keys_grow_get_replace_remove_and_reuse() {
    let checksum = run_main("nmapset", "use core::{ assert };\n\
         use nmapset::{ HashMap };\n\
         pub fn main() -> i32 {\n\
         \x20   let mut acc = 0;\n\
         \x20   let mut m: HashMap<i32, str> = HashMap.new();\n\
         \x20   for (let i = 0; i < 64; i += 1) {\n\
         \x20       assert(m.put(i, f\"v{i}\"), \"i32 map: first put must add\");\n\
         \x20   }\n\
         \x20   assert(m.len() == 64, \"i32 map: len after 64 puts\");\n\
         \x20   for (let i = 0; i < 64; i += 1) {\n\
         \x20       assert(m.has(i), \"i32 map: has after put\");\n\
         \x20       let p = m.get(i);\n\
         \x20       assert(p != nil, \"i32 map: get after put\");\n\
         \x20       assert(p == f\"v{i}\", \"i32 map: value round-trips\");\n\
         \x20   }\n\
         \x20   acc = acc + m.len();\n\
         \x20   assert(m.put(5, \"new\") == false, \"i32 map: replace answers false\");\n\
         \x20   assert(m.get(5) == \"new\", \"i32 map: replace took\");\n\
         \x20   assert(m.len() == 64, \"i32 map: replace keeps len\");\n\
         \x20   acc = acc + 1;\n\
         \x20   assert(m.remove(6), \"i32 map: remove answers true\");\n\
         \x20   assert(m.remove(6) == false, \"i32 map: second remove answers false\");\n\
         \x20   assert(m.has(6) == false, \"i32 map: removed key is absent\");\n\
         \x20   assert(m.get(6) == nil, \"i32 map: removed key gets nil\");\n\
         \x20   assert(m.len() == 63, \"i32 map: len after remove\");\n\
         \x20   assert(m.put(6, \"again\"), \"i32 map: re-insert adds (DEAD reuse)\");\n\
         \x20   assert(m.get(6) == \"again\", \"i32 map: re-insert value\");\n\
         \x20   acc = acc + 2;\n\
         \x20   assert(m.get(1000) == nil, \"i32 map: never-key gets nil\");\n\
         \x20   assert(m.put(-7, \"neg\"), \"i32 map: negative key adds\");\n\
         \x20   assert(m.get(-7) == \"neg\", \"i32 map: negative key round-trips\");\n\
         \x20   acc = acc + m.len();\n\
         \x20   return acc;\n\
         }\n");
    assert_eq!(checksum, 132, "must equal the mapset scenario's checksum");
}

/// mapset's str-key + HashSet scenario verbatim: content-equal str keys
/// (the host stores owned copies; equality is octet equality), the set
/// with no value machinery, duplicate adds, removes.
#[test]
fn hashmap_str_keys_and_hashset() {
    let checksum = run_main("nmapset", "use core::{ assert };\n\
         use nmapset::{ HashMap, HashSet };\n\
         pub fn main() -> i32 {\n\
         \x20   let mut acc = 0;\n\
         \x20   let mut sm: HashMap<str, i32> = HashMap.new();\n\
         \x20   for (let i = 0; i < 48; i += 1) {\n\
         \x20       assert(sm.put(f\"k{i}\", i * 2), \"str map: first put adds\");\n\
         \x20   }\n\
         \x20   assert(sm.len() == 48, \"str map: len\");\n\
         \x20   for (let i = 0; i < 48; i += 1) {\n\
         \x20       let p = sm.get(f\"k{i}\");\n\
         \x20       assert(p != nil, \"str map: hit found\");\n\
         \x20       assert(p == i * 2, \"str map: hit round-trips\");\n\
         \x20   }\n\
         \x20   assert(sm.get(\"nope\") == nil, \"str map: miss\");\n\
         \x20   assert(sm.put(f\"k10\", 999) == false, \"str map: replace answers false\");\n\
         \x20   assert(sm.get(f\"k10\") == 999, \"str map: replace took\");\n\
         \x20   assert(sm.remove(f\"k3\"), \"str map: remove\");\n\
         \x20   assert(sm.has(f\"k3\") == false, \"str map: removed is absent\");\n\
         \x20   acc = acc + sm.len();\n\
         \x20   let mut s: HashSet<i32> = HashSet.new();\n\
         \x20   for (let i = 0; i < 50; i += 1) {\n\
         \x20       assert(s.put(i), \"set: first add\");\n\
         \x20   }\n\
         \x20   assert(s.len() == 50, \"set: len\");\n\
         \x20   for (let i = 0; i < 50; i += 1) {\n\
         \x20       assert(s.put(i) == false, \"set: dup add answers false\");\n\
         \x20   }\n\
         \x20   assert(s.len() == 50, \"set: dup adds keep len\");\n\
         \x20   assert(s.has(37), \"set: has\");\n\
         \x20   assert(s.has(50) == false, \"set: absent\");\n\
         \x20   assert(s.remove(37), \"set: remove\");\n\
         \x20   assert(s.remove(37) == false, \"set: second remove\");\n\
         \x20   acc = acc + s.len();\n\
         \x20   let mut ss: HashSet<str> = HashSet.new();\n\
         \x20   for (let i = 0; i < 20; i += 1) {\n\
         \x20       assert(ss.put(f\"s{i}\"), \"str set: first add\");\n\
         \x20   }\n\
         \x20   assert(ss.has(\"s7\"), \"str set: has\");\n\
         \x20   assert(ss.put(\"s7\") == false, \"str set: dup\");\n\
         \x20   assert(ss.remove(\"s8\"), \"str set: remove\");\n\
         \x20   acc = acc + ss.len();\n\
         \x20   return acc;\n\
         }\n");
    assert_eq!(checksum, 47 + 49 + 19, "must equal the mapset scenario's live counts");
}

/// mapset's str-key tombstone round trip: remove → re-add lands the key
/// on its DEAD slot, and the re-added value is the fresh cell's.
#[test]
fn str_key_remove_readd_get_tombstone_round_trip() {
    let checksum = run_main("nmapset", "use core::{ assert };\n\
         use nmapset::{ HashMap };\n\
         pub fn main() -> i32 {\n\
         \x20   let mut m: HashMap<str, str> = HashMap.new();\n\
         \x20   assert(m.put(\"alpha\", \"one\"), \"first put adds\");\n\
         \x20   assert(m.get(\"alpha\") == \"one\", \"value round-trips\");\n\
         \x20   assert(m.remove(\"alpha\"), \"remove answers true\");\n\
         \x20   assert(m.has(\"alpha\") == false, \"removed key is absent\");\n\
         \x20   assert(m.get(\"alpha\") == nil, \"removed key gets nil\");\n\
         \x20   assert(m.put(\"alpha\", \"two\"), \"re-add after remove adds (DEAD reuse)\");\n\
         \x20   assert(m.len() == 1, \"len back to one\");\n\
         \x20   let p = m.get(\"alpha\");\n\
         \x20   assert(p != nil, \"re-added key found\");\n\
         \x20   assert(p == \"two\", \"re-added value is the fresh cell's\");\n\
         \x20   return 7;\n\
         }\n");
    assert_eq!(checksum, 7, "must equal the mapset scenario's checksum");
}

// ---- the host experiment's own edges ----

/// A user-defined key type is refused AT COMPILE TIME now (phase 3):
/// the class bound is the literal union, `Pt` names no member, and the
/// diagnostic names the offending type and the whole union (`mapset`
/// remains the general-key implementation; a native crossing can no
/// longer be reached to trap). The old `Hashable` trait is gone, so
/// the honest port drops the import too.
#[test]
fn a_user_record_key_fails_at_compile_time_naming_the_union() {
    let ds = diags_of(
        "nmapset",
        "use core::{ assert };\n\
         use nmapset::{ HashMap };\n\
         struct Pt { x: i32; y: i32 }\n\
         pub fn main() -> i32 {\n\
         \x20   let mut pm: HashMap<Pt, i32> = HashMap.new();\n\
         \x20   assert(pm.put(Pt { x: 1, y: 2 }, 10), \"record map: first put\");\n\
         \x20   return pm.len();\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`Pt` does not satisfy `K` requires `i8 | i16 | i32 | i64 | u8 | u16 | u32 | u64 | bool | str | bytes`")),
        "the diagnostic names the offending type and the union: {ds:?}"
    );
}

/// A key type outside the closed set is refused at the instantiation —
/// the union bound's A5 diagnostic names the offending type and the
/// whole allowed set (phase 3: admission IS the union).
#[test]
fn an_unhashable_key_type_is_refused_at_the_instantiation() {
    let ds = diags_of(
        "nmapset",
        "struct Boxy { v: i32 }\n\
         use nmapset::{ HashMap };\n\
         pub fn main() -> i32 {\n\
         \x20   let mut m: HashMap<Boxy, i32> = HashMap.new();\n\
         \x20   return 0;\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`Boxy` does not satisfy `K` requires `i8 | i16 | i32 | i64 | u8 | u16 | u32 | u64 | bool | str | bytes`")),
        "admission names the missing impl: {ds:?}"
    );
}

/// The package unit: a consumer instantiates the wrapper classes (the
/// generic templates live in nmapset, monomorphized at the consumer),
/// and the host surface crossed through the `[deps]` mount.
#[test]
fn nmapset_instantiates_the_wrapper_classes() {
    let out = compile_pkg_app(
        "nmapset",
        "use nmapset::{ HashMap, HashSet };\n\
         pub fn main() -> i32 {\n\
         \x20   let mut m: HashMap<i32, i32> = HashMap.new();\n\
         \x20   let mut s: HashSet<i32> = HashSet.new();\n\
         \x20   m.put(1, 2);\n\
         \x20   s.put(3);\n\
         \x20   return m.len() + s.len();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    for name in ["HashMap<i32, i32>", "HashSet<i32>"] {
        assert!(
            p.types.types.iter().any(|t| p.name_of(t.name) == name),
            "{name} instantiated:\n{}",
            rut_driver::ir_dump_of(&p.funcs, &p.interner)
        );
    }
}

// ---- the get parity law ----

/// `get` parity between the twins, the SAME source against BOTH
/// packages: two gets of one key name the SAME stored cell until a
/// replace re-stores — held `*V`s go stale (they keep reading the
/// pre-replace cell), a fresh get reads the replacement, remove nils
/// the slot but not an independently held pointer. Identical answers,
/// identical checksum.
const PARITY_BODY: &str = "\
         use core::{ assert };\n\
         pub fn main() -> i32 {\n\
         \x20   let mut fails = 0;\n\
         \x20   let mut m: HashMap<str, str> = HashMap.new();\n\
         \x20   assert(m.put(\"k\", \"old\"), \"first put adds\");\n\
         \x20   let p1 = m.get(\"k\");\n\
         \x20   let p2 = m.get(\"k\");\n\
         \x20   assert(p1 != nil && p2 != nil, \"both gets found\");\n\
         \x20   assert(p1 == \"old\" && p2 == \"old\", \"both reads see the stored cell\");\n\
         \x20   assert(m.put(\"k\", \"new\") == false, \"replace answers false\");\n\
         \x20   assert(p1 == \"old\" && p2 == \"old\", \"held pointers keep the pre-replace cell\");\n\
         \x20   assert(m.get(\"k\") == \"new\", \"a fresh get reads the replacement\");\n\
         \x20   assert(m.len() == 1, \"replace keeps len\");\n\
         \x20   fails = fails + 1;\n\
         \x20   assert(m.get(\"nope\") == nil, \"miss gets nil\");\n\
         \x20   assert(m.has(\"k\"), \"has after put\");\n\
         \x20   assert(m.remove(\"k\"), \"remove answers true\");\n\
         \x20   assert(m.get(\"k\") == nil, \"removed key gets nil\");\n\
         \x20   assert(p1 == \"old\", \"the held pointer survives the slot's nil-store\");\n\
         \x20   fails = fails + m.len();\n\
         \x20   return 40 + fails;\n\
         }\n";

#[test]
fn get_staleness_parity_between_nmapset_and_mapset() {
    let mapset_r = run_main("mapset", &format!("use mapset::{{ HashMap }};\n{PARITY_BODY}"));
    let nmapset_r = run_main("nmapset", &format!("use nmapset::{{ HashMap }};\n{PARITY_BODY}"));
    assert_eq!(mapset_r, 41, "mapset: the staleness scenario's checksum");
    assert_eq!(nmapset_r, mapset_r, "nmapset must match mapset exactly");
}
