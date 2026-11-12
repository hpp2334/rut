//! The read-only dep walk's gate (`crate::walk` — the extension's
//! manifest walk moved into the crate), driven through the public ABI
//! over `spawn_native` with a RECORDING fs-backed env: every law the
//! old walk carried (the lock feed, budgets, first-mount-wins,
//! dev-deps root-only, direct-deps accumulate, the mod-tree
//! spellings) plus the phase's new ones (the store's mount door — a
//! hit mounts without ONE write, a miss hints once per reason and
//! never fetches; monotone idempotency under re-resolve).
//!
//! The store rides a tempdir `$RUT_HOME` (the address rut-driver
//! derives — `store_dir`/`deps_dir`, never re-derived here), the
//! project rides a sibling tempdir, and the env's paths are rooted
//! strings the recording `Io` resolves (lexical `..` collapsing — the
//! host-normalizes law `env` spells).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use futures::executor::LocalPool;
use rut_lsp_wasm::env::Io;
use rut_lsp_wasm::RutLsp;

/// a full 64-hex pin — the lock stores, the dir name abbreviates
const KIT_PIN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const KIT_URL: &str = "https://cdn.jsdelivr.net/gh/example/rut@std-v1/dist/std/kit.zip";
/// a second pin — the meta-mismatch world vouches this one
const OTHER_PIN: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

// ---- the recording, fs-backed host env ----

/// the host env the native tests inject: rooted-string paths (lexically
/// normalized — `..` collapses the way a host would), every call
/// recorded. The [`Io`] trait has NO write lane, so the read-only law
/// is structural; the recordings prove what the walk touched, and the
/// store-tree snapshots prove nothing moved.
#[derive(Default)]
struct FsIo {
    reads: RefCell<Vec<String>>,
    is_dirs: RefCell<Vec<String>>,
    notes: RefCell<Vec<String>>,
}

impl FsIo {
    /// a rooted string → the fs path (lexical `..`/`.` collapsing; the
    /// walk joins with `/` only, the host normalizes)
    fn resolve(&self, path: &str) -> PathBuf {
        let mut out: Vec<&str> = Vec::new();
        for seg in path.split('/') {
            match seg {
                "" | "." => {}
                ".." => {
                    out.pop();
                }
                s => out.push(s),
            }
        }
        PathBuf::from("/").join(out.join("/"))
    }
}

impl Io for FsIo {
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.reads.borrow_mut().push(path.to_string());
        std::fs::read(self.resolve(path)).ok()
    }

    fn read_dir(&self, path: &str) -> Option<Vec<String>> {
        let rd = std::fs::read_dir(self.resolve(path)).ok()?;
        Some(
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect(),
        )
    }

    fn is_dir(&self, path: &str) -> bool {
        self.is_dirs.borrow_mut().push(path.to_string());
        self.resolve(path).is_dir()
    }

    fn notify(&self, message: &str, _is_error: bool) {
        self.notes.borrow_mut().push(message.to_string());
    }
}

// ---- the harness ----

/// one test's scratch tree (the house pattern: temp_dir + pid —
/// nextest gives every test its own process)
fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-lsp-walk-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("the scratch tree creates");
    d
}

/// write one file under `dir` at `rel` (created with parents)
fn put(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().expect("a parent")).expect("the parent creates");
    std::fs::write(p, text).expect("the file writes");
}

/// one rut.lock's canonical text for `rows`
fn lock_text(rows: Vec<(&str, &str, &str)>) -> String {
    let text = rut_driver::render_lock(&rut_driver::LockFile::from_rows(
        rows.into_iter()
            .map(|(pkg, url, sha256)| rut_driver::LockRow {
                pkg: pkg.into(),
                url: url.into(),
                sha256: sha256.into(),
            })
            .collect(),
    ));
    text.to_string()
}

/// the store dir one pinned url dep lives at (`<name>@<pin16>` under
/// `$RUT_HOME/deps` — rut-driver's derivation, the same address
/// `rut install` publishes to), laid with a vouching meta.json and an
/// empty `unpack/` the caller fills
fn store_dep(home: &Path, name: &str, pin: &str, url: &str) -> PathBuf {
    let dir = home.join("deps").join(rut_driver::store_dir(name, pin));
    std::fs::create_dir_all(dir.join(rut_driver::UNPACK_DIR)).expect("the dep dir creates");
    std::fs::write(
        dir.join(rut_driver::META_NAME),
        rut_driver::store_meta_json(&rut_driver::StoreMeta {
            name: name.into(),
            url: url.into(),
            sha256: pin.into(),
            format: rut_driver::store_format(url).into(),
            format_version: rut_driver::STORE_META_FORMAT_VERSION,
        }),
    )
    .expect("the meta writes");
    dir
}

/// a plain lib manifest's jsonc — name + optional deps rows
fn manifest(name: &str, deps: &str) -> String {
    if deps.is_empty() {
        format!("{{\n  \"name\": \"{name}\"\n}}\n")
    } else {
        format!("{{\n  \"name\": \"{name}\",\n  \"deps\": {deps}\n}}\n")
    }
}

/// one root manifest spelling BOTH tables
fn manifest_dev(name: &str, deps: &str, dev_deps: &str) -> String {
    format!("{{\n  \"name\": \"{name}\",\n  \"deps\": {deps},\n  \"dev-deps\": {dev_deps}\n}}\n")
}

/// the project root fixture: lock + manifest + `src/mod.rut`
fn project(root: &Path, name: &str, lock: Option<&str>, deps: &str, body: &str) {
    if let Some(lock) = lock {
        put(root, "rut.lock", lock);
    }
    put(root, "rut.jsonc", &manifest(name, deps));
    put(root, "src/mod.rut", body);
}

/// boot a fresh core with the recording env installed, and point
/// `$RUT_HOME` at `home` (the store the walk's url rows derive into)
fn boot(home: &Path) -> (RutLsp, LocalPool, Rc<FsIo>) {
    // Env mutation is safe under nextest's one-process-per-test model
    // (the suite's runner): this test owns its process's environment.
    std::env::set_var("RUT_HOME", home);
    let (lsp, mut pool) = RutLsp::spawn_native();
    let io = Rc::new(FsIo::default());
    pool.run_until(lsp.set_env_io(io.clone()));
    (lsp, pool, io)
}

/// parse an export's JSON answer
fn take(json: String) -> serde_json::Value {
    serde_json::from_str(&json).expect("the envelope is valid JSON")
}

/// (line, character) of the `occurrence`-th needle — the ASCII docs
/// make UTF-16 columns plain byte counts
fn pos_of(src: &str, needle: &str, occurrence: usize) -> (u32, u32) {
    let mut at = 0usize;
    for _ in 0..occurrence {
        at = src[at..].find(needle).expect("the needle rides the doc") + at + 1;
    }
    let at = at - 1;
    let line = src[..at].matches('\n').count() as u32;
    let ch = (at - (src[..at].rfind('\n').map(|i| i + 1).unwrap_or(0))) as u32;
    (line, ch)
}

