//! the `dump` subcommand: the AST + IR readbacks for one file.

/// The module name the single-source lane mounts under — the label the
/// graph's own diags carry (`--> main:2:11`).
const MODULE: &str = "main";

/// the `dump` arg surface.
#[derive(clap::Args)]
pub(crate) struct DumpArgs {
    /// the `.rut` file to dump
    path: String,
}

pub(crate) fn dump(args: &DumpArgs) {
    let path = args.path.as_str();
    let src = load(path);
    let out = rut_driver::compile_module(&src, mode_of(path), MODULE);
    if !out.diags.is_empty() {
        print!("{}", render(&src, &out.diags));
        std::process::exit(1);
    }
    println!("== AST ==");
    print!("{}", out.ast_dump);
    println!("== IR ==");
    print!("{}", out.ir_dump);
}

/// Render the diag list against the loaded text. The compile diags are
/// attributed to the mounted module ([`MODULE`]) while the CLI holds the
/// file's source, so key it under that label — the renderer then shows
/// line/column and the snippet. Parse diags carry no file (the `""`
/// single-file lane) and keep that rendering.
fn render(src: &str, diags: &[rut_lexer::diag::Diag]) -> String {
    if diags.iter().all(|d| d.file.is_none()) {
        return rut_lexer::diag::render_diags(src, diags);
    }
    let mut sources = std::collections::BTreeMap::new();
    sources.insert(String::new(), src);
    sources.insert(MODULE.to_string(), src);
    rut_lexer::diag::render_diags_grouped(diags, &sources, None)
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
/// runnable module. The mode rides [`rut_driver::compile_module`]'s
/// parse AND its graph mount, so the surface re-parses as Decl rather
/// than being refused as an implementation file.
fn mode_of(path: &str) -> rut_ast::parser::Mode {
    if path.ends_with(".d.rut") {
        rut_ast::parser::Mode::Decl
    } else {
        rut_ast::parser::Mode::Impl
    }
}
