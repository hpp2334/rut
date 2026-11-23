//! The host future's embedder side (phase 4): [`Completer`] — the ONE
//! helper a `pub host async fn`'s body needs — and the
//! [`register_async!`](crate::register_async) sugar that turns one
//! closure into the row family the weave drives.
//!
//! The arrangement (the host lane): calling a host
//! async fn mints a COLD engine-woven Future frame whose state field
//! holds a HOST cell — the opaque box `<name>__start` answers, a
//! [`Completer`] sealed by [`Ret`]'s into_slot. The frame's
//! `Future::yield` vtable fill is the compiler-minted wrapper; it maps
//! the cx `cancelled()` data path to the `__cancel` arm and drives the
//! `__yield` resumption probe (0 pending / 1 ready / 2 failed), then
//! `__take` marshals the answer onto the heap ON the VM thread (a
//! phase-1 return lane). The embedder's body is ONE closure: the
//! sugar mints the wake-wired state cell and HANDS it to the body
//! (the start closure's first parameter is the future's
//! [`Completer`]); the body spawns work, hands out a clone,
//! `complete`/`fail` from the worker.
//!
//! THREAD LAW: the VM stays single-threaded. Workers touch only the
//! Completer's atomics + Mutex slot — a clone shares them through one
//! `Arc`; the rut cells are minted by `__take`/`into_slot` on the VM
//! thread. `std::thread` appears only in embedder closures, never
//! here (this file's own `wakes_tests` play the embedder's worker —
//! they are the exception, not machinery).

use std::sync::{Arc, Condvar, Mutex};

use rut_core::types::{TypeId, TY_OPAQUE};

use super::boundary::{expect_kind, Ret};
use super::{Trap, TrapKind, Vm};
use crate::heap::Slot;
use crate::Opaque;
/// The probe's wire values — `<name>__yield`'s answer and the
/// completer's state word.
pub const PENDING: i32 = 0;
pub const READY: i32 = 1;
pub const FAILED: i32 = 2;

/// The machine's wake cell: a generation counter that answers "a host
/// future settled since you last looked". Every [`Completer`] is wired
/// to the ONE cell of its machine (the registry's, carried onto the
/// [`Vm`]) — a `complete`/`fail` bumps the generation and notifies
/// AFTER the settle, so a driving loop that parks on
/// [`HostWakes::wait_since`] wakes only to work worth doing (the
/// run-waits-host-futures lane's substrate; the loop itself is a
/// later phase).
///
/// One cell per machine, shared by every host future in it — a future
/// settling without a bump is structurally impossible (the old
/// detached `Completer::new` is gone; `with_wake` is the only mint).
#[derive(Default)]
pub struct HostWakes {
    mx: Mutex<u64>,
    cv: Condvar,
}

/// The shared handle — an `Arc` clone is the wiring every completer
/// carries.
pub type HostWakesHandle = Arc<HostWakes>;

impl HostWakes {
    /// The canonical mint: a fresh cell, already shared. The registry
    /// owns the machine's; tests mint their own.
    pub fn shared() -> HostWakesHandle {
        Arc::new(HostWakes::default())
    }

    /// One settle happened. Locks, increments, unlocks, notifies — the
    /// notify is OUTSIDE the lock (a woken waiter must be able to
    /// re-acquire the mutex without racing the bumper).
    pub fn bump(&self) {
        let mut g = self.mx.lock().expect("host wakes");
        *g += 1;
        drop(g);
        self.cv.notify_all();
    }

    /// The current generation — the counter is observable (the saved
    /// epoch a waiter parks against).
    pub fn generation(&self) -> u64 {
        *self.mx.lock().expect("host wakes")
    }

    /// Park until the generation moves past `seen`; answers the
    /// generation after the waking bump. Spurious wakeups re-park (the
    /// `while` is the condvar contract).
    pub fn wait_since(&self, seen: u64) -> u64 {
        let mut g = self.mx.lock().expect("host wakes");
        while *g <= seen {
            g = self.cv.wait(g).expect("host wakes");
        }
        *g
    }
}

