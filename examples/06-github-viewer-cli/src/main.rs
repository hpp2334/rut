//! 06-github-viewer-cli — the Rust half of `rgh`: mount, bind, launch,
//! pump, exit. The BRAIN is `rgh.rut` (an async free fn over the
//! redesigned rut/http lane); this file owns only I/O:
//!
//! - argv crosses in as ONE `\n`-joined `str` (the crossing
//!   set admits prims/str/bytes/opaque only — no arg lists);
//! - the std HTTP lane is bound reqwest-side (`rut_std::http::pkg()`
//!   installed — the
//!   feature is native-only by law; rgh never builds for wasm32);
//! - the example-local `rgh_host` rows carry the CLI I/O: two line
//!   writers, the file pair (`write_file` truncates, `append_file`
//!   grows — the streaming download's two halves), and the `exit` row
//!   (the process exits with the brain's i32);
//! - `boot` launches the brain (`launch_future` — the async_host
//!   standard launcher); this file then pumps the driving loop to
//!   idle (the run lane): `run_ready` + a short wall-clock
//!   sleep per spin until `pending_tasks()` hits zero. The `exit` row
//!   fires INSIDE the pump and never returns. A trap surfaces here as
//!   exit 1 with the trap name.
//!
//! The offline suite (tests/session.rs) builds its own VM over the
//! fixture lane (`http::pkg_with`) and a recording `exit` body —
//! this file never branches on test env, and nothing here touches the
//! network at test time.

use std::collections::BTreeMap;
use std::future::Future;
use std::io::Write as _;
use std::rc::Rc;
use std::task::{Context, Poll};

use rut_vm::interp::Vm;
use rut_vm::Trap;

/// The std closure mounts through the PROJECT MANIFEST (`rut.toml`) —
/// the manifest is the ONLY url carrier: it is parsed here, and every
/// url row's bytes seed the fetcher from the committed artifact
/// (offline: the seed IS the cache — the human `rut fetch` lane fills
/// the same shape over the network). Path rows mount natively inside
/// the same load; the four passes (deps walk, peer gate included) run
/// for real. The example's own CLI-I/O rows (`rgh_host`) mount beside
/// the closure — nothing fetches for them; the embedder binds the
/// bodies.
struct Table(BTreeMap<String, Vec<u8>>);

impl rut_driver::DepFetch for Table {
    fn dep_fetch(&self, url: &str) -> impl Future<Output = Result<Vec<u8>, String>> {
        let r = match self.0.get(url) {
            Some(bytes) => Ok(bytes.clone()),
            None => Err(format!("no seeded bytes for {url}")),
        };
        std::future::ready(r)
    }
}

/// The std-only driver for the `_with` lane: the fetched futures are
/// `ready`, so one noop-waker poll settles them.
fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let mut cx = Context::from_waker(std::task::Waker::noop());
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