/// analyze `doc` at `uri`, answer the definition targets at the
/// needle — the uris, sorted (the world-identity the idempotency law
/// compares)
fn def_uris_at(
    lsp: &RutLsp,
    pool: &mut LocalPool,
    uri: &str,
    doc: &str,
    needle: &str,
) -> Vec<String> {
    let uri = uri.to_string();
    take(pool.run_until(lsp.analyze(uri.clone(), doc.to_string())));
    let (line, ch) = pos_of(doc, needle, 1);
    let locs = take(pool.run_until(lsp.definition(uri, line, ch)));
    let mut uris: Vec<String> = locs
        .as_array()
        .map(|a| {
            a.iter()
                .map(|l| l["uri"].as_str().unwrap_or("").to_string())
                .collect()
        })
        .unwrap_or_default();
    uris.sort();
    uris
}

/// [`def_uris_at`] at the shared scratch doc uri
fn def_uris(lsp: &RutLsp, pool: &mut LocalPool, doc: &str, needle: &str) -> Vec<String> {
    def_uris_at(lsp, pool, "file:///ws/walkdoc.rut", doc, needle)
}

/// the walk over `roots` — the `WalkOut` JSON
fn resolve(lsp: &RutLsp, pool: &mut LocalPool, roots: &[&str]) -> serde_json::Value {
    let roots: Vec<String> = roots.iter().map(|s| s.to_string()).collect();
    take(pool.run_until(lsp.resolve_deps(serde_json::to_string(&roots).unwrap())))
}

/// a recursive listing of one tree — the no-write law's before/after
fn snapshot(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&d)
            .expect("the tree lists")
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            out.push(p.strip_prefix(dir).unwrap().to_string_lossy().into_owned());
            if p.is_dir() {
                stack.push(p);
            }
        }
    }
    out.sort();
    out
}

/// the walk's origin spelling for a path under `base`: joined with `/`
/// exactly as the walk does (`..` preserved — the HOST env normalizes,
/// the walk never munges)
fn walk_path(base: &Path, rel: &str) -> String {
    format!("{}/{}", base.display(), rel)
}

/// an absolute-path origin as the definition layer spells it on the
/// wire (`file://` + the path — `origin_uri`'s law)
fn file_uri(path: String) -> String {
    format!("file://{path}")
}

// ---- the lock feed ----

#[test]
fn a_malformed_lock_hints_once_and_the_empty_ledger_stands() {
    let ws = scratch("lock-bad");
    let proj = ws.join("proj");
    project(
        &proj,
        "walkapp",
        Some("{\"format\": \"rutlock\", \"format_version\": 1, \"deps\": []"),
        &format!("{{\"kit\": {{\"url\": \"{KIT_URL}\"}}}}"),
        "fn main() -> nil {\n}\n",
    );
    let (lsp, mut pool, io) = boot(&ws.join("store"));

    let out = resolve(&lsp, &mut pool, &[proj.to_str().unwrap()]);
    // the url row answers unpinned — the empty ledger stands, never
    // half-parsed or guessed pins
    assert_eq!(out["missing"].as_array().unwrap().len(), 1, "{out}");
    assert!(
        out["missing"][0]["sha256"].is_null(),
        "no lock row ⇒ unpinned: {}",
        out["missing"][0]
    );
    assert_eq!(out["missing"][0]["url"].as_str().unwrap(), KIT_URL, "{out}");
    // exactly two hints — the malformed lock itself, and the honest
    // unpinned refusal it leaves behind — each once per reason
    let notes = io.notes.borrow();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(
        notes.iter().any(|n| n.contains("rut.lock")),
        "the lock hint names the file: {notes:?}"
    );
    assert!(
        notes.iter().any(|n| n.contains("no sha256 pin")),
        "the unpinned refusal: {notes:?}"
    );
    // the ledger state is observable through the ABI too: the manifest
    // re-parse answers the url row unpinned
    let table = take(pool.run_until(lsp.parse_manifest(manifest(
        "walkapp",
        &format!("{{\"kit\": {{\"url\": \"{KIT_URL}\"}}}}"),
    ))));
    assert!(
        table["deps"][0]["source"]["sha256"].is_null(),
        "the empty ledger stands: {table}"
    );
}

#[test]
fn a_missing_lock_clears_the_prior_roots_ledger() {
    let ws = scratch("lock-miss");
    let home = ws.join("store");
    let r1 = ws.join("r1");
    let r2 = ws.join("r2");
    // root 1: a lock pinning the url, and a store hit behind it
    project(
        &r1,
        "appone",
        Some(&lock_text(vec![("kit", KIT_URL, KIT_PIN)])),
        &format!("{{\"kit\": {{\"url\": \"{KIT_URL}\"}}}}"),
        "fn main() -> nil {\n}\n",
    );
    let kit_dir = store_dep(&home, "kit", KIT_PIN, KIT_URL);
    put(
        &kit_dir,
        &format!("{}/rut.jsonc", rut_driver::UNPACK_DIR),
        &manifest("kit", ""),
    );
    put(
        &kit_dir,
        &format!("{}/src/mod.rut", rut_driver::UNPACK_DIR),
        "pub fn kit_tag() -> i32 {\n    return 1;\n}\n",
    );
    // root 2: the SAME url dep, but NO rut.lock beside its manifest
    project(
        &r2,
        "apptwo",
        None,
        &format!("{{\"kit\": {{\"url\": \"{KIT_URL}\"}}}}"),
        "fn main() -> nil {\n}\n",
    );

    let (lsp, mut pool, io) = boot(&home);
    let out = resolve(
        &lsp,
        &mut pool,
        &[r1.to_str().unwrap(), r2.to_str().unwrap()],
    );
    // root 1's row MOUNTED from the store (no missing row for it);
    // root 2's row answered unpinned — the ledger it would have read
    // was cleared by the lock miss, so even a warm store never opens
    assert_eq!(out["missing"].as_array().unwrap().len(), 1, "{out}");
    assert!(
        out["missing"][0]["sha256"].is_null(),
        "root 2's ledger CLEARED — the same url answers unpinned: {out}"
    );
    // … and the only hint is root 2's unpinned refusal (the miss lane
    // is expected, never a hint of its own)
    let notes = io.notes.borrow();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("no sha256 pin"), "{notes:?}");
}

// ---- the store's mount door ----

/// the store-hit fixture: a kit tree with a mod child and an embedded
/// dep group (the unpack law's shape — the pkg's own path rows point
/// into `deps/<identity>/` beside its manifest)
fn kit_store(home: &Path) -> PathBuf {
    let dir = store_dep(home, "kit", KIT_PIN, KIT_URL);
    put(
        &dir,
        &format!("{}/rut.jsonc", rut_driver::UNPACK_DIR),
        &manifest(
            "kit",
            "{\"kitcore\": {\"path\": \"deps/kitcore@feed0123456789abc\"}}",
        ),
    );
    put(
        &dir,
        &format!("{}/src/mod.rut", rut_driver::UNPACK_DIR),
        "pub mod extra;\npub class KitBox {\n    n: i32;\n}\n",
    );
    put(
        &dir,
        &format!("{}/src/extra/mod.rut", rut_driver::UNPACK_DIR),
        "pub fn kit_extra() -> i32 {\n    return 7;\n}\n",
    );
    // the embedded group: a whole package under deps/<identity>/
    let core_dir = dir
        .join(rut_driver::UNPACK_DIR)
        .join("deps/kitcore@feed0123456789abc");
    put(&core_dir, "rut.jsonc", &manifest("kitcore", ""));
    put(
        &core_dir,
        "src/mod.rut",
        "pub fn core_tag() -> i32 {\n    return 9;\n}\n",
    );
    dir
}