/// The shared state: atomics + a Mutex slot + the machine's wake
/// cell — the ONLY thing a worker thread touches (the thread law).
struct Inner<T> {
    /// 0 pending / 1 ready / 2 failed — the release/acquire pair with
    /// the slot below makes `poll`'s answer authoritative.
    done: std::sync::atomic::AtomicU64,
    slot: Mutex<Option<Result<T, String>>>,
    /// the machine's ONE wake cell — `complete`/`fail` bump it AFTER
    /// the settle (the signal never precedes the settle)
    wakes: HostWakesHandle,
}

/// A host future's completion cell: `complete`/`fail` are callable
/// from ANY thread (worker → VM is one-directional, atomics + Mutex
/// only); `poll` is the driving loop's probe; `take_result` drains the
/// answer on the VM thread (a `fail` message becomes the trap at the
/// await). Clone = another handle on the SAME cell — the closure hands
/// one to its worker and boxes one for the frame's state field.
///
/// The whole cell is one Arc internally, so the boxed clone and the
/// worker's clone share the state with no aliasing hazards and no
/// rut-vm-side threads.
pub struct Completer<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Clone for Completer<T> {
    fn clone(&self) -> Self {
        Completer {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Completer<T> {
    /// Mint a state cell wired to the machine's wake cell — every
    /// settle bumps the generation and wakes the parked driving loop.
    /// The old detached `new` is gone (no compat inside the
    /// workspace): an unwired settle — a future that completes while
    /// nobody notices — is exactly the silent-exit bug class this
    /// lane exists to kill. The sugars mint through
    /// `Vm::host_wakes`; the body never mints by hand.
    pub fn with_wake(wakes: HostWakesHandle) -> Completer<T> {
        Completer {
            inner: Arc::new(Inner {
                done: std::sync::atomic::AtomicU64::new(PENDING as u64),
                slot: Mutex::new(None),
                wakes,
            }),
        }
    }

    /// The cell's identity: the `Arc`'s address. Every clone answers the
    /// same number; two live cells never collide. IDENTITY ONLY — the
    /// value names nothing else, must never be dereferenced or
    /// persisted, and means nothing once the cell is gone (the std
    /// http lane's cancel registry keys its id → source map by it;
    /// entries die with the futures they park).
    pub fn id(&self) -> usize {
        Arc::as_ptr(&self.inner) as usize
    }
}

impl<T: Ret> Completer<T> {
    /// Settle the future with its answer — from any thread. A late
    /// answer after a cancel is the DISCLOSED best-effort law: the
    /// thread runs to its blocking completion, the result is simply
    /// never taken.
    ///
    /// The wake ORDER is the law: the slot stores and `done` flips
    /// (Release) BEFORE the bump — a waiter woken by the bump reads a
    /// settled cell, and a waiter that parks after the settle never
    /// blocks (the generation already moved).
    pub fn complete(&self, v: T) {
        *self.inner.slot.lock().expect("completer slot") = Some(Ok(v));
        self.inner
            .done
            .store(READY as u64, std::sync::atomic::Ordering::Release);
        self.inner.wakes.bump();
    }

    /// Settle the future with a failure — the message traps at the
    /// await, through `__take`. Any thread. The wake order is
    /// `complete`'s: settle first, bump after — a failure wakes the
    /// machine too (the await must trap, not hang).
    pub fn fail(&self, msg: String) {
        *self.inner.slot.lock().expect("completer slot") = Some(Err(msg));
        self.inner
            .done
            .store(FAILED as u64, std::sync::atomic::Ordering::Release);
        self.inner.wakes.bump();
    }

    /// The driving loop's probe: 0 pending / 1 ready / 2 failed.
    pub fn poll(&self) -> i32 {
        self.inner.done.load(std::sync::atomic::Ordering::Acquire) as i32
    }

    /// Drain the answer (VM thread): a failed completer's message
    /// becomes the trap — the wrapper's `__take` call is where the
    /// await fails.
    pub fn take_result(&self) -> Result<T, Trap> {
        match self.poll() {
            PENDING => Err(Trap::new(
                TrapKind::Invalid,
                "async host fn: the completer is still pending — take polls first (the weave's law)",
            )),
            READY => {
                let taken = self
                    .inner
                    .slot
                    .lock()
                    .expect("completer slot")
                    .take();
                match taken {
                    Some(Ok(v)) => Ok(v),
                    _ => Err(Trap::new(
                        TrapKind::Invalid,
                        "async host fn: the answer was already taken (one await, one take)",
                    )),
                }
            }
            _ => {
                let msg = self
                    .inner
                    .slot
                    .lock()
                    .expect("completer slot")
                    .take();
                let msg = match msg {
                    Some(Err(m)) => m,
                    _ => "the host future failed".to_string(),
                };
                Err(Trap::new(TrapKind::Invalid, msg))
            }
        }
    }
}

/// The state cell's crossing: a host async fn's `__start` answers a
/// `Completer` and it crosses boxed under `opaque` — the same boot id
/// every host box carries, so the minted `__start` thunk's ret and the
/// registered body's SIG join by identity. `into_slot` news
/// the host box ON THE VM THREAD (the mint law); the rows recover it
/// through `Opaque<Completer<T>>`'s typed read (the payload token
/// checks the answer type).
impl<T: Ret + 'static> Ret for Completer<T> {
    const TY: TypeId = TY_OPAQUE;
    fn rust_name() -> &'static str {
        "Completer"
    }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "Completer")?;
        let b = Opaque::<Completer<T>>::from_handle(&vm.heap.opaque_handle(unsafe { slot.r }))?;
        // a clone of the boxed handle — the same shared cell (the Arc),
        // never a copy of the state
        b.with(|c| c.clone())
    }
    fn into_slot(self, vm: &mut Vm) -> Result<Slot, Trap> {
        let boxed = Opaque::alloc(vm, self)?;
        boxed.handle().clone().into_slot(vm)
    }
}

