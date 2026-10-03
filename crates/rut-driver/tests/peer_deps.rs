//! The dep-kinds loader land: the three manifest
//! tables, the `optional` attribute, the `lib` group key, the four
//! mount passes, and the D1/D3/D4 diagnostics — pinned by the hermetic
//! fixture pkgs under `tests/data/peers/` (tiny self-owned
//! pouch/nmapset-shaped pkgs; no reliance on the real `rut/` pkgs).
//!
//! Every test cites the missing-peer-matrix cell it pins
//! (docs/dep-kinds-survey.md §3) — the suite IS the ruling, executable.
//! Every fixture avoids the T10 collision shape (one consumer per
//! shared inline pkg — T10 itself lives in dep_dedup.rs). D2 — the
//! reference-site dedicated diagnostic — landed with phase 2's graph/
//! bundle work and resolves against the session-resident registry this
//! phase populated; T4 pins the upgraded text.

use std::path::Path;


const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/peers");




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
            .pkg(rut_driver::Pkg::source(spec, src))
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
    rut_native::load_dir(Path::new(&format!("{DATA}/{rel}")))
        .map_err(|e| e.to_string())
}

fn compile(rel: &str) -> Result<rut_driver::GraphOutput, String> {
    let loaded = load(rel)?;
    Ok(graph_of(
        rut_driver::RutRun::new()
            .pkgs(&loaded)
            .entrypoint(&loaded.root)
            .compile(),
    ))
}

/// Mount + compile green: no mount error, no diags.
fn green(rel: &str) {
    match compile(rel) {
        Ok(g) => assert!(
            g.diags.is_empty(),
            "{rel}: {}",
            g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
        ),
        Err(e) => panic!("{rel}: mount failed: {e}"),
    }
}

// "The group mounted" is observable through the FIXTURE bodies
// themselves: each consumer names its group's wrapper (JVec/JMap) and
// dispatches it — if the group never mounted, the name is unknown and
// the fixture's own compile refuses. The negatives read the closure:
// the gate is presence-based, so the group's peer being absent from the
// session IS the group-not-mounted law. (The impl-registration rows —
// the old observable — are gone; satisfaction is structural, nothing
// registers.)

fn source_of(s: &rut_driver::Loaded, spec: &str) -> String {
    match &s.pkg(spec).expect(spec).body {
        rut_driver::PkgBody::Source { text, .. } => text.clone(),
        other => panic!("{spec}: no source body: {other:?}"),
    }
}

const POUCH_GROUP: &str = "class JVecI64";
const NMAPSET_GROUP: &str = "class JMapStrI64";

#[test]
fn t1_optional_peers_absent_is_silent() {
    // matrix row 2: optional peers absent, integration never touched —
    // NOTHING happens. Silent success IS the feature (json mounts
    // light); no transitive pull of pouch/nmapset.
    let s = load("cons_light").expect("mount must succeed");
    assert_eq!(s.root, "cons_light");
    assert!(s.pkg("json").is_some());
    assert!(s.pkg("pouch").is_none(), "pouch must not be pulled transitively");
    assert!(s.pkg("nmapset").is_none(), "nmapset must not be pulled transitively");
    assert!(!source_of(&s, "json").contains(POUCH_GROUP), "no group may mount");
    assert!(!source_of(&s, "json").contains(NMAPSET_GROUP), "no group may mount");
    green("cons_light");
}

#[test]
fn t2_peer_present_group_mounts_and_dispatches() {
    // matrix row 4: peer PRESENT in the consumer's closure (any
    // reason) → the integration group mounts automatically
    // (presence-based resolution); the group's wrapper dispatches;
    // placement green.
    let s = load("cons_pouch").expect("mount");
    // the mounted source stays pristine — the groups ride the compile
    assert!(!source_of(&s, "json").contains(POUCH_GROUP), "no source append");
    assert!(
        s.pkg("nmapset").is_none(),
        "nmapset is absent — inert: the group's peer never mounted"
    );
    // green IS the positive: cons.rut names the group's JVec and
    // dispatches it
    green("cons_pouch");
}

