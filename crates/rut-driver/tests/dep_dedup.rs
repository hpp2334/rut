//! The splice-dedup land (dep-kinds survey §2.4, phase 2): `Unit::Inline`
//! carries the ordered `(origin spec, own source)` leaf list instead of
//! pre-combined text, and composition skips specs already present —
//! first position wins, topological order preserved. The t1 collision
//! shape (two pkgs both splicing a shared transitive dep, imported
//! separately) becomes structurally impossible: T10 compiles it green.
//! T13 pins the other half — the dedup is semantics-preserving: in the
//! no-collision case the composed text is byte-identical to the old
//! per-dep `extra + "\n" + src` shape, so every existing program
//! compiles identically (the rest of T13 is the gates: every existing
//! suite green untouched, bench pins BIT-IDENTICAL).
//!
//! Fixtures: the hermetic sibling world under `tests/data/sibs/`
//! (`sib_a`/`sib_b` share the transitive `pouch` dep; `cons_sibs`
//! imports both).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use rut_driver::Seeds;
use rut_parser::Mode;

const SIBS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/sibs");

mod common;
use common as _;




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

fn load(rel: &str) -> Result<rut_driver::Loaded, String> {
    rut_native::load_dir(Path::new(&format!("{SIBS}/{rel}")))
    .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------
// content identity — the coexistence law (wire v11)
// ----------------------------------------------------------------------

/// One util pkg whose manifest carries a distinguishing comment (the
/// fingerprint basis covers `rut.jsonc` byte-for-byte, so two builds
/// of one NAME get different identities).
fn write_util(root: &Path, dirname: &str, version: i64, tag: &str) {
    let dir = root.join(dirname);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("rut.jsonc"),
        format!(r#"{{"format": "rutbundle", "format_version": 11, "name": "util"}} // {tag}"#),
    )
    .unwrap();
    std::fs::write(
        dir.join("mod.rut"),
        format!("pub fn tag() -> i64 {{ return {version}; }}\n"),
    )
    .unwrap();
}

/// One consumer pkg over a url-dep util — packs to an archive whose
/// rode-along group rides under the util's content identity. The lock
/// rides too (the realistic fixture: a project that fetches installed).
fn write_consumer(
    root: &Path,
    name: &str,
    url: &str,
    bytes: &[u8],
    fndecl: &str,
) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("rut.jsonc"),
        format!(
            r#"{{"format": "rutbundle", "format_version": 11, "name": "{name}", "deps": {{"util": {{"url": "{url}"}}}}}}"#
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("mod.rut"),
        format!("use util::{{tag}};\n{fndecl}\n"),
    )
    .unwrap();
    let mut lock = std::collections::BTreeMap::new();
    lock.insert(url.to_string(), bytes.to_vec());
    common::write_lock(&dir, &lock);
}

fn identity_pkgs<'a>(loaded: &'a rut_driver::Loaded, name: &str) -> Vec<&'a rut_driver::Pkg> {
    loaded
        .pkgs
        .iter()
        .filter(|p| rut_driver::bundle::parse_identity(&p.spec).map(|(n, _)| n) == Some(name))
        .collect()
}