/// The registration sugar for the host future lane — the mirror of
/// [`register!`](crate::register) for `pub host async fn` rows. ONE
/// closure (plus an optional abort hook) becomes the row family the
/// weave drives:
///
/// ```ignore
/// rut_vm::register_async!(hosts, "mypkg::fetch", (String,) -> Vec<u8>,
///     |c: Completer<Vec<u8>>, url: String| {
///         let w = c.clone();
///         std::thread::spawn(move || { let data = fetch(url); w.complete(data); });
///     });
/// ```
///
/// The closure's FIRST parameter is the future's [`Completer`] — the
/// sugar mints it (through `Vm::host_wakes`, the machine's ONE wake
/// cell) and hands it in; the body clones it for its worker and never
/// mints. The closure takes NO vm parameter (the
/// work leaves the VM; the virtual clock and the queues need none of
/// it). `T: Ret` — the compiler enforces "async
/// answers are crossing types". The macro emits all five bindings the
/// program can join:
///
/// - `<name>` — the decl row itself; the weave never dispatches it
///   (the call site mints the frame instead), so the body is the
///   teaching trap.
/// - `<name>__start(args...) -> opaque` — mints the wake-wired
///   Completer, hands it to the closure, boxes it: the state cell.
/// - `<name>__yield(state, cx) -> i32` — the resumption probe
///   (0 pending / 1 ready / 2 failed).
/// - `<name>__take(state) -> T` — the answer onto the heap, marshaled
///   ON the VM thread; a failed completer traps here.
/// - `<name>__cancel(state)` — always registered (the wrapper's
///   cancel branch calls it unconditionally and the boot join is
///   eager): the abort closure when one is given, else the disclosed
///   no-op (the thread runs to its blocking completion; late results
///   are discarded).
///
/// The abort closure receives a `Completer<T>` clone of the state
/// cell — an `Arc` clone, safe to move anywhere, touching only the
/// atomics + Mutex slot (the thread law).
#[macro_export]
macro_rules! register_async {
    ($hosts:expr, $name:literal, ($($t:ty),* $(,)?) -> $ret:ty, $start:expr $(,)?) => {
        $crate::register_async!(
            $hosts, $name, ($($t,)*) -> $ret, $start,
            move |_c: $crate::Completer<$ret>| {}
        )
    };
    ($hosts:expr, $name:literal, ($($t:ty),* $(,)?) -> $ret:ty, $start:expr, $abort:expr $(,)?) => {
        $crate::__register_async_rows!(
            $hosts, $name, ($($t,)*), $ret, $start, $abort
        )
    };
}

