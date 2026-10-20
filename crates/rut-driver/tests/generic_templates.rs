//! The generic-templates phase gate (v22 — generic bodies ride the
//! binary):
//!
//! 1. **The pinned parity law** — a consumer-spelled shape served from
//!    a bundle's TEMPLATES (the IR link lane) behaves identically to
//!    the same consumer over the DIRECTORY world (the source lane): the
//!    closure compiles clean, verifies clean, runs to the same answer,
//!    and the fn inventories agree.
//! 2. **The templates section** — every committed std bundle with an
//!    open generic surface carries template rows whose fids sit inside
//!    the declared window and whose param rows spell `#<param>`
//!    placeholders.
//! 3. **The version gate** — a v21 header refuses with the standard
//!    version error (v22's templates section orphans every v21
//!    artifact; the recipe is re-tag + re-pin).
//! 4. **Strip reconciles** — `--strip` + a generic surface is legal
//!    unconditionally: a successful pack's templates cover by
//!    construction, the stripped bundle carries no source, and it
//!    still serves consumer-spelled shapes.

use std::path::PathBuf;

// ---- the parity law -------------------------------------------------

/// A boot-arg consumer-spelled shape (`Vec<i64>`): the IR lane's
/// bread-and-butter — the bundle's templates serve it, the directory
/// lane recompiles from source.
const VEC_APP: &str = r#"
use pouch::{ Vec };

entry fn main() -> i32 {
    let items: Vec<i32> = Vec<i32>::new();
    items.push(7);
    items.push(8);
    items.push(9);
    let n = items.len();
    let first = items[0];
    return n * 100 + first + items[1] + items[2];
}
"#;

fn pkg_world_from_dirs(names: &[&str]) -> Vec<rut_driver::Mounted> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut world = Vec::new();
    for d in names {
        world.extend(
            rut_native::dir_pkgs(&base.join(format!("rut/{d}")))
                .expect("mount the dir pkg")
                .pkgs,
        );
    }
    world
}

fn pkg_world_from_bundles(names: &[&str]) -> Vec<rut_driver::Mounted> {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut world = Vec::new();
    for b in names {
        // the bundle carries its deps closed (the v11 deps/ wire) — ONE
        // archive, no second mount under other specs
        let bytes = std::fs::read(base.join(format!("dist/std/{b}.rutbundle")))
            .unwrap_or_else(|e| panic!("dist/std/{b}.rutbundle: {e}"));
        world.extend(
            rut_driver::Mounted::from_bundle(&bytes)
                .expect("mount the bundle")
                .pkgs,
        );
    }
    world
}

