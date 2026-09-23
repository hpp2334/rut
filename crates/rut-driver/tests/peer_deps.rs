//! The dep-kinds loader land (RFC 0045, phase 1): the three manifest
//! tables, the `optional` attribute, the `lib` group key, the four
//! mount passes, and the D1/D3/D4 diagnostics — pinned by the hermetic
//! fixture pkgs under `tests/data/peers/` (tiny self-owned
//! pouch/nmapset-shaped pkgs; no reliance on the real `rut/` pkgs).
//!
//! Every test cites the missing-peer-matrix cell it pins
//! (docs/dep-kinds-survey.md §3) — the suite IS the ruling, executable.
//! Every fixture avoids the T10 collision shape (one consumer per
//! shared inline pkg), so today's splice graph cannot move. D2 — the
//! reference-site dedicated diagnostic — is phase 2's: T4 pins the
//! session-resident registry handoff and today's baseline only.

use std::path::Path;

use rut_driver::Session;

const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/peers");

fn load(rel: &str) -> Result<(Session, String), String> {
    rut_driver::load_dir_session(Path::new(&format!("{DATA}/{rel}")))
}

fn compile(rel: &str) -> Result<rut_driver::GraphOutput, String> {
    let (session, root) = load(rel)?;
    Ok(rut_driver::compile_graph(&session, &root))
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

fn source_of(s: &Session, spec: &str) -> String {
    s.resolve(spec).expect(spec).source.clone().expect("source")
}

const POUCH_GROUP: &str = "impl JsonSerialize for Vec<T>";
const NMAPSET_GROUP: &str = "impl JsonSerialize for Map<K, V>";

#[test]
fn t1_optional_peers_absent_is_silent() {
    // matrix row 2: optional peers absent, integration never touched —
    // NOTHING happens. Silent success IS the feature (json mounts
    // light); no transitive pull of pouch/nmapset.
    let (s, root) = load("cons_light").expect("mount must succeed");
    assert_eq!(root, "cons_light");
    assert!(s.resolve("json").is_ok());
    assert!(s.resolve("pouch").is_err(), "pouch must not be pulled transitively");
    assert!(s.resolve("nmapset").is_err(), "nmapset must not be pulled transitively");
    assert!(!source_of(&s, "json").contains(POUCH_GROUP), "no group may mount");
    assert!(!source_of(&s, "json").contains(NMAPSET_GROUP), "no group may mount");
    green("cons_light");
}

#[test]
fn t2_peer_present_group_mounts_and_dispatches() {
    // matrix row 4: peer PRESENT in the consumer's closure (any
    // reason) → the integration group mounts automatically
    // (presence-based resolution); the group's impl dispatches through
    // the trait; orphan/placement green.
    let (s, _) = load("cons_pouch").expect("mount");
    assert!(source_of(&s, "json").contains(POUCH_GROUP), "the pouch group must mount");
    assert!(!source_of(&s, "json").contains(NMAPSET_GROUP), "nmapset is absent — inert");
    green("cons_pouch");
}

#[test]
fn t3_both_peers_mount_in_name_order() {
    // matrix row 4, both peers: both groups mount, in peer-name
    // (BTreeMap) order — `nmapset` before `pouch` — after the base;
    // both impl sets dispatch.
    let (s, _) = load("cons_both").expect("mount");
    let src = source_of(&s, "json");
    let a = src.find(NMAPSET_GROUP).expect("nmapset group mounted");
    let b = src.find(POUCH_GROUP).expect("pouch group mounted");
    assert!(a < b, "groups mount in peer-name (BTreeMap) order");
    assert!(src.contains("pub trait JsonSerialize"), "the base rides first");
    green("cons_both");
}

#[test]
fn t4_reference_with_peer_absent_registry_handoff() {
    // matrix row 3 — the phase-1 half: json's peer declaration is
    // session-resident (the registry phase 2's D2 upgrade resolves
    // against), and the reference misses TODAY with the bare
    // NoModule text — the baseline D2 replaces. The dedicated diag is
    // deliberately NOT implemented this phase.
    let (s, _) = load("cons_ref").expect("the mount itself is silent (optional peer)");
    let decl = s
        .peer_decls()
        .get("json")
        .and_then(|p| p.get("pouch"))
        .expect("json's pouch declaration is recorded");
    assert!(decl.optional);
    assert_eq!(decl.lib.as_deref(), Some("./serde_pouch.rut"));
    let err = s.resolve("pouch").unwrap_err();
    assert!(
        err.to_string().contains("no module with that name is mounted"),
        "today's bare NoModule baseline: {err}"
    );
    let g = compile("cons_ref").unwrap();
    assert!(
        g.diags.iter().any(|d| d.msg.contains("cannot resolve `pouch`")),
        "the reference must still miss: {:?}",
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
        err.contains("add `nmapset = { path = \"..\" }` to your `rut.toml` `[deps]`"),
        "{err}"
    );
    assert!(err.contains("(RFC 0045 §3)"), "{err}");
}

#[test]
fn t6_self_build_dev_deps_guarantee_presence() {
    // matrix row 5: self-build/dev mode — the dev pass mounts the
    // peers (transitively: pouch rides base), the gate sees presence,
    // the groups mount; "no missing case exists".
    let (s, root) = load("json").expect("self-build mounts");
    assert_eq!(root, "json");
    assert!(s.resolve("pouch").is_ok(), "the dev pass mounted pouch");
    assert!(s.resolve("nmapset").is_ok(), "the dev pass mounted nmapset");
    assert!(s.resolve("base").is_ok(), "the dev walk is transitive");
    let src = source_of(&s, "json");
    assert!(src.contains(POUCH_GROUP));
    assert!(src.contains(NMAPSET_GROUP));
    let g = rut_driver::compile_graph(&s, &root);
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
        err.contains("cannot read a manifest there (a packaging bug in json_broken; RFC 0045 §3)"),
        "{err}"
    );
}

