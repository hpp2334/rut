//! `bench_cross` — the crossing-tax benchmark's host half (the
//! crossing-fastpath plan, phase 0): two nops behind the `.d.rut`
//! surface `rut/bench-cross` (`cli/tests/data/bench-cross` mirrors it),
//! bound by [`install_std_bench_cross`].
//!
//! The bodies are deliberately EMPTY — `nop` is the identity, `nop4`
//! sums its four args — so the bench row that mounts the pkg
//! (`benches/workloads/crossing-nop`) can diff a host call against an
//! inline rut fn with the same body and read the crossing tax off the
//! difference. Both bodies register INFALLIBLE (they cannot trap), so
//! the direct adapter shape is what the row measures. Nothing here is
//! on any production path — the pkg exists to be measured.

use rut_vm::interp::{HostRegistry, Vm};

/// Install the benchmark nops: `nop(x: i64) -> i64` and
/// `nop4(a, b, c, d: i64) -> i64`, the `.d.rut` rows verbatim.
pub fn install_std_bench_cross(hosts: &mut HostRegistry) {
    rut_vm::register!(hosts, "bench_cross::nop", (i64,) -> i64, |_vm: &mut Vm, x: i64| x);
    rut_vm::register!(
        hosts,
        "bench_cross::nop4",
        (i64, i64, i64, i64) -> i64,
        |_vm: &mut Vm, a: i64, b: i64, c: i64, d: i64| a + b + c + d,
    );
}