fn compile_app(
    world: Vec<rut_driver::Mounted>,
    deps: &[&str],
    src: &str,
) -> rut_driver::GraphOutput {
    let loaded = rut_driver::Walk {
        pkgs: world,
        root: String::new(),
    };
    let c = loaded
        .pkgs
        .iter()
        .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
        .pkg(
            rut_driver::Mounted::source("app", src)
                .manifest(rut_driver::Manifest::lib("app", deps)),
        )
        .entrypoint("app")
        .compile()
        .expect("the consumer compiles");
    assert!(
        c.graph.diags.is_empty(),
        "diags:\n{}",
        c.graph
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
    c.graph
}

/// The canonical fn inventory: (name, param spells, op count) — stable
/// across the two lanes' fn-table ordinals and dense type ids. The op
/// count is the pinned fold-parity bound: the materialization's copy
/// fold eliminates exactly the copies the fresh compile's `one_round`
/// folded — `Mov` copies, plus template `MovRef` copies whose
/// substituted reg types are prims (the fresh emitter's `Mov` spellings)
/// — so the materialized bodies land on the fresh compile's op counts.
fn inventory(prog: &rut_core::binary::Program) -> Vec<String> {
    let flat = rut_core::link::flatten(prog.clone());
    let mut out = Vec::new();
    for f in &flat.funcs {
        let spell = |id: u32| -> String {
            let d = flat.types.dense(id) as usize;
            match flat.types.types.get(d) {
                Some(t) => flat.interner.name(t.name).to_string(),
                None => format!("?oob({d})"),
            }
        };
        let params: Vec<String> = f.params.iter().map(|&t| spell(t)).collect();
        out.push(format!(
            "{}({}) ops={}",
            flat.func_name(f),
            params.join(","),
            f.code.len()
        ));
    }
    out.sort();
    out
}

#[test]
fn bundle_served_bodies_match_the_directory_lane() {
    let dir = compile_app(pkg_world_from_dirs(&["pouch"]), &["pouch"], VEC_APP);
    let bundle = compile_app(pkg_world_from_bundles(&["pouch"]), &["pouch"], VEC_APP);
    let dprog = dir.program.clone().expect("dir program");
    let bprog = bundle.program.clone().expect("bundle program");
    // both lanes verify clean
    let dflat = rut_core::link::flatten(dprog.clone());
    let bflat = rut_core::link::flatten(bprog.clone());
    rut_vm::verify::verify(&dflat).expect("the directory lane verifies");
    rut_vm::verify::verify(&bflat).expect("the bundle (template-served) lane verifies");
    // and the closures agree fn-for-fn (the byte-equality law's
    // canonical form: the same instantiations, the same shapes)
    let dinv = inventory(&dprog);
    let binv = inventory(&bprog);
    let dset: std::collections::BTreeSet<&String> = dinv.iter().collect();
    let bset: std::collections::BTreeSet<&String> = binv.iter().collect();
    let only_dir: Vec<&&String> = dset.difference(&bset).collect();
    let only_bundle: Vec<&&String> = bset.difference(&dset).collect();
    assert!(
        only_dir.is_empty() && only_bundle.is_empty(),
        "the two lanes' closures disagree:\n  dir-only: {only_dir:?}\n  bundle-only: {only_bundle:?}"
    );
}

#[test]
fn template_served_shapes_run_the_same_answer() {
    // the same consumer, both lanes, one answer — the IR lane's
    // runtime equivalence fence
    for world in [
        pkg_world_from_dirs(&["pouch"]),
        pkg_world_from_bundles(&["pouch"]),
    ] {
        let graph = compile_app(world, &["pouch"], VEC_APP);
        let prog = graph.program.expect("linked");
        let flat = rut_core::link::flatten(prog);
        rut_vm::verify::verify(&flat).expect("verify");
        let mut vm = rut_vm::interp::Vm::builder()
            .program(std::rc::Rc::new(flat))
            .limits(rut_vm::interp::Limits {
                fuel: Some(50_000_000),
                heap_limit_bytes: Some(64 * 1024 * 1024),
                interrupt_every: 1024,
            })
            .hooks(rut_vm::interp::HostHooks::default())
            .build()
            .expect("vm");
        let out: i32 = vm.call("main", ()).expect("main runs");
        assert_eq!(out, 324);
    }
}

/// A nmapset consumer-spelled shape (`HashSet<str>`, `HashMap<i64, str>`):
/// the named acceptance pkg, both lanes, same closures + the same
/// runtime answer. Two laws make the template-served lane honest here:
/// (a) the template body's RECEIVER spells the class's own bare row
/// (`HashMap` — the record shape), whose tie to the type parameters
/// rides the template row (`data` + `param_rows`), not an argument
/// spell of its own — the owner pins it to the instantiation row
/// (`HashMap<i64, str>`, the exact text a fresh compile mints); and
/// (b) the owner's minted identity rows claim under the SUBSTITUTED
/// args — a mint claiming under the placeholder spellings (`#K, #V`)
/// would fold onto the template-era ledger row at link and the served
/// bodies would speak the placeholder shape. The runtime fence covers
/// the value-typed native dispatch: `HashMap<i64, str>::get` must pick
/// the str-value variant through the template-served body (the answer
/// 3*10 + 2 + 1 = 33 — a wrong variant returns nil and the checksum
/// drops to 32).
const NMAP_APP: &str = r#"
use nmapset::{ HashSet, HashMap };

entry fn main() -> i32 {
    let words: [str] = ["a", "b", "a", "c"];
    let set: HashSet<str> = HashSet<str>::new();
    for (let w of words) {
        let added = set.put(w);
        if (added) { set.put(w); }
    }
    let map: HashMap<i64, str> = HashMap<i64, str>::new();
    map.put(1, "one");
    map.put(2, "two");
    let two: ?str = map.get(2);
    let extra = if (two != nil) { 1 } else { 0 };
    return set.len() * 10 + map.len() + extra;
}
"#;

#[test]
fn nmapset_bundle_served_shapes_match_the_directory_lane() {
    let dir_world = pkg_world_from_dirs(&["nmapset"]);
    let dir_ctx = rut_driver::host_pkg_ctx(&dir_world);
    let bundle_world = pkg_world_from_bundles(&["nmapset"]);
    let bundle_ctx = rut_driver::host_pkg_ctx(&bundle_world);
    let dir = compile_app(dir_world, &["nmapset"], NMAP_APP);
    let bundle = compile_app(bundle_world, &["nmapset"], NMAP_APP);
    let dprog = dir.program.clone().expect("dir program");
    let bprog = bundle.program.clone().expect("bundle program");
    let dflat = rut_core::link::flatten(dprog.clone());
    let bflat = rut_core::link::flatten(bprog.clone());
    rut_vm::verify::verify(&dflat).expect("the directory lane verifies");
    rut_vm::verify::verify(&bflat).expect("the bundle (template-served) lane verifies");
    let dinv = inventory(&dprog);
    let binv = inventory(&bprog);
    let dset: std::collections::BTreeSet<&String> = dinv.iter().collect();
    let bset: std::collections::BTreeSet<&String> = binv.iter().collect();
    let only_dir: Vec<&&String> = dset.difference(&bset).collect();
    let only_bundle: Vec<&&String> = bset.difference(&dset).collect();
    assert!(
        only_dir.is_empty() && only_bundle.is_empty(),
        "the two lanes' closures disagree:\n  dir-only: {only_dir:?}\n  bundle-only: {only_bundle:?}"
    );
    // the runtime fence: the same answer through the template-served
    // bodies, host-bound (the nmap_host fns serve the map/set natives)
    for (flat, tag, ctx) in [(&dflat, "dir", &dir_ctx), (&bflat, "bundle", &bundle_ctx)] {
        let mut hosts = rut_vm::interp::HostRegistry::new();
        hosts.install_host_pkg(ctx, rut_std::nmap::pkg());
        let mut vm = rut_vm::interp::Vm::builder()
            .program(std::rc::Rc::new(flat.clone()))
            .limits(rut_vm::interp::Limits {
                fuel: Some(50_000_000),
                heap_limit_bytes: Some(64 * 1024 * 1024),
                interrupt_every: 1024,
            })
            .hooks(rut_vm::interp::HostHooks::default())
            .hosts(hosts)
            .build()
            .unwrap_or_else(|e| panic!("{tag}: vm: {e:?}"));
        let out: i32 = vm
            .call("main", ())
            .unwrap_or_else(|e| panic!("{tag}: {e:?}"));
        assert_eq!(
            out, 33,
            "{tag}: the value-typed native dispatch through the served bodies"
        );
    }
}

// ---- the templates section ------------------------------------------

#[test]
fn committed_generic_bundles_carry_template_rows_in_surface_order() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for name in ["json", "flow", "pouch", "nmapset"] {
        let bytes = std::fs::read(base.join(format!("dist/std/{name}.rutbundle")))
            .unwrap_or_else(|e| panic!("dist/std/{name}.rutbundle: {e}"));
        for pkg in rut_driver::Mounted::from_bundle(&bytes)
            .expect("mount")
            .pkgs
        {
            let rut_driver::MountedBody::Compiled(p) = &pkg.body else {
                continue;
            };
            if p.templates.is_empty() {
                continue; // a concrete-only unit carries no window
            }
            // the window is coherent: every row's fid inside it, and
            // the param rows spell `#<param>` placeholders
            for t in &p.templates {
                assert!(
                    t.fid >= p.template_first_fid && t.fid < p.template_end_fid,
                    "{}: template fid {} outside the window [{}, {})",
                    p.name,
                    t.fid,
                    p.template_first_fid,
                    p.template_end_fid
                );
                assert!(
                    !t.param_rows.is_empty(),
                    "{}: a template row with no param rows",
                    p.name
                );
                for &r in &t.param_rows {
                    let spell = p.type_name(r).to_string();
                    assert!(
                        spell.starts_with('#') || spell.starts_with("?#"),
                        "{}: a template param row spells `{spell}`, not a placeholder",
                        p.name
                    );
                }
            }
            // the exported generic surface justifies the window: free
            // generic fns, generic methods, or generic type exports
            let surface_generics = p.surface.fn_generics.len()
                + p.surface
                    .inherents
                    .iter()
                    .map(|ih| ih.methods.iter().filter(|m| m.local == 0).count())
                    .sum::<usize>()
                + p.surface
                    .type_exports
                    .iter()
                    .filter(|t| t.is_generic)
                    .count();
            assert!(
                surface_generics > 0,
                "{}: templates without an exported generic surface",
                p.name
            );
        }
    }
}

