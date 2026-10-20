//! the invocation designation: which `entry fn` the run calls.

/// The entry-fn set of the root module. Exactly one `entry fn` runs
/// the program; several require `--entry <name>`; none is nothing to
/// run. `main` is an ordinary NAME — the compiler has no main
/// convention to fall back to.
pub(crate) fn designate(entry: Option<String>, prog: &rut_core::binary::Program) -> String {
    let entries: Vec<String> = prog
        .exports
        .iter()
        .map(|(n, _)| prog.name_of(*n).to_string())
        .collect();
    match entry {
        Some(name) => {
            if !entries.iter().any(|e| *e == name) {
                match entries.as_slice() {
                    [] => eprintln!(
                        "run: no `entry fn {name}` — this program declares no entry fns at all"
                    ),
                    _ => eprintln!(
                        "run: no `entry fn {name}` — this program's entry fns: {}",
                        entries.join(", ")
                    ),
                }
                std::process::exit(1);
            }
            name
        }
        None => match entries.as_slice() {
            [one] => one.clone(),
            [] => {
                eprintln!(
                    "no `entry fn` — nothing to run (declare one: `entry fn main() {{ .. }}`)"
                );
                std::process::exit(1);
            }
            many => {
                eprintln!(
                    "{} entry fns ({}) — name one with `--entry <fn>`",
                    many.len(),
                    many.join(", ")
                );
                std::process::exit(1);
            }
        },
    }
}