#[test]
fn t7b_broken_peer_path_is_inert_for_consumers() {
    // matrix row 6, consumer half: consumer-mode peer paths are never
    // read — the optional peer is absent (inert), the group never
    // mounts, no error.
    let (s, _) = load("cons_broken").expect("the broken path must be inert for a consumer");
    assert!(
        !source_of(&s, "json_broken").contains(POUCH_GROUP),
        "the group must not mount"
    );
    green("cons_broken");
}

#[test]
fn t7c_presence_is_by_name_the_path_is_never_read() {
    // §2.2: presence is by NAME — the consumer supplies pouch by its
    // OWN path, so the group mounts even though json_broken's peer
    // path is broken (first-mount-wins: the consumer's path won).
    let (s, _) = load("cons_broken_supply").expect("mount");
    assert!(
        source_of(&s, "json_broken").contains(POUCH_GROUP),
        "the group mounts on presence, not on the declarer's path"
    );
    green("cons_broken_supply");
}

#[test]
fn t8_peer_gate_is_one_post_closure_pass() {
    // peer-of-peer: consumer → json + pouch → base. The gate runs
    // after the WHOLE closure exists (base included); groups add no
    // new pkg names, so no fixpoint is needed.
    let (s, _) = load("cons_chain").expect("mount");
    assert!(s.resolve("base").is_ok(), "pouch's own dep walked");
    assert!(
        source_of(&s, "json").contains(POUCH_GROUP),
        "the group mounted with the full closure in the session"
    );
    green("cons_chain");
}

#[test]
fn t9_both_kinds_pairing_end_to_end() {
    // the sanctioned pairing: pouch is json's OPTIONAL peer and its DEV
    // dep at once — parses; the dev pass mounts, the gate sees
    // presence, the group mounts — the ruling's json shape end-to-end.
    let (s, root) = load("json").expect("self-build");
    let decl = s
        .peer_decls()
        .get("json")
        .and_then(|p| p.get("pouch"))
        .expect("the pairing parses");
    assert!(decl.optional, "the peer half is optional");
    assert!(s.resolve("pouch").is_ok(), "the dev half mounted");
    assert!(source_of(&s, "json").contains(POUCH_GROUP));
    let g = rut_driver::compile_graph(&s, &root);
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
        "`pouch` appears in both `[deps]` and `[peer-deps]` — a package is either pulled transitively or required of the consumer, never both (RFC 0045 §2)"
    );
}

#[test]
fn mount_dir_mounts_no_dev_deps_and_runs_no_gate() {
    // §2.2 pass 2: mount_dir offers a pkg to someone else's program —
    // it is not "building the pkg itself", so the dev pass and the
    // peer gate are not its business (the peer DECLARATIONS are still
    // recorded for the registry).
    let mut s = Session::new();
    let name = rut_driver::mount_dir(&mut s, Path::new(&format!("{DATA}/json"))).expect("mount");
    assert_eq!(name, "json");
    assert!(s.resolve("json").is_ok());
    assert!(s.resolve("pouch").is_err(), "dev-deps must not ride the embedder path");
    assert!(s.resolve("nmapset").is_err());
    assert!(
        s.peer_decls().get("json").is_some(),
        "the peer declarations are recorded for the registry"
    );
}