// ---- the version gate -------------------------------------------------

#[test]
fn v21_binaries_refuse_with_the_standard_version_error() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(rut_core::binary::MAGIC);
    bytes.extend_from_slice(&21u32.to_le_bytes());
    let err = rut_core::binary::decode(&bytes).unwrap_err();
    assert!(
        err.contains("unsupported module binary version 21"),
        "{err}"
    );
}

// ---- strip reconciles --------------------------------------------------

#[test]
fn strip_with_a_generic_surface_is_legal_and_still_serves_shapes() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let pouch_dir = base.join("rut/pouch");
    let (bytes, _sidecar) =
        rut_native::pack_dir_opts(&pouch_dir, &rut_driver::pack::PackOpts { strip: true })
            .expect("the stripped pouch pack succeeds — the templates cover by construction");
    // the stripped bundle carries NO source entries (a host group's
    // `.d.rut` declaration surface is the host envelope, not source)
    let parsed = rut_driver::bundle::parse_bundle(&bytes).expect("parse");
    assert!(
        !parsed
            .iter()
            .any(|(n, _)| n.ends_with(".rut") && !n.ends_with(".d.rut") && n != "rut.jsonc"),
        "the stripped bundle carries source: {:?}",
        parsed.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    // and it still serves a consumer-spelled shape
    let mut world = rut_driver::Mounted::from_bundle(&bytes)
        .expect("mount the stripped pouch")
        .pkgs;
    world.push(
        rut_driver::Mounted::source("app", VEC_APP)
            .manifest(rut_driver::Manifest::lib("app", &["pouch"])),
    );
    let c = rut_driver::Walk {
        pkgs: world,
        root: String::new(),
    }
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
    .entrypoint("app")
    .compile()
    .expect("the consumer of the stripped bundle compiles");
    assert!(
        c.graph.diags.is_empty(),
        "diags:\n{}",
        c.graph
            .diags
            .iter()
            .map(|d| d.msg.clone())
            .collect::<Vec<_>>()
            .join("\n")
    );
    let flat = rut_core::link::flatten(c.graph.program.expect("linked"));
    rut_vm::verify::verify(&flat).expect("verify");
}

