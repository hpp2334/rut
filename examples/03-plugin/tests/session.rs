//! The example's gate: drive the moderator through the typed `Plugin`
//! surface and assert the exact transcript — `cargo test --workspace`
//! runs it.

use plugin::Plugin;

fn load() -> Plugin {
    let src =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/plugin.rut")).unwrap();
    let limits = rut_vm::interp::Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    Plugin::load(&src, &limits).unwrap()
}

#[test]
fn moderated_session_transcript() {
    let mut p = load();
    p.join("ada").unwrap();
    p.msg("ada", "hello world").unwrap();
    p.msg("ada", "!stats").unwrap();
    p.msg("bob", "hi").unwrap();
    p.msg("bob", "spam").unwrap();
    p.msg("bob", "spam").unwrap();
    p.msg("bob", "let me back in").unwrap();
    p.tick(1).unwrap();
    p.leave("ada").unwrap();
    // the plugin counted the broadcast messages, not commands or mutes
    assert_eq!(p.shutdown().unwrap(), 3);
    let want = vec![
        "[system] *** ada joined (1 online)",
        "[broadcast] <ada> hello world",
        "[direct] ada: 1 online, 1 msgs, 0 mutes",
        "[broadcast] <bob> hi",
        "[broadcast] <bob> spam",
        "[system] bob muted for flooding",
        "[muted] bob",
        "[system] [tick 1] 1 online, 3 msgs, 1 mutes",
        "[system] ada left (0 online)",
        "[system] shutdown: 0 online, 3 msgs, 1 mutes",
    ];
    assert_eq!(p.transcript(), want);

    // every emitted line crossed the re-entrant vm.call inside `emit`
    assert_eq!(p.stats(), (10, 10));
}

#[test]
fn unknown_topic_is_dropped_not_trapped() {
    let mut p = load();
    p.fire("bogus", &[]).unwrap();
    assert!(p.transcript().is_empty());
    // the server keeps running after the drop
    p.join("ada").unwrap();
    assert_eq!(p.transcript().len(), 1);
}

#[test]
fn muted_user_cannot_rejoin() {
    let mut p = load();
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
    let mut a = load();
    let mut b = load();
    a.join("ada").unwrap();
    assert!(b.transcript().is_empty());
    assert_eq!(a.transcript().len(), 1);
}
