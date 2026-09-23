//! rut-bench-probe — the in-process half of `benches/` (see
//! `benches/README.md`).
//!
//! The cross-runtime runner (`benches/run.mjs`) treats rut like any other
//! runtime: it spawns `rut run <file>` and measures process wall time and
//! peak RSS. This probe adds what a subprocess cannot see — the compile /
//! decode+verify / execute split, fuel used (RFC 0040), and the VM-heap
//! high-water mark (RFC 0039). It compiles the workload ONCE, then runs
//! `main` on a fresh `Vm` per iteration and reports min/median exec time.
//!
//! Output is a single JSON object on stdout:
//!
//! ```json
//! {"workload":"sieve","iters":5,"compile_ms":1.2,"verify_ms":0.03,
//!  "exec_ms":[3.1,2.9,...],"exec_min_ms":2.9,"exec_median_ms":3.0,
//!  "fuel":123456,"heap_peak_bytes":4096,"trapped":null}
//! ```
//!
//! Usage: `rut-bench-probe --workload <file.rut | module-dir> [--iters N]
//! [--fuel N] [--heap-bytes N]`.

use std::rc::Rc;
use std::time::Instant;

use rut_core::binary::decode;
use rut_std::logger::install_std_log;
use rut_vm::interp::{HostHooks, Limits, Vm};

struct Args {
    workload: String,
    iters: usize,
    fuel: Option<u64>,
    heap_bytes: Option<u64>,
}

fn usage() -> ! {
    eprintln!(
        "usage: rut-bench-probe --workload <file.rut | module-dir> [--iters N] \
         [--fuel N] [--heap-bytes N]"
    );
    std::process::exit(2);
}

fn parse_args() -> Args {
    let argv: Vec<String> = std::env::args().collect();
    let mut workload: Option<String> = None;
    let mut iters: usize = 5;
    let mut fuel: Option<u64> = None;
    let mut heap_bytes: Option<u64> = Some(1024 * 1024 * 1024);
    let mut i = 1;
    while i < argv.len() {
        let take = |i: &mut usize| -> String {
            *i += 1;
            match argv.get(*i) {
                Some(v) => v.clone(),
                None => usage(),
            }
        };
        match argv[i].as_str() {
            "--workload" => workload = Some(take(&mut i)),
            "--iters" => iters = take(&mut i).parse().unwrap_or_else(|_| usage()),
            "--fuel" => fuel = Some(take(&mut i).parse().unwrap_or_else(|_| usage())),
            "--heap-bytes" => {
                heap_bytes = Some(take(&mut i).parse().unwrap_or_else(|_| usage()))
            }
            "-h" | "--help" => usage(),
            other => {
                eprintln!("unknown argument: {other}");
                usage();
            }
        }
        i += 1;
    }
    Args {
        workload: workload.unwrap_or_else(|| usage()),
        iters: iters.max(1),
        fuel,
        heap_bytes,
    }
}

/// Minimal JSON string escape (workload paths are the only strings out).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn median(mut xs: Vec<f64>) -> f64 {
    if xs.is_empty() {
        return 0.0;
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = xs.len();
    if n % 2 == 1 {
        xs[n / 2]
    } else {
        (xs[n / 2 - 1] + xs[n / 2]) / 2.0
    }
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("rut-bench-probe: {msg}");
    std::process::exit(1);
}

