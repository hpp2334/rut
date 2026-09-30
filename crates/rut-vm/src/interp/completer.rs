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
//! phase-1 return lane). The embedder's body is ONE closure: spawn
//! work, hand out a [`Completer`] clone, `complete`/`fail` from the
//! worker.
//!
//! THREAD LAW: the VM stays single-threaded. Workers touch only the
//! Completer's atomics + Mutex slot — a clone shares them through one
//! `Arc`; the rut cells are minted by `__take`/`into_slot` on the VM
//! thread. `std::thread` appears only in embedder closures, never here.

use std::sync::{Arc, Mutex};

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

/// The shared state: atomics + a Mutex slot, nothing else — the ONLY
/// thing a worker thread touches (the thread law).
struct Inner<T> {
    /// 0 pending / 1 ready / 2 failed — the release/acquire pair with
    /// the slot below makes `poll`'s answer authoritative.
    done: std::sync::atomic::AtomicU64,
    slot: Mutex<Option<Result<T, String>>>,
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
        Completer { inner: self.inner.clone() }
    }
}

impl<T> Default for Completer<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Completer<T> {
    pub fn new() -> Completer<T> {
        Completer {
            inner: Arc::new(Inner {
                done: std::sync::atomic::AtomicU64::new(PENDING as u64),
                slot: Mutex::new(None),
            }),
        }
    }
}

impl<T: Ret> Completer<T> {
    /// Settle the future with its answer — from any thread. A late
    /// answer after a cancel is the DISCLOSED best-effort law: the
    /// thread runs to its blocking completion, the result is simply
    /// never taken.
    pub fn complete(&self, v: T) {
        *self.inner.slot.lock().expect("completer slot") = Some(Ok(v));
        self.inner
            .done
            .store(READY as u64, std::sync::atomic::Ordering::Release);
    }

    /// Settle the future with a failure — the message traps at the
    /// await, through `__take`. Any thread.
    pub fn fail(&self, msg: String) {
        *self.inner.slot.lock().expect("completer slot") = Some(Err(msg));
        self.inner
            .done
            .store(FAILED as u64, std::sync::atomic::Ordering::Release);
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
///     |url: String| -> Completer<Vec<u8>> {
///         let c = Completer::new();
///         let w = c.clone();
///         std::thread::spawn(move || { let data = fetch(url); w.complete(data); });
///         c
///     });
/// ```
///
/// The closure takes NO vm parameter (the work leaves the VM; the
/// virtual clock and the queues need none of it) and answers the
/// future's [`Completer`]. `T: Ret` — the compiler enforces "async
/// answers are crossing types". The macro emits all five bindings the
/// program can join:
///
/// - `<name>` — the decl row itself; the weave never dispatches it
///   (the call site mints the frame instead), so the body is the
///   teaching trap.
/// - `<name>__start(args...) -> opaque` — calls the closure, boxes
///   the Completer: the state cell.
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
                Ok(state.with(|c| c.poll())?)
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
            move |_vm: &mut $crate::interp::Vm| Ok($start()),
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
            move |_vm: &mut $crate::interp::Vm, a0: $t0| Ok($start(a0)),
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
            move |_vm: &mut $crate::interp::Vm, a0: $t0, a1: $t1| Ok($start(a0, a1)),
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
            move |_vm: &mut $crate::interp::Vm, a0: $t0, a1: $t1, a2: $t2| Ok($start(a0, a1, a2)),
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
            move |_vm: &mut $crate::interp::Vm, a0: $t0, a1: $t1, a2: $t2, a3: $t3| {
                Ok($start(a0, a1, a2, a3))
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
            move |_vm: &mut $crate::interp::Vm, a0: $t0, a1: $t1, a2: $t2, a3: $t3, a4: $t4| {
                Ok($start(a0, a1, a2, a3, a4))
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
            move |_vm: &mut $crate::interp::Vm, a0: $t0, a1: $t1, a2: $t2, a3: $t3, a4: $t4, a5: $t5| {
                Ok($start(a0, a1, a2, a3, a4, a5))
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
            move |_vm: &mut $crate::interp::Vm, a0: $t0, a1: $t1, a2: $t2, a3: $t3, a4: $t4, a5: $t5, a6: $t6| {
                Ok($start(a0, a1, a2, a3, a4, a5, a6))
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
            move |_vm: &mut $crate::interp::Vm, a0: $t0, a1: $t1, a2: $t2, a3: $t3, a4: $t4, a5: $t5, a6: $t6, a7: $t7| {
                Ok($start(a0, a1, a2, a3, a4, a5, a6, a7))
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


