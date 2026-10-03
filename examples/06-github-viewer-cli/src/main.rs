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
//! - `boot` launches the brain (`launch_future` — the futures
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

use std::future::Future;
use std::io::Write as _;
use std::task::{Context, Poll};

use rut_vm::interp::Vm;
use rut_vm::Trap;

/// The std-only driver for the Loader's future: the remote's fetch
/// futures come back READY (its wire runs on its own worker thread),
/// so one noop-waker poll settles them.
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

/// The manifest lane's walk: the project root through the walk door
/// with the project-local remote named (the manifest is
/// the ONLY url carrier — a cold start networks on its one miss, the
/// pinned `http` bundle; a warm start is pure cache), then the
/// embedder half every lane owns (rgh_host's rows, offered beside the
/// closure).
fn load_rgh() -> Result<(rut_driver::Loaded, rut_driver::Loaded), String> {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let remote = rut_native::HttpRemote::project_local(base);
    let loaded = block_on(rut_native::load_path_session_with(base, &remote))
        .map_err(|e| e.to_string())?;
    // the example's own CLI-I/O pkg (nothing fetches — the embedder
    // binds the bodies)
    let rgh_host = rut_native::dir_pkgs(&base.join("rgh_host")).map_err(|e| e.to_string())?;
    Ok((loaded, rgh_host))
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>().join("\n");

    // the std closure walks through the manifest lane (rut.jsonc is
    // the url carrier; the committed artifact seeds the fetcher), the
    // brain's source is the root, and the async pair + calc offer
    // beside it — the passes run for real, peer gate included
    let (loaded, rgh_host) = match load_rgh() {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let root = loaded.root.clone();
    let async_pkgs = rut_native::dir_pkgs(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../rut/futures"))
        .expect("the async pair walks")
        .pkgs;

    let src =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rgh.rut"))
            .expect("read rgh.rut");
    let mut chain = rut_driver::RutRun::new().pkgs(&loaded);
    // the async pair (the launcher set `boot` drives)
    for p in async_pkgs {
        chain = chain.pkg(p);
    }
    let chain = chain
        .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
        .pkgs(&rgh_host)
        .host_pkg(rut_std::math::pkg())
        .host_pkg(rut_std::nmap::pkg())
        .host_pkg(rut_std::async_host::pkg())
        .host_pkg(rut_std::http::pkg());
    let compiled = match chain.entrypoint(&root).compile() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rgh: {e}");
            std::process::exit(1);
        }
    };
    if !compiled.graph.diags.is_empty() {
        eprint!("{}", rut_lexer::diag::render_diags(&src, &compiled.graph.diags));
        std::process::exit(1);
    }
    let Some(ref prog) = compiled.graph.program else {
        eprintln!("rgh: no binary emitted");
        std::process::exit(1);
    };
    if let Err(e) = rut_vm::verify::verify(prog) {
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

    // the installs rode the chain (calc's Math surface, the nmap
    // table's rows, the async engine's launcher set, the std HTTP
    // lane — the reqwest side, UA + redirects live in rut-std); the
    // compiled registry takes the example's own raw CLI-I/O rows
    // (registered under their full `rgh_host::` names — this host's
    // own pkg, bound by hand)
    // the decl ↔ the bodies, loudly: every mounted pkg's host rows
    // must have a binding with the declared signature
    let mut world: Vec<rut_driver::Pkg> = loaded.pkgs.clone();
    world.extend(rgh_host.pkgs.iter().cloned());
    let expected = rut_driver::declared_host_fns(&world);
    let mut compiled2 = compiled;
    let hosts = &mut compiled2.hosts;
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
    compiled2.hosts.verify_against(&expected);

    let mut vm = match rut_vm::interp::Vm::builder()
        .compiled(compiled2)
        .limits(limits)
        .hooks(rut_vm::interp::HostHooks::default())
        .build()
    {
        Ok(vm) => vm,
        Err(e) => {
            eprintln!("rgh: boot: {}", e.msg);
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
