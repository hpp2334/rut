//! the `dump` subcommand: the AST + IR readbacks for one file.

/// the `dump` arg surface.
#[derive(clap::Args)]
pub(crate) struct DumpArgs {
    /// the `.rut` file to dump
    path: String,
}

pub(crate) fn dump(args: &DumpArgs) {
    let path = args.path.as_str();
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

fn load(path: &str) -> String {
    // one file is one module unit — there is no include form (
    // use paths are inter-module), so loading is a plain read
    match rut_native::load_module_source(std::path::Path::new(path)) {
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
