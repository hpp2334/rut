//! the `rut` binary — run / dump.

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
            // fuel is opt-in and LOUD: absent → uncapped; a missing or
            // unparsable value is a usage error — no silent default
            let fuel: Option<u64> = match args.iter().position(|a| a == "--fuel") {
                None => None,
                Some(i) => match args.get(i + 1) {
                    Some(v) => match v.parse() {
                        Ok(n) => Some(n),
                        Err(_) => {
                            eprintln!("run: --fuel needs a number of ops (got `{v}`)");
                            std::process::exit(2);
                        }
                    },
                    None => {
                        eprintln!("run: --fuel needs a number of ops (got nothing)");
                        std::process::exit(2);
                    }
                },
            };
            run(path, fuel);
        }
        "fmt" => {
            let Some(path) = args.get(2) else {
                eprintln!("fmt: missing <file.rut | dir>");
                std::process::exit(2);
            };
            let check = args.iter().skip(2).any(|a| a == "--check");
            fmt(path, check);
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
    eprintln!(
        "rut — run <file.rut | dir | mod.rutbundle> [--fuel N] | fmt <file.rut | dir> [--check] | pack <dir> [-o out.rutbundle] | dump <file.rut>"
    );
}

fn load(path: &str) -> String {
    // one file is one module unit — there is no include form (
    // use paths are inter-module), so loading is a plain read
    match rut_driver::load_module_source(std::path::Path::new(path)) {
        Ok(src) => src,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(2);
        }
    }
}

/// `.d.rut` parses in declaration mode — a surface, not a
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
        // graph, mount std, compile, link
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
        // the std mount order (the rut-json survey §1.2's ruling): ink
        // is independent; json slots after pouch and nmapset — the dep
        // graph's new edges (json -> pouch, json -> nmapset) make that
        // the only graph-respecting position. strbuild is 10th, after
        // json (the amendment): position-free for the graph —
        // the row names the reading order, and json's own `[deps]`
        // pulls the pkg regardless. The http pair closes the list on
        // the same law (http after its http_host dep; http's own
        // `[deps]` pulls http_host regardless)
        for (name, dir) in [
            ("rt", "rut/rt"),
            ("ink", "rut/ink"),
            ("pouch", "rut/pouch"),
            ("nmapset", "rut/nmapset"),
            ("json", "rut/json"),
            ("strbuild", "rut/strbuild"),
            ("async_engine", "rut/async_engine"),
            ("async_host", "rut/async_host"),
            ("http_host", "rut/http_host"),
            ("http", "rut/http"),
        ] {
            if src.contains(&format!("use {name}::")) {
                rut_driver::mount_dir(&mut s, &tree.join(dir)).expect("mount tree pkg");
            }
        }
        // the loose-file gate (the survey §1.2's decision): the CLI is a
        // full host — a loose file that `use json::` gets the
        // peer-gated container groups exactly like a module-dir program
        rut_driver::assemble_peers(&mut s).expect("assemble peer groups");
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
    // the async host set: launch/abort/sleep bodies for the
    // `async_engine` rows — reached only by a program that mounts the
    // async packages (a `use async_host::` pulls the tree pkg)
    rut_std::async_host::install_std_async(&mut hosts);
    // the string builder's bodies (the host strbuild pkg): reached only
    // by a program that mounts the strbuild pkg (a `use strbuild::` /
    // `use json::` pulls it — json's writer rides the builder)
    rut_std::strbuild::install_std_strbuild(&mut hosts);
    // the std HTTP lane (the rut/http plan): get + the Response
    // readbacks — reached only by a program that mounts the http
    // packages (a `use http::` / `use http_host::` pulls the tree
    // pkgs; reqwest is the CLI's, native-only)
    rut_std::http::install_std_http(&mut hosts);
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
    // the async driving loop: drain the ready
    // queue, advance the virtual clock to the next sleep deadline,
    // repeat — idle when no frames and no timers remain. Capped, so a
    // program that never idles fails loudly instead of hanging.
    for _ in 0..1_000_000 {
        if let Err(t) = vm.run_ready() {
            eprintln!("trap: {} — {}", t.name(), t.msg);
            std::process::exit(1);
        }
        match vm.next_deadline() {
            Some(d) => vm.set_now(d),
            None => {
                if vm.pending_tasks() == 0 {
                    break;
                }
            }
        }
    }
}

/// `rut pack <dir> [-o out.rutbundle]` — pack a module directory into a
/// deterministic v5 **compiled** `.rutbundle` (linkable pkgs ride as
/// `.rutc` binaries, splice-needed deps as source).
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
        .unwrap_or_else(|| rut_bundle::default_out_path(p));
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

/// `rut fmt <file.rut | dir> [--check]` — the source formatter: canonical house layout over the AST reprint, comments recovered
/// and reattached verbatim, style from the nearest ancestor `rut.toml`'s
/// `[style]` block. Default: rewrite in place. `--check`: write nothing,
/// exit 1 when anything would change.
fn fmt(path: &str, check: bool) {
    let p = std::path::Path::new(path);
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if p.is_dir() {
        let mut stack = vec![p.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let ep = e.path();
                if ep.is_dir() {
                    stack.push(ep);
                } else if ep.extension().map_or(false, |x| x == "rut") {
                    files.push(ep);
                }
            }
        }
        files.sort();
        if files.is_empty() {
            eprintln!("fmt: no .rut files under {path}");
            std::process::exit(2);
        }
    } else {
        files.push(p.to_path_buf());
    }

    let mut changed: Vec<String> = Vec::new();
    for f in &files {
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("fmt: cannot read {}: {e}", f.display());
                std::process::exit(2);
            }
        };
        let style = match style_for(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("fmt: {}: {e}", f.display());
                std::process::exit(1);
            }
        };
        let mode = if f.to_string_lossy().ends_with(".d.rut") {
            rut_parser::Mode::Decl
        } else {
            rut_parser::Mode::Impl
        };
        match rut_fmt::format(&src, mode, &style) {
            Ok(out) => {
                if out != src {
                    changed.push(f.display().to_string());
                    if !check {
                        if let Err(e) = std::fs::write(f, &out) {
                            eprintln!("fmt: cannot write {}: {e}", f.display());
                            std::process::exit(2);
                        }
                    }
                }
            }
            Err(diags) => {
                eprintln!("fmt: {} refuses — the source does not parse clean:", f.display());
                for d in &diags {
                    eprintln!("  {d}");
                }
                std::process::exit(1);
            }
        }
    }
    if check {
        if changed.is_empty() {
            println!("fmt: {} file(s) formatted", files.len());
        } else {
            for c in &changed {
                println!("unformatted: {c}");
            }
            std::process::exit(1);
        }
    } else if changed.is_empty() {
        println!("fmt: {} file(s) already formatted", files.len());
    } else {
        for c in &changed {
            println!("formatted: {c}");
        }
    }
}

/// the style for a file: the nearest ancestor `rut.toml`'s `[style]`
/// block, parsed and validated by the fmt crate; no manifest → defaults
fn style_for(f: &std::path::Path) -> Result<rut_fmt::Style, String> {
    let mut dir = f.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    loop {
        let manifest = dir.join("rut.toml");
        if manifest.is_file() {
            let text = std::fs::read_to_string(&manifest).map_err(|e| e.to_string())?;
            let m = rut_bundle::parse_manifest(&text).map_err(|e| e.to_string())?;
            return rut_fmt::style::from_manifest(&m.style);
        }
        if !dir.pop() {
            return Ok(rut_fmt::Style::default());
        }
    }
}
