//! 03-plugin — a chat-room server (Rust) with a rut moderator plugin.
//!
//! The host fires events (`join`/`msg`/`leave`/`tick`) into the exports
//! the plugin subscribed; the plugin's decisions come back through the
//! host's bus box — each `emit` re-entering rut to format the wire line
//! while the emitting handler is still parked mid-op (RFC 0022 §1).
//!
//! The plugin loads twice from the same source: as a module **directory**
//! (`plugin/rut.toml`) and as a packed **`.rutbundle`** produced by
//! `rut_driver::pack_dir` — the two forms of one contract (RFC 0038).
//! Both run the identical scripted session; the transcript printed at
//! the end is what `tests/session.rs` asserts.

use std::path::Path;

use rut_vm::interp::Limits;

fn limits() -> Limits {
    Limits {
        fuel: Some(1_000_000),
        heap_limit_bytes: Some(4 * 1024 * 1024),
        interrupt_every: 1024,
    }
}

/// The scripted session, driven entirely through `Plugin`'s typed methods.
fn session(p: &mut plugin::Plugin) -> i64 {
    p.join("ada").unwrap();
    p.msg("ada", "hello world").unwrap();
    p.msg("ada", "!stats").unwrap(); // a command — answered on `direct`
    p.msg("bob", "hi").unwrap();
    p.msg("bob", "spam").unwrap(); // streak 2
    p.msg("bob", "spam").unwrap(); // streak 3 -> muted
    p.msg("bob", "let me back in").unwrap(); // -> the muted sink
    p.tick(1).unwrap();
    p.leave("ada").unwrap();
    p.unknown().unwrap(); // no handler: logged, no trap
    p.shutdown().unwrap()
}

fn main() {
    let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/plugin"));

    // form 1: the module directory
    let mut p = plugin::Plugin::load(dir, &limits()).unwrap();
    let dir_transcript = {
        let total = session(&mut p);
        println!("== transcript (module directory) ==");
        for line in p.transcript() {
            println!("{line}");
        }
        println!("-- {total} messages moderated --");
        p.transcript()
    };

    // form 2: the same directory, packed — deterministically (RFC 0038 §3)
    let bytes = rut_driver::pack_dir(dir).unwrap();
    let bundle = std::env::temp_dir().join("rut-03-plugin-demo.rutbundle");
    std::fs::write(&bundle, &bytes).unwrap();
    let mut p = plugin::Plugin::load(&bundle, &limits()).unwrap();
    let total = session(&mut p);
    println!();
    println!(
        "== packed form ({} -> {} bytes, written to {}) ==",
        dir.display(),
        bytes.len(),
        bundle.display()
    );
    println!(
        "transcript identical to the directory form: {}",
        p.transcript() == dir_transcript
    );
    println!("-- {total} messages moderated --");
}
