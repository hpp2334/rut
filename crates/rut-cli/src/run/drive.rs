//! the VM drive: boot on the compiled run, call the entry, drive the
//! async loop to idle.

/// Boot the VM, call the designated entry fn, then run the async
/// driving loop.
pub(crate) fn run_entry(compiled: rut_driver::Compiled, entry: &str, fuel: Option<u64>) {
    let hooks = rut_vm::interp::HostHooks::default();
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
        .hooks(hooks)
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
    // the async driving loop: drain the ready
    // queue, advance the virtual clock to the next sleep deadline,
    // repeat — idle when no frames and no timers remain. Capped, so a
    // program that never idles fails loudly instead of hanging.
    for _ in 0..1_000_000 {
        if let Err(t) = vm.run_ready() {
            eprintln!("trap: {} — {}", t.name(), t.msg);
            std::process::exit(1);
        }
        match vm.next_deadline() {
            Some(d) => vm.set_now(d),
            None => {
                if vm.pending_tasks() == 0 {
                    break;
                }
            }
        }
    }
}
