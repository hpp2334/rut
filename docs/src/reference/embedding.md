# Embedding and native modules

The host is a Rust program. It owns the VM, mounts packages, binds native
function **bodies** to declared **surfaces**, and drives rut entry points.
Compiling and type-checking rut code never requires any Rust: surfaces are
declared in rut source ([host fns and declaration files](host-fns.md)), and
the boot join proves every referenced native member has a bound,
signature-equal implementation before the first instruction runs.

There is no load-time execution: loading verifies and links; the host runs
entry points explicitly.

## The embed loop

```rust
use std::rc::Rc;

// 1. Mount the packages the program uses. core + calc are the base.
let mut session = rut_driver::Session::new();
rut_driver::mount_std(&mut session);        // core + calc
rut_driver::mount_std_async(&mut session);  // async_engine + async_host (optional)
rut_driver::mount_dir(&mut session, "plugins/server")?;
rut_driver::assemble_peers(&mut session)?;  // peer-gated impl groups

// 2. Compile the program against the mounted surfaces.
let out = rut_driver::compile_module_in(&mut session, &src,
                                        rut_parser::Mode::Impl, "app");
if !out.diags.is_empty() { /* render and exit */ }
let prog = rut_core::binary::decode(&out.binary.unwrap())?;
rut_vm::verify::verify(&prog)?;

// 3. Bind bodies BEFORE the Vm exists — the registry is a pre-VM table.
let mut hosts = rut_vm::interp::HostRegistry::new();
rut_std::math::install_std_math(&mut hosts);
rut_vm::register!(hosts, "server::emit", (OpaqueRef, &str, &str) -> (),
    |vm: &mut rut_vm::interp::Vm, bus, topic, payload| -> Result<(), rut_vm::Trap> {
        // re-entrant rut calls are legal here (see "Native fn rules")
        Ok(())
    });
// The decl ↔ impl contract check. Panics, loudly, on any mismatch —
// an embedder wiring bug is never a rut diagnostic.
hosts.verify_against(&session.expected_host_fns());

// 4. Boot and drive.
let limits = rut_vm::interp::Limits {
    fuel: Some(1_000_000),
    heap_limit_bytes: Some(64 * 1024 * 1024),
    interrupt_every: 1024,
};
let mut vm = rut_vm::interp::Vm::new(
    Rc::new(prog), &limits, rut_vm::interp::HostHooks::default(), hosts,
)?;
let answer: i64 = vm.call("compute", (41,))?;
```

The registry is consumed by `Vm::new`: every host thunk the program declares
is resolved against it once, at boot. A declared-but-unbound fn is a
**construction error**, never a mid-run trap.

## Driver API (`rut-driver`)

| API | Meaning |
|---|---|
| `Session::new()` | an empty mounting session |
| `mount_std(&mut s)` | mount `core` + `calc` |
| `mount_std_async(&mut s)` | mount `async_engine` + `async_host` |
| `mount_dir(&mut s, dir)` | mount a package directory (`rut.toml`); returns its name |
| `assemble_peers(&mut s)` | append peer-gated impl groups ([dependency kinds](dependency-kinds.md)) |
| `compile_module(src, mode, name)` | full pipeline over one module against a fresh core+calc session |
| `compile_module_in(&mut s, src, mode, name)` | the same against a caller-built session; returns diags, AST/IR dumps, and the binary |
| `compile_graph(&s, root)` | compile a whole module directory graph |
| `s.expected_host_fns()` | the mounted surfaces' declared host rows — the check table for `verify_against` |
| `load_path_session(path)` | load a module **directory** or `.rutbundle`; returns `(session, root)` |
| `pack_dir(dir)` | pack a module directory into a deterministic v5 **compiled** `.rutbundle` — root + linkable deps as `.rutc` binaries, splice-needed deps as source groups; returns the bytes ([module bundles](bundles.md)) |

The container, manifest grammar, and v5 reader live in the `rut-bundle`
crate — std-only, filesystem-free over a one-method `Source` trait.

`mode` is `Mode::Impl` for `.rut` and `Mode::Decl` for `.d.rut`
([host fns and declaration files](host-fns.md)).

## VM API (`rut_vm::interp`)

