use rut_vm::interp::{HostRegistry, Limits, Vm};
use std::path::PathBuf;

#[test]
fn scratch_bundle_vec_new() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut world = rut_native::dir_pkgs(&base.join("rut/ink")).expect("mount ink").pkgs;
    for b in ["pouch"] {
        let bytes = std::fs::read(base.join(format!("dist/std/{b}.rutbundle"))).unwrap();
        world.extend(rut_driver::Pkg::from_bundle(&bytes).unwrap().pkgs);
    }
    world.push(
        rut_driver::Pkg::source("app", r#"
use pouch::{ Vec };
use ink::{ Logger };

entry fn main() -> nil {
    let log = Logger::new("flow");
    let nums: Vec<i32> = Vec::new();
    nums.push(1); nums.push(2);
    log.info(f"n={nums.len}");
}
"#.to_string()).manifest(rut_driver::Manifest::lib("app", &["ink", "pouch"])),
    );
    for w in &world {
        if let rut_driver::PkgBody::Compiled(p) = &w.body {
            println!("== unit {} scope={} templates={} first={}", p.name, p.scope, p.templates.len(), p.template_first_fid);
            for r in &p.inst_fns {
                let (data, name) = match &r.kind {
                    rut_core::binary::InstFnKind::Method { data, name, subst, .. } => (format!("{}", p.interner.name(*data)), format!("{} subst={:?}", p.interner.name(*name), subst.iter().map(|&t| p.type_name(t).to_string()).collect::<Vec<_>>())),
                    rut_core::binary::InstFnKind::Free { name, subst, .. } => (String::new(), format!("{} subst={:?}", p.interner.name(*name), subst.iter().map(|&t| p.type_name(t).to_string()).collect::<Vec<_>>())),
                    _ => (String::new(), "thunk".into()),
                };
                println!("   row owner={} fid={} {}/{}", p.interner.name(r.owner), r.fid, data, name);
            }
        }
    }
    let compiled = rut_driver::RutRun::new()
        .pkgs(&rut_driver::Loaded { pkgs: world, root: String::new() })
        .entrypoint("app")
        .compile()
        .unwrap();
    assert!(compiled.graph.diags.is_empty(), "diags:\n{}", compiled.graph.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n"));
    let prog = compiled.graph.program.expect("linked");
    let flat = rut_core::link::flatten(prog);
    rut_vm::verify::verify(&flat).expect("verify");
    println!("ok");
}
