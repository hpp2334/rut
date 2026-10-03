//! The strbuild pkg's gate (the strbuild batch; docs/strbuild-survey.md
//! is the contract): the six-member class face over the HOST builder
//! (`strbuild_host::*` — the ink/Logger pattern; core ships zero
//! string-building machinery), driven as a real `[deps]` pkg through
//! the `strbuildpkg` fixture. json_pkg's conventions: canonical-string
//! answers, plain VM, no DOM.
//!
//! The pins (survey §1.1/§1.5/§2 — the host admits nothing new):
//! - append/len/build round trips (ASCII, astral, empty)
//! - the codepoint rule: astral = ONE codepoint in `len`; a surrogate
//!   append_code mints U+FFFD at the host fn (the str.from_code rule)
//! - the growth law: geometric growth is invisible to content
//! - the share/copy law: alias and param appends visible
//!   through the original; build twice = same text; the built str
//!   immune to later appends; finishing THROUGH an alias answers the
//!   shared buffer
//! - with_cap: honored as the mint allocation, advisory as behavior —
//!   identical content/len for every hint; a negative hint is the
//!   host fn's `Invalid` trap, unchanged by the face
//! - the charge law: builder growth consults the engine heap budget
//!   BEFORE growing — a builder growing past a small budget traps OOM,
//!   mirroring the engine cell's discipline

use std::path::Path;
use std::rc::Rc;

use rut_vm::interp::{HostHooks, HostRegistry, Limits, Vm};

const PKG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/strbuildpkg");




/// One source pkg over the auto core — the chain's graph. Closure and
/// shape refusals come back as one span-0 diagnostic.
#[allow(dead_code)] // not every suite in this file needs both lanes
fn compiled(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(rut_driver::RutRun::new().pkg(rut_driver::Pkg::source(spec, src)).entrypoint(spec).compile())
}

/// [`compiled`] with calc offered (the old `mount_std` shape: core
/// auto-rides, `calc` is an ordinary pkg).
#[allow(dead_code)]
fn compiled_std(spec: &str, src: &str) -> rut_driver::GraphOutput {
    graph_of(
        rut_driver::RutRun::new()
            .pkg(rut_driver::Pkg::source(spec, src))
            .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
            .entrypoint(spec)
            .compile(),
    )
}

#[allow(dead_code)]
fn graph_of(c: Result<rut_driver::Compiled, rut_driver::RunError>) -> rut_driver::GraphOutput {
    match c {
        Ok(c) => c.graph,
        Err(e) => rut_driver::GraphOutput {
            diags: vec![rut_lexer::diag::Diag::new(rut_lexer::span::Span::new(0, 0), e.msg)],
            program: None,
        },
    }
}

fn vm() -> Vm {
    vm_with_heap(64 * 1024 * 1024)
}

/// A VM over the fixture with a caller-chosen heap budget — the
/// charge-hook test grows the builder past a SMALL budget on purpose.
fn vm_with_heap(heap_limit_bytes: u64) -> Vm {
    let loaded = rut_native::load_dir(Path::new(PKG)).expect("mount");
    let g = rut_driver::RutRun::new()
        .pkgs(&loaded)
        .pkg(rut_native::tree_pkg("calc").expect("the toolchain tree's rut/calc"))
        .entrypoint(&loaded.root)
        .compile()
        .expect("compile the walk");
    assert!(
        g.graph.diags.is_empty(),
        "diags: {}",
        g.graph.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let flat = rut_core::link::flatten(g.graph.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = Limits {
        fuel: Some(50_000_000),
        heap_limit_bytes: Some(heap_limit_bytes),
        interrupt_every: 1024,
    };
    // the builder's bodies (the host strbuild pkg): the fixture's
    // closure declares the `strbuild_host` rows through the pkg's own dep
    let ctx = rut_driver::host_pkg_ctx(&loaded.pkgs);
    let mut hosts = HostRegistry::new();
    hosts.install_host_pkg(&ctx, rut_std::strbuild::pkg());
    let mut vm = Vm::builder().program(Rc::new(flat)).limits(limits).hooks(HostHooks::default()).hosts(hosts).build()
    .expect("vm");
    vm
}

/// Every fixture entry answers `OK` or a `BAD:<claim>` string naming
/// exactly which pin lied.
fn assert_ok(entry: &str) {
    let mut vm = vm();
    let out: String = vm.call(entry, ()).unwrap_or_else(|e| panic!("{entry}: {e:?}"));
    assert_eq!(out, "OK", "{entry}");
}

#[test]
fn append_len_build_round_trip() {
    assert_ok("roundtrip");
}

#[test]
fn empty_builder_builds_the_empty_str() {
    assert_ok("roundtrip_empty");
}

#[test]
fn round_trip_over_astral_text() {
    assert_ok("roundtrip_astral");
}

#[test]
fn append_code_counts_codepoints_and_mints_fffd_for_surrogates() {
    assert_ok("append_code_law");
    assert_ok("append_code_surrogate_mints_fffd");
}

#[test]
fn geometric_growth_is_invisible_to_content() {
    assert_ok("growth_is_invisible_to_content");
}

#[test]
fn share_copy_law_alias_param_build_twice_immune_str() {
    assert_ok("share_copy_law");
    assert_ok("param_shares_the_cell");
    assert_ok("finish_under_alias");
}

#[test]
fn cap_hint_is_advisory() {
    assert_ok("cap_hint_is_advisory");
}

#[test]
fn negative_cap_is_the_host_invalid_trap() {
    // the host fn's message spells it: `sb_new(cap): capacity must be >= 0`
    let mut vm = vm();
    let err = vm.call::<_, rut_vm::Value>("neg_cap", ()).unwrap_err();
    assert_eq!(err.kind, rut_vm::TrapKind::Invalid, "{}", err.msg);
    assert!(
        err.msg.contains("capacity must be >= 0"),
        "the trap names the bad hint: {}",
        err.msg
    );
}

#[test]
fn builder_growth_charges_the_heap_budget() {
    // the charge law: the host builder consults `charge_public` BEFORE
    // growing — the same entry that answers OK under a 64 MiB budget
    // traps OutOfMemory under a 64 KiB one (the wasm cap governs host
    // growth exactly as it governed the engine cell)
    assert_ok("growth_is_invisible_to_content");
    let err = vm_with_heap(64 * 1024)
        .call::<_, rut_vm::Value>("growth_is_invisible_to_content", ())
        .unwrap_err();
    assert_eq!(err.kind, rut_vm::TrapKind::OutOfMemory, "{}", err.msg);
}