/// The row emitter. The user's two expressions are HOISTED into
/// hygienic locals before the registered `move` closures capture them
/// — an adapter that constructed the user closure per call would be
/// `FnOnce`, and the registry demands `FnMut`.
#[doc(hidden)]
#[macro_export]
macro_rules! __register_async_rows {
    // -- the fixed-arity rows, shared by every arity --
    ($hosts:expr, $name:literal, ($($t:ty,)*), $ret:ty, $start:expr, $abort:expr) => {{
        let __abort = $abort;
        $hosts.register::<_, ($crate::Opaque<$crate::Completer<$ret>>,), (), _>(
            concat!($name, "__cancel"),
            move |_vm: &mut $crate::interp::Vm,
                  state: $crate::Opaque<$crate::Completer<$ret>>| {
                let c = state.with(|c| c.clone())?;
                __abort(c);
                Ok(())
            },
        );
        $hosts.register::<_, ($crate::Opaque<$crate::Completer<$ret>>, $crate::OpaqueRef), ::std::primitive::i32, _>(
            concat!($name, "__yield"),
            |_vm: &mut $crate::interp::Vm,
             state: $crate::Opaque<$crate::Completer<$ret>>,
             _cx: $crate::OpaqueRef| {
                state.with(|c| c.poll())
            },
        );
        $hosts.register::<_, ($crate::Opaque<$crate::Completer<$ret>>,), $ret, _>(
            concat!($name, "__take"),
            |_vm: &mut $crate::interp::Vm,
             state: $crate::Opaque<$crate::Completer<$ret>>| {
                // the answer crosses through the adapter's own
                // `Ret::into_slot` — the marshal happens ON the VM
                // thread; a failed completer's message IS the trap
                state.with(|c| c.take_result())?
            },
        );
        let __start = $start;
        $crate::__register_async_start!(
            $hosts, $name, ($($t,)*), $ret, __start
        );
    }};
}

