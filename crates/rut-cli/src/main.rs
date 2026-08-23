//! the `rut` binary — run / dump (RFC 0041 §2).

use std::cell::RefCell;
use std::rc::Rc;

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
    std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(2);
    })
}

fn run(path: &str, fuel: Option<u64>) {
    let src = load(path);
    let out = rutc::compile_module(&src, rutc::Mode::Impl, "main");
    if !out.diags.is_empty() {
        print!("{}", rutc::diag::render_diags(&src, &out.diags));
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
    if let Err(e) = rut_core::verify::verify(&prog) {
        eprintln!("verify: {e}");
        std::process::exit(1);
    }
    let hooks = rut_core::interp::HostHooks {
        print: Some(Rc::new(RefCell::new(|s: &str| println!("{s}")))),
    };
    let limits = rut_core::interp::Limits {
        fuel,
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = match rut_core::interp::Vm::new(std::rc::Rc::new(prog), &limits, hooks) {
        Ok(vm) => vm,
        Err(t) => {
            eprintln!("boot: {}", t.msg);
            std::process::exit(1);
        }
    };
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
    let out = rutc::compile_module(&src, rutc::Mode::Impl, "main");
    if !out.diags.is_empty() {
        print!("{}", rutc::diag::render_diags(&src, &out.diags));
        std::process::exit(1);
    }
    println!("== AST =="); 
    print!("{}", out.ast_dump);
    println!("== IR ==");
    print!("{}", out.ir_dump);
}
