//! 06-github-viewer-cli — the Rust half of `rgh`: mount, bind, cross in,
//! exit. The BRAIN is `rgh.rut`; this file owns only I/O:
//!
//! - argv crosses in as ONE `\n`-joined `str` (the RFC 0023 §2 crossing
//!   set admits prims/str/bytes/opaque only — no arg lists);
//! - the std HTTP lane is bound reqwest-side (`install_std_http` — the
//!   feature is native-only by law; rgh never builds for wasm32);
//! - the example-local `rgh_host` rows carry the CLI I/O: two line
//!   writers and one `std::fs::write`;
//! - the exit code IS `rgh_main`'s i32 return.
//!
//! The offline suite (tests/session.rs) builds its own VM over the
//! fixture lane (`install_std_http_with`) — this file never branches on
//! test env, and nothing here touches the network at test time.

use std::io::Write as _;
use std::rc::Rc;

use rut_vm::interp::Vm;
use rut_vm::Trap;

const MOUNT_DIRS: &[&str] = &[
    // the brain's libs: pouch (Vec) + nmapset + json, the std order
    // (json slots after pouch and nmapset), and the peer gate runs on
    // the whole closure (the CLI's own loose-file recipe) — json's
    // impl-only integration groups mount because pouch/nmapset are in
    // it. The http pair closes the list on the same law (http after its
    // http_host dep; http's own `[deps]` pulls http_host regardless).
    "rut/pouch",
    "rut/nmapset",
    "rut/json",
    "rut/http_host",
    "rut/http",
];

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>().join("\n");

    let mut session = rut_driver::Session::new();
    rut_driver::mount_std(&mut session);
    let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for dir in MOUNT_DIRS {
        rut_driver::mount_dir(&mut session, &tree.join(dir)).expect("mount tree pkg");
    }
    rut_driver::mount_dir(
        &mut session,
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rgh_host"),
    )
    .expect("mount rgh_host");
    rut_driver::assemble_peers(&mut session).expect("assemble peer groups");

    let src =
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("rgh.rut"))
            .expect("read rgh.rut");
    let out = rut_driver::compile_module_in(&mut session, &src, rut_parser::Mode::Impl, "rgh");
    if !out.diags.is_empty() {
        eprint!("{}", rut_lexer::diag::render_diags(&src, &out.diags));
        std::process::exit(1);
    }
    let Some(binary) = out.binary else {
        eprintln!("rgh: no binary emitted");
        std::process::exit(1);
    };
    let prog = match rut_core::binary::decode(&binary) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("rgh: decode: {e}");
            std::process::exit(1);
        }
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

    let mut hosts = rut_vm::interp::HostRegistry::new();
    // calc's rows (mount_std mounts the Math surface) and the nmap
    // table's rows (nmapset rides the plan's mount list — a mounted
    // decl pkg's rows demand bodies, RFC 0025)
    rut_std::math::install_std_math(&mut hosts);
    rut_std::nmap::install_std_nmap(&mut hosts);
    // the std HTTP lane — the reqwest side (UA + redirects live in
    // rut-std); the fixture lane is the tests', never this file's
    rut_std::http::install_std_http(&mut hosts);
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
    // the decl ↔ the bodies, loudly (RFC 0025): every mounted pkg's
    // host rows must have a binding with the declared signature
    hosts.verify_against(&session.expected_host_fns());

    let mut vm = match rut_vm::interp::Vm::new(Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts) {
        Ok(vm) => vm,
        Err(t) => {
            eprintln!("rgh: boot: {}", t.msg);
            std::process::exit(1);
        }
    };
    match vm.call::<_, i32>("rgh_main", (args,)) {
        Ok(code) => std::process::exit(code),
        Err(t) => {
            eprintln!("rgh: trap: {} — {}", t.name(), t.msg);
            std::process::exit(1);
        }
    }
}