/// The start row + the decl-row trap body, per arity (0..=8 — the
/// boundary's own cap). The user closure is registered FORWARDER-ONLY:
/// the adapter decodes the row's params and calls the closure with
/// them, plain.
#[doc(hidden)]
#[macro_export]
macro_rules! __register_async_start {
    ($hosts:expr, $name:literal, (), $ret:ty, $start:expr) => {
        $hosts.register::<_, (), $crate::Completer<$ret>, _>(
            concat!($name, "__start"),
            move |__vm: &mut $crate::interp::Vm| {
                // the sugar mints the state cell wired to the
                // machine's ONE wake cell and hands it to the body —
                // the body never mints, so an unwired settle (a
                // future that completes while nobody notices) cannot
                // exist
                let __c = $crate::Completer::with_wake(__vm.host_wakes());
                $start(__c.clone());
                Ok(__c)
            },
        );
        $hosts.register::<_, (), $ret, _>(
            $name,
            move |_vm: &mut $crate::interp::Vm| -> ::std::result::Result<$ret, $crate::Trap> {
                Err($crate::Trap::new(
                    $crate::TrapKind::Invalid,
                    format!(
                        "async host fn `{}` called through its decl row — the weave mints the future at the call site; `await` (or `launch_future`) drives it",
                        $name
                    ),
                ))
            },
        );
    };
    ($hosts:expr, $name:literal, ($t0:ty,), $ret:ty, $start:expr) => {
        $hosts.register::<_, ($t0,), $crate::Completer<$ret>, _>(
            concat!($name, "__start"),
            move |__vm: &mut $crate::interp::Vm, a0: $t0| {
                let __c = $crate::Completer::with_wake(__vm.host_wakes());
                $start(__c.clone(), a0);
                Ok(__c)
            },
        );
        $hosts.register::<_, ($t0,), $ret, _>(
            $name,
            move |_vm: &mut $crate::interp::Vm, _a0: $t0| -> ::std::result::Result<$ret, $crate::Trap> {
                Err($crate::Trap::new(
                    $crate::TrapKind::Invalid,
                    format!(
                        "async host fn `{}` called through its decl row — the weave mints the future at the call site; `await` (or `launch_future`) drives it",
                        $name
                    ),
                ))
            },
        );
    };
    ($hosts:expr, $name:literal, ($t0:ty, $t1:ty,), $ret:ty, $start:expr) => {
        $hosts.register::<_, ($t0, $t1), $crate::Completer<$ret>, _>(
            concat!($name, "__start"),
            move |__vm: &mut $crate::interp::Vm, a0: $t0, a1: $t1| {
                let __c = $crate::Completer::with_wake(__vm.host_wakes());
                $start(__c.clone(), a0, a1);
                Ok(__c)
            },
        );
        $hosts.register::<_, ($t0, $t1), $ret, _>(
            $name,
            move |_vm: &mut $crate::interp::Vm, _a0: $t0, _a1: $t1| -> ::std::result::Result<$ret, $crate::Trap> {
                Err($crate::Trap::new(
                    $crate::TrapKind::Invalid,
                    format!(
                        "async host fn `{}` called through its decl row — the weave mints the future at the call site; `await` (or `launch_future`) drives it",
                        $name
                    ),
                ))
            },
        );
    };
    ($hosts:expr, $name:literal, ($t0:ty, $t1:ty, $t2:ty,), $ret:ty, $start:expr) => {
        $hosts.register::<_, ($t0, $t1, $t2), $crate::Completer<$ret>, _>(
            concat!($name, "__start"),
            move |__vm: &mut $crate::interp::Vm, a0: $t0, a1: $t1, a2: $t2| {
                let __c = $crate::Completer::with_wake(__vm.host_wakes());
                $start(__c.clone(), a0, a1, a2);
                Ok(__c)
            },
        );
        $hosts.register::<_, ($t0, $t1, $t2), $ret, _>(
            $name,
            move |_vm: &mut $crate::interp::Vm, _a0: $t0, _a1: $t1, _a2: $t2| -> ::std::result::Result<$ret, $crate::Trap> {
                Err($crate::Trap::new(
                    $crate::TrapKind::Invalid,
                    format!(
                        "async host fn `{}` called through its decl row — the weave mints the future at the call site; `await` (or `launch_future`) drives it",
                        $name
                    ),
                ))
            },
        );
    };
    ($hosts:expr, $name:literal, ($t0:ty, $t1:ty, $t2:ty, $t3:ty,), $ret:ty, $start:expr) => {
        $hosts.register::<_, ($t0, $t1, $t2, $t3), $crate::Completer<$ret>, _>(
            concat!($name, "__start"),
            move |__vm: &mut $crate::interp::Vm, a0: $t0, a1: $t1, a2: $t2, a3: $t3| {
                let __c = $crate::Completer::with_wake(__vm.host_wakes());
                $start(__c.clone(), a0, a1, a2, a3);
                Ok(__c)
            },
        );
        $hosts.register::<_, ($t0, $t1, $t2, $t3), $ret, _>(
            $name,
            move |_vm: &mut $crate::interp::Vm, _a0: $t0, _a1: $t1, _a2: $t2, _a3: $t3| -> ::std::result::Result<$ret, $crate::Trap> {
                Err($crate::Trap::new(
                    $crate::TrapKind::Invalid,
                    format!(
                        "async host fn `{}` called through its decl row — the weave mints the future at the call site; `await` (or `launch_future`) drives it",
                        $name
                    ),
                ))
            },
        );
    };
    ($hosts:expr, $name:literal, ($t0:ty, $t1:ty, $t2:ty, $t3:ty, $t4:ty,), $ret:ty, $start:expr) => {
        $hosts.register::<_, ($t0, $t1, $t2, $t3, $t4), $crate::Completer<$ret>, _>(
            concat!($name, "__start"),
            move |__vm: &mut $crate::interp::Vm, a0: $t0, a1: $t1, a2: $t2, a3: $t3, a4: $t4| {
                let __c = $crate::Completer::with_wake(__vm.host_wakes());
                $start(__c.clone(), a0, a1, a2, a3, a4);
                Ok(__c)
            },
        );
        $hosts.register::<_, ($t0, $t1, $t2, $t3, $t4), $ret, _>(
            $name,
            move |_vm: &mut $crate::interp::Vm, _a0: $t0, _a1: $t1, _a2: $t2, _a3: $t3, _a4: $t4| -> ::std::result::Result<$ret, $crate::Trap> {
                Err($crate::Trap::new(
                    $crate::TrapKind::Invalid,
                    format!(
                        "async host fn `{}` called through its decl row — the weave mints the future at the call site; `await` (or `launch_future`) drives it",
                        $name
                    ),
                ))
            },
        );
    };
    ($hosts:expr, $name:literal, ($t0:ty, $t1:ty, $t2:ty, $t3:ty, $t4:ty, $t5:ty,), $ret:ty, $start:expr) => {
        $hosts.register::<_, ($t0, $t1, $t2, $t3, $t4, $t5), $crate::Completer<$ret>, _>(
            concat!($name, "__start"),
            move |__vm: &mut $crate::interp::Vm,
                  a0: $t0,
                  a1: $t1,
                  a2: $t2,
                  a3: $t3,
                  a4: $t4,
                  a5: $t5| {
                let __c = $crate::Completer::with_wake(__vm.host_wakes());
                $start(__c.clone(), a0, a1, a2, a3, a4, a5);
                Ok(__c)
            },
        );
        $hosts.register::<_, ($t0, $t1, $t2, $t3, $t4, $t5), $ret, _>(
            $name,
            move |_vm: &mut $crate::interp::Vm, _a0: $t0, _a1: $t1, _a2: $t2, _a3: $t3, _a4: $t4, _a5: $t5| -> ::std::result::Result<$ret, $crate::Trap> {
                Err($crate::Trap::new(
                    $crate::TrapKind::Invalid,
                    format!(
                        "async host fn `{}` called through its decl row — the weave mints the future at the call site; `await` (or `launch_future`) drives it",
                        $name
                    ),
                ))
            },
        );
    };
    ($hosts:expr, $name:literal, ($t0:ty, $t1:ty, $t2:ty, $t3:ty, $t4:ty, $t5:ty, $t6:ty,), $ret:ty, $start:expr) => {
        $hosts.register::<_, ($t0, $t1, $t2, $t3, $t4, $t5, $t6), $crate::Completer<$ret>, _>(
            concat!($name, "__start"),
            move |__vm: &mut $crate::interp::Vm,
                  a0: $t0,
                  a1: $t1,
                  a2: $t2,
                  a3: $t3,
                  a4: $t4,
                  a5: $t5,
                  a6: $t6| {
                let __c = $crate::Completer::with_wake(__vm.host_wakes());
                $start(__c.clone(), a0, a1, a2, a3, a4, a5, a6);
                Ok(__c)
            },
        );
        $hosts.register::<_, ($t0, $t1, $t2, $t3, $t4, $t5, $t6), $ret, _>(
            $name,
            move |_vm: &mut $crate::interp::Vm, _a0: $t0, _a1: $t1, _a2: $t2, _a3: $t3, _a4: $t4, _a5: $t5, _a6: $t6| -> ::std::result::Result<$ret, $crate::Trap> {
                Err($crate::Trap::new(
                    $crate::TrapKind::Invalid,
                    format!(
                        "async host fn `{}` called through its decl row — the weave mints the future at the call site; `await` (or `launch_future`) drives it",
                        $name
                    ),
                ))
            },
        );
    };
    ($hosts:expr, $name:literal, ($t0:ty, $t1:ty, $t2:ty, $t3:ty, $t4:ty, $t5:ty, $t6:ty, $t7:ty,), $ret:ty, $start:expr) => {
        $hosts.register::<_, ($t0, $t1, $t2, $t3, $t4, $t5, $t6, $t7), $crate::Completer<$ret>, _>(
            concat!($name, "__start"),
            move |__vm: &mut $crate::interp::Vm,
                  a0: $t0,
                  a1: $t1,
                  a2: $t2,
                  a3: $t3,
                  a4: $t4,
                  a5: $t5,
                  a6: $t6,
                  a7: $t7| {
                let __c = $crate::Completer::with_wake(__vm.host_wakes());
                $start(__c.clone(), a0, a1, a2, a3, a4, a5, a6, a7);
                Ok(__c)
            },
        );
        $hosts.register::<_, ($t0, $t1, $t2, $t3, $t4, $t5, $t6, $t7), $ret, _>(
            $name,
            move |_vm: &mut $crate::interp::Vm, _a0: $t0, _a1: $t1, _a2: $t2, _a3: $t3, _a4: $t4, _a5: $t5, _a6: $t6, _a7: $t7| -> ::std::result::Result<$ret, $crate::Trap> {
                Err($crate::Trap::new(
                    $crate::TrapKind::Invalid,
                    format!(
                        "async host fn `{}` called through its decl row — the weave mints the future at the call site; `await` (or `launch_future`) drives it",
                        $name
                    ),
                ))
            },
        );
    };
}