/// A NON-boot-arg consumer shape (`JsonArr<JsonI64>` — the args are
/// pkg rows, not boot prims) served from the json bundle's templates:
/// compile, verify, and RUN. The runtime fence pins the decode path's
/// SOpt law end-to-end through the template-served bodies: `null`
/// decodes to a PRESENT `SOpt` whose inner is nil (the null→absent
/// row), `5` to a present inner — and the array decode answers the
/// document's length. (The bundle lane here is compile+verify+run;
/// the op-count equality harnesses are the parity tests above, which
/// cover boot-arg shapes — this test's shape spends its fence on the
/// runtime answer.)
#[test]
fn json_bundle_served_non_boot_arg_shapes() {
    let app = r#"
use json::{ JsonArr, JsonI64, SOpt, decodeJson };

entry fn main() -> str {
    let (v, e) = decodeJson<JsonArr<JsonI64>>("[7,8,9]");
    if (e != nil) { return "err-arr"; }
    let a: [JsonI64] = v.to_arr();
    let (o, e2) = decodeJson<SOpt<JsonI64>>("null");
    if (e2 != nil) { return "err-opt"; }
    if (o == nil) { return "opt-gone"; }
    let inner: ?JsonI64 = o.get();
    if (inner != nil) { return "bad-null"; }
    let (p, e3) = decodeJson<SOpt<JsonI64>>("5");
    if (e3 != nil) { return "err-five"; }
    let five = if (p != nil && p.get() != nil) { 1 } else { 0 };
    return f"n={a.len()}:{five}";
}
"#;
    let world = pkg_world_from_bundles(&["json"]);
    let ctx = rut_driver::host_pkg_ctx(&world);
    let graph = compile_app(world, &["json"], app);
    let flat = rut_core::link::flatten(graph.program.expect("linked"));
    rut_vm::verify::verify(&flat).expect("verify");
    let mut hosts = rut_vm::interp::HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::nmap::pkg());
    hosts.install_host_pkg(&ctx, rut_std::strbuild::pkg());
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(rut_vm::interp::Limits {
            fuel: Some(400_000_000),
            heap_limit_bytes: Some(256 * 1024 * 1024),
            interrupt_every: 1024,
        })
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(hosts)
        .build()
        .expect("vm");
    let out: String = vm.call("main", ()).unwrap_or_else(|e| panic!("run: {e:?}"));
    assert_eq!(
        out, "n=3:1",
        "the template-served decode answers the document"
    );
}
