//! VM-verified equivalence — the user's third ruling: reformatting must
//! never change program behavior. Each probe runs the FULL pipeline
//! (compile → link → `Vm::call`) on the original and on its formatted
//! text; both must log IDENTICAL lines through `ink_host`. This is the
//! honest compensation for the AST's absent paren nodes: only the VM
//! can prove the re-paren table preserved meaning.

use rut_vm::interp::{HostHooks, HostRegistry, Limits, Vm};
use std::rc::Rc;

/// compile + run `main`, returning the logged lines in order
fn run(src: &str) -> Result<Vec<String>, String> {
    use std::cell::RefCell;
    let lines: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink_lines = Rc::clone(&lines);

    let mut session = rut_driver::Session::new();
    rut_driver::mount_std(&mut session);
    let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    rut_driver::mount_dir(&mut session, &tree.join("rut/ink")).map_err(|e| e.to_string())?;
    rut_driver::mount_dir(&mut session, &tree.join("rut/pouch")).map_err(|e| e.to_string())?;
    rut_driver::assemble_peers(&mut session).map_err(|e| e.to_string())?;
    let out = rut_driver::compile_module_in(&mut session, src, rut_parser::Mode::Impl, "probe");
    if !out.diags.is_empty() {
        return Err(format!("compile: {}", out.diags[0].msg));
    }
    let binary = out.binary.ok_or("no binary emitted")?;
    let prog = rut_core::binary::decode(&binary).map_err(|e| format!("decode: {e}"))?;
    rut_vm::verify::verify(&prog).map_err(|e| format!("verify: {e}"))?;

    let ctx = session.host_pkg_context();
    let mut hosts = HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::logger::pkg(move |msg: &str| {
        sink_lines.borrow_mut().push(msg.to_string());
    }));
    hosts.install_host_pkg(&ctx, rut_std::math::pkg());
    hosts.verify_against(&ctx.flatten());
    let limits = Limits {
        fuel: Some(10_000_000),
        heap_limit_bytes: Some(8 << 20),
        interrupt_every: 1024,
    };
    let mut vm = Vm::new(Rc::new(prog), &limits, HostHooks::default(), hosts)
        .map_err(|t| format!("boot: {}", t.msg))?;
    let result = vm.call::<_, ()>("main", ()).map_err(|t| format!("run: {}", t.msg));
    let out = lines.borrow().clone();
    result?;
    Ok(out)
}

fn probe(name: &str, src: &str) {
    let want = run(src).unwrap_or_else(|e| panic!("{name}: original does not run: {e}"));
    assert!(!want.is_empty(), "{name}: the probe logged nothing");
    let fmted = rut_fmt::format(src, rut_parser::Mode::Impl, &rut_fmt::Style::default())
        .unwrap_or_else(|ds| panic!("{name}: fmt refused: {}", ds[0]));
    let got = run(&fmted).unwrap_or_else(|e| panic!("{name}: FORMATTED text does not run: {e}"));
    assert_eq!(want, got, "{name}: the VM output changed under formatting");
    // idempotence rides along (cheap here, asserted corpus-wide elsewhere)
    let again = rut_fmt::format(&fmted, rut_parser::Mode::Impl, &rut_fmt::Style::default())
        .unwrap_or_else(|ds| panic!("{name}: fmt(fmt) refused: {}", ds[0]));
    assert_eq!(fmted, again, "{name}: fmt(fmt(x)) != fmt(x)");
}

#[test]
fn precedence_matrix_survives_formatting() {
    probe(
        "precedence",
        r#"
use ink::{ Logger };
entry fn main() {
    let log = Logger.new("p");
    let a = 1 + 2 * 3 - 4 / 2;          // 5
    let b = (1 + 2) * (3 - 4) / 2;      // -1.5 → int -1
    let c = 1 - 2 - 3;                  // left-assoc: -4
    let d = 1 - (2 - 3);                // parens: 2
    let e = 2 << 3 + 1;                 // shifts bind looser: 32
    let f = 10 & 6 | 12 ^ 4;            // bit ladder
    let g = 1 < 2 == true;              // comparison level
    let h = 3 % 2 * 4;                  // 4
    log.info(f"{a} {b} {c} {d} {e} {f} {g} {h}");
}
"#,
    );
}

