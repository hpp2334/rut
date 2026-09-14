//! 03-plugin — a chat-room server (Rust) with a rut moderator plugin.
//!
//! The host fires events (`join`/`msg`/`leave`/`tick`) into the exports
//! the plugin subscribed; the plugin's decisions come back through the
//! host's bus box — each `emit` re-entering rut to format the wire line
//! while the emitting handler is still parked mid-op (RFC 0022 §1).
//! The whole session below is driven through `Plugin`'s typed methods;
//! the transcript at the end is what `tests/session.rs` asserts.

use rut_vm::interp::Limits;

fn main() {
    let src =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/plugin.rut")).unwrap();
    let limits = Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut p = plugin::Plugin::load(&src, &limits).unwrap();

    p.join("ada").unwrap();
    p.msg("ada", "hello world").unwrap();
    p.msg("ada", "!stats").unwrap(); // a command — answered on `direct`
    p.msg("bob", "hi").unwrap();
    p.msg("bob", "spam").unwrap(); // streak 2
    p.msg("bob", "spam").unwrap(); // streak 3 -> muted
    p.msg("bob", "let me back in").unwrap(); // -> the muted sink
    p.tick(1).unwrap();
    p.leave("ada").unwrap();
    p.fire("bogus", &[]).unwrap(); // no handler: logged, no trap

    let total = p.shutdown().unwrap();
    let (emits, renders) = p.stats();
    println!();
    println!("-- transcript ({emits} emits, {renders} nested renders) --");
    for line in p.transcript() {
        println!("{line}");
    }
    println!("-- {total} messages moderated --");
}
