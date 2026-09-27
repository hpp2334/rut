//! The engine frame-cell convention (RFC 0018) — the ONE layout law the
//! compiler's weave (`rut-lir`) and the driving loop (`rut-vm`) share.
//!
//! An async fn compiles into a hidden `TyKind::Data` frame type whose
//! record cell carries the coroutine state in FIXED fields; every body
//! local is cell-backed behind them. The cx (`RunContext`) is an
//! engine-minted one-field record over the frame edge; the checkpoint
//! enum is a per-fn `TyKind::Enum` whose member index IS the resume
//! state — the frame's state field holds the member's immortal
//! singleton slot, `Op::BrTable` dispatches on it, and the DONE /
//! retired sentinel is the null slot (so "already finished" and "dropped
//! at its checkpoint" are the same retirement, and a re-drive of a
//! retired frame answers at the brtable's default arm).

/// Field 0 — the checkpoint state: an enum-member singleton of the fn's
/// checkpoint enum while live/parked, null once completed or dropped.
pub const STATE_FIELD: u32 = 0;
/// Field 1 — the task's cancellation flag (`cx.cancelled()` reads it;
/// `abort`/`cancel` writes it engine-side).
pub const CANCELLED_FIELD: u32 = 1;
/// Field 2 — the awaiter edge: while THIS frame parks on a future, the
/// AWAITED frame's field holds this frame's slot, so the driving loop
/// can re-enqueue the awaiter when the awaited frame completes. Null
/// otherwise (set only on the park path — the invariant the loop's
/// completion enqueue reads).
pub const AWAITER_FIELD: u32 = 2;
/// Field 3 — this frame's own park edge: the future THIS frame is
/// parked on, kept so the resume arm can clear the awaited frame's
/// awaiter edge (the one-directional pair never forms a cycle).
pub const PENDING_FIELD: u32 = 3;
/// First body-local field. Parameters first (call-site written), then
/// bindings in declaration order — every local is cell-backed uniformly.
pub const LOCALS_BASE: u32 = 4;

/// The cx record's name — the surface spelling `RunContext` resolves to
/// it in type position, and the driving loop finds it by this name to
/// mint the per-drive cx.
pub const RUN_CONTEXT_TYPE: &str = "RunContext";
/// The cx record's single field: the frame edge.
pub const RUN_CONTEXT_FRAME_FIELD: &str = "frame";

/// The hidden frame type's name prefix (`#frame@<fn>`); `#` is
/// unspellable in rut source, so the namespace is engine-reserved.
pub const FRAME_PREFIX: &str = "#frame@";
/// The checkpoint enum's name prefix (`#ckpt@<fn>`).
pub const CKPT_PREFIX: &str = "#ckpt@";
/// The sleep future's reserved frame/ckpt names (no node suffix — the
/// engine-minted body finds them by exactly these names).
pub const SLEEP_FRAME: &str = "#frame@sleep";
pub const SLEEP_CKPT: &str = "#ckpt@sleep";