// ---- the wake substrate's own tests (the thread law's disclosed
// exception: these play the embedder's worker) ----

#[cfg(test)]
mod wakes_tests {
    use super::*;
    use crate::interp::HostRegistry;
    use std::rc::Rc;
    use std::time::Duration;

    /// bump() increments the generation, and a waiter parked on the
    /// current epoch wakes exactly when the NEXT bump lands (its
    /// answer names the epoch it woke on — an immediate-return bug
    /// would answer the stale one).
    #[test]
    fn bump_moves_the_generation_and_wakes_a_parked_waiter() {
        let wakes = HostWakes::shared();
        assert_eq!(wakes.generation(), 0, "a fresh cell starts at 0");
        wakes.bump();
        assert_eq!(wakes.generation(), 1, "one bump, one generation");

        // park a waiter on the CURRENT generation; it may only answer
        // once the next bump moves past it
        let waiter = {
            let wakes = wakes.clone();
            std::thread::spawn(move || wakes.wait_since(1))
        };
        std::thread::sleep(Duration::from_millis(50)); // let it park
        wakes.bump();
        assert_eq!(
            waiter.join().expect("the waiter thread"),
            2,
            "the parked waiter woke on the bump and saw its generation"
        );
        assert_eq!(wakes.generation(), 2);
    }