#[test]
fn t3_both_peers_mount_in_name_order() {
    // matrix row 4, both peers: both groups mount, in peer-name
    // (BTreeMap) order — `nmapset` before `pouch` — after the base;
    // both wrappers dispatch. (The old observable for the ORDER half —
    // the impl rows' compile order — died with the registry; the mount
    // walk's BTreeMap order is the loader's own iteration law.)
    let s = load("cons_both").expect("mount");
    assert!(!source_of(&s, "json").contains(POUCH_GROUP), "no source append");
    // green IS the positive: cons.rut names BOTH groups' wrappers
    // (JVec and JMap) and dispatches both
    green("cons_both");
}

#[test]
fn t4_reference_with_peer_absent_gets_the_dedicated_diag() {
    // matrix row 3 — D2: the consumer REFERENCES pouch
    // while pouch is absent; the miss is a declared optional peer of a
    // mounted pkg, so `resolve` answers with the DEDICATED diagnostic —
    // pkg + peer + the integration it unlocks + the fix — never the
    // bare NoModule text. (Phase 1 pinned the bare baseline here; the
    // baseline flipped when phase 2's D2 upgrade landed.)
    let s = load("cons_ref").expect("the mount itself is silent (optional peer)");
    let decl = s
        .pkg("json")
        .unwrap()
        .peers
        .get("pouch")
        .expect("json's pouch declaration is recorded");
    assert!(decl.optional);
    assert_eq!(decl.lib.as_deref(), Some("./serde_pouch.rut"));
    // the closure miss IS the run chain's Err: the resolver's own text
    let err = rut_driver::RutRun::new()
        .pkgs(&s)
        .entrypoint(&s.root)
        .compile()
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "cannot resolve `pouch` — `json`'s pouch integration is not mounted because the optional peer `pouch` is absent from this program's closure; add `\"pouch\": { \"path\": \"..\" }` to your `rut.jsonc` `deps`",
        "the dedicated diag, exact survey text"
    );
    // the same text surfaces at the reference site through the graph
    let g = compile("cons_ref").unwrap();
    assert!(
        g.diags
            .iter()
            .any(|d| d.msg.contains("cannot resolve `pouch`")
                && d.msg.contains("`json`'s pouch integration is not mounted")
                && d.msg.contains("")),
        "the reference must miss with D2: {:?}",
        g.diags
    );
    assert!(
        !g.diags.iter().any(|d| d.msg.contains("no module with that name is mounted")),
        "the bare NoModule text must be gone: {:?}",
        g.diags
    );
}

#[test]
fn t5_required_peer_missing_is_loud_d1() {
    // matrix row 1: required peer absent from the consumer's closure →
    // the LOUD mount error naming the pkg, the peer, and the fix.
    // Never silent, not auto-pulled.
    let err = match compile("cons_req") {
        Err(e) => e,
        Ok(_) => panic!("the required-peer miss must be a LOUD mount error"),
    };
    assert!(
        err.contains("pkg `json_required` requires the peer `nmapset`"),
        "{err}"
    );
    assert!(err.contains("and `nmapset` is not in this program's closure"), "{err}");
    assert!(err.contains("peers are not pulled transitively"), "{err}");
    assert!(
        err.contains("add `\"nmapset\": { \"path\": \"..\" }` to your `rut.jsonc` `deps`"),
        "{err}"
    );
    assert!(err.contains(""), "{err}");
}

