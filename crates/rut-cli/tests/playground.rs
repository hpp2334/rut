//! Playground classics gate (RFC 0041 §3) — every `demo/src/examples/*.rut`
//! compiles AND runs through the full pipeline, and its output matches the
//! case's inline `expected` lines (the `EXPECTED` table below; the demo's
//! `src/examples/index.ts` carries the same values in TypeScript). The demo
//! reads the same sources raw, so the playground's static-preview promises
//! are exactly what the pipeline produces — never a hand-written guess.

use std::cell::RefCell;
use std::rc::Rc;

/// the classics' expected outputs — carried VERBATIM from the retired
/// `demo/src/examples/*.expected` sidecars (md5 receipts in
/// docs/demo-no-sidecars-survey.md §1.1); enforced here natively and
/// by the demo smoke through wasm — each gate enforces its own copy
/// against the same engine, so neither copy can silently rot
const EXPECTED: &[(&str, &[&str])] = &[
    ("bytes", &[
        "round=true octets=8 chars=8",
        "header=RUT scratch.len=16",
        "alias same content: true",
        "clone same content: true",
        "octets=6 chars=5",
    ]),
    ("checked-arith", &[
        "wrap=4 under=255",
        "over=(44, false)",
        "ok=(255, true)",
        "under=(255, false)",
        "mul=(44, false)",
        "i32 top=(-2147483648, false) wrap=-2147483648",
        "roundtrip=0",
    ]),
    ("classes", &["count=2 area=12"]),
    ("closures-generics", &[
        "add=3 area=3.1415927 sum=6",
        "head=10 name=a",
    ]),
    ("literals", &["a=10 e=1.5 d64=1.5 ch=h p.x=1 zero[0]=9 len=3 bin=64"]),
    ("maps", &[
        "rut=3 runs=1",
        "replace=false rut=9",
        "removed=true len=2 has=false",
        "miss is nil: true",
        "a=10 b=20",
        "miss is nil: true",
        "a=11 len=2",
        "first=true again=false len=1",
        "has x=true has z=false",
    ]),
    ("matrix-mul", &["out[0]=21 out[last]=107"]),
    ("node-cycle", &["head.next alive: true"]),
    ("opaque", &[
        "point 1 2",
        "sour? true wrong? true",
        "is str: false",
        "one cell: 9 5 5 9",
        "same session: true",
        "distinct boxes: false",
        "value 5",
        "str box misses i32: true",
        "3 boxes; first is Point: false",
        "vec 1",
    ]),
    // the trailing space is LOAD-BEARING: the engine emits it
    ("quicksort", &["sorted: 1 2 2 3 5 7 8 9 "]),
    ("sieve", &["25 primes up to 100, last=97"]),
    ("str-views", &[
        "word=world len=5 eq=true",
        "iterated=5 reslice=or",
        "chars=6 octets=7",
        "accent=é",
        "code=104 back=h round=true",
    ]),
    ("structs", &[
        "len=6.324555320336759 color=16711935 area=6",
        "same=true distinct=false fresh.x=7",
    ]),
    ("tree", &["nodes=15"]),
    ("type-aliases", &["trip=1500 plain=1500 ridge/trench kind=trench"]),
    ("weak-cache", &[
        "held: true id=1",
        "miss: false",
    ]),
    ("when", &["small"]),
];

fn examples_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../demo/src/examples")
}

#[test]
fn classics_run_and_match_their_expected() {
    let dir = examples_dir();
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "rut"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 12,
        "expected the classics corpus, found {} in {}",
        files.len(),
        dir.display()
    );

    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let src = std::fs::read_to_string(path).unwrap();
        let Some(expected) = EXPECTED.iter().find(|(n, _)| *n == stem).map(|(_, e)| *e) else {
            panic!("{name}: no entry in the EXPECTED table — add the case's expected lines");
        };

        // the classics use the toolchain libs (`ink`+`rt`, `pouch`) —
        // third-party pkgs mounted from the tree (the driver doesn't
        // know them); `nmapset` is the map lane (survey D6: it pulls
        // `nmap_host` through its `[deps]`)
        let mut s = rut_driver::Session::new();
        rut_driver::mount_std(&mut s);
        let tree = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        rut_driver::mount_dir(&mut s, &tree.join("rut/ink")).expect("mount ink (+rt)");
        rut_driver::mount_dir(&mut s, &tree.join("rut/pouch")).expect("mount pouch");
        rut_driver::mount_dir(&mut s, &tree.join("rut/nmapset")).expect("mount nmapset (+nmap_host)");
        let out = rut_driver::compile_module_in(&mut s, &src, rut_parser::Mode::Impl, "main");
        if !out.diags.is_empty() {
            failures.push(format!(
                "{name}: diags:\n{}",
                rut_lexer::diag::render_diags(&src, &out.diags)
            ));
            continue;
        }
        let Some(binary) = out.binary else {
            failures.push(format!("{name}: no binary emitted"));
            continue;
        };
        let prog = match rut_core::binary::decode(&binary) {
            Ok(p) => p,
            Err(e) => {
                failures.push(format!("{name}: decode: {e}"));
                continue;
            }
        };
        if let Err(e) = rut_vm::verify::verify(&prog) {
            failures.push(format!("{name}: verify: {e}"));
            continue;
        }
        let lines: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = lines.clone();
        let limits = rut_vm::interp::Limits {
            fuel: Some(5_000_000),
            heap_limit_bytes: Some(4 * 1024 * 1024),
            interrupt_every: 1024,
        };
        // the bindings BEFORE the Vm (RFC 0025): the compiled program
        // carries calc's and rt:log's thunks (mount = declare = bind);
        // `install_std_nmap` rides like the CLI's — reached only by a
        // program that declares the nmap lane
        let mut hosts = rut_vm::interp::HostRegistry::new();
        rut_std::logger::install_std_log(&mut hosts, move |msg| {
            sink.borrow_mut().push(msg.to_string())
        });
        rut_std::math::install_std_math(&mut hosts);
        rut_std::nmap::install_std_nmap(&mut hosts);
        let mut vm = match rut_vm::interp::Vm::new(
            Rc::new(prog),
            &limits,
            rut_vm::interp::HostHooks::default(),
            hosts,
        ) {
            Ok(vm) => vm,
            Err(t) => {
                failures.push(format!("{name}: boot: {}", t.msg));
                continue;
            }
        };
        if let Err(t) = vm.call::<_, ()>("main", ()) {
            failures.push(format!("{name}: trap: {} — {}", t.name(), t.msg));
            continue;
        }
        let got = lines.borrow().clone();
        let want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        if got != want {
            failures.push(format!("{name}:\n  got:  {got:?}\n  want: {want:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
