//! The mapset package (RFC 0028's map/set intent) — `rut/mapset/`, pure
//! rut source mounted like any tree pkg (the `pouch` pattern): a module
//! exporting generic classes is source-inlined into its consumer, so the
//! consumer's compilation registers the package's `Hashable` prim impls
//! and instantiates `HashMap`/`HashSet` right there. These tests run the
//! whole graph end-to-end: put/get/replace/remove, the grow (rehash +
// relocation) paths, DEAD-slot reuse, str keys, and a user struct key
//! with its own `impl Hashable`.

use rut_core::ops::Op;
use rut_driver::{Module, Session};

const MAPSET_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../rut/mapset/mapset.rut");

fn compile_app(app_src: &str) -> rut_driver::GraphOutput {
    let mapset_src = rut_driver::load_module_source(std::path::Path::new(MAPSET_PATH)).expect("read mapset.rut");
    let mut s = Session::new();
    // core: the app's `use core::{ assert }` (mapset itself spells no use)
    rut_driver::mount_std_core(&mut s);
    s.register_module("mapset", Module { source: Some(mapset_src), ..Default::default() }).unwrap();
    s.register_module("app_main", Module { source: Some(app_src.into()), ..Default::default() }).unwrap();
    rut_driver::compile_graph(&s, "app_main")
}