    /// The settle-then-signal order: a worker completes; a waiter that
    /// parks AFTER the completion never blocks — the generation
    /// already moved, so `wait_since` on any pre-settle epoch returns
    /// immediately (the immediate-return shape the driving loop's
    /// race-free re-check rides).
    #[test]
    fn the_settle_precedes_the_signal_and_a_late_waiter_never_blocks() {
        let wakes = HostWakes::shared();
        let c: Completer<u64> = Completer::with_wake(wakes.clone());

        // the worker settles from another thread; the join proves the
        // settle AND its bump are done before the waiter arrives
        let worker = {
            let c = c.clone();
            std::thread::spawn(move || c.complete(7))
        };
        worker.join().expect("the worker thread");

        assert_eq!(c.poll(), READY, "the settle landed");
        assert_eq!(wakes.generation(), 1, "one settle, one bump");
        // the immediate-return shape: parking on a pre-settle epoch
        // answers at once — the signal did not precede the settle
        assert_eq!(wakes.wait_since(0), 1);
        // and the answer the signal woke for is actually there
        assert_eq!(c.take_result().expect("the answer"), 7);
    }

    /// A `fail` settles AND bumps — a failed host future wakes the
    /// machine too (the await must trap, not hang).
    #[test]
    fn fail_settles_and_bumps() {
        let wakes = HostWakes::shared();
        let c: Completer<u64> = Completer::with_wake(wakes.clone());
        c.fail("wire dead".to_string());
        assert_eq!(c.poll(), FAILED);
        assert_eq!(wakes.generation(), 1, "a failure is a settle");
        match c.take_result() {
            Err(trap) => assert!(trap.msg.contains("wire dead")),
            Ok(_) => panic!("a failed completer traps"),
        }
    }