/// The manifest lane's mount: load the project root with the seeded
/// fetcher, then the embedder half every lane owns (rgh_host's rows).
fn load_rgh_session() -> Result<(rut_driver::Session, String), String> {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest_text = std::fs::read_to_string(base.join("rut.toml"))
        .map_err(|e| format!("rut.toml: {e}"))?;
    let manifest =
        rut_bundle::parse_manifest(&manifest_text).map_err(|e| e.to_string())?;
    let dist = base.join("../../dist/std");
    let mut table = BTreeMap::new();
    for desc in manifest.deps.values() {
        let Some(url) = desc.get("url") else { continue };
        let artifact = url.rsplit('/').next().unwrap_or_default();
        let bytes = std::fs::read(dist.join(artifact))
            .map_err(|e| format!("the seed is the cache — cannot read {artifact}: {e}"))?;
        table.insert(url.clone(), bytes);
    }
    let (mut session, root) =
        block_on(rut_driver::load_dir_session_with(&base, &Table(table)))?;
    // the example's own CLI-I/O rows (nothing fetches — the embedder
    // binds the bodies)
    rut_driver::mount_dir(
        &mut session,
        &base.join("rgh_host"),
    )
    .map_err(|e| e.to_string())?;
    Ok((session, root))
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>().join("\n");

    // the std closure mounts through the manifest lane (rut.toml is
    // the url carrier; the committed artifact seeds the fetcher), and
    // the brain's module registers with it — the four passes run for
    // real, peer gate included
    let (mut session, root) = match load_rgh_session() {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    rut_driver::mount_std(&mut session);
    // the async pair (the launcher set `boot` drives)
    rut_driver::mount_std_async(&mut session);
    rut_driver::assemble_peers(&mut session).expect("assemble peer groups");

    let src =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rgh.rut"))
            .expect("read rgh.rut");
    let g = rut_driver::compile_graph(&session, &root);
    if !g.diags.is_empty() {
        eprint!("{}", rut_lexer::diag::render_diags(&src, &g.diags));
        std::process::exit(1);
    }
    let Some(prog) = g.program else {
        eprintln!("rgh: no binary emitted");
        std::process::exit(1);
    };
    if let Err(e) = rut_vm::verify::verify(&prog) {
        eprintln!("rgh: verify: {e}");
        std::process::exit(1);
    }

    // 02-digest's limits, one notch up on fuel: the real CDN tree
    // (351 entries of JSON) decodes well inside it — the fixture suite
    // measures the actual spend.
    let limits = rut_vm::interp::Limits {
        fuel: Some(250_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };

    // the mount snapshot the installs answer to (owned; the session
    // dies when main's boot half ends)
    let ctx = session.host_pkg_context();
    let mut hosts = rut_vm::interp::HostRegistry::new();
    // calc's rows (mount_std mounts the Math surface) and the nmap
    // table's rows (nmapset rides the plan's mount list — a mounted
    // decl pkg's rows demand bodies)
    hosts.install_host_pkg(&ctx, rut_std::math::pkg());
    hosts.install_host_pkg(&ctx, rut_std::nmap::pkg());
    // the async engine's rows (the launcher set boot drives)
    hosts.install_host_pkg(&ctx, rut_std::async_host::pkg());
    // the std HTTP lane — the reqwest side (UA + redirects live in
    // rut-std); the fixture lane is the tests', never this file's
    hosts.install_host_pkg(&ctx, rut_std::http::pkg());
    // the example's own CLI-I/O rows
    rut_vm::register!(hosts, "rgh_host::out", (&str,) -> (),
        |_vm: &mut Vm, s: &str| -> Result<(), Trap> {
            let mut o = std::io::stdout().lock();
            let _ = o.write_all(s.as_bytes());
            let _ = o.write_all(b"\n");
            let _ = o.flush();
            Ok(())
        });
    rut_vm::register!(hosts, "rgh_host::eprint", (&str,) -> (),
        |_vm: &mut Vm, s: &str| -> Result<(), Trap> {
            let mut o = std::io::stderr().lock();
            let _ = o.write_all(s.as_bytes());
            let _ = o.write_all(b"\n");
            let _ = o.flush();
            Ok(())
        });
    rut_vm::register!(hosts, "rgh_host::write_file", (&str, Vec<u8>) -> Option<String>,
        |_vm: &mut Vm, path: &str, data: Vec<u8>| -> Result<Option<String>, Trap> {
            match std::fs::write(path, &data) {
                Ok(()) => Ok(None), // the flat nil — written
                Err(e) => Ok(Some(e.to_string())), // the io error's text
            }
        });
    rut_vm::register!(hosts, "rgh_host::append_file", (&str, Vec<u8>) -> Option<String>,
        |_vm: &mut Vm, path: &str, data: Vec<u8>| -> Result<Option<String>, Trap> {
            use std::io::Write as _;
            let mut f = match std::fs::OpenOptions::new().create(true).append(true).open(path) {
                Ok(f) => f,
                Err(e) => return Ok(Some(e.to_string())),
            };
            match f.write_all(&data).and_then(|_| f.flush()) {
                Ok(()) => Ok(None),
                Err(e) => Ok(Some(e.to_string())),
            }
        });
    rut_vm::register!(hosts, "rgh_host::exit", (i32,) -> (),
        |_vm: &mut Vm, code: i32| -> Result<(), Trap> {
            // the brain's one-way door: the pump dies here with the
            // code it carried
            std::process::exit(code);
        });
    // the decl ↔ the bodies, loudly: every mounted pkg's
    // host rows must have a binding with the declared signature
    hosts.verify_against(&ctx.flatten());

    let mut vm = match rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts) {
        Ok(vm) => vm,
        Err(t) => {
            eprintln!("rgh: boot: {}", t.msg);
            std::process::exit(1);
        }
    };
    // launch the brain, then pump the driving loop to idle: real
    // reqwest workers settle the completers from their threads (the
    // wall-clock spin), `exit` fires mid-spin and never returns
    if let Err(t) = vm.call::<_, ()>("boot", (args,)) {
        eprintln!("rgh: boot fn: {} — {}", t.name(), t.msg);
        std::process::exit(1);
    }
    loop {
        match vm.run_ready() {
            Ok(_) => {}
            Err(t) => {
                eprintln!("rgh: trap: {} — {}", t.name(), t.msg);
                std::process::exit(1);
            }
        }
        if vm.pending_tasks() == 0 {
            // the brain retired WITHOUT exiting — a brain bug: loud
            eprintln!("rgh: the brain retired without exit (a brain bug)");
            std::process::exit(1);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}