| API | Meaning |
|---|---|
| `HostRegistry::new()` | an empty binding table |
| `hosts.register::<_, (P…), R, _>(name, f)` | bind one body; the closure's Rust shape **is** the declared row |
| `hosts.verify_against(&expected)` | panic on declared-unbound / bound-undeclared / signature drift |
| `Vm::new(prog, &limits, hooks, hosts)` | boot; joins every declared host thunk to its binding |
| `vm.call::<A, R>(export, args)` | call an export with Rust values, get a Rust value back ([value boundary](value-boundary.md)) |
| `vm.resume::<R>()` | resume a budget-parked call after refueling |
| `vm.run_ready()` | drain the async ready queue once; returns tasks run |
| `vm.next_deadline() -> Option<u64>` | the earliest timer deadline, if any |
| `vm.set_now(t_ms)` | advance the virtual clock |
| `vm.pending_tasks() -> usize` | unfinished async tasks |
| `vm.fuel_used` / `vm.heap_usage()` | budget meters |
| `vm.alloc_opaque_str(s)` | mint an `opaque` box over host-built text (the logger's named logger) |
| `vm.call_host_row(row, &[Value])` | dispatch a registered row by name (test/tooling reads) |

The canonical async driving loop (single-threaded; the host owns it):

```rust
loop {
    vm.run_ready()?;
    match vm.next_deadline() {
        Some(d) => vm.set_now(d),          // advance to the next timer
        None if vm.pending_tasks() == 0 => break,
        None => std::thread::sleep(std::time::Duration::from_millis(2)),
    }
}
```

Cap the loop in embedders that cannot prove termination, so a program that
never idles fails loudly instead of hanging.

## Limits

```rust
pub struct Limits {
    pub fuel: Option<u64>,          // ops per turn; None = unlimited
    pub heap_limit_bytes: Option<u64>,
    pub interrupt_every: u32,       // ops between interrupt checks
}
```

Exhaustion and interrupts surface as traps ([resource limits](resource-limits.md)).
A trap inside re-entrant work propagates to the embedder parked at the host
op; `vm.resume::<R>()` re-runs the turn.

## Native fn rules

- A body is `FnMut(&mut Vm, P…) -> R` or `-> Result<R, Trap>`; the Rust
  parameter types **are** the declared row (compile error at the register
  site for a type that does not cross — [value boundary](value-boundary.md)).
- Native fns run **outside** the op budget. The host is trusted to be fast —
  or to hand the work to a future ([async host fns](host-fns.md)).
- Re-entrancy: a body may call `vm.call` on rut exports. Nested calls draw
  the same fuel pool; borrows held by the outer body are guarded
  ([value boundary](value-boundary.md)).
- Failures are values: return `Err(Trap)` — `Trap { kind, msg }`. The fn
  traps cleanly and the error propagates as an `Err` to the embedder with
  the rut backtrace intact. Traps never unwind Rust.

| `TrapKind` | raised by |
|---|---|
| `OutOfFuel` | budget exhaustion |
| `OutOfMemory` | heap limit |
| `Interrupted` | interrupt flag |
| `Overflow` / `DivByZero` | arithmetic |
| `IndexOutOfBounds` | sequence access |
| `Panic` | `panic` |
| `BadUnbox` | a failed unbox |
| `NilDeref` | nil where a reference is required |
| `Invalid` | everything else, including boundary mismatches |

## The engine's own natives

The VM boots with internal native calls in the same dispatch family — the
`str`/`bytes` members, the `f"..."` concatenation lowering, array
length/slice, and stack-trace capture. They are call slots
fixed at boot, never IR-level special forms; hosts see them exactly like
their own registered modules.

## Host-side helpers (`rut-std`)

| installer | binds |
|---|---|
| `math::install_std_math` | `calc`'s float functions, both widths |
| `logger::install_std_log(&mut hosts, sink)` | `rt:log`'s two rows, routed to a `FnMut(&str)` sink |
| `nmap::install_std_nmap` | the native key table behind `nmapset` ([stdlib](stdlib.md)) |
| `strbuild::install_std_strbuild` | the string builder's rows behind `strbuild` ([stdlib](stdlib.md)) — growth is charged against the embedder's heap budget |
| `async_host::install_std_async` | the launcher rows (`__launch`/`__abort`/`__sleep`/`__sleep_yield`) |
| `http::install_std_http` | the std HTTP lanes (reqwest; native builds only) |
| `bench_cross::install_std_bench_cross` | the crossing-tax benchmark rows |

## In-tree examples

- [03 — Plugin](../examples/03-plugin.md): a Rust chat server driving a rut
  moderator plugin, loaded from a module directory and from a packed
  `.rutbundle`; re-entrant `emit` crossings.
- [06 — GitHub viewer CLI](../examples/06-github-viewer-cli.md): mounts the
  std packages, binds example-local I/O rows, launches an async entry fn,
  and pumps the driving loop to idle.
