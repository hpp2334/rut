//! The diagnostic-emission laws through the `rut run` door: every error
//! site prints EXACTLY ONCE (a failed inline splice used to leak a copy
//! of the callee's diagnostics — the same error printed twice), and
//! identical sibling sites all report (the first failure used to starve
//! every body behind it — one site stood in for three).

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-cli-diags-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, text).unwrap();
}

fn rut() -> &'static str {
    env!("CARGO_BIN_EXE_rut")
}

fn run(dir: &Path) -> (i32, String) {
    let out = Command::new(rut())
        .args(["run", dir.to_str().unwrap()])
        .output()
        .expect("run rut run");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn count(needle: &str, hay: &str) -> usize {
    hay.matches(needle).count()
}

/// The smallest runnable world: one manifest, one source, no deps.
fn pkg(tag: &str, src: &str) -> PathBuf {
    let root = scratch(tag);
    let dir = root.join("hello");
    write(
        &dir,
        "rut.jsonc",
        r#"{"format": "rutbundle", "format_version": 10, "name": "hello"}"#,
    );
    write(&dir, "mod.rut", src);
    dir
}

#[test]
fn identical_sibling_sites_all_report_exactly_once() {
    let dir = pkg(
        "siblings",
        "entry fn main() -> nil {\n    let a = nope1();\n    let b = nope1();\n    let c = nope1();\n    return;\n}\n",
    );
    let (code, stderr) = run(&dir);
    assert_eq!(code, 1, "stderr: {stderr}");
    // each sibling line flags, each exactly once
    for line in ["2", "3", "4"] {
        let arrow = format!("--&gt; hello:{line}:");
        let arrow = arrow.replace("&gt;", ">");
        assert_eq!(
            count(&arrow, &stderr),
            1,
            "site at line {line} must report exactly once; stderr:\n{stderr}"
        );
    }
    assert_eq!(
        count("error:", &stderr),
        3,
        "exactly three error blocks (one per sibling site); stderr:\n{stderr}"
    );
}

#[test]
fn a_failed_inline_splice_prints_the_callee_error_once() {
    // main calls both helpers; each helper's body fails. The inline
    // splice of a helper's body used to leave its copy of the
    // diagnostics behind AND the real body compile printed them again.
    let dir = pkg(
        "splice",
        "fn f1() -> nil { let a = nope1(); return; }\n\
         fn f2() -> nil { let b = nope2(); return; }\n\
         \n\
         entry fn main() -> nil {\n    f1();\n    f2();\n    return;\n}\n",
    );
    let (code, stderr) = run(&dir);
    assert_eq!(code, 1, "stderr: {stderr}");
    assert_eq!(
        count("unknown function `nope1`", &stderr),
        1,
        "nope1 reports exactly once; stderr:\n{stderr}"
    );
    assert_eq!(
        count("unknown function `nope2`", &stderr),
        1,
        "nope2 reports exactly once; stderr:\n{stderr}"
    );
}

#[test]
fn a_failure_in_one_body_never_stars_the_next() {
    // the drain used to stop at the FIRST failing body — f2's sites
    // (and every body queued behind it) never printed
    let dir = pkg(
        "starve",
        "fn f1() -> nil { let a = nope1(); return; }\n\
         fn f2() -> nil { let b = nope2(); return; }\n\
         fn f3() -> nil { let c = nope3(); return; }\n\
         \n\
         entry fn main() -> nil {\n    f1();\n    f2();\n    f3();\n    return;\n}\n",
    );
    let (code, stderr) = run(&dir);
    assert_eq!(code, 1, "stderr: {stderr}");
    for name in ["nope1", "nope2", "nope3"] {
        assert_eq!(
            count(&format!("unknown function `{name}`"), &stderr),
            1,
            "{name} reports exactly once; stderr:\n{stderr}"
        );
    }
}

#[test]
fn a_use_of_a_failed_binding_diagnoses_only_the_let() {
    // the let's own error is the site; later uses of the (unbound)
    // name stay silent — a second diagnostic would read as a false
    // error for a name the programmer plainly declared
    let dir = pkg(
        "poison",
        "struct P { a: i32; }\n\
         entry fn main() -> i32 {\n    let p = P { a: 1, a: 2 };\n    return p.a;\n}\n",
    );
    let (code, stderr) = run(&dir);
    assert_eq!(code, 1, "stderr: {stderr}");
    assert!(
        stderr.contains("duplicate field `a` in the `P` literal"),
        "the let's own law diagnoses; stderr:\n{stderr}"
    );
    assert_eq!(
        count("error:", &stderr),
        1,
        "exactly one error block — the failed let's use stays silent; stderr:\n{stderr}"
    );
}
