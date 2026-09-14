//! The example's gate: drive the moderator through the typed `Plugin`
//! surface and assert the exact transcript — `cargo test --workspace`
//! runs it. The plugin loads from a module directory (`plugin/rut.toml`)
//! and from a packed `.rutbundle` (RFC 0038); both forms must behave
//! identically.

use std::path::Path;

use plugin::Plugin;

fn dir() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/plugin"))
}

fn limits() -> rut_vm::interp::Limits {
    rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    }
}

fn load_dir() -> Plugin {
    Plugin::load(dir(), &limits()).unwrap()
}

/// The scripted session; returns `(transcript, total)`.
fn session(p: &mut Plugin) -> (Vec<String>, i64) {
    p.join("ada").unwrap();
    p.msg("ada", "hello world").unwrap();
    p.msg("ada", "!stats").unwrap();
    p.msg("bob", "hi").unwrap();
    p.msg("bob", "spam").unwrap();
    p.msg("bob", "spam").unwrap();
    p.msg("bob", "let me back in").unwrap();
    p.tick(1).unwrap();
    p.leave("ada").unwrap();
    let total = p.shutdown().unwrap();
    (p.transcript(), total)
}

fn want_transcript() -> Vec<String> {
    vec![
        "[system] *** ada joined (1 online)".to_string(),
        "[broadcast] <ada> hello world".to_string(),
        "[direct] ada: 1 online, 1 msgs, 0 mutes".to_string(),
        "[broadcast] <bob> hi".to_string(),
        "[broadcast] <bob> spam".to_string(),
        "[system] bob muted for flooding".to_string(),
        "[muted] bob".to_string(),
        "[system] [tick 1] 1 online, 3 msgs, 1 mutes".to_string(),
        "[system] ada left (0 online)".to_string(),
        "[system] shutdown: 0 online, 3 msgs, 1 mutes".to_string(),
    ]
}

#[test]
fn moderated_session_transcript() {
    let mut p = load_dir();
    let (lines, total) = session(&mut p);
    assert_eq!(lines, want_transcript());

    // every emitted line crossed the re-entrant vm.call inside `emit`
    assert_eq!(p.stats(), (10, 10));

    // the plugin counted the broadcast messages, not commands or mutes
    assert_eq!(total, 3);
}

#[test]
fn bundle_form_matches_the_directory() {
    // pack the same directory, load the bundle, run the same session
    let bytes = rut_driver::pack_dir(dir()).unwrap();
    let path = std::env::temp_dir().join(format!("rut-03-plugin-test-{}.rutbundle", std::process::id()));
    std::fs::write(&path, &bytes).unwrap();
    let mut p = Plugin::load(&path, &limits()).unwrap();
    let (lines, total) = session(&mut p);
    assert_eq!(lines, want_transcript());
    assert_eq!(total, 3);
    assert_eq!(p.stats(), (10, 10));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn packing_is_deterministic() {
    let a = rut_driver::pack_dir(dir()).unwrap();
    let b = rut_driver::pack_dir(dir()).unwrap();
    assert_eq!(a, b, "same directory => byte-identical bundle");
}

#[test]
fn bad_bundles_are_refused_at_load() {
    let base = std::env::temp_dir().join(format!("rut-03-plugin-ref-{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();

    // unknown format_version — refused before anything else is read
    let manifest =
        "format = \"rutbundle\"\nformat_version = 99\nname = \"app:plugin\"\nentry.lib = \"./plugin.rut\"\n";
    let bytes = rut_driver::write_bundle(&[
        ("rut.toml".to_string(), manifest.as_bytes().to_vec()),
        ("plugin.rut".to_string(), b"fn x() {} \n".to_vec()),
    ])
    .unwrap();
    let path = base.join("v99.rutbundle");
    std::fs::write(&path, &bytes).unwrap();
    let err = match Plugin::load(&path, &limits()) {
        Err(e) => e,
        Ok(_) => panic!("unknown format_version must refuse"),
    };
    assert!(err.msg.contains("format_version"), "{}", err.msg);

    // a corrupted payload byte fails the CRC check
    let mut bytes = rut_driver::pack_dir(dir()).unwrap();
    let at = 30 + "format = \"rutbundle\"\n".len();
    bytes[at] ^= 0x01;
    let path = base.join("corrupt.rutbundle");
    std::fs::write(&path, &bytes).unwrap();
    let err = match Plugin::load(&path, &limits()) {
        Err(e) => e,
        Ok(_) => panic!("a corrupted bundle must refuse"),
    };
    assert!(err.msg.contains("CRC"), "{}", err.msg);

    std::fs::remove_dir_all(&base).unwrap();
}

#[test]
fn unknown_topic_is_dropped_not_trapped() {
    let mut p = load_dir();
    p.fire("bogus", &[]).unwrap();
    assert!(p.transcript().is_empty());
    // the server keeps running after the drop
    p.join("ada").unwrap();
    assert_eq!(p.transcript().len(), 1);
}

#[test]
fn muted_user_cannot_rejoin() {
    let mut p = load_dir();
    p.join("ada").unwrap();
    p.msg("bob", "hi").unwrap();
    p.msg("bob", "x").unwrap();
    p.msg("bob", "x").unwrap(); // streak 3 -> muted
    let n = p.transcript().len();
    p.join("bob").unwrap();
    let lines = p.transcript();
    assert_eq!(lines.len(), n + 1);
    assert_eq!(lines.last().unwrap(), "[system] bob tried to rejoin while muted");
}

#[test]
fn sessions_are_independent() {
    let mut a = load_dir();
    let mut b = load_dir();
    a.join("ada").unwrap();
    assert!(b.transcript().is_empty());
    assert_eq!(a.transcript().len(), 1);
}