#[test]
fn a_store_hit_mounts_through_the_dir_lanes_without_one_write() {
    let ws = scratch("store-hit");
    let home = ws.join("store");
    let proj = ws.join("proj");
    project(
        &proj,
        "walkapp",
        Some(&lock_text(vec![("kit", KIT_URL, KIT_PIN)])),
        &format!("{{\"kit\": {{\"url\": \"{KIT_URL}\"}}}}"),
        "fn main() -> nil {\n}\n",
    );
    kit_store(&home);
    let before = snapshot(&home);

    let (lsp, mut pool, io) = boot(&home);
    let out = resolve(&lsp, &mut pool, &[proj.to_str().unwrap()]);
    // a clean mount: no refused rows, no hints
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    assert_eq!(io.notes.borrow().len(), 0, "{:?}", io.notes.borrow());

    // go-to-definition lands in REAL files under the store — the pkg's
    // root (stamped at mod path ""), the mod child (its own path), and
    // the EMBEDDED group mounted through the pkg's own path row
    let doc = "use kit::{ KitBox };\nuse kit::extra::{ kit_extra };\nuse kitcore::{ core_tag };\nfn main() -> nil {\n}\n";
    let kit = walk_path(
        &home,
        &format!(
            "deps/{}/{}",
            rut_driver::store_dir("kit", KIT_PIN),
            rut_driver::UNPACK_DIR
        ),
    );
    assert_eq!(
        def_uris(&lsp, &mut pool, doc, "KitBox"),
        vec![file_uri(format!("{kit}/src/mod.rut"))],
        "the pkg root lands in the store"
    );
    assert_eq!(
        def_uris(&lsp, &mut pool, doc, "kit_extra"),
        vec![file_uri(format!("{kit}/src/extra/mod.rut"))],
        "the mod child lands at its own path (the dir-form spelling)"
    );
    assert_eq!(
        def_uris(&lsp, &mut pool, doc, "core_tag"),
        vec![file_uri(format!(
            "{kit}/deps/kitcore@feed0123456789abc/src/mod.rut"
        ))],
        "the embedded group mounts through the pkg's own path row"
    );

    // THE NO-WRITE LAW: the store tree is exactly where it was — the
    // walk only ever read (the Io grant has no write lane at all)
    assert_eq!(snapshot(&home), before, "the store stays untouched");
}

#[test]
fn a_missing_store_hints_once_per_dep_and_never_fetches() {
    let ws = scratch("store-miss");
    let proj = ws.join("proj");
    project(
        &proj,
        "walkapp",
        Some(&lock_text(vec![("kit", KIT_URL, KIT_PIN)])),
        &format!("{{\"kit\": {{\"url\": \"{KIT_URL}\"}}}}"),
        "fn main() -> nil {\n}\n",
    );
    let (lsp, mut pool, io) = boot(&ws.join("store")); // no store at all

    let out = resolve(&lsp, &mut pool, &[proj.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 1, "{out}");
    assert_eq!(
        out["missing"][0]["sha256"].as_str().unwrap(),
        KIT_PIN,
        "{out}"
    );
    // the wire shape the e2e locks: exactly the missing row's keys
    assert_eq!(
        out["missing"][0]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        vec!["cache_path", "sha256", "url"],
        "{out}"
    );
    assert!(
        out["missing"][0]["cache_path"]
            .as_str()
            .unwrap()
            .starts_with(".rut/cache/"),
        "the Rust-computed cache path rides: {out}"
    );
    // ONE hint, naming the CLI alternative — and re-resolving never
    // repeats it (the once-per-reason gate lives in the core)
    let out2 = resolve(&lsp, &mut pool, &[proj.to_str().unwrap()]);
    assert_eq!(out2["missing"].as_array().unwrap().len(), 1, "{out2}");
    let notes = io.notes.borrow();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("rut install"), "{notes:?}");
    assert!(notes[0].contains("kit"), "names the dep: {notes:?}");
    // never fetched: the only store probe was the dir check — no
    // artifact read, no meta read (the dir miss refuses first)
    assert!(
        io.reads.borrow().iter().all(|r| !r.contains("/deps/")),
        "no store file was read: {:?}",
        io.reads.borrow()
    );
}

#[test]
fn an_unpinned_row_hints_and_refuses_even_with_a_warm_store() {
    let ws = scratch("unpinned");
    let home = ws.join("store");
    let proj = ws.join("proj");
    // no lock at all — the row is unpinned — but the store it WOULD
    // mount from is warm (the address is derivable from the manifest
    // name alone); the door never opens
    project(
        &proj,
        "walkapp",
        None,
        &format!("{{\"kit\": {{\"url\": \"{KIT_URL}\"}}}}"),
        "fn main() -> nil {\n}\n",
    );
    kit_store(&home);
    let (lsp, mut pool, io) = boot(&home);

    let out = resolve(&lsp, &mut pool, &[proj.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 1, "{out}");
    assert!(out["missing"][0]["sha256"].is_null(), "{out}");
    let notes = io.notes.borrow();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("no sha256 pin"), "{notes:?}");
    // never mounted: the kit surface answers nothing
    let doc = "use kit::{ KitBox };\nfn main() -> nil {\n}\n";
    assert!(
        def_uris(&lsp, &mut pool, doc, "KitBox").is_empty(),
        "an unpinned row never mounts"
    );
}

