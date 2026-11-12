//! A module-resident generic fn in a PACKED package, instantiated by a
//! consumer the pack closure never saw: the delivery law's template
//! lane must serve it. The surface rows spell module fns MOD-QUALIFIED
//! (`sub::mutate`) while the consumer's use leaf — and the owner's own
//! pack-time ledger rows — spell the BARE leaf; the template match
//! reconciles the two spellings (the leaf fallback), so a
//! consumer-spelled `mutate<u64>` materializes from the packed
//! templates even when the pack root never called it (the pack-time
//! ledger lane has nothing to answer).

use std::path::{Path, PathBuf};

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rut-modgen-{tag}-{}", std::process::id()));
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

fn graph_of(c: Result<rut_driver::Compiled, rut_driver::RunError>) -> rut_driver::GraphOutput {
    match c {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(
                rut_lexer::span::Span::new(0, 0),
                e.msg,
            )],
            program: None,
        },
    }
}

const LIB: &str = r#"
pub class Ctx {
  v: u64;
}

impl Ctx {
  [constructor] pub fn of(v: u64) -> Self {
    return Self { v: v };
  }
}

pub class MutationOf<A> {
  f: opaque;
  run_atom: u64;
}

impl<A> MutationOf<A> {
  [constructor] pub fn of(f: opaque, run_atom: u64) -> Self {
    return Self { f: f, run_atom: run_atom };
  }
  pub fn atom_id(self) -> u64 {
    return self.run_atom;
  }
}

pub fn seal(f: opaque) -> u64 {
  return 41;
}

pub fn mutate<A>(f: fn(Ctx, A)) -> MutationOf<A> {
  let wrap = fn (ctx: Ctx, b: opaque) {
    f(ctx, unopaque<A>(b));
  };
  let run_atom = seal(opaque(wrap));
  return MutationOf<A>::of(opaque(f), run_atom);
}
"#;

#[test]
fn packed_module_generic_serves_consumer_typed_instantiation() {
    let world = scratch("world");
    write(&world, "lib_x/rut.jsonc", r#"{ "name": "lib_x" }"#);
    write(&world, "lib_x/src/mod.rut", "pub mod sub;\n");
    write(&world, "lib_x/src/sub/mod.rut", LIB);
    // The pack root LINKS lib_x but never calls `mutate` — the pack-time
    // ledger carries NO instantiation of the generic; only the template
    // lane can answer the consumer.
    write(
        &world,
        "root/rut.jsonc",
        r#"{ "name": "root", "deps": { "lib_x": { "path": "../lib_x" } } }"#,
    );
    write(
        &world,
        "root/src/mod.rut",
        "use lib_x::sub::{ Ctx };\nentry fn start() -> u64 {\n    return Ctx::of(3).v;\n}\n",
    );

    let bytes = rut_native::pack_dir(&world.join("root")).expect("pack");
    let _ = std::fs::remove_dir_all(&world);

    // The consumer EXPLICITLY instantiates the module-resident generic.
    let mut loaded = rut_driver::Mounted::from_bundle(&bytes).expect("load");
    loaded.pkgs.push(
        rut_driver::Mounted::source(
            "consumer",
            "use lib_x::sub::{ Ctx, mutate };\nentry fn start() -> u64 {\n    let m = mutate<u64>(fn (ctx: Ctx, v: u64) { return; });\n    return 41;\n}\n",
        )
        .manifest(rut_driver::Manifest::lib("consumer", &["lib_x"])),
    );
    let g = graph_of(
        loaded
            .pkgs
            .iter()
            .fold(rut_driver::RutRun::new(), |r, p| r.pkg(p.clone()))
            .entrypoint("consumer")
            .compile(),
    );
    assert!(
        g.diags.is_empty(),
        "the consumer-spelled instantiation serves from the packed templates: {:?}",
        g.diags
    );
    let flat = rut_core::link::flatten(g.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = rut_vm::interp::Limits {
        fuel: Some(2_000_000),
        heap_limit_bytes: Some(16 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = rut_vm::interp::Vm::builder()
        .program(std::rc::Rc::new(flat))
        .limits(limits)
        .hooks(rut_vm::interp::HostHooks::default())
        .hosts(rut_vm::interp::HostRegistry::new())
        .build()
        .expect("vm");
    let got: u64 = vm.call::<_, u64>("start", ()).expect("run");
    assert_eq!(got, 41, "the materialized template body ran");
}