#[test]
fn t6_self_build_dev_deps_guarantee_presence() {
    // matrix row 5: self-build/dev mode — the dev pass mounts the
    // peers (transitively: pouch rides base), the gate sees presence,
    // the groups mount; "no missing case exists".
    let s = load("json").expect("self-build mounts");
    assert_eq!(s.root, "json");
    assert!(s.pkg("pouch").is_some(), "the dev pass mounted pouch");
    assert!(s.pkg("nmapset").is_some(), "the dev pass mounted nmapset");
    assert!(s.pkg("base").is_some(), "the dev walk is transitive");
    let g = graph_of(
        rut_driver::RutRun::new()
            .pkgs(&s)
            .entrypoint(&s.root)
            .compile(),
    );
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn t7_broken_peer_path_is_loud_d3_at_self_build() {
    // matrix row 6, self-build half: a [peer-deps] path that does not
    // resolve is the LOUD packaging-bug error at the pkg's own build.
    let err = load("json_broken").unwrap_err();
    assert!(
        err.contains(
            "pkg `json_broken`'s [peer-deps] entry `pouch` points at `../does-not-exist`"
        ),
        "{err}"
    );
    assert!(
        err.contains("cannot read a manifest there (a packaging bug in json_broken)"),
        "{err}"
    );
}

#[test]
fn t7b_broken_peer_path_is_inert_for_consumers() {
    // matrix row 6, consumer half: consumer-mode peer paths are never
    // read — the optional peer is absent (inert), the group never
    // mounts, no error.
    let s = load("cons_broken").expect("the broken path must be inert for a consumer");
    assert!(
        s.pkg("pouch").is_none(),
        "the group's peer must not mount (consumer paths are never read)"
    );
    green("cons_broken");
}

#[test]
fn t7c_presence_is_by_name_the_path_is_never_read() {
    // §2.2: presence is by NAME — the consumer supplies pouch by its
    // OWN path, so the group mounts even though json_broken's peer
    // path is broken (first-mount-wins: the consumer's path won).
    let _s = load("cons_broken_supply").expect("mount");
    // green IS the positive: cons.rut names json_broken's group wrapper
    // (JVec) and dispatches it
    green("cons_broken_supply");
}

#[test]
fn t8_peer_gate_is_one_post_closure_pass() {
    // peer-of-peer: consumer → json + pouch → base. The gate runs
    // after the WHOLE closure exists (base included); groups add no
    // new pkg names, so no fixpoint is needed.
    let s = load("cons_chain").expect("mount");
    assert!(s.pkg("base").is_some(), "pouch's own dep walked");
    // green IS the positive: the group's wrapper dispatches with the
    // full closure in the session
    green("cons_chain");
}

#[test]
fn t9_both_kinds_pairing_end_to_end() {
    // the sanctioned pairing: pouch is json's OPTIONAL peer and its DEV
    // dep at once — parses; the dev pass mounts, the gate sees
    // presence, the group mounts — the ruling's json shape end-to-end.
    let s = load("json").expect("self-build");
    let decl = s
        .pkg("json")
        .unwrap()
        .peers
        .get("pouch")
        .expect("the pairing parses");
    assert!(decl.optional, "the peer half is optional");
    assert!(s.pkg("pouch").is_some(), "the dev half mounted");
    let g = graph_of(
        rut_driver::RutRun::new()
            .pkgs(&s)
            .entrypoint(&s.root)
            .compile(),
    );
    assert!(
        g.diags.is_empty(),
        "{}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn t11_d4_through_the_loader() {
    // matrix row for T11, end-to-end: `[deps]` + `[peer-deps]` same
    // name → D4, the manifest error naming both rows.
    let err = load("bad_d4").unwrap_err();
    assert_eq!(
        err,
        "`pouch` appears in both `[deps]` and `[peer-deps]` — a package is either pulled transitively or required of the consumer, never both"
    );
}

#[test]
fn mount_dir_mounts_no_dev_deps_and_runs_no_gate() {
    // §2.2 pass 2: mount_dir offers a pkg to someone else's program —
    // it is not "building the pkg itself", so the dev pass and the
    // peer gate are not its business (the peer DECLARATIONS are still
    // recorded for the registry).
    let s = rut_native::dir_pkgs(Path::new(&format!("{DATA}/json"))).expect("mount");
    assert_eq!(s.root, "json");
    assert!(s.pkg("json").is_some());
    assert!(s.pkg("pouch").is_none(), "dev-deps must not ride the embedder path");
    assert!(s.pkg("nmapset").is_none());
    assert!(
        s.pkg("json").unwrap().peers.get("pouch").is_some()
            || s.pkg("json").unwrap().peers.get("nmapset").is_some(),
        "the peer declarations are recorded on the pkg"
    );
}
