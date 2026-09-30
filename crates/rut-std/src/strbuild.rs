//! `strbuild`'s host half: the growable string builder's five native
//! functions `strbuild_host::sb_new` / `sb_push` / `sb_push_code` /
//! `sb_len` / `sb_finish` (the ink/Logger pattern — the builder's state
//! is an `opaque` handle owning Rust state rut never sees; the rows are
//! declared in `rut/strbuild_host/` under the pkg-name scope, the
//! rut class over them lives in `rut/strbuild/`).
//!
//! Core ships ZERO string-building machinery: the engine's growable
//! cell is gone, and the builder is this host package. The laws move
//! over verbatim:
//!
//! - **the charge law**: growth consults [`Vm::charge_public`] BEFORE
//!   the buffer grows — the geometric next-capacity minus the current —
//!   so the embedder's heap budget (the wasm 4 MiB cap included)
//!   governs builder growth exactly as it governed the engine cell.
//!   The policy mirrors the engine's block store: geometric doubling,
//!   class-rounded through the same size classes.
//! - **the U+FFFD rule**: `sb_push_code` mints U+FFFD for invalid
//!   scalars — the `str.from_code` rule, decided here, not in the
//!   wrapper.
//! - **the finish law**: `sb_finish` is the ONE materialization (a
//!   fresh immutable `str` via the boundary's `String` return, charged
//!   like any allocation); the builder KEEPS its buffer — finish twice
//!   answers the same text, and the built str is immune to later
//!   appends.
//! - **the share law**: the box is a reference cell, so appends through
//!   an alias are visible through the original; `len` is the tracked
//!   codepoint count (O(1), never a scan).
//!
//! Bindings are the MAGIC shape (the `&str` param is a zero-copy
//! borrow of the block store, scoped to exactly the call by the
//! handler's HRTB); the payload is a `RefCell<BuilderBuf>` behind an
//! `Opaque` handle — `Drop` at cell death frees the buffer.

use std::cell::RefCell;

use rut_vm::interp::{HostPkg, Vm};
use rut_vm::{Opaque, OpaqueRef, Trap, TrapKind};

/// The builder's payload: octets + the tracked codepoint count + the
/// geometric growth policy and its charge computation. `cap` mirrors
/// the engine's `StrVal.cap` — the capacity the buffer actually holds,
/// what the accounting charges against.
pub struct BuilderBuf {
    octets: Vec<u8>,
    /// codepoints appended so far (tracked at push, O(1) `len`)
    chars: u32,
    cap: usize,
}

/// The block store's size classes (bytes), mirrored from
/// `rut_vm::heap::blocks` — the class rounding the engine's own
/// buffers grow through, so a host builder's allocation profile reads
/// the same in a heap peak as an engine cell's. Above `SMALL_MAX` the
/// engine takes a dedicated allocation rounded to 8; the mirror does
/// the same rounding.
const CLASSES: [usize; 24] = [
    16, 32, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 640, 768, 896,
    1024, 1280, 1536, 1792, 2048,
];
const SMALL_MAX: usize = *CLASSES.last().unwrap();

/// Class-round a capacity request the way the block store does: the
/// smallest class at or above `want`, 8-rounded past the table.
fn class_round(want: usize) -> usize {
    let req = (want.max(1) + 7) & !7;
    if req > SMALL_MAX {
        return req;
    }
    for c in CLASSES {
        if c >= req {
            return c;
        }
    }
    SMALL_MAX
}

impl BuilderBuf {
    /// One empty UTF-8 buffer pre-sized to `cap` octets (advisory —
    /// class-rounded, behavior identical for every hint).
    fn with_cap(cap: usize) -> BuilderBuf {
        let cap = class_round(cap);
        BuilderBuf { octets: Vec::new(), chars: 0, cap }
    }

    /// The growth a push of `extra` octets needs, charged BEFORE the
    /// buffer grows: the geometric next-capacity (`max(need, 2 * cap)`,
    /// class-rounded — the block store's `grow` policy) minus the
    /// current, or `None` when the capacity already holds the push.
    fn growth_charge(&self, extra: usize) -> Option<u64> {
        let need = self.octets.len() + extra;
        if need <= self.cap {
            return None;
        }
        let target = class_round(need.max(self.cap.saturating_mul(2)));
        Some((target - self.cap) as u64)
    }