#[test]
fn a_meta_mismatch_refuses_with_one_hint_and_never_mounts() {
    let ws = scratch("meta-mismatch");
    let home = ws.join("store");
    let proj = ws.join("proj");
    project(
        &proj,
        "walkapp",
        Some(&lock_text(vec![("kit", KIT_URL, KIT_PIN)])),
        &format!("{{\"kit\": {{\"url\": \"{KIT_URL}\"}}}}"),
        "fn main() -> nil {\n}\n",
    );
    // the dir sits at the right ADDRESS but vouches a different pin —
    // corrupt by definition, refused at the door
    let dir = store_dep(&home, "kit", KIT_PIN, KIT_URL);
    std::fs::write(
        dir.join(rut_driver::META_NAME),
        rut_driver::store_meta_json(&rut_driver::StoreMeta {
            name: "kit".into(),
            url: KIT_URL.into(),
            sha256: OTHER_PIN.into(),
            format: rut_driver::store_format(KIT_URL).into(),
            format_version: rut_driver::STORE_META_FORMAT_VERSION,
        }),
    )
    .expect("the meta overwrites");
    put(
        &dir,
        &format!("{}/src/mod.rut", rut_driver::UNPACK_DIR),
        "pub fn kit_tag() -> i32 {\n    return 1;\n}\n",
    );
    let (lsp, mut pool, io) = boot(&home);

    let out = resolve(&lsp, &mut pool, &[proj.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 1, "{out}");
    assert_eq!(
        out["missing"][0]["sha256"].as_str().unwrap(),
        KIT_PIN,
        "{out}"
    );
    let notes = io.notes.borrow();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("does not verify") && notes[0].contains("rut install"),
        "{notes:?}"
    );
    let doc = "use kit::{ kit_tag };\nfn main() -> nil {\n}\n";
    assert!(
        def_uris(&lsp, &mut pool, doc, "kit_tag").is_empty(),
        "a mismatched entry never mounts"
    );
    // a MALFORMED meta is the same REFUSAL — and the same REASON (the
    // entry at the address does not verify), so the once-gate keeps
    // the hint at one across the re-resolve
    std::fs::write(dir.join(rut_driver::META_NAME), "{ not json").expect("the meta corrupts");
    let out = resolve(&lsp, &mut pool, &[proj.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 1, "{out}");
    assert_eq!(io.notes.borrow().len(), 1, "{:?}", io.notes.borrow());
}

// ---- the walk's world laws ----

#[test]
fn first_mount_wins_across_roots() {
    let ws = scratch("first-mount");
    let r1 = ws.join("r1");
    let r2 = ws.join("r2");
    project(
        &r1,
        "appone",
        None,
        "{\"shared\": {\"path\": \"../shared-one\"}}",
        "fn main() -> nil {\n}\n",
    );
    project(
        &r2,
        "apptwo",
        None,
        "{\"shared\": {\"path\": \"../shared-two\"}}",
        "fn main() -> nil {\n}\n",
    );
    put(&ws.join("shared-one"), "rut.jsonc", &manifest("shared", ""));
    put(
        &ws.join("shared-one"),
        "src/mod.rut",
        "pub fn from_root_one() -> i32 {\n    return 1;\n}\n",
    );
    put(&ws.join("shared-two"), "rut.jsonc", &manifest("shared", ""));
    put(
        &ws.join("shared-two"),
        "src/mod.rut",
        "pub fn from_root_two() -> i32 {\n    return 2;\n}\n",
    );
    let (lsp, mut pool, _io) = boot(&ws.join("store"));
    let out = resolve(
        &lsp,
        &mut pool,
        &[r1.to_str().unwrap(), r2.to_str().unwrap()],
    );
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    // root 1 mounted `shared`; root 2's same-named pkg never re-mounts
    // (a diamond's pins cannot double-stamp) — its fn is NOT in the
    // world, and the first mount's file is the one that answers
    let doc = "use shared::{ from_root_one };\nfn main() -> nil {\n}\n";
    assert_eq!(
        def_uris(&lsp, &mut pool, doc, "from_root_one"),
        vec![file_uri(walk_path(&r1, "../shared-one/src/mod.rut"))],
        "the FIRST mount's file answers"
    );
    let doc = "use shared::{ from_root_two };\nfn main() -> nil {\n}\n";
    assert!(
        def_uris(&lsp, &mut pool, doc, "from_root_two").is_empty(),
        "root 2's same-named pkg never re-mounts"
    );
}

#[test]
fn dev_deps_walk_at_the_root_only() {
    let ws = scratch("dev-deps");
    let root = ws.join("proj");
    // the root: deps → mid, dev-deps → devkit (a dev-only tool still
    // walks at the root)
    put(
        &root,
        "rut.jsonc",
        &manifest_dev(
            "walkapp",
            "{\"mid\": {\"path\": \"../mid\"}}",
            "{\"devkit\": {\"path\": \"../devkit\"}}",
        ),
    );
    put(&root, "src/mod.rut", "fn main() -> nil {\n}\n");
    // a TRANSITIVE dep's dev table never enters the consumer's world:
    // mid's own dev row (neverdev) stays out at depth 2
    put(
        &ws.join("mid"),
        "rut.jsonc",
        &manifest_dev("mid", "{}", "{\"neverdev\": {\"path\": \"../neverdev\"}}"),
    );
    put(
        &ws.join("mid"),
        "src/mod.rut",
        "pub fn mid_tag() -> i32 {\n    return 2;\n}\n",
    );
    put(&ws.join("devkit"), "rut.jsonc", &manifest("devkit", ""));
    put(
        &ws.join("devkit"),
        "src/mod.rut",
        "pub fn dev_tag() -> i32 {\n    return 1;\n}\n",
    );
    put(&ws.join("neverdev"), "rut.jsonc", &manifest("neverdev", ""));
    put(
        &ws.join("neverdev"),
        "src/mod.rut",
        "pub fn never_tag() -> i32 {\n    return 4;\n}\n",
    );

    let (lsp, mut pool, _io) = boot(&ws.join("store"));
    let out = resolve(&lsp, &mut pool, &[root.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    let doc = "use devkit::{ dev_tag };\nuse mid::{ mid_tag };\nfn main() -> nil {\n}\n";
    assert_eq!(
        def_uris(&lsp, &mut pool, doc, "dev_tag"),
        vec![file_uri(walk_path(&root, "../devkit/src/mod.rut"))],
        "the ROOT's dev-deps walk"
    );
    assert_eq!(
        def_uris(&lsp, &mut pool, doc, "mid_tag").len(),
        1,
        "ordinary deps keep walking at depth"
    );
    let doc = "use neverdev::{ never_tag };\nfn main() -> nil {\n}\n";
    assert!(
        def_uris(&lsp, &mut pool, doc, "never_tag").is_empty(),
        "a transitive dep's dev table never enters the world"
    );
}

#[test]
fn direct_deps_accumulate_across_roots() {
    let ws = scratch("direct");
    let r1 = ws.join("r1");
    let r2 = ws.join("r2");
    project(
        &r1,
        "appone",
        None,
        "{\"alpha\": {\"path\": \"../alpha\"}}",
        "fn main() -> nil {\n}\n",
    );
    project(
        &r2,
        "apptwo",
        None,
        "{\"beta\": {\"path\": \"../beta\"}}",
        "fn main() -> nil {\n}\n",
    );
    // alpha's OWN dep is transitive from every root's view — gamma is
    // never a row of a root manifest
    put(
        &ws.join("alpha"),
        "rut.jsonc",
        &manifest("alpha", "{\"gamma\": {\"path\": \"../gamma\"}}"),
    );
    put(
        &ws.join("alpha"),
        "src/mod.rut",
        "pub fn alpha_tag() -> i32 {\n    return 1;\n}\n",
    );
    put(&ws.join("beta"), "rut.jsonc", &manifest("beta", ""));
    put(
        &ws.join("beta"),
        "src/mod.rut",
        "pub fn beta_tag() -> i32 {\n    return 2;\n}\n",
    );
    put(&ws.join("gamma"), "rut.jsonc", &manifest("gamma", ""));
    put(
        &ws.join("gamma"),
        "src/mod.rut",
        "pub fn gamma_tag() -> i32 {\n    return 3;\n}\n",
    );

    let (lsp, mut pool, _io) = boot(&ws.join("store"));
    resolve(
        &lsp,
        &mut pool,
        &[r1.to_str().unwrap(), r2.to_str().unwrap()],
    );
    // both roots walked — the bare doc's auto-import tier offers BOTH
    // roots' dep names (direct), and never the transitive gamma
    let uri = "file:///ws/walkbare.rut".to_string();
    let doc = "fn main() -> nil {\n}\n";
    take(pool.run_until(lsp.analyze(uri.clone(), doc.to_string())));
    let items = take(pool.run_until(lsp.complete(uri.clone(), doc.to_string(), 1, 0)));
    let arr = items.as_array().unwrap();
    for want in ["alpha_tag", "beta_tag"] {
        let hit = arr
            .iter()
            .find(|i| i["label"].as_str().unwrap_or("") == want)
            .unwrap_or_else(|| panic!("the direct dep's fn offers: {want}"));
        assert!(
            hit["additionalTextEdits"]
                .as_array()
                .is_some_and(|e| !e.is_empty()),
            "the offer carries the use-insert edit: {hit}"
        );
    }
    assert!(
        arr.iter().all(|i| i["label"] != "gamma_tag"),
        "a transitive-only dep never offers: {:?}",
        arr.iter().map(|i| &i["label"]).collect::<Vec<_>>()
    );

    // … and once the doc `use`s a dep, its fn is a PLAIN bare item —
    // exactly one offer, no import edit riding (the old push-lane
    // bare-scope law, re-homed through the walk)
    let used = "use alpha::{ alpha_tag };
fn main() -> nil {
}
";
    take(pool.run_until(lsp.analyze(uri.clone(), used.to_string())));
    let items = take(pool.run_until(lsp.complete(uri, used.to_string(), 1, 0)));
    let hits: Vec<&serde_json::Value> = items
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["label"] == "alpha_tag")
        .collect();
    assert_eq!(hits.len(), 1, "one `alpha_tag` item: {hits:?}");
    assert!(
        hits[0]["additionalTextEdits"].is_null(),
        "the imported fn completes plain: {}",
        hits[0]
    );
}

#[test]
fn the_mod_tree_spellings_mount_and_hint_on_both() {
    let ws = scratch("mod-spells");
    let root = ws.join("proj");
    project(
        &root,
        "walkapp",
        None,
        "{\"gadgets\": {\"path\": \"../gadgets\"}}",
        "fn main() -> nil {\n}\n",
    );
    let g = ws.join("gadgets");
    put(&g, "rut.jsonc", &manifest("gadgets", ""));
    put(
        &g,
        "src/mod.rut",
        "pub mod dirform;\npub mod fileform;\npub mod both;\n",
    );
    // the DIRECTORY form: NAME/mod.rut beside the declaring file, with
    // the recursion continuing beside the CHILD file
    put(&g, "src/dirform/mod.rut", "pub mod nested;\n");
    put(
        &g,
        "src/dirform/nested/mod.rut",
        "pub fn nested_tag() -> i32 {\n    return 1;\n}\n",
    );
    // the SINGLE-FILE form: NAME.rut beside the declaring file
    put(
        &g,
        "src/fileform.rut",
        "pub fn file_tag() -> i32 {\n    return 2;\n}\n",
    );
    // BOTH under one edge — the loader's loud ambiguity, hinted once,
    // neither mounts
    put(
        &g,
        "src/both/mod.rut",
        "pub fn dir_spelling() -> i32 {\n    return 3;\n}\n",
    );
    put(
        &g,
        "src/both.rut",
        "pub fn file_spelling() -> i32 {\n    return 4;\n}\n",
    );

    let (lsp, mut pool, io) = boot(&ws.join("store"));
    let out = resolve(&lsp, &mut pool, &[root.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    // the ambiguity hint fired exactly once
    let notes = io.notes.borrow();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("both/mod.rut and both.rut exist"),
        "{notes:?}"
    );
    // both legal spellings mounted — the dir form's nested child at
    // its own path, the file form at its own file (origins carry the
    // joined `..` — the HOST env normalizes, the walk never munges)
    let doc = "use gadgets::dirform::nested::{ nested_tag };\nuse gadgets::fileform::{ file_tag };\nfn main() -> nil {\n}\n";
    assert_eq!(
        def_uris(&lsp, &mut pool, doc, "nested_tag"),
        vec![file_uri(walk_path(
            &root,
            "../gadgets/src/dirform/nested/mod.rut"
        ))],
        "the dir form's nested child mounts beside the declaring file"
    );
    assert_eq!(
        def_uris(&lsp, &mut pool, doc, "file_tag"),
        vec![file_uri(walk_path(&root, "../gadgets/src/fileform.rut"))],
        "the single-file spelling mounts"
    );
    // the mod EDGE itself jumps at the mounted child — the doc AT the
    // root file's own origin (its stamped twin keys the lane)
    let root_src = walk_path(&root, "../gadgets/src/mod.rut");
    let root_doc = "pub mod dirform;\npub mod fileform;\npub mod both;\n";
    let uris = def_uris_at(&lsp, &mut pool, &root_src, root_doc, "dirform");
    assert_eq!(
        uris,
        vec![file_uri(walk_path(&root, "../gadgets/src/dirform/mod.rut"))],
        "the edge names its child file"
    );
    // and neither ambiguous spelling indexed
    let doc = "use gadgets::both::{ dir_spelling };\nfn main() -> nil {\n}\n";
    assert!(
        def_uris(&lsp, &mut pool, doc, "dir_spelling").is_empty(),
        "the ambiguous edge mounts neither"
    );
}

#[test]
fn the_depth_budget_truncates_the_walk() {
    let ws = scratch("depth");
    let root = ws.join("proj");
    // a chain 10 manifests deep: root (depth 1) → c2 → … → c10. The
    // cap is 8 — everything past c8 never mounts, and the walk ends
    // without a fuss (truncated, not failed)
    project(
        &root,
        "walkapp",
        None,
        "{\"c2\": {\"path\": \"../c2\"}}",
        "fn main() -> nil {\n}\n",
    );
    for n in 2..=10usize {
        let dir = ws.join(format!("c{n}"));
        let deps = if n < 10 {
            format!("{{\"c{}\": {{\"path\": \"../c{}\"}}}}", n + 1, n + 1)
        } else {
            String::new()
        };
        put(&dir, "rut.jsonc", &manifest(&format!("c{n}"), &deps));
        put(
            &dir,
            "src/mod.rut",
            &format!("pub fn chain{n}_tag() -> i32 {{\n    return {n};\n}}\n"),
        );
    }
    let (lsp, mut pool, _io) = boot(&ws.join("store"));
    let out = resolve(&lsp, &mut pool, &[root.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    let doc = "use c8::{ chain8_tag };\nfn main() -> nil {\n}\n";
    assert_eq!(
        def_uris(&lsp, &mut pool, doc, "chain8_tag").len(),
        1,
        "depth 8 still mounts (the cap is inclusive)"
    );
    let doc = "use c9::{ chain9_tag };\nfn main() -> nil {\n}\n";
    assert!(
        def_uris(&lsp, &mut pool, doc, "chain9_tag").is_empty(),
        "depth 9 never mounts — the budget truncates"
    );
}

#[test]
fn the_file_budget_truncates_the_walk() {
    let ws = scratch("files");
    let root = ws.join("proj");
    // 205 single-file children under one root: the entries row + 199
    // children fit the 200-file budget, the rest skip
    let mut decls = String::new();
    for n in 0..205 {
        decls.push_str(&format!("pub mod kid{n};\n"));
    }
    project(&root, "walkapp", None, "", &decls);
    for n in 0..205 {
        put(
            &root,
            &format!("src/kid{n}.rut"),
            &format!("pub fn kid{n}_tag() -> i32 {{\n    return {n};\n}}\n"),
        );
    }
    let (lsp, mut pool, _io) = boot(&ws.join("store"));
    let out = resolve(&lsp, &mut pool, &[root.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    let doc = "use walkapp::kid3::{ kid3_tag };\nuse walkapp::kid204::{ kid204_tag };\nfn main() -> nil {\n}\n";
    assert_eq!(
        def_uris(&lsp, &mut pool, doc, "kid3_tag").len(),
        1,
        "early children mount"
    );
    assert!(
        def_uris(&lsp, &mut pool, doc, "kid204_tag").is_empty(),
        "the 200-file budget truncates before kid204"
    );
}

#[test]
fn a_malformed_manifest_hints_once_and_mounts_nothing() {
    let ws = scratch("manifest-bad");
    let root = ws.join("proj");
    put(
        &root,
        "rut.lock",
        &lock_text(vec![("kit", KIT_URL, KIT_PIN)]),
    );
    put(&root, "rut.jsonc", "{ \"name\": \"walkapp\", ");
    put(&root, "src/mod.rut", "fn main() -> nil {\n}\n");
    let (lsp, mut pool, io) = boot(&ws.join("store"));

    let out = resolve(&lsp, &mut pool, &[root.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    let notes = io.notes.borrow();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("rut.jsonc"), "{notes:?}");
    resolve(&lsp, &mut pool, &[root.to_str().unwrap()]);
    assert_eq!(
        io.notes.borrow().len(),
        1,
        "the manifest hint fires once per reason too"
    );
}

#[test]
fn a_path_dep_with_no_manifest_hints_once() {
    let ws = scratch("path-nomanifest");
    let root = ws.join("proj");
    project(
        &root,
        "walkapp",
        None,
        "{\"lost\": {\"path\": \"../lost\"}}",
        "fn main() -> nil {\n}\n",
    );
    std::fs::create_dir_all(ws.join("lost")).expect("the empty dir creates");
    let (lsp, mut pool, io) = boot(&ws.join("store"));
    let out = resolve(&lsp, &mut pool, &[root.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    let notes = io.notes.borrow();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("has no rut.jsonc") && notes[0].contains("one manifest per module"),
        "{notes:?}"
    );
}

// ---- the entry's own laws ----

#[test]
fn resolving_without_an_env_is_the_error_envelope() {
    let (lsp, mut pool) = RutLsp::spawn_native();
    let out = resolve(&lsp, &mut pool, &["/ws/nowhere"]);
    assert!(
        out["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no host environment"),
        "the env-less core refuses loudly: {out}"
    );
}

#[test]
fn malformed_roots_json_is_the_error_envelope() {
    let (lsp, mut pool, _io) = boot(Path::new("/nonexistent-rut-home"));
    let out = take(pool.run_until(lsp.resolve_deps("{\"not\":\"an array".to_string())));
    assert!(
        out["error"].as_str().is_some_and(|e| !e.is_empty()),
        "{out}"
    );
}

#[test]
fn resolve_twice_answers_the_same_world() {
    let ws = scratch("idempotent");
    let home = ws.join("store");
    let proj = ws.join("proj");
    project(
        &proj,
        "walkapp",
        Some(&lock_text(vec![("kit", KIT_URL, KIT_PIN)])),
        &format!("{{\"kit\": {{\"url\": \"{KIT_URL}\"}}}}"),
        "fn main() -> nil {\n}\n",
    );
    kit_store(&home);
    let (lsp, mut pool, io) = boot(&home);
    resolve(&lsp, &mut pool, &[proj.to_str().unwrap()]);
    let doc = "use kit::{ KitBox };\nuse kitcore::{ core_tag };\nfn main() -> nil {\n}\n";
    let first = def_uris(&lsp, &mut pool, doc, "KitBox");
    assert_eq!(first.len(), 1, "{first:?}");
    // the re-resolve: same world (replace-by-origin keeps exactly one
    // of each), no new hints, nothing doubled
    resolve(&lsp, &mut pool, &[proj.to_str().unwrap()]);
    assert_eq!(def_uris(&lsp, &mut pool, doc, "KitBox"), first);
    let twice = def_uris(&lsp, &mut pool, doc, "core_tag");
    assert_eq!(
        twice.len(),
        1,
        "the embedded group stays singular: {twice:?}"
    );
    assert_eq!(io.notes.borrow().len(), 0, "a clean world stays silent");
}

// ---- the pruned push lane's behaviors, re-homed through the walk ----

/// the named-dep use-path tier (the old `add_def_named` ABI test): a
/// PATH dep's public names complete at `use toolkit::⏐`, and the
/// module name itself at `use ⏐` — everything through the env, zero
/// push entries
#[test]
fn a_path_dep_feeds_the_use_path_tier() {
    let ws = scratch("pathtier");
    let proj = ws.join("proj");
    project(
        &proj,
        "walkapp",
        None,
        "{\"toolkit\": {\"path\": \"../toolkit\"}}",
        "fn main() -> nil {\n}\n",
    );
    put(&ws.join("toolkit"), "rut.jsonc", &manifest("toolkit", ""));
    put(
        &ws.join("toolkit"),
        "src/mod.rut",
        "pub class Widget {\n    id: i32;\n}\npub fn tag() -> i32 {\n    return 1;\n}\n",
    );
    let (lsp, mut pool, io) = boot(&ws.join("store"));

    let out = resolve(&lsp, &mut pool, &[proj.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    assert_eq!(io.notes.borrow().len(), 0, "{:?}", io.notes.borrow());

    // `use toolkit::⏐` — the dep's public names
    let uri = "file:///ws/main.rut".to_string();
    let doc = "use toolkit::\nfn main() -> nil {\n}\n";
    take(pool.run_until(lsp.analyze(uri.clone(), doc.to_string())));
    let items = take(pool.run_until(lsp.complete(uri.clone(), doc.to_string(), 0, 13)));
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(
        labels.contains(&"Widget"),
        "the dep type completes: {labels:?}"
    );
    assert!(labels.contains(&"tag"), "the dep fn completes: {labels:?}");
    // `use ⏐` — the module-name tier sees the walked dep
    let mods = take(pool.run_until(lsp.complete(uri, doc.to_string(), 0, 4)));
    let names: Vec<&str> = mods
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"toolkit"),
        "the module name completes: {names:?}"
    );
}

/// the real std transport through the store (the old `add_bundle`
/// pouch test): a `.zip` dep whose `unpack/` is exactly what
/// install's read_srczip lane lays — the surface type-checks and F12
/// lands in the REAL tree, never a `bundle:` origin
const POUCH_ZIP: &[u8] = include_bytes!("../../../dist/std/pouch.zip");
const POUCH_URL: &str = "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v15/dist/std/pouch.zip";
/// self-consistent only — readers never re-hash (meta vouches the pin)
const POUCH_PIN: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

/// THE FULL STD WORLD (the playground's own shape): the store's std
/// set declared + pinned + mounted at once. The pinning law this gate
/// owns: the check lane must keep answering through a world that
/// carries `calc` — `math.rut`'s `pub use calc_host::{ PI, .. }`
/// resolves against calc_host's SURFACE LETS (the `pub let PI: f64 =
/// ..;` rows fold into the host body's consts inside the lowering;
/// the manifest's `consts` table is retired), and without them those
/// fourteen unknown-import errors stamp `calc/math` and eat every one
/// of the doc's own diags (foreign squiggles skip by law, and the
/// graph stops at the failing pkg).
///
/// calc rides the SURFACE-LET world the repo tree now spells (the
/// next envelope's shape): its store entry is hand-laid from the
/// tree's own files — the v17 zip's exact layout, a rewritten
/// manifest pointing at the embedded `deps/calc_host@<fp16>/` group.
/// The other eight ride the committed frozen zips (no consts of
/// their own — the strict store lane reads them clean).
const STD_ZIPS: &[(&str, &[u8])] = &[
    ("flow", include_bytes!("../../../dist/std/flow.zip")),
    ("futures", include_bytes!("../../../dist/std/futures.zip")),
    ("http", include_bytes!("../../../dist/std/http.zip")),
    ("ink", include_bytes!("../../../dist/std/ink.zip")),
    ("json", include_bytes!("../../../dist/std/json.zip")),
    ("nmapset", include_bytes!("../../../dist/std/nmapset.zip")),
    ("pouch", include_bytes!("../../../dist/std/pouch.zip")),
    ("strbuild", include_bytes!("../../../dist/std/strbuild.zip")),
];
/// one made-up pin the nine ride (distinct per name — the address is
/// `<name>@<pin16>`; readers never re-hash, meta vouches)
fn nine_pin(name: &str) -> String {
    let salt = name.len();
    (0..64)
        .map(|i| char::from_digit(((i * 7 + salt * 13) % 16) as u32, 16).expect("hex digit"))
        .collect()
}

/// the calc store entry, hand-laid in the next envelope's shape: the
/// rewritten manifest (its dep row pointing INSIDE at the embedded
/// group), the tree's own sources, and the calc_host group's surface
/// carrying the `pub let` PI family
fn calc_store_entry(home: &Path, pin: &str, url: &str) {
    let dir = store_dep(home, "calc", pin, url);
    let unpack = dir.join(rut_driver::UNPACK_DIR);
    // the rewritten manifest — no wire pair, the row points inside
    put(
        &unpack,
        "rut.jsonc",
        "{\n  \"name\": \"calc\",\n  \"deps\": {\n    \"calc_host\": { \"path\": \
         \"deps/calc_host@0123456789abcdef\" }\n  }\n}\n",
    );
    put(
        &unpack,
        "src/mod.rut",
        include_str!("../../../rut/calc/src/mod.rut"),
    );
    put(
        &unpack,
        "src/math.rut",
        include_str!("../../../rut/calc/src/math.rut"),
    );
    // the embedded host group: the directory manifest grammar + the
    // surface whose `pub let` rows carry the PI family
    let group = unpack.join("deps/calc_host@0123456789abcdef");
    put(
        &group,
        "rut.jsonc",
        "{\n  \"name\": \"calc_host\",\n  \"type\": \"host\",\n  \"entry\": { \"type\": \
         \"./calc.d.rut\" }\n}\n",
    );
    put(
        &group,
        "src/calc.d.rut",
        include_str!("../../../rut/calc_host/src/calc.d.rut"),
    );
}

#[test]
fn the_full_std_world_keeps_the_check_lane_alive() {
    let ws = scratch("nine");
    let home = ws.join("store");
    let proj = ws.join("proj");
    let url = |n: &str| format!("https://cdn.example/rut/dist/std/{n}.zip");
    let lock = rut_driver::render_lock(&rut_driver::LockFile::from_rows(
        STD_ZIPS
            .iter()
            .map(|(n, _)| rut_driver::LockRow {
                pkg: (*n).to_string(),
                url: url(n),
                sha256: nine_pin(n),
            })
            .chain([rut_driver::LockRow {
                pkg: "calc".to_string(),
                url: url("calc"),
                sha256: nine_pin("calc"),
            }])
            .collect(),
    ))
    .to_string();
    let deps = format!(
        "{{{}}}",
        STD_ZIPS
            .iter()
            .map(|(n, _)| format!("\"{n}\": {{\"url\": \"{}\"}}", url(n)))
            .chain([format!("\"calc\": {{\"url\": \"{}\"}}", url("calc"))])
            .collect::<Vec<_>>()
            .join(",")
    );
    project(
        &proj,
        "playground",
        Some(&lock),
        &deps,
        "entry fn main() {\n}\n",
    );
    for (n, zip) in STD_ZIPS {
        let dir = store_dep(&home, n, &nine_pin(n), &url(n));
        for (rel, bytes) in rut_driver::bundle::read_srczip(zip).expect("the std zip unpacks") {
            let p = dir.join(rut_driver::UNPACK_DIR).join(&rel);
            std::fs::create_dir_all(p.parent().expect("a parent")).expect("the parent creates");
            std::fs::write(p, bytes).expect("the file writes");
        }
    }
    calc_store_entry(&home, &nine_pin("calc"), &url("calc"));

    let (lsp, mut pool, io) = boot(&home);
    let out = resolve(&lsp, &mut pool, &[proj.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    assert_eq!(io.notes.borrow().len(), 0, "{:?}", io.notes.borrow());

    // the doc IS the project's entry (the uri the walk's origin stamps)
    // — the check lane compiles it against the whole mounted world
    let uri = proj.join("src/mod.rut").to_str().unwrap().to_string();
    // (a) the doc's own type error surfaces — the surface-let consts
    // keep calc's re-exports from eating it
    let bad = "entry fn main() {\n    let x = 1 + \"nope\";\n}\n";
    let a = take(pool.run_until(lsp.analyze(uri.clone(), bad.to_string())));
    assert!(
        a["diags"].as_array().unwrap().iter().any(|d| d["message"]
            .as_str()
            .unwrap_or_default()
            .contains("equal width")),
        "the doc's own diag surfaces through the calc-carrying world: {a}"
    );
    // (b) the std caller forms check clean: the use-bound module path
    // (`math::sqrt` / `math::PI` through the pub-use re-export) and a
    // plain std use, over the SAME store-mounted world
    let used = "use calc::{ math };\nuse ink::{ Logger };\nuse pouch::{ Vec };\nentry fn main() {\n    let log = Logger(\"w\");\n    log.info(f\"{math::sqrt(4.0)} {math::PI}\");\n    let v: Vec<i32> = Vec::new();\n    v.push(1);\n}\n";
    let a = take(pool.run_until(lsp.analyze(uri, used.to_string())));
    assert_eq!(
        a["diags"].as_array().unwrap().len(),
        0,
        "the std caller forms check clean: {a}"
    );
}

#[test]
fn a_std_zip_store_dep_mounts_the_pouch_surface() {
    let ws = scratch("pouch");
    let home = ws.join("store");
    let proj = ws.join("proj");
    project(
        &proj,
        "walkapp",
        Some(&lock_text(vec![("pouch", POUCH_URL, POUCH_PIN)])),
        &format!("{{\"pouch\": {{\"url\": \"{POUCH_URL}\"}}}}"),
        "fn main() -> nil {\n}\n",
    );
    let dir = store_dep(&home, "pouch", POUCH_PIN, POUCH_URL);
    for (rel, bytes) in rut_driver::bundle::read_srczip(POUCH_ZIP).expect("the pouch zip unpacks") {
        let p = dir.join(rut_driver::UNPACK_DIR).join(&rel);
        std::fs::create_dir_all(p.parent().expect("a parent")).expect("the parent creates");
        std::fs::write(p, bytes).expect("the file writes");
    }

    let (lsp, mut pool, io) = boot(&home);
    let out = resolve(&lsp, &mut pool, &[proj.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    assert_eq!(io.notes.borrow().len(), 0, "{:?}", io.notes.borrow());

    // the surface resolves and type-checks through the store mount
    let uri = "file:///ws/uses-pouch.rut".to_string();
    let doc = "use pouch::{ Vec };\nfn main() -> nil {\n    let v: Vec<i32> = Vec::new();\n    v.push(1);\n}\n";
    let a = take(pool.run_until(lsp.analyze(uri, doc.to_string())));
    assert_eq!(
        a["diags"].as_array().unwrap().len(),
        0,
        "the store surface type-checks: {a}"
    );
    // F12 lands in the REAL unpack tree
    let targets = def_uris(&lsp, &mut pool, doc, "Vec");
    assert!(!targets.is_empty(), "Vec resolves");
    let prefix = file_uri(walk_path(
        &home,
        &format!(
            "deps/{}/{}",
            rut_driver::store_dir("pouch", POUCH_PIN),
            rut_driver::UNPACK_DIR
        ),
    ));
    assert!(
        targets.iter().all(|u| u.starts_with(&prefix)),
        "definitions land under the store's unpack tree: {targets:?}"
    );
}

/// the HOST flavor of the same transport: a `type = "host"` pkg whose
/// surface is a MOD TREE (the myhost shape — `pub mod net;` in the
/// root `.d.rut`, children in `NAME/mod.d.rut`, recursively). The
/// walk mounts the tree Decl-flavored (the loader's own law): the rows
/// complete, the surface really type-checks (a host tree that never
/// mounts its children lowers inert — the check lane then squiggles
/// `cannot resolve` on a dep the editor demonstrably serves; the
/// playground's custom-dep lane found exactly that), and F12 lands in
/// the real child files.
const MYHOST_ZIP: &[u8] = include_bytes!("../../../dist/std/myhost.zip");
const MYHOST_URL: &str = "https://cdn.jsdelivr.net/gh/hpp2334/rut@std-v16/dist/std/myhost.zip";
/// self-consistent only — readers never re-hash (meta vouches the pin)
const MYHOST_PIN: &str = "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";

#[test]
fn a_std_zip_store_dep_mounts_the_host_mod_tree() {
    let ws = scratch("myhost");
    let home = ws.join("store");
    let proj = ws.join("proj");
    project(
        &proj,
        "walkapp",
        Some(&lock_text(vec![("myhost", MYHOST_URL, MYHOST_PIN)])),
        &format!("{{\"myhost\": {{\"url\": \"{MYHOST_URL}\"}}}}"),
        "fn main() -> nil {\n}\n",
    );
    let dir = store_dep(&home, "myhost", MYHOST_PIN, MYHOST_URL);
    for (rel, bytes) in rut_driver::bundle::read_srczip(MYHOST_ZIP).expect("the myhost zip unpacks")
    {
        let p = dir.join(rut_driver::UNPACK_DIR).join(&rel);
        std::fs::create_dir_all(p.parent().expect("a parent")).expect("the parent creates");
        std::fs::write(p, bytes).expect("the file writes");
    }

    let (lsp, mut pool, io) = boot(&home);
    let out = resolve(&lsp, &mut pool, &[proj.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");
    assert_eq!(io.notes.borrow().len(), 0, "{:?}", io.notes.borrow());

    // the rows complete through the mounted tree — the flat row and
    // the module child
    let uri = "file:///ws/uses-myhost.rut".to_string();
    let doc = "use myhost::\nfn main() -> nil {\n}\n";
    take(pool.run_until(lsp.analyze(uri.clone(), doc.to_string())));
    let items = take(pool.run_until(lsp.complete(uri, doc.to_string(), 0, 12)));
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(labels.contains(&"legacy_ping"), "{labels:?}");
    assert!(labels.contains(&"net"), "{labels:?}");

    // the check lane fires at the PROJECT's own entry (the doc's pkg
    // context rides the walk's origin stamp): the host surface really
    // type-checks — and the lane demonstrably ran, pinned by the
    // positive squiggle on the next line
    let entry_uri = proj.join("src/mod.rut").to_str().unwrap().to_string();
    let used = "use myhost::{ legacy_ping };\nfn main() -> nil {\n    let ok = legacy_ping();\n}\n";
    let a = take(pool.run_until(lsp.analyze(entry_uri.clone(), used.to_string())));
    assert_eq!(
        a["diags"].as_array().unwrap().len(),
        0,
        "the host surface type-checks through the mounted tree: {a}"
    );
    let bad = "fn main() -> nil {\n    let x = 1 + \"nope\";\n}\n";
    let a = take(pool.run_until(lsp.analyze(entry_uri.clone(), bad.to_string())));
    assert!(
        a["diags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["message"].as_str().unwrap().contains("equal width")),
        "the check lane demonstrably ran: {a}"
    );

    // F12 lands in the REAL child file of the mounted decl tree
    let netdoc = "use myhost::net::{ connect };\nfn main() -> nil {\n}\n";
    let targets = def_uris_at(&lsp, &mut pool, &entry_uri, netdoc, "connect");
    let want = file_uri(walk_path(
        &dir,
        &format!("{}/src/net/{}", rut_driver::UNPACK_DIR, "mod.d.rut"),
    ));
    assert_eq!(targets, vec![want], "the net child lands at its own file");
}

/// the store-mounted mod tree feeds completion AND the segment hover
/// (the old `add_bundle_mounts_a_mod_tree` law, re-homed)
#[test]
fn a_store_mounted_mod_tree_feeds_completion_and_hover() {
    let ws = scratch("storemods");
    let home = ws.join("store");
    let proj = ws.join("proj");
    project(
        &proj,
        "walkapp",
        Some(&lock_text(vec![("kit", KIT_URL, KIT_PIN)])),
        &format!("{{\"kit\": {{\"url\": \"{KIT_URL}\"}}}}"),
        "fn main() -> nil {\n}\n",
    );
    kit_store(&home);
    let (lsp, mut pool, _io) = boot(&home);

    let out = resolve(&lsp, &mut pool, &[proj.to_str().unwrap()]);
    assert_eq!(out["missing"].as_array().unwrap().len(), 0, "{out}");

    // `use kit::extra::⏐` — the mounted child's exports
    let uri = "file:///ws/uses-kit.rut".to_string();
    let doc = "use kit::extra::\nfn main() -> nil {\n}\n";
    take(pool.run_until(lsp.analyze(uri.clone(), doc.to_string())));
    let items = take(pool.run_until(lsp.complete(uri.clone(), doc.to_string(), 0, 16)));
    let labels: Vec<&str> = items
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect();
    assert!(labels.contains(&"kit_extra"), "{labels:?}");
    // the segment hover resolves through the mounted tree (the
    // well-formed use — the typing-state doc above is completion's)
    pool.run_until(lsp.forget(uri.clone()));
    let used = "use kit::extra::{ kit_extra };\nfn main() -> nil {\n}\n";
    take(pool.run_until(lsp.analyze(uri.clone(), used.to_string())));
    let h = take(pool.run_until(lsp.hover(uri, 0, 11))); // inside `extra`
    let md = h["contents"]["value"].as_str().unwrap();
    assert!(md.contains("file module"), "{md}");
    assert!(md.contains("extra/mod.rut"), "{md}");
}
