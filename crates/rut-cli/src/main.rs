//! the `rut` binary — run / pack / fetch.

mod cache;
mod dump;
mod fetch;
mod fmt;
mod pack;
mod run;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(cmd) = args.get(1).map(|s| s.as_str()) else {
        usage();
        return;
    };
    match cmd {
        "run" => {
            let Some(path) = args.get(2) else {
                eprintln!("run: missing <dir | mod.rutbundle>");
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
            // the symbol table is opt-in against a compiled bundle: the
            // source/dir lanes compile fresh and need no map — a
            // mismatched input is a usage error, never a silent ignore
            let symbols = match args.iter().position(|a| a == "--symbols") {
                None => None,
                Some(i) => match args.get(i + 1) {
                    Some(v) => Some(v.clone()),
                    None => {
                        eprintln!("run: --symbols needs a path to a `.rutsym` symbol table (got nothing)");
                        std::process::exit(2);
                    }
                },
            };
            // the entry designation is opt-in against ambiguity: with
            // several `entry fn`s the CLI refuses to guess — a missing
            // value is a usage error, never a silent default
            let entry = match args.iter().position(|a| a == "--entry") {
                None => None,
                Some(i) => match args.get(i + 1) {
                    Some(v) => Some(v.clone()),
                    None => {
                        eprintln!("run: --entry needs the entry fn's name (got nothing)");
                        std::process::exit(2);
                    }
                },
            };
            run::run(path, fuel, symbols, entry);
        }
        "fmt" => {
            let Some(path) = args.get(2) else {
                eprintln!("fmt: missing <file.rut | dir>");
                std::process::exit(2);
            };
            let check = args.iter().skip(2).any(|a| a == "--check");
            fmt::fmt(path, check);
        }
        "pack" => {
            let Some(dir) = args.get(2) else {
                eprintln!("pack: missing <dir>");
                std::process::exit(2);
            };
            pack::pack(dir, arg_flag(&args, "-o").as_deref(), args.iter().any(|a| a == "--strip"));
        }
        "fetch" => {
            let Some(dir) = args.get(2) else {
                eprintln!("fetch: missing <dir>");
                std::process::exit(2);
            };
            fetch::fetch_cmd(dir);
        }
        "dump" => {
            let Some(path) = args.get(2) else {
                eprintln!("dump: missing <file.rut>");
                std::process::exit(2);
            };
            dump::dump(path);
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
        "rut — run <dir | mod.rutbundle> [--entry <fn>] [--fuel N] [--symbols <file.rutsym>] | fmt <file.rut | dir> [--check] | pack <dir> [-o out.rutbundle] [--strip] | fetch <dir> | dump <file.rut>"
    );
}