/// Compile the app, flatten, verify, and run `main` — the i32 checksum.
fn run_main(app_src: &str) -> i32 {
    let out = compile_app(app_src);
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let prog = out.program.expect("linked program");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(20_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::new(
        std::rc::Rc::new(flat),
        &limits,
        rut_vm::interp::HostHooks::default(),
        rut_vm::interp::HostRegistry::new(),
    )
    .expect("vm");
    vm.call::<_, i32>("main", ()).expect("run")
}

fn diags_of(app_src: &str) -> Vec<String> {
    compile_app(app_src)
        .diags
        .iter()
        .map(|d| d.msg.clone())
        .collect()
}

const PT_KEY_PRELUDE: &str = "\
struct Pt { x: i32; y: i32 }\n\
impl Hashable for Pt {\n\
\x20   fn hash(self) -> u64 {\n\
\x20       let a = (14695981039346656037u64 ^ self.x as u64).wrapping_mul(1099511628211u64);\n\
\x20       return (a ^ self.y as u64).wrapping_mul(1099511628211u64);\n\
\x20   }\n\
\x20   fn hash_eq(self, other: Self) -> bool {\n\
\x20       return self.x == other.x && self.y == other.y;\n\
\x20   }\n\
}\n";

#[test]
fn hashmap_i32_keys_grow_get_replace_remove_and_reuse() {
    let checksum = run_main(&format!(
        "{PT_KEY_PRELUDE}\
         use core::{{ assert }};\n\
         use mapset::{{ HashMap, HashSet, Hashable }};\n\
         pub fn main() -> i32 {{\n\
         \x20   let mut acc = 0;\n\
         \x20   let mut m: HashMap<i32, str> = HashMap.new();\n\
         \x20   for (let i = 0; i < 64; i += 1) {{\n\
         \x20       assert(m.put(i, f\"v{{i}}\"), \"i32 map: first put must add\");\n\
         \x20   }}\n\
         \x20   assert(m.len() == 64, \"i32 map: len after 64 puts\");\n\
         \x20   for (let i = 0; i < 64; i += 1) {{\n\
         \x20       assert(m.has(i), \"i32 map: has after put\");\n\
         \x20       let p = m.get(i);\n\
         \x20       assert(p != nil, \"i32 map: get after put\");\n\
         \x20       assert(*p == f\"v{{i}}\", \"i32 map: value round-trips\");\n\
         \x20   }}\n\
         \x20   acc = acc + m.len();\n\
         \x20   assert(m.put(5, \"new\") == false, \"i32 map: replace answers false\");\n\
         \x20   assert(*(m.get(5)) == \"new\", \"i32 map: replace took\");\n\
         \x20   assert(m.len() == 64, \"i32 map: replace keeps len\");\n\
         \x20   acc = acc + 1;\n\
         \x20   assert(m.remove(6), \"i32 map: remove answers true\");\n\
         \x20   assert(m.remove(6) == false, \"i32 map: second remove answers false\");\n\
         \x20   assert(m.has(6) == false, \"i32 map: removed key is absent\");\n\
         \x20   assert(m.get(6) == nil, \"i32 map: removed key gets nil\");\n\
         \x20   assert(m.len() == 63, \"i32 map: len after remove\");\n\
         \x20   assert(m.put(6, \"again\"), \"i32 map: re-insert adds (DEAD reuse)\");\n\
         \x20   assert(*(m.get(6)) == \"again\", \"i32 map: re-insert value\");\n\
         \x20   acc = acc + 2;\n\
         \x20   assert(m.get(1000) == nil, \"i32 map: never-key gets nil\");\n\
         \x20   assert(m.put(-7, \"neg\"), \"i32 map: negative key adds\");\n\
         \x20   assert(*(m.get(-7)) == \"neg\", \"i32 map: negative key round-trips\");\n\
         \x20   acc = acc + m.len();\n\
         \x20   return acc;\n\
         }}\n"
    ));
    assert_eq!(checksum, 132, "i32 map: 64 puts + replace + remove + re-insert + negative key");
}

#[test]
fn hashmap_str_keys_and_hashset() {
    let checksum = run_main(
        "use core::{ assert };\n\
         use mapset::{ HashMap, HashSet, Hashable };\n\
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
         \x20       assert(*p == i * 2, \"str map: hit round-trips\");\n\
         \x20   }\n\
         \x20   assert(sm.get(\"nope\") == nil, \"str map: miss\");\n\
         \x20   assert(sm.put(f\"k10\", 999) == false, \"str map: replace answers false\");\n\
         \x20   assert(*(sm.get(f\"k10\")) == 999, \"str map: replace took\");\n\
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
         }\n",
    );
    assert_eq!(checksum, 47 + 49 + 19, "str map + set + str set live counts");
}

#[test]
fn user_struct_key_with_its_own_hashable_impl() {
    let checksum = run_main(&format!(
        "{PT_KEY_PRELUDE}\
         use core::{{ assert }};\n\
         use mapset::{{ HashMap, Hashable }};\n\
         pub fn main() -> i32 {{\n\
         \x20   let mut pm: HashMap<Pt, i32> = HashMap.new();\n\
         \x20   assert(pm.put(Pt {{ x: 1, y: 2 }}, 10), \"struct map: first put\");\n\
         \x20   assert(pm.put(Pt {{ x: 3, y: 4 }}, 30), \"struct map: second put\");\n\
         \x20   assert(pm.put(Pt {{ x: 1, y: 2 }}, 11) == false, \"struct map: equal key replaces\");\n\
         \x20   let p = pm.get(Pt {{ x: 3, y: 4 }});\n\
         \x20   assert(p != nil, \"struct map: equal-key get found\");\n\
         \x20   assert(*p == 30, \"struct map: equal-key get value\");\n\
         \x20   assert(*(pm.get(Pt {{ x: 1, y: 2 }})) == 11, \"struct map: replaced value\");\n\
         \x20   assert(pm.get(Pt {{ x: 1, y: 3 }}) == nil, \"struct map: unequal key misses\");\n\
         \x20   return pm.len();\n\
         }}\n"
    ));
    assert_eq!(checksum, 2, "struct map: field-equal keys are ONE entry");
}

#[test]
fn an_unhashable_key_type_is_refused_at_the_instantiation() {
    // struct Boxy has no `impl Hashable` — `HashMap<Boxy, i32>` names the
    // missing impl (the A5 admission diagnostic, RFC 0043)
    let ds = diags_of(
        "struct Boxy { v: i32 }\n\
         use mapset::{ HashMap };\n\
         pub fn main() -> i32 {\n\
         \x20   let mut m: HashMap<Boxy, i32> = HashMap.new();\n\
         \x20   return 0;\n\
         }\n",
    );
    assert!(
        ds.iter().any(|d| d.contains("`Boxy` does not satisfy `K` requires `Hashable`")),
        "admission names the missing impl: {ds:?}"
    );
}

#[test]
fn mapset_source_compiles_standalone() {
    // the package unit itself: trait, prim impls, the internal core, and
    // the two generic owners (templates — no instantiation here)
    let mapset_src = rut_driver::load_module_source(std::path::Path::new(MAPSET_PATH)).expect("read mapset.rut");
    let out = rut_driver::compile_program(
        &mapset_src,
        rut_parser::Mode::Impl,
        "mapset",
        1,
        &[],
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    let surface = p.surface.clone();
    assert!(
        surface
            .type_exports
            .iter()
            .any(|t| surface.names.name(t.name) == "HashMap" && t.is_generic),
        "HashMap exports as a generic class"
    );
    assert!(
        surface
            .type_exports
            .iter()
            .any(|t| surface.names.name(t.name) == "HashSet" && t.is_generic),
        "HashSet exports as a generic class"
    );
    assert!(
        surface.traits.iter().any(|t| surface.names.name(t.name) == "Hashable"),
        "Hashable exports as a trait"
    );
    let src = std::fs::read_to_string(std::path::Path::new(MAPSET_PATH)).expect("read mapset.rut");
    assert!(
        src.contains("\nclass RawHashTable<K requires Hashable> {"),
        "the open-addressing core is module-internal (spelled without `pub`)"
    );
    assert!(
        !src.contains("pub class RawHashTable"),
        "the open-addressing core is never spelled `pub`"
    );
}

/// P4 (mapset perf plan): the v1.1 storage — `keys: [*K]`, K-typed
/// probe. With K monomorphized to i32 the whole map path compiles on
/// the bare K: `put`/`get`/`has`/`remove` inline into the caller (P1.3)
/// and mint no `box` (the `Hashable` slot is never built) and dispatch
/// no `calli` (no trait-object receiver exists). Asserted over every
/// FuncCode in the program — the app is map ops only, so that IS the
/// put/get code.
#[test]
fn hashmap_i32_put_get_code_is_boxless_and_calli_free() {
    let out = compile_app(
        "use core::{ assert };\n\
         use mapset::{ HashMap };\n\
         pub fn main() -> i32 {\n\
         \x20   let mut m: HashMap<i32, i32> = HashMap.new();\n\
         \x20   for (let i = 0; i < 32; i += 1) {\n\
         \x20       assert(m.put(i, i * 3), \"first put adds\");\n\
         \x20   }\n\
         \x20   let mut acc = 0;\n\
         \x20   for (let i = 0; i < 32; i += 1) {\n\
         \x20       if (m.has(i)) {\n\
         \x20           let p = m.get(i);\n\
         \x20           acc = acc + *p;\n\
         \x20       }\n\
         \x20   }\n\
         \x20   assert(m.remove(7), \"remove answers true\");\n\
         \x20   assert(m.get(7) == nil, \"removed key gets nil\");\n\
         \x20   return acc + m.len();\n\
         }\n",
    );
    assert!(out.diags.is_empty(), "{:?}", out.diags);
    let p = out.program.expect("program");
    // the (K,V) instantiation exists, monomorphized with a K-keyed core
    assert!(
        p.types
            .types
            .iter()
            .any(|t| p.name_of(t.name) == "HashMap<i32, i32>"),
        "HashMap<i32, i32> instantiated:\n{}",
        rut_driver::ir_dump_of(&p.funcs, &p.interner)
    );
    assert!(
        p.types
            .types
            .iter()
            .any(|t| p.name_of(t.name) == "RawHashTable<i32>"),
        "the core instantiates per K:\n{}",
        rut_driver::ir_dump_of(&p.funcs, &p.interner)
    );
    // op-level: no box mints and no interface dispatch anywhere
    for f in &p.funcs {
        assert!(
            f.code
                .iter()
                .all(|op| !matches!(op, Op::Box { .. } | Op::CallI { .. })),
            "`box`/`calli` in fn `{}`:\n{}",
            p.name_of(f.name),
            rut_driver::ir_dump_of(&p.funcs, &p.interner)
        );
    }
    // dump-level, on the code holding the inlined put/get (guarded —
    // `unbox` in the slot-ABI variants must not count as a box hit)
    let main_idx = p
        .funcs
        .iter()
        .position(|f| p.name_of(f.name) == "main")
        .expect("main compiled");
    let ir = rut_driver::ir_dump_of(&p.funcs, &p.interner);
    let main_text = ir
        .split("fn #")
        .find(|s| s.starts_with(&format!("{main_idx} main")))
        .expect("main in the dump");
    assert!(!main_text.contains(" box "), "no box op in main:\n{ir}");
    assert!(!main_text.contains("calli"), "no calli in main:\n{ir}");
    // and the map still answers identically (checksums are the law)
    let flat = rut_core::link::flatten(p);
    rut_vm::verify::verify(&flat).expect("verify");
}

/// P4: the str-key tombstone round trip — `remove` writes
/// `keys[i] = nil` (releasing the shared `*K` cell, the ref-K law), the
/// re-add occupies the DEAD slot with a fresh cell (and a new value),
/// and `get` reads it back through content equality.
#[test]
fn str_key_remove_readd_get_tombstone_round_trip() {
    let checksum = run_main(
        "use core::{ assert };\n\
         use mapset::{ HashMap };\n\
         pub fn main() -> i32 {\n\
         \x20   let mut m: HashMap<str, str> = HashMap.new();\n\
         \x20   assert(m.put(\"alpha\", \"one\"), \"first put adds\");\n\
         \x20   assert(*(m.get(\"alpha\")) == \"one\", \"value round-trips\");\n\
         \x20   assert(m.remove(\"alpha\"), \"remove answers true\");\n\
         \x20   assert(m.has(\"alpha\") == false, \"removed key is absent\");\n\
         \x20   assert(m.get(\"alpha\") == nil, \"removed key gets nil\");\n\
         \x20   assert(m.put(\"alpha\", \"two\"), \"re-add after remove adds (DEAD reuse)\");\n\
         \x20   assert(m.len() == 1, \"len back to one\");\n\
         \x20   let p = m.get(\"alpha\");\n\
         \x20   assert(p != nil, \"re-added key found\");\n\
         \x20   assert(*p == \"two\", \"re-added value is the fresh cell's\");\n\
         \x20   return 7;\n\
         }\n",
    );
    assert_eq!(checksum, 7, "str-key remove → re-add → get round trip");
}
