//! the `rut` binary — run / dump (RFC 0041 §2).

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(cmd) = args.get(1).map(|s| s.as_str()) else {
        usage();
        return;
    };
    match cmd {
        "run" => {
            let Some(path) = args.get(2) else {
                eprintln!("run: missing <file.rut | dir | mod.rutbundle>");
                std::process::exit(2);
            };
            let fuel: Option<u64> = arg_flag(&args, "--fuel").map(|v| v.parse().unwrap_or(10_000_000));
            run(path, fuel);
        }
        "pack" => {
            let Some(dir) = args.get(2) else {
                eprintln!("pack: missing <dir>");
                std::process::exit(2);
            };
            pack(dir, arg_flag(&args, "-o").as_deref());
        }
        "dump" => {
            let Some(path) = args.get(2) else {
                eprintln!("dump: missing <file.rut>");
                std::process::exit(2);
            };
            dump(path);
        }
        _ => usage(),
    }
}

fn arg_flag(args: &[String], name: &str) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == name {
            return it.next().cloned();
        }
    }
    None
}

fn usage() {
    eprintln!("rut — run <file.rut | dir | mod.rutbundle> [--fuel N] | pack <dir> [-o out.rutbundle] | dump <file.rut>");
}

fn load(path: &str) -> String {
    // one file is one module unit — there is no include form (RFC 0035 §1:
    // use paths are inter-module), so loading is a plain read
    match rut_driver::load_module_source(std::path::Path::new(path)) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(2);
        }
    }
}

/// `.d.rut` parses in declaration mode (RFC 0030 §3) — a surface, not a
/// runnable module.
fn mode_of(path: &str) -> rut_parser::Mode {
    if path.ends_with(".d.rut") {
        rut_parser::Mode::Decl
    } else {
        rut_parser::Mode::Impl
    }
}

fn run(path: &str, fuel: Option<u64>) {
    if path.ends_with(".d.rut") {
        eprintln!("run: {path} is a declaration file (a `.d.rut` surface) — nothing to run");
        std::process::exit(2);
    }
    let p = std::path::Path::new(path);
    let packed = p.is_dir() || p.extension().map_or(false, |e| e == "rutbundle");
    let prog = if packed {
        // a module directory (`rut.toml`) or a `.rutbundle` — load the
        // graph, mount std, compile, link (RFC 0035 §1 / RFC 0038 §5)
        let (mut session, root) = match rut_driver::load_path_session(p) {
            Ok(x) => x,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(2);
            }
        };
        rut_driver::mount_std(&mut session);
        let g = rut_driver::compile_graph(&session, &root);
        if !g.diags.is_empty() {
            for d in &g.diags {
                eprintln!("{}", d.msg);
            }
            std::process::exit(1);
        }
        match g.program {
            Some(p) => p,
            None => {
                eprintln!("no program emitted");
                std::process::exit(1);
            }
        }
    } else {
        let src = load(path);
        // single-file convenience: a `use ink::` / `use pouch::` pulls the
        // toolchain's tree pkg — the driver does not know these names, and
        // the demo cases + `benches/workloads` rely on the CLI being a
        // full host (it binds math + the logger below)
        let mut s = rut_driver::Session::new();
        rut_driver::mount_std(&mut s);
        let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (name, dir) in [("ink", "rut/ink"), ("pouch", "rut/pouch")] {
            if src.contains(&format!("use {name}::")) {
                rut_driver::mount_dir(&mut s, &tree.join(dir)).expect("mount tree pkg");
            }
        }
        let out = rut_driver::compile_module_in(&mut s, &src, mode_of(path), "main");
        if !out.diags.is_empty() {
            print!("{}", rut_lexer::diag::render_diags(&src, &out.diags));
            std::process::exit(1);
        }
        let Some(binary) = out.binary else {
            eprintln!("no binary emitted");
            std::process::exit(1);
        };
        match rut_core::binary::decode(&binary) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("decode: {e}");
                std::process::exit(1);
            }
        }
    };
    if let Err(e) = rut_vm::verify::verify(&prog) {
        eprintln!("verify: {e}");
        std::process::exit(1);
    }
    let hooks = rut_vm::interp::HostHooks::default();
    let limits = rut_vm::interp::Limits {
        fuel,
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    // the CLI is a host: it mounts core+calc (mount_std) and binds their
    // bodies — math always, the logger to stdout when a program uses ink
    let mut hosts = rut_vm::interp::HostRegistry::new();
    rut_std::math::install_std_math(&mut hosts);
    rut_std::logger::install_std_log(&mut hosts, |s| println!("{s}"));
    // the nmap experiment's native key table (the mapset-host plan) — a
    // program only reaches it when it declares `use nmap_host::{...}` or a
    // pkg that does (`nmapset`)
    rut_std::nmap::install_std_nmap(&mut hosts);
    // the crossing-tax benchmark's nops (the crossing-fastpath plan,
    // phase 0) — reached only by a program that declares
    // `use bench_cross::{...}` (the bench row)
    rut_std::bench_cross::install_std_bench_cross(&mut hosts);
    let mut vm = match rut_vm::interp::Vm::new(std::rc::Rc::new(prog), &limits, hooks, hosts) {
        Ok(vm) => vm,
        Err(t) => {
            eprintln!("boot: {}", t.msg);
            std::process::exit(1);
        }
    };
    // (bindings were installed into the registry before `Vm::new` above)
    match vm.call::<_, ()>("main", ()) {
        Ok(_) => {}
        Err(t) => {
            eprintln!("trap: {} — {}", t.name(), t.msg);
            std::process::exit(1);
        }
    }
}

/// `rut pack <dir> [-o out.rutbundle]` — pack a module directory into a
/// deterministic `.rutbundle` (RFC 0038).
fn pack(dir: &str, out: Option<&str>) {
    let p = std::path::Path::new(dir);
    let bytes = match rut_driver::pack_dir(p) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("pack: {e}");
            std::process::exit(1);
        }
    };
    let out = out
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let stem = p.file_name().unwrap_or(p.as_os_str()).to_string_lossy();
            p.with_file_name(format!("{stem}.rutbundle"))
        });
    if let Err(e) = std::fs::write(&out, &bytes) {
        eprintln!("pack: cannot write {}: {e}", out.display());
        std::process::exit(1);
    }
    println!("packed {} -> {} ({} bytes)", p.display(), out.display(), bytes.len());
}

fn dump(path: &str) {
    let src = load(path);
    let out = rut_driver::compile_module(&src, mode_of(path), "main");
    if !out.diags.is_empty() {
        print!("{}", rut_lexer::diag::render_diags(&src, &out.diags));
        std::process::exit(1);
    }
    println!("== AST =="); 
    print!("{}", out.ast_dump);
    println!("== IR ==");
    print!("{}", out.ir_dump);
}