fn main() {
    let args = parse_args();

    let path = std::path::Path::new(&args.workload);

    // ---- compile (frontend + LIR + binary emit) ----
    // A loose `.rut` workload uses the toolchain libs (`ink`+`rt`,
    // `pouch`) — third-party pkgs mounted from the tree, plus the
    // engine's core/calc. A module-DIR workload (`rut.toml`, the
    // `nmapset` bench dirs) loads its own `[deps]` graph instead and
    // yields an already-linked program.
    let t0 = Instant::now();
    let (session, prog) = if path.is_dir() {
        let (mut session, root) = rut_driver::load_path_session(path)
            .unwrap_or_else(|e| fail(format!("load {}: {e}", path.display())));
        rut_driver::mount_std(&mut session);
        let out = rut_driver::compile_graph(&session, &root);
        if !out.diags.is_empty() {
            for d in &out.diags {
                eprintln!("{}", d.msg);
            }
            fail("compile failed");
        }
        let prog = out.program.unwrap_or_else(|| fail("no program emitted"));
        (session, prog)
    } else {
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| fail(format!("cannot read {}: {e}", path.display())));
        let mut session = rut_driver::Session::new();
        rut_driver::mount_std(&mut session);
        let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        // the std mount order (the rut-json survey §1.2's ruling): json
        // slots after pouch and nmapset
        rut_driver::mount_dir(&mut session, &tree.join("rut/ink")).expect("mount ink (+rt)");
        rut_driver::mount_dir(&mut session, &tree.join("rut/pouch")).expect("mount pouch");
        rut_driver::mount_dir(&mut session, &tree.join("rut/nmapset")).expect("mount nmapset");
        rut_driver::mount_dir(&mut session, &tree.join("rut/json")).expect("mount json");
        rut_driver::assemble_peers(&mut session).expect("assemble peer groups");
        let out = rut_driver::compile_module_in(&mut session, &src, rut_parser::Mode::Impl, "bench");
        if !out.diags.is_empty() {
            print!("{}", rut_lexer::diag::render_diags(&src, &out.diags));
            fail("compile failed");
        }
        let binary = out.binary.unwrap_or_else(|| fail("no binary emitted"));
        let prog = decode(&binary).unwrap_or_else(|e| fail(format!("decode: {e}")));
        (session, prog)
    };
    let compile_ms = t0.elapsed().as_secs_f64() * 1e3;

    // ---- decode + verify (load) ----
    let t1 = Instant::now();
    rut_vm::verify::verify(&prog).unwrap_or_else(|e| fail(format!("verify: {e}")));
    let verify_ms = t1.elapsed().as_secs_f64() * 1e3;

    // ---- execute `main` on a fresh VM per iteration ----
    let limits = Limits {
        fuel: args.fuel,
        heap_limit_bytes: args.heap_bytes,
        interrupt_every: 1024,
    };
    let prog = Rc::new(prog);
    let mut exec_ms: Vec<f64> = Vec::with_capacity(args.iters);
    let mut fuel: u64 = 0;
    let mut heap_peak_bytes: u64 = 0;
    let mut trapped: Option<String> = None;

    for _ in 0..args.iters {
        let mut hosts = rut_vm::interp::HostRegistry::new();
        install_std_log(&mut hosts, |_msg| {});
        rut_std::math::install_std_math(&mut hosts);
        // the nmap experiment's native key table (the mapset-host plan) —
        // bound only when the program's dep graph declares `nmap_host::` (the
        // engine's `calc` is expected of every workload; `nmap_host` is a tree
        // pkg like any other, and `verify_against` is exact in BOTH
        // directions — bound-but-undeclared is an embedder bug, RFC 0025)
        if session
            .expected_host_fns()
            .keys()
            .any(|name| name.starts_with("nmap_host::"))
        {
            rut_std::nmap::install_std_nmap(&mut hosts);
        }
        // the crossing-tax benchmark's nops (the crossing-fastpath plan,
        // phase 0) — same exactness law: bound only when the program's
        // dep graph declares `bench_cross::`
        if session
            .expected_host_fns()
            .keys()
            .any(|name| name.starts_with("bench_cross::"))
        {
            rut_std::bench_cross::install_std_bench_cross(&mut hosts);
        }
        hosts.verify_against(&session.expected_host_fns());
        let mut vm = Vm::new(
            Rc::clone(&prog),
            &limits,
            HostHooks::default(),
            hosts,
        )
        .unwrap_or_else(|t| fail(format!("boot: {}", t.msg)));
        // the workloads log their final checksum through `rt:log`; the
        // probe discards it (like the old `print: None`) so stdout stays a
        // single JSON object. Bindings + contract happened pre-Vm above.
        let t = Instant::now();
        let res = vm.call::<_, ()>("main", ());
        exec_ms.push(t.elapsed().as_secs_f64() * 1e3);
        fuel = fuel.max(vm.fuel_used);
        heap_peak_bytes = heap_peak_bytes.max(vm.heap_peak());
        if let Err(trap) = res {
            trapped = Some(format!("{}: {}", trap.name(), trap.msg));
            break;
        }
    }

    let min = exec_ms.iter().cloned().fold(f64::INFINITY, f64::min);
    let json = format!(
        "{{\"workload\":\"{}\",\"iters\":{},\"compile_ms\":{:.4},\"verify_ms\":{:.4},\
         \"exec_ms\":[{}],\"exec_min_ms\":{:.4},\"exec_median_ms\":{:.4},\
         \"fuel\":{},\"heap_peak_bytes\":{},\"trapped\":{}}}",
        json_escape(&args.workload),
        exec_ms.len(),
        compile_ms,
        verify_ms,
        exec_ms
            .iter()
            .map(|v| format!("{v:.4}"))
            .collect::<Vec<_>>()
            .join(","),
        if min.is_finite() { min } else { 0.0 },
        median(exec_ms.clone()),
        fuel,
        heap_peak_bytes,
        match &trapped {
            Some(t) => format!("\"{}\"", json_escape(t)),
            None => "null".to_string(),
        }
    );
    println!("{json}");
    if trapped.is_some() {
        std::process::exit(1);
    }
}
