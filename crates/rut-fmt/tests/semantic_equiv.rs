//! VM-verified equivalence — the user's third ruling: reformatting must
//! never change program behavior. Each probe runs the FULL pipeline
//! (compile → link → `Vm::call`) on the original and on its formatted
//! text; both must log IDENTICAL lines through `ink_host`. This is the
//! honest compensation for the AST's absent paren nodes: only the VM
//! can prove the re-paren table preserved meaning.

use rut_vm::interp::{HostRegistry, Limits, Vm};
use std::rc::Rc;

/// compile + run `main`, returning the logged lines in order
fn run(src: &str) -> Result<Vec<String>, String> {
    use std::cell::RefCell;
    let lines: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let sink_lines = Rc::clone(&lines);

    let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut world = Vec::new();
    for d in ["rut/ink", "rut/pouch", "rut/futures"] {
        world.extend(
            rut_native::dir_pkgs(&tree.join(d))
                .map_err(|e| e.to_string())?
                .pkgs,
        );
    }
    world.push(rut_native::tree_pkg("calc_host").expect("the toolchain tree's rut/calc_host"));
    let compiled = rut_driver::Walk {
        pkgs: world.clone(),
        root: String::new(),
    }
    .pkgs
    .iter()
    .fold(rut_driver::RutRun::new(), |r, p| r.mount(p.clone()))
    .mount(
        // the probe DECLARES its uses structurally — the world's
        // heads (the fmt lane must not change them, and doesn't:
        // both texts compile over the same declaration)
        rut_driver::Mounted::source("probe", src).manifest(rut_driver::Manifest::lib(
            "probe",
            &["calc_host", "ink", "pouch", "futures"],
        )),
    )
    .entrypoint("probe")
    .compile()
    .map_err(|e| format!("compile: {e}"))?;
    if !compiled.graph.diags.is_empty() {
        return Err(format!("compile: {}", compiled.graph.diags[0].msg));
    }
    let prog = compiled.graph.program.as_ref().ok_or("no binary emitted")?;
    rut_vm::verify::verify(prog).map_err(|e| format!("verify: {e}"))?;
    let prog = prog.clone();

    let ctx = rut_driver::host_pkg_ctx(&world);
    let mut hosts = HostRegistry::new();
    hosts.install_host_pkg(
        &ctx,
        rut_std::logger::pkg(move |msg: &str| {
            sink_lines.borrow_mut().push(msg.to_string());
        }),
    );
    hosts.install_host_pkg(&ctx, rut_std::math::pkg());
    // the async intrinsics' bodies (`__launch`/`__abort`/`__sleep`) —
    // declared by the mounted async_host pkg, bound here like the CLI's
    // run chain binds them; inert for probes that never launch
    hosts.install_host_pkg(&ctx, rut_std::async_host::pkg());
    hosts.verify_against(&ctx.flatten());
    let limits = Limits {
        fuel: Some(10_000_000),
        heap_limit_bytes: Some(8 << 20),
        interrupt_every: 1024,
    };
    let mut vm = Vm::builder()
        .program(Rc::new(prog))
        .limits(limits)
        .hosts(hosts)
        .build()
        .map_err(|t| format!("boot: {}", t.msg))?;
    vm.call::<_, ()>("main", ())
        .map_err(|t| format!("run: {}", t.msg))?;
    // the engine's driving protocol: launched futures progress only
    // when driven — the native embedder's lane drains to idle after
    // the entry (the CLI run lane's shape); idle is the honest return
    vm.run_until_idle(rut_vm::interp::Wait::Block)
        .map_err(|t| format!("drive: {}", t.msg))?;
    let out = lines.borrow().clone();
    Ok(out)
}

fn probe(name: &str, src: &str) {
    let want = run(src).unwrap_or_else(|e| panic!("{name}: original does not run: {e}"));
    assert!(!want.is_empty(), "{name}: the probe logged nothing");
    let fmted = rut_fmt::format(src, rut_ast::parser::Mode::Impl, &rut_fmt::Style::default())
        .unwrap_or_else(|ds| panic!("{name}: fmt refused: {}", ds[0].msg));
    let got = run(&fmted).unwrap_or_else(|e| panic!("{name}: FORMATTED text does not run: {e}"));
    assert_eq!(want, got, "{name}: the VM output changed under formatting");
    // idempotence rides along (cheap here, asserted corpus-wide elsewhere)
    let again = rut_fmt::format(
        &fmted,
        rut_ast::parser::Mode::Impl,
        &rut_fmt::Style::default(),
    )
    .unwrap_or_else(|ds| panic!("{name}: fmt(fmt) refused: {}", ds[0].msg));
    assert_eq!(fmted, again, "{name}: fmt(fmt(x)) != fmt(x)");
}

