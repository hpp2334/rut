//! the `fmt` subcommand: canonical house layout over the AST reprint.

/// the `fmt` arg surface.
#[derive(clap::Args)]
pub(crate) struct FmtArgs {
    /// the `.rut` file or directory tree to format
    path: String,
    /// write nothing; exit 1 when anything would change
    #[arg(long)]
    check: bool,
}

/// `rut fmt <file.rut | dir> [--check]` — the source formatter: canonical house layout over the AST reprint, comments recovered
/// and reattached verbatim, style from the nearest ancestor `rut.jsonc`'s
/// `[style]` block. Default: rewrite in place. `--check`: write nothing,
/// exit 1 when anything would change.
pub(crate) fn fmt(args: &FmtArgs) {
    let path = args.path.as_str();
    let check = args.check;
    let p = std::path::Path::new(path);
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if p.is_dir() {
        let mut stack = vec![p.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
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
                eprintln!(
                    "fmt: {} refuses — the source does not parse clean:",
                    f.display()
                );
                let label = f.display().to_string();
                let mut sources = std::collections::BTreeMap::new();
                sources.insert(label.clone(), src.as_str());
                eprint!(
                    "{}",
                    rut_lexer::diag::render_diags_grouped(&diags, &sources, Some(&label))
                );
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

/// the style for a file: the nearest ancestor `rut.jsonc`'s `[style]`
/// block, parsed and validated by the fmt crate; no manifest → defaults
fn style_for(f: &std::path::Path) -> Result<rut_fmt::Style, String> {
    let mut dir = f.parent().map(|d| d.to_path_buf()).unwrap_or_default();
    loop {
        let manifest = dir.join("rut.jsonc");
        if manifest.is_file() {
            let text = std::fs::read_to_string(&manifest).map_err(|e| e.to_string())?;
            let m = rut_driver::bundle::parse_manifest(&text).map_err(|e| e.to_string())?;
            return rut_fmt::style::from_manifest(&m.style);
        }
        if !dir.pop() {
            return Ok(rut_fmt::Style::default());
        }
    }
}
