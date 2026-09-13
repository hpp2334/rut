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
                eprintln!("run: missing <file.rut>");
                std::process::exit(2);
            };
            let fuel: Option<u64> = arg_flag(&args, "--fuel").map(|v| v.parse().unwrap_or(10_000_000));
            run(path, fuel);
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
    eprintln!("rut — run <file.rut> [--fuel N] | dump <file.rut>");
}

fn load(path: &str) -> String {
    // one file is one module: its `import { .. } from "./sibling.rut"`
    // lines are intra-module includes, inlined exactly as the loader
    // does for mounted modules (RFC 0035 §1) — so `rut run main.rut`
    // sees the whole multi-file module
    let expanded = rut_driver::expand_module_source(std::path::Path::new(path));
    match expanded {
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
    let src = load(path);
    let out = rut_driver::compile_module(&src, mode_of(path), "main");
    if !out.diags.is_empty() {
        print!("{}", rut_lexer::diag::render_diags(&src, &out.diags));
        std::process::exit(1);
    }
    let Some(binary) = out.binary else {
        eprintln!("no binary emitted");
        std::process::exit(1);
    };
    let prog = match rut_core::binary::decode(&binary) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("decode: {e}");
            std::process::exit(1);
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
    let mut vm = match rut_vm::interp::Vm::new(std::rc::Rc::new(prog), &limits, hooks) {
        Ok(vm) => vm,
        Err(t) => {
            eprintln!("boot: {}", t.msg);
            std::process::exit(1);
        }
    };
    // the host half of `std:log` (RFC 0022/0026)
    rut_std::logger::install_std_log(&mut vm, |msg| println!("{msg}"));
    // the host half of `std:math` (RFC 0028)
    rut_std::math::install_std_math(&mut vm);
    match vm.call("main", &[]) {
        Ok(_) => {}
        Err(t) => {
            eprintln!("trap: {} — {}", t.name(), t.msg);
            std::process::exit(1);
        }
    }
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
