//! Pure pack tests (moved from the driver's bundle tests): determinism,
//! the bundle-shaped refusal, and the archive's entry listing. The
//! session round trips stay with rut-driver — this crate never builds
//! a session. The in-memory [`Source`] at the bottom is the wasm/test
//! shape of the dependency injection.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rut_bundle::{pack, FsSource, Source};

/// A one-file module in a temp dir (one directory, one entry file).
fn make_dir(base: &Path) -> PathBuf {
    let dir = base.join("mod");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("rut.toml"),
        // bundle-shaped: the keys a `rut pack` needs are already there,
        // and directory loading ignores them (RFC 0038 §2, layout v4 —
        // v4 adds RFC 0041 §5's `entry.libs` files; the packer emits v4)
        "format = \"rutbundle\"\nformat_version = 4\nname = \"mod\"\nentry.lib = \"./mod.rut\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("mod.rut"),
        "pub fn seven() -> i32 { return 7; }\nfn main() -> i32 { return seven(); }\n",
    )
    .unwrap();
    dir
}

#[test]
fn packing_is_deterministic() {
    let base = std::env::temp_dir().join(format!("rut-bundle-det-{}", std::process::id()));
    let dir = make_dir(&base);
    let a = pack(&dir, &FsSource).unwrap();
    let b = pack(&dir, &FsSource).unwrap();
    assert_eq!(a, b, "same dir => byte-identical bundle");
    // the manifest and the entry are in the archive, manifest first
    let entries = rut_bundle::parse_bundle(&a).unwrap();
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["rut.toml", "mod.rut"]);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn pack_requires_bundle_shaped_manifest() {
    // a bare directory manifest packs to a bundle no loader would accept,
    // so `pack` refuses up front and says what to add
    let base = std::env::temp_dir().join(format!("rut-bundle-shape-{}", std::process::id()));
    let dir = base.join("mod");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("rut.toml"), "name = \"mod\"\nentry.lib = \"./m.rut\"\n").unwrap();
    std::fs::write(dir.join("m.rut"), "pub fn f() -> i32 { return 0; }\n").unwrap();
    let err = pack(&dir, &FsSource).unwrap_err();
    assert!(err.contains("bundle-shaped"), "{err}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn packs_the_dep_graph_and_lists_its_entries() {
    // the pack half of the driver's load-by-name test: a v2+ bundle
    // embeds the whole `[deps]` graph — source deps AND host-pkg
    // (`.d.rut`) deps — each under its own `<pkg>/` group
    let base = std::env::temp_dir().join(format!("rut-bundle-list-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);

    // the dep: a source pkg
    let m = base.join("m");
    std::fs::create_dir_all(&m).unwrap();
    std::fs::write(m.join("rut.toml"), "name = \"m\"\nentry.lib = \"./m.rut\"\n").unwrap();
    std::fs::write(m.join("m.rut"), "pub fn four() -> i32 { return 4; }\n").unwrap();
    // the dep's dep: a host pkg (declaration-only surface)
    let s = base.join("s");
    std::fs::create_dir_all(&s).unwrap();
    std::fs::write(
        s.join("rut.toml"),
        "name = \"s\"\nentry.type = \"./s.d.rut\"\nhost_scope = \"s\"\n",
    )
    .unwrap();
    std::fs::write(s.join("s.d.rut"), "pub host fn ping(x: i32) -> i32;\n").unwrap();
    // m uses s
    std::fs::write(
        m.join("rut.toml"),
        "name = \"m\"\nentry.lib = \"./m.rut\"\n[deps]\ns = { path = \"../s\" }\n",
    )
    .unwrap();

    // the consumer: m (+ transitively s)
    let main = base.join("consumer");
    std::fs::create_dir_all(&main).unwrap();
    std::fs::write(
        main.join("rut.toml"),
        "format = \"rutbundle\"\nformat_version = 4\nname = \"main\"\nentry.lib = \"./entry.rut\"\n[deps]\nm = { path = \"../m\" }\n",
    )
    .unwrap();
    std::fs::write(
        main.join("entry.rut"),
        "use m::{ four };\npub fn main() -> i32 { return four(); }\n",
    )
    .unwrap();

    let bytes = pack(&main, &FsSource).unwrap();
    let entries = rut_bundle::parse_bundle(&bytes).unwrap();
    let mut names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        vec!["entry.rut", "m/m.rut", "m/rut.toml", "rut.toml", "s/rut.toml", "s/s.d.rut"],
        "the dep graph rides the bundle, recursively"
    );

    // determinism with deps too
    assert_eq!(bytes, pack(&main, &FsSource).unwrap(), "same dir => byte-identical bundle");

    let _ = std::fs::remove_dir_all(&base);
}

/// The in-memory [`Source`] — the wasm/test shape: a path→bytes map.
#[derive(Default)]
struct MapSource(BTreeMap<PathBuf, Vec<u8>>);

impl Source for MapSource {
    fn read(&self, p: &Path) -> Result<Vec<u8>, String> {
        self.0
            .get(p)
            .cloned()
            .ok_or_else(|| format!("cannot read {}: no such path", p.display()))
    }
}

#[test]
fn packs_over_an_in_memory_source() {
    // no filesystem at all: the whole module lives in a map, the packer
    // reads only the manifest-named paths through the trait
    let root = PathBuf::from("mod");
    let mut files = MapSource::default();
    files.0.insert(
        root.join("rut.toml"),
        b"format = \"rutbundle\"\nformat_version = 4\nname = \"mod\"\nentry.lib = \"./mod.rut\"\n"
            .to_vec(),
    );
    files.0.insert(
        root.join("mod.rut"),
        b"pub fn seven() -> i32 { return 7; }\nfn main() -> i32 { return seven(); }\n".to_vec(),
    );

    let a = pack(&root, &files).unwrap();
    let b = pack(&root, &files).unwrap();
    assert_eq!(a, b, "same map => byte-identical bundle");

    // the bytes match a filesystem pack of the same files — the source
    // is the only variable, and it changes nothing
    let base = std::env::temp_dir().join(format!("rut-bundle-map-{}", std::process::id()));
    let dir = make_dir(&base);
    let from_fs = pack(&dir, &FsSource).unwrap();
    assert_eq!(a, from_fs, "the map packs byte-identically to the same directory");
    let _ = std::fs::remove_dir_all(&base);

    // a manifest-named file the map lacks is a read error through the trait
    files.0.remove(&root.join("mod.rut"));
    let err = pack(&root, &files).unwrap_err();
    assert!(err.contains("cannot read"), "{err}");
}