#[test]
fn precedence_matrix_survives_formatting() {
    probe(
        "precedence",
        r#"
use ink::{ Logger };
entry fn main() {
    let log = Logger::new("p");
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
    let log = Logger::new("u");
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
    let log = Logger::new("b");
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
    let log = Logger::new("w");
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
    let log = Logger::new("f");
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
    let log = Logger::new("c");
    let empty: Vec<i32> = Vec::new();
    let xs = [1, 2, 3];
    let rep = [0; 3];
    let p = P { x: 1, y: 2 };
    log.info(f"{empty.len()} {xs.len()} {rep[1]} {p.x + p.y}");
}
"#,
    );
}

#[test]
fn struct_field_init_shorthand_survives_formatting() {
    // `Task { id, .. }` — the field-init shorthand. The parser desugars
    // it to `id: id` (a single-segment path over the name token), so
    // fmt reprints it EXPANDED (a reprint normalization — literals are
    // span-verbatim now, but the desugar is a tree shape the AST no
    // longer records): round-trip, idempotence, and identical VM output.
    let src = r#"
use ink::{ Logger };
class Task {
    id: i32;
    title: str;
    completed: bool;
}
entry fn main() {
    let log = Logger::new("sh");
    let id = 7;
    let t = Task { id, title: "Eat an apple", completed: false };
    log.info(f"{t.id} {t.title} {t.completed}");
}
"#;
    let want = run(src).unwrap_or_else(|e| panic!("shorthand: original does not run: {e}"));
    assert_eq!(want, vec!["7 Eat an apple false"]);
    let fmted = rut_fmt::format(src, rut_ast::parser::Mode::Impl, &rut_fmt::Style::default())
        .unwrap_or_else(|ds| panic!("shorthand: fmt refused: {}", ds[0].msg));
    assert!(
        fmted.contains("id: id"),
        "shorthand must reprint expanded to `id: id`: {fmted}"
    );
    let got = run(&fmted).unwrap_or_else(|e| panic!("shorthand: FORMATTED text does not run: {e}"));
    assert_eq!(
        want, got,
        "shorthand: the VM output changed under formatting"
    );
    let again = rut_fmt::format(
        &fmted,
        rut_ast::parser::Mode::Impl,
        &rut_fmt::Style::default(),
    )
    .unwrap_or_else(|ds| panic!("shorthand: fmt(fmt) refused: {}", ds[0].msg));
    assert_eq!(fmted, again, "shorthand: fmt(fmt(x)) != fmt(x)");
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
    let log = Logger::new("cm"); // trailing on a statement
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
    let log = Logger::new("px");
    let xs = Vec<i32>::from([10, 20, 30]);
    let a = xs[1] + xs.len();            // index + method
    let pair = pick(xs, 2);              // the (T, ok) record
    if (pair.1) {
        log.info(f"{a} {pair.0}");
    }
}
"#,
    );
}

#[test]
fn an_async_block_tail_survives_formatting() {
    // the awaited async block's tail is the program's observable: fmt
    // used to drop the tail, and the formatted text logged `tail 0`
    // where the original logged `tail 3`. (The tail's value does not
    // ride the future's result in this build — `return`s set T — so
    // the tail is observed through the captured cell it writes.)
    probe(
        "async-tail",
        r#"
use ink::{ Logger };
use futures::{ launch_future };
class Box {
    v: i32;
}
entry fn main() {
    let log = Logger::new("a");
    let b = Box { v: 0 };
    let f = async {
        let g = async {
            b.v = 1 + 2
        };
        await g;
        log.info(f"tail {b.v}");
    };
    launch_future(f);
}
"#,
    );
}

#[test]
fn struct_literal_in_postfix_position_survives_formatting() {
    // ground truth 1: a composite literal leaves the atom frame, so
    // `(P { a: 1 }).a` needs its group — fmt used to drop it and the
    // formatted text stopped compiling (`expected ';', found '.'`).
    // The field value is the program's observable.
    probe(
        "struct-postfix",
        r#"
use ink::{ Logger };
class P {
    a: i32;
}
entry fn main() {
    let log = Logger::new("sp");
    let x = (P { a: 1 }).a;
    log.info(f"{x}");
}
"#,
    );
}

#[test]
fn loop_header_const_survives_formatting() {
    // the loop-header `const` (the for-of opt-out) and a `const`
    // binding ride fmt without changing behavior — the const law makes
    // the flag semantic, so a lost or misprinted `const` would change
    // what the formatted text may reassign, which the VM run below
    // would catch (the const header's write-through must still run).
    // The byte-level round-trip + idempotence pins live in pin.rs
    // (`loop_header_const_survives_parse_fmt_parse`); the field-write
    // target prints bare (`t.done = ..` — the same `Assign` as the
    // paren spelling, so the group drops) and must re-parse to the
    // same program.
    probe(
        "loop-const",
        r#"
use ink::{ Logger };
class Task {
    done: bool;
}
entry fn main() {
    let log = Logger::new("lm");
    const tasks = [Task { done: false }, Task { done: true }];
    for (const t of tasks) {
        t.done = !t.done;
    }
    let total = 0;
    for (let i = 0; i < 4; i += 1) {
        total += i;
    }
    let first = tasks[0].done;
    let second = tasks[1].done;
    log.info(f"{first} {second} {total}");
}
"#,
    );
}