#[test]
fn unary_chains_and_as_survive_formatting() {
    probe(
        "unary-as",
        r#"
use ink::{ Logger };
entry fn main() {
    let log = Logger.new("u");
    let x = 7;
    let a = -x + -(-x);                 // -7 + 7 = 0
    let b = !(a == 0);                  // false
    let d = 300 as u8;                   // 44 (truncate)
    let e = (1 + 2) as i64 * 2;          // 6 — the cast's operand is a GROUP
    let f = -(d as i32);                 // unary over a cast group: -44
    log.info(f"{a} {b} {d} {e} {f}");
}
"#,
    );
}

#[test]
fn bool_ladder_and_is_survive_formatting() {
    probe(
        "bool-is",
        r#"
use ink::{ Logger };
class Box {
    v: i32;
}
entry fn main() {
    let log = Logger.new("b");
    let bx = Box { v: 9 };
    let t = bx is Box;                   // the class check
    let y = true && false || true;       // && over ||
    let z = 1 == 2 || 3 >= 3 && 4 > 3;   // mixed ladder
    log.info(f"{t} {y} {z}");
}
"#,
    );
}

#[test]
fn when_arms_trailing_commas_and_else_survive_formatting() {
    probe(
        "when",
        r#"
use ink::{ Logger };
entry fn main() {
    let log = Logger.new("w");
    let describe = fn (n: i32) -> str {
        return when (n) {
            0 -> "zero",
            1, 2, 3 -> "small",
            else -> "big",
        };
    };
    log.info(describe(0));
    log.info(describe(2));
    log.info(describe(9));
}
"#,
    );
}

#[test]
fn fstring_holes_and_escapes_survive_formatting() {
    probe(
        "fstring",
        r#"
use ink::{ Logger };
entry fn main() {
    let log = Logger.new("f");
    let name = "fmt";
    let n = 42;
    log.info(f"hi {name} n={n} sum={n + 8} braced={{literal}} tab\tend");
}
"#,
    );
}

#[test]
fn empty_containers_and_struct_literals_survive_formatting() {
    probe(
        "containers",
        r#"
use ink::{ Logger };
use pouch::{ Vec };
class P {
    x: i32;
    y: i32;
}
entry fn main() {
    let log = Logger.new("c");
    let empty: Vec<i32> = Vec.new();
    let xs = [1, 2, 3];
    let rep = [0; 3];
    let p = P { x: 1, y: 2 };
    log.info(f"{empty.len()} {xs.len()} {rep[1]} {p.x + p.y}");
}
"#,
    );
}

#[test]
fn comments_around_everything_survive_formatting() {
    probe(
        "comments",
        r#"
// leading file comment
use ink::{ Logger };

// a fn with comments EVERYWHERE
entry fn main() {
    // before a statement
    let log = Logger.new("cm"); // trailing on a statement
    /* a block comment
       spanning lines */
    let x = 1 + // inside an expression
        2;
    // before the log
    log.info(f"{x}"); // the tail
}
// the file tail
"#,
    );
}

#[test]
fn index_field_calls_survive_formatting() {
    probe(
        "postfix",
        r#"
use ink::{ Logger };
use pouch::{ Vec };
pub fn pick(xs: Vec<i32>, i: i32) -> (i32, bool) {
    if (i >= 0 && i < xs.len()) {
        return (xs[i], true);
    }
    return (0, false);
}
entry fn main() {
    let log = Logger.new("px");
    let xs = Vec<i32>.from([10, 20, 30]);
    let a = xs[1] + xs.len();            // index + method
    let pair = pick(xs, 2);              // the (T, ok) record
    if (pair.1) {
        log.info(f"{a} {pair.0}");
    }
}
"#,
    );
}