#[test]
fn same_name_two_identities_one_program() {
    // THE COEXISTENCE LAW: two archives carry a group of the SAME name
    // with different content — both mount side by side (each under its
    // content identity), each consumer binds its own, and one program
    // runs over both. The diamond dissolves: first-mount-wins was
    // per-name; it is per-identity now.
    let root = std::env::temp_dir().join(format!("rut-coexist-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    write_util(&root, "util_7", 7, "the v1 build");
    write_util(&root, "util_9", 9, "the v2 build");
    let u1 = rut_native::pack_dir(&root.join("util_7")).expect("pack util v1");
    let u2 = rut_native::pack_dir(&root.join("util_9")).expect("pack util v2");
    let url1 = "https://fixtures.test/util-v1.rutbundle";
    let url2 = "https://fixtures.test/util-v2.rutbundle";

    write_consumer(&root, "a", url1, &u1, "pub fn from_a() -> i64 { return tag(); }");
    write_consumer(&root, "b", url2, &u2, "pub fn from_b() -> i64 { return tag(); }");
    let mut fetch_map = std::collections::BTreeMap::new();
    fetch_map.insert(url1.to_string(), u1.clone());
    fetch_map.insert(url2.to_string(), u2.clone());
    let a = rut_native::pack_dir_fetched(&root.join("a"), &fetch_map).expect("pack a");
    let b = rut_native::pack_dir_fetched(&root.join("b"), &fetch_map).expect("pack b");

    let app = root.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("rut.jsonc"),
        format!(
            r#"{{"name": "app", "deps": {{"a": {{"url": "{url1}"}}, "b": {{"url": "{url2}"}}}}}}"#
        ),
    )
    .unwrap();
    std::fs::write(
        app.join("mod.rut"),
        "use a::{from_a};\nuse b::{from_b};\nentry fn main() -> i64 { return from_a() * 10 + from_b(); }\n",
    )
    .unwrap();
    let mut map = std::collections::BTreeMap::new();
    map.insert(url1.to_string(), a);
    map.insert(url2.to_string(), b);
    common::write_lock(&app, &map);
    let loaded = common::block_on(rut_native::load_dir_with(&app, &common::Table::from(map)))
        .expect("the diamond loads");
    // both identities of `util` mounted
    let utils = identity_pkgs(&loaded, "util");
    assert_eq!(utils.len(), 2, "two identities of one name: {:?}", loaded.pkgs.iter().map(|p| p.spec.clone()).collect::<Vec<_>>());
    // and the program runs over BOTH (v1 answers 7, v2 answers 9)
    let g = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile the diamond");
    assert!(
        g.graph.diags.is_empty(),
        "{}",
        g.graph.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("; ")
    );
    let flat = rut_core::link::flatten(g.graph.program.expect("linked"));
    rut_vm::verify::verify(&flat).expect("verify");
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(rut_vm::interp::Limits {
            fuel: Some(2_000_000),
            heap_limit_bytes: Some(16 * 1024 * 1024),
            interrupt_every: 1024,
        })
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    assert_eq!(vm.call::<_, i64>("main", ()).expect("run"), 79, "v1's 7 and v2's 9 both bind");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn same_identity_via_two_routes_mounts_once() {
    // THE EXACT-DEDUP LAW: two routes carry the SAME group (same name,
    // same bytes ⇒ same identity) — it mounts ONCE, and both consumers
    // bind the one body.
    let root = std::env::temp_dir().join(format!("rut-exact-dedup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    write_util(&root, "util_7", 7, "one build");
    write_util(&root, "util_7b", 7, "one build");
    let u1 = rut_native::pack_dir(&root.join("util_7")).expect("pack util");
    let u1b = rut_native::pack_dir(&root.join("util_7b")).expect("pack util again");
    assert_eq!(u1, u1b, "same content ⇒ byte-identical bundle (the determinism law)");
    let url1 = "https://fixtures.test/util-v1.rutbundle";
    let url2 = "https://fixtures.test/util-v1-again.rutbundle";

    write_consumer(&root, "a", url1, &u1, "pub fn from_a() -> i64 { return tag(); }");
    write_consumer(&root, "c", url2, &u1b, "pub fn from_c() -> i64 { return tag(); }");
    let mut fetch_map = std::collections::BTreeMap::new();
    fetch_map.insert(url1.to_string(), u1.clone());
    fetch_map.insert(url2.to_string(), u1b.clone());
    let a = rut_native::pack_dir_fetched(&root.join("a"), &fetch_map).expect("pack a");
    let c = rut_native::pack_dir_fetched(&root.join("c"), &fetch_map).expect("pack c");

    let app = root.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("rut.jsonc"),
        format!(
            r#"{{"name": "app", "deps": {{"a": {{"url": "{url1}"}}, "c": {{"url": "{url2}"}}}}}}"#
        ),
    )
    .unwrap();
    std::fs::write(
        app.join("mod.rut"),
        "use a::{from_a};\nuse c::{from_c};\nentry fn main() -> i64 { return from_a() + from_c(); }\n",
    )
    .unwrap();
    let mut map = std::collections::BTreeMap::new();
    map.insert(url1.to_string(), a);
    map.insert(url2.to_string(), c);
    common::write_lock(&app, &map);
    let loaded = common::block_on(rut_native::load_dir_with(&app, &common::Table::from(map)))
        .expect("the shared-dep world loads");
    let utils = identity_pkgs(&loaded, "util");
    assert_eq!(
        utils.len(), 1,
        "one identity mounts once: {:?}",
        loaded.pkgs.iter().map(|p| p.spec.clone()).collect::<Vec<_>>()
    );
    let g = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile");
    assert!(g.graph.diags.is_empty(), "{}", g.graph.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("; "));
    let flat = rut_core::link::flatten(g.graph.program.expect("linked"));
    rut_vm::verify::verify(&flat).expect("verify");
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(rut_vm::interp::Limits {
            fuel: Some(2_000_000),
            heap_limit_bytes: Some(16 * 1024 * 1024),
            interrupt_every: 1024,
        })
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    assert_eq!(vm.call::<_, i64>("main", ()).expect("run"), 14, "both consumers bind the one body");
    let _ = std::fs::remove_dir_all(&root);
}

/// Compile, flatten, verify, and run `main` — the i64 result.
fn run_main(loaded: rut_driver::Loaded) -> i64 {
    let g = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile the walk");
    assert!(
        g.graph.diags.is_empty(),
        "{}",
        g.graph.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let flat = rut_core::link::flatten(g.graph.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp:: Vm::builder().program(std::rc::Rc::new(flat)).limits(limits).hooks(rut_vm::interp::HostHooks::default()).hosts(rut_vm::interp::HostRegistry::new()).build()
    .expect("vm");
    vm.call::<_, i64>("main", ()).expect("run")
}

#[test]
fn t10_sibling_imports_of_a_shared_inline_pkg_compile() {
    // the t1 collision shape: cons_sibs imports sib_a AND sib_b; both
    // are inline units whose leaves carry the shared transitive dep
    // (pouch). Under dedup-by-origin pouch splices once, `Vec` is
    // defined once, and the consumer compiles green and runs — the
    // `appkit` workaround retires as a NECESSITY (it stays legal; the
    // example is another lane's property and is not touched).
    let loaded = load("cons_sibs").expect("mount");
    assert_eq!(loaded.root, "cons_sibs");
    assert_eq!(run_main(loaded), 16, "Slot::first() + Crate::rest() = 7 + 9");
}

#[test]
fn walked_pkgs_carry_their_manifest_and_first_mount_edge() {
    // the structural-manifest law: every walked pkg rides its OWN
    // parsed manifest (dep rows included), and `mounted_by` records the
    // FIRST-mount edge — the parent spec whose dep row pulled it (the
    // walk root itself has none). sib_a walks before sib_b, so the
    // shared `pouch` records sib_a's edge even though sib_b declares
    // it too.
    let loaded = load("cons_sibs").expect("mount");
    let by_spec: BTreeMap<_, _> = loaded.pkgs.iter().map(|p| (p.spec.as_str(), p)).collect();
    let root = by_spec["cons_sibs"];
    assert!(root.mounted_by.is_none(), "the walk root has no parent");
    assert_eq!(root.manifest.name.as_deref(), Some("cons_sibs"));
    assert!(root.manifest.deps.contains_key("sib_a"), "the root's own dep rows ride");
    let sib_a = by_spec["sib_a"];
    assert_eq!(sib_a.mounted_by.as_deref(), Some("cons_sibs"));
    assert_eq!(sib_a.manifest.name.as_deref(), Some("sib_a"));
    assert!(sib_a.manifest.deps.contains_key("pouch"));
    let pouch = by_spec["pouch"];
    assert_eq!(
        pouch.mounted_by.as_deref(),
        Some("sib_a"),
        "first mount wins — sib_a's edge, not sib_b's"
    );
    assert!(pouch.manifest.deps.contains_key("base"), "pouch's own dep rows ride");
}

#[test]
fn t10_the_collision_was_real() {
    // the control: WITHOUT dedup this exact shape spliced pouch's text
    // twice into one unit — and a doubly-spliced text does not compile
    // (duplicate type definitions). The fixture only goes green because
    // the dedup removed the second splice.
    let pouch_src = std::fs::read_to_string(Path::new(&format!("{SIBS}/pouch/mod.rut")))
        .expect("pouch fixture source");
    let twice = format!("{pouch_src}\n\n{pouch_src}");
    // loading is OFF here: the spliced text's `use` statements name deps
    // this bare compile binds nothing of, and the validated-must-bind
    // gate would preempt the duplicate error this test exists to prove —
    // the use rows err-and-continue in collect, the doubly-spliced text
    // still answers with its duplicate type definitions
    let out = rut_driver::compile_program_resolved(&twice, Mode::Impl, "twice", 1, &[], false, &Seeds::none(), &rut_driver::ModInputs::flat(), &[], &[]);
    assert!(
        out.program.is_none()
            && out.diags.iter().any(|d| d.msg.contains("duplicate")),
        "the same inline text spliced twice must be a compile error: {:?}",
        out.diags
    );
}

#[test]
fn t13_no_collision_chain_compiles_byte_identically() {
    // T13's in-tree pin: for a no-collision splice chain the graph must
    // compose EXACTLY the old law's text — each dep's combined source +
    // "\n", then "\n" + the unit's own source. The chain world compiles
    // through the graph; the comparison world compiles the hand-written
    // old-law composition directly (same spec, same scope, same empty
    // bound list — `compile_program_resolved` is a pure function of
    // those inputs, so equal binaries prove equal composed text).
    let cell_src = "\
pub class Cell<T> {
    v: T;
}

impl<T> Cell<T> {
    pub fn make(v: T) -> Self {
        return Self { v: v };
    }

    pub fn get(self) -> T {
        return self.v;
    }
}
";
    let wrap_src = "\
use cell::{Cell};

pub class Wrap {
    c: Cell<i64>;
}

impl Wrap {
    pub fn make(v: i64) -> Self {
        let mut inner = Cell<i64>::make(v);
        return Self { c: inner };
    }

    pub fn get(self) -> i64 {
        return self.c.get();
    }
}
";
    let app_src = "\
use wrap::{Wrap};

entry fn main() -> i64 {
    let w = Wrap::make(5);
    return w.get();
}
";

    // the chain world: app -> wrap (inline) -> cell (generic export)
    // `inline` marks the splice law's input explicitly now: the generic
    // export alone no longer forces it (owner-anchored instantiation)
    // each hand pkg declares the head it names (the nameability law —
    // the declarations bound uses, never the reverse)
    let chain = rut_driver::RutRun::new()
        .pkg(rut_driver::Pkg::source("cell", cell_src))
        .pkg(
            rut_driver::Pkg::source("wrap", wrap_src)
                .manifest(rut_driver::Manifest::lib("wrap", &["cell"])),
        )
        .pkg(
            rut_driver::Pkg::source("app", app_src)
                .manifest(rut_driver::Manifest::lib("app", &["wrap"])),
        );
    let g = graph_of(chain.entrypoint("app").compile());
    assert!(g.diags.is_empty(), "{:?}", g.diags);
    let chain = g.program.expect("linked program");

    // one compilation model: the graph compiles each owner once and the
    // consumers request instantiations — the chain's law is behavior
    // (it runs), plus the single-instantiation ledger proving the
    // bodies materialized once
    let flat = rut_core::link::flatten(chain.clone());
    rut_vm::verify::verify(&flat).expect("verify");
    let insts: Vec<String> = chain
        .inst_fns
        .iter()
        // the template window (v22) is not instantiation bodies: those
        // rows are the placeholder compilations (subst = `#<param>`
        // rows) the bundle link lane materializes from
        .filter(|r| r.fid < chain.template_first_fid)
        .filter(|r| chain.interner.name(r.owner) == "cell")
        .map(|r| match &r.kind {
            rut_core::binary::InstFnKind::Method { data, name, .. } => {
                format!("{}${}", chain.interner.name(*data), chain.interner.name(*name))
            }
            _ => String::new(),
        })
        .collect();
    assert_eq!(
        insts.len(),
        insts.iter().cloned().collect::<HashSet<String>>().len(),
        "one compiled body per instantiation: {insts:?}"
    );
}

#[test]
fn t13_no_collision_multi_dep_composition_byte_identical() {
    // the multi-dep variant: TWO inline deps, each with its own
    // transitive inline leaf — the extension order (first dep's leaves,
    // then second's) must reproduce the old law's concatenation.
    let cell_src = "\
pub class Cell<T> {
    v: T;
}

impl<T> Cell<T> {
    pub fn make(v: T) -> Self {
        return Self { v: v };
    }

    pub fn get(self) -> T {
        return self.v;
    }
}
";
    let wrap_src = "\
use cell::{Cell};

pub class Wrap {
    c: Cell<i64>;
}

impl Wrap {
    pub fn make(v: i64) -> Self {
        let mut inner = Cell<i64>::make(v);
        return Self { c: inner };
    }

    pub fn get(self) -> i64 {
        return self.c.get();
    }
}
";
    let box_src = "\
pub class Boxx<T> {
    v: T;
}

impl<T> Boxx<T> {
    pub fn make(v: T) -> Self {
        return Self { v: v };
    }

    pub fn get(self) -> T {
        return self.v;
    }
}
";
    let held_src = "\
use boxx::{Boxx};

pub class Held {
    b: Boxx<i64>;
}

impl Held {
    pub fn make(v: i64) -> Self {
        let mut inner = Boxx<i64>::make(v);
        return Self { b: inner };
    }

    pub fn get(self) -> i64 {
        return self.b.get();
    }
}
";
    let app_src = "\
use wrap::{Wrap};
use held::{Held};

entry fn main() -> i64 {
    let w = Wrap::make(5);
    let h = Held::make(7);
    return w.get() + h.get();
}
";
    // `inline` marks the splice law's input explicitly now: the generic
    // export alone no longer forces it (owner-anchored instantiation)
    // each hand pkg declares the heads it names (the nameability law)
    let g = graph_of(
        rut_driver::RutRun::new()
            .pkg(rut_driver::Pkg::source("cell", cell_src))
            .pkg(
                rut_driver::Pkg::source("wrap", wrap_src)
                    .manifest(rut_driver::Manifest::lib("wrap", &["cell"])),
            )
            .pkg(rut_driver::Pkg::source("boxx", box_src))
            .pkg(
                rut_driver::Pkg::source("held", held_src)
                    .manifest(rut_driver::Manifest::lib("held", &["boxx"])),
            )
            .pkg(
                rut_driver::Pkg::source("app", app_src)
                    .manifest(rut_driver::Manifest::lib("app", &["wrap", "held"])),
            )
            .entrypoint("app")
            .compile(),
    );
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    // one compilation model: the multi-dep chain compiles, verifies, and
    // the two generic exports' bodies materialized once each (the
    // ledger's cell rows are unique)
    let chain = g.program.expect("linked program");
    let flat = rut_core::link::flatten(chain);
    rut_vm::verify::verify(&flat).expect("verify");
    let insts: Vec<String> = flat
        .inst_fns
        .iter()
        .filter(|r| flat.interner.name(r.owner) == "cell")
        .map(|r| match &r.kind {
            rut_core::binary::InstFnKind::Method { data, name, .. } => {
                format!("{}${}", flat.interner.name(*data), flat.interner.name(*name))
            }
            _ => String::new(),
        })
        .collect();
    assert_eq!(
        insts.len(),
        insts.iter().cloned().collect::<HashSet<String>>().len(),
        "one compiled body per instantiation: {insts:?}"
    );
}