    /// One cell per machine: the registry's Arc and the Vm's Arc are
    /// the same object — the builder carries the registry's cell onto
    /// the machine, so every host future minted through the sugars
    /// bumps the cell the driving loop parks on.
    #[test]
    fn one_cell_per_machine_the_registry_and_the_vm_share_it() {
        let hosts = HostRegistry::new();
        let registry_wakes = hosts.wakes();
        let prog = rut_core::binary::Program {
            types: rut_core::types::TypeTable::boot(),
            ..rut_core::binary::Program::default()
        };
        let vm = Vm::builder()
            .program(Rc::new(prog))
            .hosts(hosts)
            .build()
            .expect("bare vm");
        assert!(
            Arc::ptr_eq(&registry_wakes, &vm.host_wakes()),
            "the registry's wake cell IS the machine's"
        );
    }

    /// A sugar-minted cell is wired: the exact mint the sugars'
    /// `__start` rows emit — `Completer::with_wake(vm.host_wakes())` —
    /// rides the machine's cell, so a body that settles its handed
    /// completer from a worker bumps the SAME Arc the registry handed
    /// the Vm. (The macro expansion itself is compile-checked by the
    /// registration tests; dispatch needs a compiled program, which is
    /// the driver tests' lane.)
    #[test]
    fn a_sugar_minted_completer_bumps_the_machines_cell() {
        let hosts = HostRegistry::new();
        let machine_wakes = hosts.wakes();
        let prog = rut_core::binary::Program {
            types: rut_core::types::TypeTable::boot(),
            ..rut_core::binary::Program::default()
        };
        let vm = Vm::builder()
            .program(Rc::new(prog))
            .hosts(hosts)
            .build()
            .expect("bare vm");

        // the sugar's `__start` mint, verbatim
        let c: Completer<i64> = Completer::with_wake(vm.host_wakes());
        assert_eq!(machine_wakes.generation(), 0, "nothing settled yet");

        // the body hands a clone to its worker; the worker settles
        let w = c.clone();
        std::thread::spawn(move || w.complete(5));

        // the MACHINE's cell moved — the driving loop's park would wake
        assert_eq!(machine_wakes.wait_since(0), 1);
        assert_eq!(c.poll(), READY);
        assert_eq!(c.take_result().expect("the answer"), 5);
    }
}
