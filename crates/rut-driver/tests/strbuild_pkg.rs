//! The strbuild pkg's phase-1 gate (the strbuild batch;
//! docs/strbuild-survey.md is the contract): the six-member class face
//! over the engine `StrBuf` cell, driven as a real `[deps]` pkg through
//! the `strbuildpkg` fixture. json_pkg's conventions: canonical-string
//! answers, plain VM, no DOM, no host surface.
//!
//! The pins (survey §1.1/§1.5/§2 — the engine admits nothing new):
//! - append/len/build round trips (ASCII, astral, empty)
//! - the codepoint rule: astral = ONE codepoint in `len`; a surrogate
//!   append_code mints U+FFFD at the nat (the str.from_code rule)
//! - the growth law: geometric growth is invisible to content
//! - the share/copy law (RFC 0044): alias and param appends visible
//!   through the original; build twice = same text; the built str
//!   immune to later appends
//! - with_cap: honored as the mint allocation, advisory as behavior —
//!   identical content/len for every hint; a negative hint is the
//!   nat's `Invalid` trap, unchanged by the face

use std::path::Path;
use std::rc::Rc;

use rut_vm::interp::{HostHooks, HostRegistry, Limits, Vm};

const PKG: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/strbuildpkg");

fn vm() -> Vm {
    let (mut session, root) = rut_driver::load_dir_session(Path::new(PKG)).expect("mount");
    rut_driver::mount_std(&mut session);
    let g = rut_driver::compile_graph(&session, &root);
    assert!(
        g.diags.is_empty(),
        "diags: {}",
        g.diags.iter().map(|d| d.msg.clone()).collect::<Vec<_>>().join("\n")
    );
    let flat = rut_core::link::flatten(g.program.expect("linked program"));
    rut_vm::verify::verify(&flat).expect("verify");
    let limits = Limits {
        fuel: Some(50_000_000),
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    let mut vm = Vm::new(
        Rc::new(flat),
        &limits,
        HostHooks::default(),
        HostRegistry::new(),
    )
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
}

#[test]
fn cap_hint_is_advisory() {
    assert_ok("cap_hint_is_advisory");
}

#[test]
fn negative_cap_is_the_nat_invalid_trap() {
    // the nat's message spells it: `StrBuf(cap): capacity must be >= 0`
    let mut vm = vm();
    let err = vm.call::<_, rut_vm::Value>("neg_cap", ()).unwrap_err();
    assert_eq!(err.kind, rut_vm::TrapKind::Invalid, "{}", err.msg);
    assert!(
        err.msg.contains("capacity must be >= 0"),
        "the trap names the bad hint: {}",
        err.msg
    );
}
