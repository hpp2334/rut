//! the VM drive: boot on the compiled run, call the entry, drive to
//! idle through the engine's protocol.

/// Boot the VM, call the designated entry fn, then run the engine's
/// driving protocol ([`rut_vm::interp::Vm::run_until_idle`]) to idle.
pub(crate) fn run_entry(compiled: rut_driver::Compiled, entry: &str, fuel: Option<u64>) {
    let limits = rut_vm::interp::Limits {
        fuel,
        heap_limit_bytes: Some(64 * 1024 * 1024),
        interrupt_every: 1024,
    };
    // the installs rode the chain (`.pkg(..)`); the compiled run
    // carries the joined registry — the builder door takes it whole
    let mut vm = match rut_vm::interp::Vm::builder()
        .compiled(compiled)
        .limits(limits)
        .build()
    {
        Ok(vm) => vm,
        Err(e) => {
            eprintln!("boot: {}", e.msg);
            std::process::exit(1);
        }
    };
    // (the host bindings were installed against the rows snapshot at
    // `.compile()`; the registry came back on `Compiled`)
    match vm.call::<_, ()>(entry, ()) {
        Ok(_) => {}
        Err(t) => {
            eprintln!("trap: {} — {}", t.name(), t.msg);
            std::process::exit(1);
        }
    }
    // the async driving loop, the engine's protocol at the
    // [`rut_vm::interp::Wait::Block`] policy — the native embedder's
    // lane: drain the ready queue, advance the virtual clock to the
    // next sleep deadline, and park on the wake cell while host
    // futures settle on their workers. Idle is the honest return;
    // under Block the only exits are idle or trap: a host future
    // that never completes hangs the run by law (the wait-forever
    // law — no budget, no hang guard; best-effort cancellation is
    // the disclosed contract).
    if let Err(t) = vm.run_until_idle(rut_vm::interp::Wait::Block) {
        eprintln!("trap: {} — {}", t.name(), t.msg);
        std::process::exit(1);
    }
}