    /// Append octets + their codepoint count in place, consulting
    /// `charge` first when a grow is needed. A failed charge leaves the
    /// text untouched (only spare capacity was to be reserved).
    fn push(&mut self, extra: &[u8], extra_chars: u32, charge: impl FnOnce(u64) -> Result<(), Trap>) -> Result<(), Trap> {
        if let Some(bytes) = self.growth_charge(extra.len()) {
            charge(bytes)?;
            let need = self.octets.len() + extra.len();
            let target = class_round(need.max(self.cap.saturating_mul(2)));
            self.octets.reserve_exact(target - self.octets.len());
            self.cap = self.octets.capacity().max(target);
        }
        self.octets.extend_from_slice(extra);
        self.chars = self.chars.wrapping_add(extra_chars);
        Ok(())
    }
}

/// Build `strbuild_host`'s pkg. The bindings are TYPED: the derived
/// signatures match `rut/strbuild_host/strbuild_host.d.rut`, and the
/// install checks the contract before any rut code runs.
pub fn pkg() -> HostPkg {
    let mut pkg = HostPkg::new("strbuild_host");
    pkg.register::<_, (i32,), OpaqueRef, _>(
        "sb_new",
        |vm: &mut Vm, cap: i32| -> Result<OpaqueRef, Trap> {
            if cap < 0 {
                return Err(Trap::new(
                    TrapKind::Invalid,
                    format!("sb_new(cap): capacity must be >= 0, got {cap}"),
                ));
            }
            // the pre-size is advisory but REAL memory: the class-rounded
            // hint charges before the buffer exists (the engine cell
            // charged exactly the same way), so a huge hint under a small
            // budget traps here, not at the first push
            let buf = BuilderBuf::with_cap(cap as usize);
            vm.charge_public(buf.cap as u64)?;
            let b = Opaque::alloc(vm, RefCell::new(buf))?;
            Ok(b.handle().clone())
        },
    );
    pkg.register::<_, (Opaque<RefCell<BuilderBuf>>, &str), (), _>(
        "sb_push",
        |vm: &mut Vm, b: Opaque<RefCell<BuilderBuf>>, s: &str| -> Result<(), Trap> {
            // zero-copy: `s` borrows the block store for exactly this call
            let chars = if s.is_ascii() { s.len() as u32 } else { s.chars().count() as u32 };
            b.with_mut(vm, |vm, buf| buf.borrow_mut().push(s.as_bytes(), chars, |d| vm.charge_public(d)))?
        },
    );
    pkg.register::<_, (Opaque<RefCell<BuilderBuf>>, u32), (), _>(
        "sb_push_code",
        |vm: &mut Vm, b: Opaque<RefCell<BuilderBuf>>, cp: u32| -> Result<(), Trap> {
            // invalid scalars (surrogates) mint U+FFFD — the `str.from_code` rule
            let ch = char::from_u32(cp).unwrap_or('\u{FFFD}');
            let mut raw = [0u8; 4];
            let enc = ch.encode_utf8(&mut raw).as_bytes();
            b.with_mut(vm, |vm, buf| {
                buf.borrow_mut().push(enc, 1, |d| vm.charge_public(d))
            })?
        },
    );
    pkg.register::<_, (Opaque<RefCell<BuilderBuf>>,), i32, _>(
        "sb_len",
        |_vm: &mut Vm, b: Opaque<RefCell<BuilderBuf>>| b.with(|buf| buf.borrow().chars as i32),
    );
    pkg.register::<_, (Opaque<RefCell<BuilderBuf>>,), String, _>(
        "sb_finish",
        |vm: &mut Vm, b: Opaque<RefCell<BuilderBuf>>| -> Result<String, Trap> {
            // the ONE materialization: the octets copy out to a fresh
            // immutable str (the boundary's `String` return allocates it);
            // the builder KEEPS its buffer — finish twice answers the
            // same text, immune to later appends
            b.with(|buf| String::from_utf8(buf.borrow().octets.clone()))?
                .map_err(|_| Trap::new(TrapKind::Invalid, "sb_finish: builder octets are not UTF-8"))
        },
    );
    pkg.build()
}
