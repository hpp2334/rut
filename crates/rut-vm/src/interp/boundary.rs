//! The typed host boundary (RFC 0023, revised): Rust types are the
//! currency of `Vm::call`/`Vm::resume` and (Phase 2) of host-fn bodies;
//! `Value`/`Slot` are the internal marshaling formats, invisible outside
//! the crate. Owned impls (`String`, `Vec<u8>`) are the explicit "I keep
//! this data" copy; `&str`/`&[u8]` host params (Phase 2) borrow the block
//! store zero-copy. `Option<T>` reads a crossing `?T` NIL-FLATTENED (the
//! err-channel phase 3: nil → `None`, the box's payload → `Some`) — the
//! read direction only, no `into_slot`: there is no `Value::Opt`, so a
//! host cannot mint a some-payload for rut yet. `Value` itself is also a
//! `Ret`: the positional decode for hosts that read the raw driver
//! result (`Value::Tuple` for a pair return) instead of a typed shape.

use rut_core::types::{PrimTy, TypeId, TyKind};
use rut_core::types::{
    TY_BOOL, TY_BYTES, TY_F32, TY_F64, TY_I16, TY_I32, TY_I64, TY_I8, TY_NIL, TY_OPAQUE,
    TY_STR, TY_U16, TY_U32, TY_U64, TY_U8, TY_VAL,
};

use super::*;
use crate::arena::OpaqueRef;
use crate::heap::{Opaque, ValSlot};

/// A rut → Rust conversion: read a slot under its declared type.
/// `declared` is program-relative (the export's/field's own id), so the
/// check catches an embedder passing the wrong Rust shape — a trap naming
/// both sides, never UB.
pub trait Ret: Sized {
    /// the fixed boot type this Rust type binds against (`u32::MAX` when
    /// the id is program-relative — tuples)
    const TY: TypeId = u32::MAX;
    /// a short Rust-side name for trap messages
    fn rust_name() -> &'static str;
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap>;
    /// Rust → rut: build a slot that owns its references (a host-fn
    /// return or `call` argument). The returned slot's reference count —
    /// if any — belongs to the caller.
    fn into_slot(self, vm: &mut Vm) -> Result<Slot, Trap> {
        let _ = vm;
        Err(Trap::new(
            TrapKind::Invalid,
            format!("`{}` does not cross as a host-fn argument or return", Self::rust_name()),
        ))
    }
}

/// check the declared type's kind; a mismatch is a trap naming both sides
fn expect_kind(vm: &Vm, slot: Slot, declared: TypeId, rust: &str) -> Result<(), Trap> {
    let bad = |m: String| {
        Trap::new(
            TrapKind::Invalid,
            format!("boundary: got `{}` where `{rust}` binds: {m}", vm.prog.type_name(declared)),
        )
    };
    if unsafe { slot.r.is_null() }
        && !matches!(vm.prog.types.kind(declared), TyKind::Prim(_) | TyKind::Nil)
    {
        // a null ref slot only ever binds a primitive (or nil)
        return Err(bad("nil does not bind a reference type".into()));
    }
    match vm.prog.types.kind(declared) {
        TyKind::Prim(PrimTy::U8) if rust == "u8" => Ok(()),
        TyKind::Prim(PrimTy::U16) if rust == "u16" => Ok(()),
        TyKind::Prim(PrimTy::U32) if rust == "u32" => Ok(()),
        TyKind::Prim(PrimTy::U64) if rust == "u64" => Ok(()),
        TyKind::Prim(PrimTy::I8) if rust == "i8" => Ok(()),
        TyKind::Prim(PrimTy::I16) if rust == "i16" => Ok(()),
        TyKind::Prim(PrimTy::I32) if rust == "i32" => Ok(()),
        TyKind::Prim(PrimTy::I64) if rust == "i64" => Ok(()),
        TyKind::Prim(PrimTy::F32) if rust == "f32" => Ok(()),
        TyKind::Prim(PrimTy::F64) if rust == "f64" => Ok(()),
        TyKind::Prim(PrimTy::Bool) if rust == "bool" => Ok(()),
        TyKind::Str if rust == "String" || rust == "&str" => Ok(()),
        TyKind::Bytes if rust == "Vec<u8>" || rust == "&[u8]" => Ok(()),
        TyKind::Opaque if rust.starts_with("Opaque") => Ok(()),
        TyKind::Nil if rust == "()" => Ok(()),
        TyKind::Data { fields } if rust.starts_with('(') => {
            // arity is checked by the caller's field loop; here only the
            // "is it the record shape the Rust tuple expects" question
            let _ = fields;
            Ok(())
        }
        _ => Err(bad("kind mismatch".into())),
    }
}

/// the `&[u8]` view over a `bytes` cell — the block store's own octets
fn bytes_view<'a>(slot: Slot) -> &'a [u8] {
    use crate::heap::ArrKind;
    match &cell_of(slot).data {
        CellData::Str(v) => v.bytes(),
        CellData::Array { items, .. } if items.borrow().kind == ArrKind::U8 => {
            let d = items.borrow();
            // SAFETY: the block lives as long as the cell (arg retention)
            unsafe { std::slice::from_raw_parts(d.block, d.len as usize) }
        }
        _ => &[],
    }
}

fn narrow_i64<T: TryFrom<i64>>(n: i64, rust: &str) -> Result<T, Trap> {
    T::try_from(n).map_err(|_| {
        Trap::new(TrapKind::Invalid, format!("boundary: `{n}` does not fit `{rust}`"))
    })
}

macro_rules! int_ret {
    ($t:ty, $name:literal, $ty:ident) => {
        impl Ret for $t {
            const TY: TypeId = $ty;
            fn rust_name() -> &'static str { $name }
            fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
                expect_kind(vm, slot, declared, $name)?;
                Ok(narrow_i64::<$t>(unsafe { slot.i }, $name)?)
            }
            #[inline] // crossing-fastpath phase 2: hot read/into_slot impls
            fn into_slot(self, _vm: &mut Vm) -> Result<Slot, Trap> {
                Ok(Slot::int(self as i64))
            }
        }
    };
}

int_ret!(i8, "i8", TY_I8);
int_ret!(i16, "i16", TY_I16);
int_ret!(i32, "i32", TY_I32);
int_ret!(i64, "i64", TY_I64);
int_ret!(u8, "u8", TY_U8);
int_ret!(u16, "u16", TY_U16);
int_ret!(u32, "u32", TY_U32);

/// raw-bit read: a rut `u64` lives in the slot as its own bit pattern —
/// no i64 narrowing applies
impl Ret for u64 {
    const TY: TypeId = TY_U64;
    fn rust_name() -> &'static str { "u64" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "u64")?;
        Ok(unsafe { slot.i } as u64)
    }
    #[inline] // hot lane
    fn into_slot(self, _vm: &mut Vm) -> Result<Slot, Trap> {
        Ok(Slot::int(self as i64))
    }
}

impl Ret for f64 {
    const TY: TypeId = TY_F64;
    fn rust_name() -> &'static str { "f64" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "f64")?;
        Ok(slot.as_f64())
    }
    #[inline] // hot lane
    fn into_slot(self, _vm: &mut Vm) -> Result<Slot, Trap> {
        Ok(Slot::float(self))
    }
}

impl Ret for f32 {
    const TY: TypeId = TY_F32;
    fn rust_name() -> &'static str { "f32" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "f32")?;
        Ok(slot.as_f64() as f32)
    }
    #[inline] // hot lane
    fn into_slot(self, _vm: &mut Vm) -> Result<Slot, Trap> {
        Ok(Slot::float(self as f64))
    }
}

impl Ret for bool {
    const TY: TypeId = TY_BOOL;
    fn rust_name() -> &'static str { "bool" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "bool")?;
        Ok(slot.as_bool())
    }
    #[inline] // hot lane
    fn into_slot(self, _vm: &mut Vm) -> Result<Slot, Trap> {
        Ok(Slot::bool(self))
    }
}

impl Ret for () {
    const TY: TypeId = TY_NIL;
    fn rust_name() -> &'static str { "()" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "()")?;
        let _ = slot;
        Ok(())
    }
    #[inline] // hot lane
    fn into_slot(self, _vm: &mut Vm) -> Result<Slot, Trap> {
        Ok(Slot::int(0)) // nil is the zero word (RFC 0015 §5)
    }
}

/// owned copy — the explicit "I keep this data" (RFC 0023 §2)
impl Ret for String {
    fn rust_name() -> &'static str { "String" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "String")?;
        Ok(cell_of(slot).as_str().to_string())
    }
    #[inline] // hot lane
    fn into_slot(self, vm: &mut Vm) -> Result<Slot, Trap> {
        vm.heap.alloc_str(self).map_err(|t| t)
    }
}

/// owned copy — the explicit "I keep this data" (RFC 0023 §2)
impl Ret for Vec<u8> {
    fn rust_name() -> &'static str { "Vec<u8>" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "Vec<u8>")?;
        Ok(cell_of(slot).bytes_copy())
    }
    #[inline] // hot lane
    fn into_slot(self, vm: &mut Vm) -> Result<Slot, Trap> {
        vm.heap.alloc_bytes(self)
    }
}

impl Ret for OpaqueRef {
    const TY: TypeId = TY_OPAQUE;
    fn rust_name() -> &'static str { "OpaqueRef" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "OpaqueRef")?;
        let p = unsafe { slot.r };
        if p.is_null() {
            return Err(Trap::new(TrapKind::NilDeref, "boundary: nil does not bind `OpaqueRef`"));
        }
        Ok(vm.heap.opaque_handle(p))
    }
    #[inline] // hot lane
    fn into_slot(self, vm: &mut Vm) -> Result<Slot, Trap> {
        let _ = vm;
        // TRANSFER: the handle's reference count becomes the slot's —
        // `mem::forget` skips the Drop release (crossing-ownership law)
        let p = self.ptr();
        std::mem::forget(self);
        Ok(Slot { r: p })
    }
}

impl<T: 'static> Ret for Opaque<T> {
    fn rust_name() -> &'static str { "Opaque" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "Opaque")?;
        let p = unsafe { slot.r };
        if p.is_null() {
            return Err(Trap::new(TrapKind::NilDeref, "boundary: nil does not bind `Opaque`"));
        }
        Opaque::from_handle(&vm.heap.opaque_handle(p))
    }
    #[inline] // hot lane
    fn into_slot(self, vm: &mut Vm) -> Result<Slot, Trap> {
        self.handle().clone().into_slot(vm)
    }
}

/// A crossing `?T` read NIL-FLATTENED (err-channel phase 3, RFC 0023 §1):
/// the null slot is `None`, a some-slot is the MakeOpt box — its payload
/// decodes as `T` under the element's own type. This is the typed twin of
/// `slot_to_value`'s `TyKind::Opt` arm, so `(?T, err)` entry returns
/// decode positionally as `(Option<T>, String)`; the caller's convention
/// (exactly one of the two channels meaningful) lives with the caller.
impl<T: Ret> Ret for Option<T> {
    fn rust_name() -> &'static str { std::any::type_name::<Self>() }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        let TyKind::Opt { elem } = vm.prog.types.kind(declared) else {
            return Err(Trap::new(
                TrapKind::Invalid,
                format!("boundary: `{}` is not the option `{}` binds", vm.prog.type_name(declared), Self::rust_name()),
            ));
        };
        if unsafe { slot.r.is_null() } {
            return Ok(None); // the flat nil: zero bits, no box (RFC 0044)
        }
        let cell = cell_of(slot);
        let CellData::Record { fields } = &cell.data else {
            return Err(Trap::new(TrapKind::Invalid, "boundary: not an option box"));
        };
        let inner = fields
            .borrow()
            .get(0)
            .ok_or_else(|| Trap::new(TrapKind::Invalid, "boundary: option box without a payload"))?;
        Ok(Some(T::from_slot(vm, inner, *elem)?))
    }
    // no into_slot: the read direction only — see the module doc
}

/// tuples cross field-by-field (RFC 0007 v1.1) — the record cell's own
/// field types drive each element's conversion
macro_rules! tuple_ret {
    ($($n:ident),+) => {
        impl<$($n: Ret),+> Ret for ($($n,)+) {
            fn rust_name() -> &'static str { std::any::type_name::<Self>() }
            fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
                use rut_core::types::TyKind;
                let rust = <Self as Ret>::rust_name();
                let TyKind::Data { fields } = vm.prog.types.kind(declared) else {
                    return Err(Trap::new(
                        TrapKind::Invalid,
                        format!("boundary: `{}` is not the record `{rust}` binds", vm.prog.type_name(declared)),
                    ));
                };
                let fields = fields.clone();
                let cell = cell_of(slot);
                let CellData::Record { fields: slots } = &cell.data else {
                    return Err(Trap::new(TrapKind::Invalid, "boundary: not a record cell"));
                };
                let want: usize = 0 $(+ { let _ = stringify!($n); 1 })+;
                let sb = slots.borrow();
                if sb.len() != want || fields.len() != want {
                    return Err(Trap::new(
                        TrapKind::Invalid,
                        format!("boundary: tuple arity {} vs {} fields", want, fields.len()),
                    ));
                }
                let mut i = 0usize;
                Ok(($(
                    {
                        let fty = fields[i].ty;
                        let s = sb.get(i).unwrap_or(Slot::null());
                        i += 1;
                        $n::from_slot(vm, s, fty)?
                    },
                )+))
            }
        }
    };
}

tuple_ret!(A1);
tuple_ret!(A1, A2);
tuple_ret!(A1, A2, A3);
tuple_ret!(A1, A2, A3, A4);
tuple_ret!(A1, A2, A3, A4, A5);
tuple_ret!(A1, A2, A3, A4, A5, A6);
tuple_ret!(A1, A2, A3, A4, A5, A6, A7);
tuple_ret!(A1, A2, A3, A4, A5, A6, A7, A8);

/// The whole crossing as one Rust value — the POSITIONAL decode (the
/// survey §4.2 driver result): a pair return arrives as `Value::Tuple`,
/// its `?T` component already nil-flattened (`Value::Nil` / the payload's
/// value), so a host reading the raw shape — the run envelope's decoder,
/// generic tooling — reads the err convention off the second component
/// without naming `T` in its type. `slot_to_value` IS the read; into_slot
/// is meaningless (a `Value` has no declared type to bind against).
impl Ret for Value {
    fn rust_name() -> &'static str { "Value" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        Ok(super::util::slot_to_value(slot, declared, &vm.prog, &vm.heap))
    }
}

/// The any-answer (nmap-hostvals P3) — the write direction the boundary
/// never had: a host fn answering a `.d.rut` `-> any` row hands back a
/// [`ValSlot`], and the CALLER's static V (the dst register's own
/// declared type, read from `FuncDef.regs` per call) gives the word its
/// meaning — the untagged-slot discipline (§0.8 h, the ArrGet law):
///
/// - `Some(ValSlot::Ref(s))` — a ref V register takes the ArrGet shape:
///   the answer is RETAINED into the register and the displaced value
///   releases (the register borrows its own rc; the store keeps its
///   own) — no box, no copy. A prim/any V register takes the 8 bytes
///   plain (a cell pointer is bits; the bytecode types them — the trust
///   law documented at the decl site: the host answers the caller's V).
/// - `Some(ValSlot::Bits(s))` — the 8 bytes move as-is; an i32 V
///   register holds the same word an i64 answer wrote (no width
///   conversion exists to skip — the survey's ArrGet receipt). A `?prim`
///   V register (P5, the wrapper's `get -> ?V`) takes the
///   ArrGet{OptPrim} shape instead: `call_host` MINTS the one-slot opt
///   value the register owns (release the displaced, retain nothing) —
///   the Some/None tag rides the `Vm::host_val_out` carry, because a
///   stored zero and the miss are the same 8 bytes and the mint must
///   never answer `some(0)` for an absent key.
/// - `None` — the miss: the flat nil, the zero word (null ref and nil
///   prim are the same 8 bytes, RFC 0044's zero).
/// - `Some(ValSlot::Empty)` — TRAPS loudly (§0.8 g): the h-family
///   placeholder is a caller bug, never a silent nil.
///
/// The rc completion for a ref V register lives in `call_host`'s
/// any write-back (retain first, then release the displaced — the order
/// is load-bearing when the answer IS the displaced cell). The read
/// direction does not exist: `TY_VAL` is host-decl-only, no rut export
/// can return it, so `from_slot` is unreachable through a real program.
impl Ret for Option<ValSlot> {
    const TY: TypeId = TY_VAL;
    fn rust_name() -> &'static str { "Option<ValSlot>" }
    fn from_slot(_vm: &Vm, _slot: Slot, _declared: TypeId) -> Result<Self, Trap> {
        Err(Trap::new(
            TrapKind::Invalid,
            "`any` is host-decl-only — embedder call results decode as `Value`, never as `ValSlot` (nmap-hostvals P3)",
        ))
    }
    #[inline] // hot lane: the answer write's slot arms
    fn into_slot(self, vm: &mut Vm) -> Result<Slot, Trap> {
        // the tagged carry (nmap-hostvals P5): call_host's ?prim write-back
        // needs the Some/None tag the returned word cannot carry (a stored
        // zero and the miss are the same 8 bytes) — stash the un-erased
        // answer; the write-back takes it one crossing later (the
        // `host_trap` channel's discipline). A PLAIN word answer is
        // unaffected: the stash is read only on the ?prim arm.
        // (`Option<ValSlot>` is Copy; `Empty` still traps below.)
        vm.host_val_out = self;
        match self {
            None => Ok(Slot::int(0)), // the miss: the flat nil (RFC 0044's zero)
            Some(ValSlot::Bits(s)) => Ok(s), // the 8 bytes move as-is
            Some(ValSlot::Ref(s)) => Ok(s),  // the word moves; rc completes in call_host
            Some(ValSlot::Empty) => Err(Trap::new(
                TrapKind::Invalid,
                "any-answer: `ValSlot::Empty` is the h-family placeholder, not an answer — a miss crosses as `None` (§0.8 g)",
            )),
        }
    }
}

/// A Rust → rut argument for `Vm::call` (owned values; `&str`/`&[u8]`
/// copy — the embedder's data, an explicit copy is the honest shape).
/// Nameable so embedders can write their own typed dispatch helpers.
pub trait CallArg {
    const NAME: &'static str;
    fn into_value(self) -> Value;
}

macro_rules! call_arg_int {
    ($t:ty, $name:literal) => {
        impl CallArg for $t {
            const NAME: &'static str = $name;
            fn into_value(self) -> Value { Value::I64(self as i64) }
        }
    };
}

call_arg_int!(i8, "i8");
call_arg_int!(i16, "i16");
call_arg_int!(i32, "i32");
call_arg_int!(i64, "i64");
call_arg_int!(u8, "u8");
call_arg_int!(u16, "u16");
call_arg_int!(u32, "u32");
call_arg_int!(u64, "u64");

impl CallArg for f64 {
    const NAME: &'static str = "f64";
    fn into_value(self) -> Value { Value::F64(self) }
}

impl CallArg for f32 {
    const NAME: &'static str = "f32";
    fn into_value(self) -> Value { Value::F64(self as f64) }
}

impl CallArg for bool {
    const NAME: &'static str = "bool";
    fn into_value(self) -> Value { Value::Bool(self) }
}

impl CallArg for String {
    const NAME: &'static str = "String";
    fn into_value(self) -> Value { Value::Str(self) }
}

impl CallArg for &str {
    const NAME: &'static str = "&str";
    fn into_value(self) -> Value { Value::Str(self.to_string()) }
}

impl CallArg for Vec<u8> {
    const NAME: &'static str = "Vec<u8>";
    fn into_value(self) -> Value { Value::Bytes(self) }
}

impl CallArg for &[u8] {
    const NAME: &'static str = "&[u8]";
    fn into_value(self) -> Value { Value::Bytes(self.to_vec()) }
}

impl CallArg for OpaqueRef {
    const NAME: &'static str = "OpaqueRef";
    fn into_value(self) -> Value { Value::Opaque(self) }
}

impl<T: 'static> CallArg for Opaque<T> {
    const NAME: &'static str = "Opaque";
    fn into_value(self) -> Value { Value::Opaque(self.handle().clone()) }
}

/// the argument bundle of a typed `call` — tuples up to arity 8
pub trait CallArgs {
    fn into_values(self) -> Vec<Value>;
}

impl CallArgs for () {
    fn into_values(self) -> Vec<Value> { Vec::new() }
}

macro_rules! call_args {
    ($($n:ident),+) => {
        impl<$($n: CallArg),+> CallArgs for ($($n,)+) {
            fn into_values(self) -> Vec<Value> {
                #[allow(non_snake_case)]
                let ($($n,)+) = self;
                vec![$($n.into_value()),+]
            }
        }
    };
}

call_args!(A1);
call_args!(A1, A2);
call_args!(A1, A2, A3);
call_args!(A1, A2, A3, A4);
call_args!(A1, A2, A3, A4, A5);
call_args!(A1, A2, A3, A4, A5, A6);
call_args!(A1, A2, A3, A4, A5, A6, A7);
call_args!(A1, A2, A3, A4, A5, A6, A7, A8);

// ---- the magic handler (RFC 0023 revised §3): the fn's Rust shape IS
// the .d.rut row — params/ret derive the signature, the adapter IS the
// dispatch-table entry ----

/// A host-fn parameter: the Rust type knows its rut type and how to read
/// itself out of a slot. `Repr<'a>` is what the registered callable's
/// parameter IS — the type itself for Copy values and handles, `&'a str`
/// for borrows — and the handler bound is HRTB over `'a`, so a borrowed
/// param cannot outlive its call: the type system blocks smuggling
/// (RFC 0023 §2).
///
/// The read is SPLIT (the crossing-fastpath plan, phase 1):
///
/// - [`HostParam::read`] is the **unchecked fast lane** — raw slot-bit
///   reads for prims, no `expect_kind`, no `narrow_i64`, no string
///   compare. It is what the host-fn entry adapters
///   ([`handler_fallible!`]/[`handler_infallible!`], the `HostSlot` code
///   pointers — the rut→host hot lane) call, and its only callers.
/// - [`HostParam::read_checked`] is the historical **checked read** —
///   `expect_kind` (the declared type's kind lookup + string-compare
///   match) plus the `narrow_i64` range check, then the same read. It is
///   off the hot path; it stays for embedder-path / debug verification
///   where the caller cannot claim the guarantees below.
///
/// SAFETY (`read`): calling it with a slot that does not hold the
/// declared repr is undefined (a prim read reinterprets the bits). The
/// adapters may skip the re-verification because the shape was CHECKED
/// ONCE, upstream of every call:
///
/// - the join verified the binding's `TY` against the mounted `.d.rut`
///   row before the Vm boots (`HostRegistry::verify_against`, RFC 0025 —
///   a panic, so a host fn can only be dispatched under the row its own
///   Rust shape derived);
/// - the checker typed every rut call site against that same row, so
///   each argument slot holds the declared repr — the same trust
///   `Slot::as_f64` runs on (RFC 0015 §5, "the verifier guarantees
///   registers hold their declared types");
/// - embedder-shaped input never reaches the slots unchecked: `Vm::call`
///   runs `value_in` (kind + range) before the slots exist.
///
/// What `read` KEEPS guards genuinely dynamic facts only (plan §0.3):
/// the nil check on ref params (the transitive `??T` coercion funnel
/// leaves "nil cannot reach here" unproven), the cell-kind match on
/// borrows (that IS the read), and the owned `String`/`Vec<u8>` copy.
pub(crate) trait HostParam {
    const TY: TypeId;
    type Repr<'a>;
    /// SAFETY (per impl): the returned value is valid for the whole
    /// host-call scope — the snapshot slots are copies of the call's arg
    /// registers (which own their references), the arena never moves
    /// cells (RFC 0016 OQ-1), and the borrowable crossing types are
    /// immutable. Nothing else may outlive the call. See the trait doc
    /// for the checked-once contract that lets the adapters skip
    /// re-verification.
    unsafe fn read<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap>;
    /// The checked twin: the pre-phase-1 read, semantics and trap
    /// messages preserved (`expect_kind` + `narrow_i64` + read). Not on
    /// the rut→host hot path — the adapters read through [`HostParam::read`].
    /// Deliberately kept OFF the hot path (the fast_lane_tests exercise
    /// it as the debug verifier); embedder marshalling is `Ret::from_slot`.
    #[allow(dead_code)] // the debug/embedder-verification twin — see doc
    unsafe fn read_checked<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap>;

    /// The any-lane hook (nmap-hostvals P3): the adapter hands the CALL
    /// SITE's static type alongside the arg slot — the same `FuncDef.regs`
    /// word the verifier checks reads against (RFC 0015 §5), so the any
    /// param can decode the untagged slot without a tag of its own. The
    /// default IS the historical read: every typed param ignores the site
    /// type (the join already proved the row), and only the any shape
    /// ([`HostVal`]) overrides it. `TY_ANY` means the site type was not
    /// snapshotted (an any binding's `call_host` always snapshots, so
    /// reaching here with it is an internal error) — a typed param still
    /// ignores it, per the trust law.
    unsafe fn read_at_site<'a>(vm: &Vm, slot: Slot, site: TypeId) -> Result<Self::Repr<'a>, Trap> {
        let _ = site;
        Self::read(vm, slot)
    }
}

macro_rules! param_prim {
    ($t:ty, $name:literal, $ty:ident) => {
        impl HostParam for $t {
            const TY: TypeId = $ty;
            type Repr<'a> = $t;
            // FAST LANE: the slot's bits ARE the value. `expect_kind` was
            // a tautology here (boot type $ty always has kind Prim($ty) —
            // the join proved the row matches at boot) and `narrow_i64`
            // re-proved what the checker already proved at the call site;
            // `as` reinterprets the raw word, it never ranges-traps.
            #[inline] // crossing-fastpath phase 2: hot read/into_slot impls
            unsafe fn read<'a>(_vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
                Ok(unsafe { slot.i } as $t)
            }
            unsafe fn read_checked<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
                expect_kind(vm, slot, $ty, $name)?;
                Ok(narrow_i64::<$t>(unsafe { slot.i }, $name)?)
            }
        }
    };
}

param_prim!(i8, "i8", TY_I8);
param_prim!(i16, "i16", TY_I16);
param_prim!(i32, "i32", TY_I32);
param_prim!(i64, "i64", TY_I64);
param_prim!(u8, "u8", TY_U8);
param_prim!(u16, "u16", TY_U16);
param_prim!(u32, "u32", TY_U32);

/// raw-bit read — a rut `u64` lives in the slot as its own bit pattern
/// (the [`Ret`] twin below), so no i64 narrowing applies: `2^63` and up
/// arrive with the sign bit set in `slot.i` and keep their bits.
impl HostParam for u64 {
    const TY: TypeId = TY_U64;
    type Repr<'a> = u64;
    #[inline] // hot lane
    unsafe fn read<'a>(_vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        Ok(unsafe { slot.i } as u64)
    }
    unsafe fn read_checked<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        expect_kind(vm, slot, TY_U64, "u64")?;
        Ok(unsafe { slot.i } as u64)
    }
}

impl HostParam for f64 {
    const TY: TypeId = TY_F64;
    type Repr<'a> = f64;
    #[inline] // hot lane
    unsafe fn read<'a>(_vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        Ok(slot.as_f64()) // the raw union read (RFC 0015 §5 trust)
    }
    unsafe fn read_checked<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        expect_kind(vm, slot, TY_F64, "f64")?;
        Ok(slot.as_f64())
    }
}

impl HostParam for f32 {
    const TY: TypeId = TY_F32;
    type Repr<'a> = f32;
    #[inline] // hot lane
    unsafe fn read<'a>(_vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        Ok(slot.as_f64() as f32)
    }
    unsafe fn read_checked<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        expect_kind(vm, slot, TY_F32, "f32")?;
        Ok(slot.as_f64() as f32)
    }
}

impl HostParam for bool {
    const TY: TypeId = TY_BOOL;
    type Repr<'a> = bool;
    #[inline] // hot lane
    unsafe fn read<'a>(_vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        Ok(slot.as_bool()) // the slot's 0/1 word
    }
    unsafe fn read_checked<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        expect_kind(vm, slot, TY_BOOL, "bool")?;
        Ok(slot.as_bool())
    }
}

impl HostParam for OpaqueRef {
    const TY: TypeId = TY_OPAQUE;
    type Repr<'a> = OpaqueRef;
    // FAST LANE + the one genuinely dynamic fact (plan §0.3): the nil
    // check STAYS — the transitive `??T` coercion funnel leaves "nil
    // cannot reach here" unproven. `expect_kind` (the Opaque-kind
    // lookup) is gone: the join pinned TY_OPAQUE to the row at boot.
    #[inline] // hot lane
    unsafe fn read<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        let p = unsafe { slot.r };
        if p.is_null() {
            return Err(Trap::new(TrapKind::NilDeref, "boundary: nil does not bind `OpaqueRef`"));
        }
        Ok(vm.heap.opaque_handle(p)) // the bump is the handle's own count
    }
    unsafe fn read_checked<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        expect_kind(vm, slot, TY_OPAQUE, "OpaqueRef")?;
        Self::read(vm, slot)
    }
}

/// the typed payload view as a parameter — the payload type token is
/// checked on the way in (`from_handle`), a wrong `T` is a trap
impl<T: 'static> HostParam for Opaque<T> {
    const TY: TypeId = TY_OPAQUE;
    type Repr<'a> = Opaque<T>;
    // FAST LANE + nil check (as `OpaqueRef`); `from_handle`'s payload
    // type token stays — it guards the Rust-side `T`, which no .d.rut
    // row can speak for.
    #[inline] // hot lane
    unsafe fn read<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        let p = unsafe { slot.r };
        if p.is_null() {
            return Err(Trap::new(TrapKind::NilDeref, "boundary: nil does not bind `Opaque`"));
        }
        Opaque::from_handle(&vm.heap.opaque_handle(p))
    }
    unsafe fn read_checked<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        expect_kind(vm, slot, TY_OPAQUE, "Opaque")?;
        Self::read(vm, slot)
    }
}

/// zero-copy borrow: the octets read straight out of the block store —
/// the `'a` the closure receives is the call's scope (trait doc)
impl HostParam for &str {
    const TY: TypeId = TY_STR;
    type Repr<'a> = &'a str;
    // FAST LANE unchanged by phase 1: this read never had `expect_kind`
    // — the `as_str` cell-kind match IS the read (the dynamic fact, plan
    // §0.3), and a wrong-shaped cell degrades to the empty view, never UB.
    #[inline] // hot lane
    unsafe fn read<'a>(_vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        let s = cell_of(slot).as_str();
        // SAFETY: arg-register retention + non-moving arena + immutability
        Ok(unsafe { std::mem::transmute::<&str, &'a str>(s) })
    }
    unsafe fn read_checked<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        Self::read(vm, slot) // the historical read: the cell-kind match, nothing more
    }
}

/// zero-copy borrow (RFC 0004 bytes)
impl HostParam for &[u8] {
    const TY: TypeId = TY_BYTES;
    type Repr<'a> = &'a [u8];
    // FAST LANE unchanged by phase 1 — as `&str`: the `bytes_view`
    // cell-kind match IS the read.
    #[inline] // hot lane
    unsafe fn read<'a>(_vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        let b = bytes_view(slot);
        // SAFETY: as `&str` — the bytes block outlives the call
        Ok(unsafe { std::mem::transmute::<&[u8], &'a [u8]>(b) })
    }
    unsafe fn read_checked<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        Self::read(vm, slot)
    }
}

/// owned copy — the explicit "I keep this data" shape
impl HostParam for String {
    const TY: TypeId = TY_STR;
    type Repr<'a> = String;
    // FAST LANE: the copy IS the read (plan §0.3 keeps the copy); the
    // `expect_kind` string-compare in front of it is gone — the join +
    // checker guarantee the cell, and `as_str` is kind-matched anyway.
    #[inline] // hot lane
    unsafe fn read<'a>(_vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        Ok(cell_of(slot).as_str().to_string())
    }
    unsafe fn read_checked<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        expect_kind(vm, slot, TY_STR, "String")?;
        Self::read(vm, slot)
    }
}

/// owned copy — the explicit "I keep this data" shape
impl HostParam for Vec<u8> {
    const TY: TypeId = TY_BYTES;
    type Repr<'a> = Vec<u8>;
    // FAST LANE: as `String` — the copy stays, the kind re-check leaves.
    #[inline] // hot lane
    unsafe fn read<'a>(_vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        Ok(cell_of(slot).bytes_copy())
    }
    unsafe fn read_checked<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        expect_kind(vm, slot, TY_BYTES, "Vec<u8>")?;
        Self::read(vm, slot)
    }
}

/// The any-arg (nmap-hostvals P3): the param shape for a `.d.rut` host fn
/// spelling `v: any` — the boundary hands the registered closure the arg's
/// raw slot TOGETHER WITH the CALL SITE's static type, undecoded. Rut
/// types live in the VM, never in Rust (§0.8 m): `ty` is the call site's
/// own `FuncDef.regs` word — the same static type the verifier checks
/// reads against (RFC 0015 §5) — and its [`TyKind`] reads through the
/// program's own table, never cloned into the handle:
///
/// ```ignore
/// // the ARG — the plan's decode, verbatim shape (the copy/alloc laws):
/// let vs = match vm.prog.types.kind(hv.ty) {
///     rut_core::types::TyKind::Prim(_) => ValSlot::Bits(hv.slot), // the 8 bytes move as-is
///     _ => { vm.heap.retain(hv.slot); ValSlot::Ref(hv.slot) }     // ref kinds: the arg's OWN cell
/// };                                                              // (a str VIEW arg stores the view)
/// ```
///
/// `Value::Str`'s owned decode is NEVER used: a ref arg crosses as its
/// origin slot (identity IS the cell — a view arg crosses the view's own
/// cell), a prim arg as its immediate bits. The slot is a BORROW of the
/// caller's arg register (the register owns its reference); a host that
/// stores the value takes its own retain — exactly the law above.
///
/// Host-decl-only by construction: [`HostParam::TY`] is `TY_VAL`, a boot
/// id rut source cannot name, so this param shape can only bind a `.d.rut`
/// row spelled `any`. There is no `CallArg` twin — the embedder `call`
/// direction has no any lane (a host-decl fn is never an export).
pub struct HostVal {
    /// the arg slot, untagged 8 bytes (RFC 0015 §5): prim sites carry the
    /// value's immediate bits, ref sites the cell handle
    pub slot: Slot,
    /// the CALL SITE's static type — decode through `vm.prog.types`
    pub ty: TypeId,
}

impl HostVal {
    /// The site type's descriptor — the read that drives the plan's
    /// `Prim(_) => Bits, _ => Ref` decode. A shared table read, never a
    /// clone (the copy/alloc laws).
    pub fn kind<'a>(&self, types: &'a rut_core::types::TypeTable) -> &'a TyKind {
        types.kind(self.ty)
    }
}

impl HostParam for HostVal {
    const TY: TypeId = TY_VAL;
    type Repr<'a> = HostVal;
    // the any lane NEVER reads without the site type — the adapter hands
    // it through `read_at_site`; a direct `read` has no site to consult
    unsafe fn read<'a>(_vm: &Vm, _slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        Err(Trap::new(
            TrapKind::Invalid,
            "internal: `any` param read without the call site's static type",
        ))
    }
    unsafe fn read_checked<'a>(vm: &Vm, slot: Slot) -> Result<Self::Repr<'a>, Trap> {
        Self::read(vm, slot)
    }
    // the override: capture (slot, site) verbatim — no decode, no box, no
    // copy; the closure decides from the site type (the plan's ARG law)
    #[inline]
    unsafe fn read_at_site<'a>(vm: &Vm, slot: Slot, site: TypeId) -> Result<Self::Repr<'a>, Trap> {
        let _ = vm;
        if site == super::TY_ANY {
            return Err(Trap::new(
                TrapKind::Invalid,
                "internal: untyped `any` argument (the call site's static type was not snapshotted)",
            ));
        }
        Ok(HostVal { slot, ty: site })
    }
}

/// The body-return kind marker, solved by which impl the callable's
/// return type matches — never written by the embedder (only named by
/// the turbofish's `_`).
pub enum Infallible {}
pub enum Fallible {}

/// The magic: one blanket family (macro, arities 0..=8, both body
/// shapes) whose `entry` adapter IS the `HostSlot` code pointer. The
/// table stays uniform — `call_host` never learns what shape produced
/// the entry (plan §3.3/§3.6).
pub(crate) trait HostHandler<A, R, K>: 'static {
    /// the derived `.d.rut` row — a fixed array, no heap
    const SIG: crate::interp::host::HostSig;
    /// `tys` is the call's arg-register static types (the caller's
    /// `FuncDef.regs` words, snapshot beside the arg slots — empty when
    /// the binding has no `any` params, the join's bit): the any lane's
    /// `read_at_site` consumes it positionally; typed params never look.
    fn entry(vm: &mut Vm, slots: &[Slot], tys: &[TypeId], ctx: crate::interp::host::Ctx) -> Slot;
}

macro_rules! handler_fallible {
    ($($n:ident),*) => {
        impl<F, $($n: HostParam,)* R: Ret> HostHandler<($($n,)*), R, Fallible> for F
        where
            F: for<'a> FnMut(&mut Vm, $($n::Repr<'a>,)*) -> Result<R, Trap> + 'static,
        {
            const SIG: crate::interp::host::HostSig =
                crate::interp::host::HostSig::new(&[$($n::TY,)*], R::TY);
            fn entry(vm: &mut Vm, slots: &[Slot], tys: &[TypeId], ctx: crate::interp::host::Ctx) -> Slot {
                // SAFETY: ctx is Box<F>, owned by vm.host_keep for the
                // machine's lifetime; single thread (RFC 0034)
                let f = unsafe { &mut *(ctx as *mut F) };
                let mut run = || -> Result<R, Trap> {
                    let mut i = 0usize;
                    // params through the unchecked fast lane (`read`):
                    // the join verified the sig (RFC 0025), the checker
                    // typed the site — no per-call re-verification. The
                    // site type rides along (the any lane's decode);
                    // typed params ignore it (the default `read_at_site`)
                    $( let $n = unsafe { $n::read_at_site(vm, slots[i], tys.get(i).copied().unwrap_or(super::TY_ANY)) }?; i += 1; )*
                    f(vm, $($n,)*)
                };
                match run() {
                    Ok(out) => match out.into_slot(vm) {
                        Ok(s) => s,
                        Err(t) => vm.trap_taken(t),
                    },
                    Err(t) => vm.trap_taken(t),
                }
            }
        }
    };
}

macro_rules! handler_infallible {
    ($($n:ident),*) => {
        impl<F, $($n: HostParam,)* R: Ret> HostHandler<($($n,)*), R, Infallible> for F
        where
            F: for<'a> FnMut(&mut Vm, $($n::Repr<'a>,)*) -> R + 'static,
        {
            const SIG: crate::interp::host::HostSig =
                crate::interp::host::HostSig::new(&[$($n::TY,)*], R::TY);
            fn entry(vm: &mut Vm, slots: &[Slot], tys: &[TypeId], ctx: crate::interp::host::Ctx) -> Slot {
                // SAFETY: as the fallible impl — Box<F> via host_keep
                let f = unsafe { &mut *(ctx as *mut F) };
                let out = {
                    let mut i = 0usize;
                    // params through the unchecked fast lane (`read`) —
                    // as the fallible impl above
                    $( let $n = match unsafe { $n::read_at_site(vm, slots[i], tys.get(i).copied().unwrap_or(super::TY_ANY)) } {
                        Ok(a) => a,
                        Err(t) => return vm.trap_taken(t),
                    }; i += 1; )*
                    f(vm, $($n,)*)
                };
                match out.into_slot(vm) {
                    Ok(s) => s,
                    Err(t) => vm.trap_taken(t),
                }
            }
        }
    };
}

handler_fallible!();
handler_fallible!(A1);
handler_fallible!(A1, A2);
handler_fallible!(A1, A2, A3);
handler_fallible!(A1, A2, A3, A4);
handler_fallible!(A1, A2, A3, A4, A5);
handler_fallible!(A1, A2, A3, A4, A5, A6);
handler_fallible!(A1, A2, A3, A4, A5, A6, A7);
handler_fallible!(A1, A2, A3, A4, A5, A6, A7, A8);

handler_infallible!();
handler_infallible!(A1);
handler_infallible!(A1, A2);
handler_infallible!(A1, A2, A3);
handler_infallible!(A1, A2, A3, A4);
handler_infallible!(A1, A2, A3, A4, A5);
handler_infallible!(A1, A2, A3, A4, A5, A6);
handler_infallible!(A1, A2, A3, A4, A5, A6, A7);
handler_infallible!(A1, A2, A3, A4, A5, A6, A7, A8);

/// The registration sugar: writes the name and the callable, nothing
/// else — the marker tuple comes from the closure's own param types.
/// The plain `register` method infers single-param closures on its own;
/// from two params on, the compiler needs the marker spelled, and this
/// macro spells it from the shape you already wrote:
///
/// ```ignore
/// rut_vm::register!(hosts, "calc::hypot", |vm: &mut Vm, a: f64, b: f64| -> Result<f64, Trap> {
///     Ok(a.hypot(b))
/// });
/// ```
/// The registration sugar for shapes the compiler cannot infer on its
/// own (two params and up): spell the marker tuple once and the closure
/// stays plain — no turbofish, no param annotations, the expected
/// signature does the typing.
///
/// ```ignore
/// rut_vm::register!(hosts, "calc::hypot", (f64, f64) -> f64, |vm, a, b| a.hypot(b));
/// rut_vm::register!(hosts, "re::boost", (i64,) -> i64, |vm, x| {
///     let y: i64 = vm.call("inner", (x,))?;
///     Ok(y + 1)
/// });
/// ```
#[macro_export]
macro_rules! register {
    ($hosts:expr, $name:literal, ($($t:ty),* $(,)?) -> $ret:ty, $closure:expr $(,)?) => {
        $hosts.register::<_, ($($t,)*), $ret, _>($name, $closure)
    };
    ($hosts:expr, $name:literal, ($($t:ty),* $(,)?), $closure:expr $(,)?) => {
        $hosts.register::<_, ($($t,)*), _, _>($name, $closure)
    };
}

// ---- the fast lane's own tests (the crossing-fastpath plan, phase 1):
// the adapters' `read` drops the static re-verification, so these pin
// what REMAINS — nil ref traps, the borrows' cell-kind match, the owned
// copies, the prim bit-fidelity — and the fast/checked split itself ----

#[cfg(test)]
mod fast_lane_tests {
    use super::*;
    use std::rc::Rc;

    /// A bare Vm: the fast `read` never touches the program tables (that
    /// is the point), so an empty program serves every test here. The
    /// boot types table is installed anyway — the CHECKED twin resolves
    /// `expect_kind` through it, which is exactly the work the fast lane
    /// skips.
    fn bare_vm() -> Vm {
        let mut prog = rut_core::binary::Program::default();
        prog.types = rut_core::types::TypeTable::boot();
        Vm::new(Rc::new(prog), &Limits::default(), HostHooks::default(), HostRegistry::new())
            .expect("bare vm")
    }

    /// Drive the adapter itself (the HostSlot code pointer shape): box
    /// the body, call its `entry` with the given snapshot. `F` solves
    /// the fallibility marker, so both adapter families get exercised.
    /// (No `any` params in these tests — the empty site-type slice.)
    fn entry_of<F, P, R, K>(vm: &mut Vm, f: F, slots: &[Slot]) -> Slot
    where
        F: HostHandler<(P,), R, K>,
    {
        let boxed = Box::new(f);
        let ctx = &*boxed as *const F as crate::interp::host::Ctx;
        std::mem::forget(boxed); // test-scoped leak: ctx must stay live
        F::entry(vm, slots, &[], ctx)
    }

    #[test]
    fn prim_params_round_trip_their_bits_through_the_fast_lane() {
        let vm = bare_vm();
        // every int width, at its extremes — `as` reinterpret, no narrow
        unsafe {
            assert_eq!(<i8 as HostParam>::read(&vm, Slot::int(i8::MIN as i64)).unwrap(), i8::MIN);
            assert_eq!(<u8 as HostParam>::read(&vm, Slot::int(255)).unwrap(), 255u8);
            assert_eq!(<u32 as HostParam>::read(&vm, Slot::int(u32::MAX as i64)).unwrap(), u32::MAX);
            // u64: the raw-bit read — 2^63..2^64-1 keep their bits
            for bits in [1i64 << 63, (1i64 << 63) + 12345, -2, -1] {
                assert_eq!(<u64 as HostParam>::read(&vm, Slot::int(bits)).unwrap(), bits as u64);
            }
            assert_eq!(<u64 as HostParam>::read(&vm, Slot::int(-1)).unwrap(), u64::MAX);
            // floats/bool: the raw union reads
            assert_eq!(<f64 as HostParam>::read(&vm, Slot::float(-0.5)).unwrap(), -0.5);
            assert_eq!(<bool as HostParam>::read(&vm, Slot::bool(true)).unwrap(), true);
        }
    }

    #[test]
    fn the_checked_twin_still_range_checks_where_the_fast_lane_reinterprets() {
        let vm = bare_vm();
        // 300 does not fit i8: the fast lane reinterprets (the join +
        // checker make that sound on the hot path), the checked twin
        // traps — the split, demonstrated on one slot
        let s = Slot::int(300);
        unsafe {
            assert_eq!(<i8 as HostParam>::read(&vm, s).unwrap(), 44i8);
            let err = <i8 as HostParam>::read_checked(&vm, s).unwrap_err();
            assert!(err.msg.contains("does not fit"), "{}", err.msg);
            // the checked twin preserves the historical semantics where
            // they differed from nothing: u64 never narrows
            assert_eq!(<u64 as HostParam>::read_checked(&vm, Slot::int(-1)).unwrap(), u64::MAX);
        }
    }

    /// `unwrap_err` for `Repr`s without `Debug` (the handle/box shapes)
    fn err_of<T>(r: Result<T, Trap>) -> Trap {
        match r {
            Ok(_) => panic!("expected a trap, got Ok"),
            Err(t) => t,
        }
    }

    #[test]
    fn a_nil_ref_param_still_traps() {
        let vm = bare_vm();
        unsafe {
            let err = err_of(<OpaqueRef as HostParam>::read(&vm, Slot::null()));
            assert_eq!(err.kind, TrapKind::NilDeref, "{}", err.msg);
            assert!(err.msg.contains("nil does not bind `OpaqueRef`"), "{}", err.msg);
            let err = err_of(<Opaque<u64> as HostParam>::read(&vm, Slot::null()));
            assert_eq!(err.kind, TrapKind::NilDeref, "{}", err.msg);
        }
    }

    #[test]
    fn the_nil_trap_fires_through_both_adapters() {
        // the full entry shape: fallible AND infallible bodies, one nil
        // snapshot slot — the macro plumbing reports the fast read's Err
        fn fallible_body(_vm: &mut Vm, _b: OpaqueRef) -> Result<i64, Trap> {
            Ok(1)
        }
        fn infallible_body(_vm: &mut Vm, _b: OpaqueRef) -> i64 {
            1
        }
        let mut vm = bare_vm();
        let out = entry_of::<_, OpaqueRef, i64, Fallible>(&mut vm, fallible_body, &[Slot::null()]);
        let t = vm.host_trap.take().expect("fallible adapter reports the nil");
        assert_eq!(t.kind, TrapKind::NilDeref, "{}", t.msg);
        let _ = out;
        let out = entry_of::<_, OpaqueRef, i64, Infallible>(&mut vm, infallible_body, &[Slot::null()]);
        let t = vm.host_trap.take().expect("infallible adapter reports the nil");
        assert_eq!(t.kind, TrapKind::NilDeref, "{}", t.msg);
        let _ = out;
    }

    #[test]
    fn the_adapters_cross_a_prim_through_the_fast_lane() {
        fn fallible_id(_vm: &mut Vm, x: i64) -> Result<i64, Trap> {
            Ok(x)
        }
        fn infallible_id(_vm: &mut Vm, x: i64) -> i64 {
            x
        }
        let mut vm = bare_vm();
        let out = entry_of::<_, i64, i64, Fallible>(&mut vm, fallible_id, &[Slot::int(-404)]);
        assert!(vm.host_trap.is_none());
        assert_eq!(unsafe { out.i }, -404);
        let out = entry_of::<_, i64, i64, Infallible>(&mut vm, infallible_id, &[Slot::int(i64::MIN)]);
        assert!(vm.host_trap.is_none());
        assert_eq!(unsafe { out.i }, i64::MIN);
    }

    #[test]
    fn borrow_reads_keep_their_cell_kind_match() {
        let mut vm = bare_vm();
        // &[u8] over a str cell: `bytes_view`'s Str branch — the octets,
        // not UB, not a reinterpret
        let s = vm.heap.alloc_str("hello".into()).unwrap();
        unsafe {
            let b = <&[u8] as HostParam>::read(&vm, s).unwrap();
            assert_eq!(b, b"hello");
        }
        // &str over a bytes cell: `as_str` degrades to the empty view —
        // the kind match IS the guard, exactly the pre-phase-1 behavior
        let bytes = vm.heap.alloc_bytes(vec![9u8, 8, 7]).unwrap();
        unsafe {
            let v = <&str as HostParam>::read(&vm, bytes).unwrap();
            assert_eq!(v, "");
            let b = <&[u8] as HostParam>::read(&vm, bytes).unwrap();
            assert_eq!(b, &[9u8, 8, 7]);
        }
    }

    #[test]
    fn owned_params_keep_their_copies() {
        let mut vm = bare_vm();
        let s = vm.heap.alloc_str("ada".into()).unwrap();
        let bytes = vm.heap.alloc_bytes(vec![9u8, 8, 7]).unwrap();
        unsafe {
            // the copy IS the read: str → String, bytes → Vec<u8>
            assert_eq!(<String as HostParam>::read(&vm, s).unwrap(), "ada");
            assert_eq!(<Vec<u8> as HostParam>::read(&vm, bytes).unwrap(), vec![9u8, 8, 7]);
        }
    }

    #[test]
    fn the_box_payload_token_still_traps_on_a_wrong_t() {
        let mut vm = bare_vm();
        let boxed = crate::heap::Opaque::alloc(&mut vm, 7i64).expect("alloc");
        let slot = Slot { r: boxed.handle().ptr() };
        // the fast read keeps `from_handle`'s type-token check — a wrong
        // `T` is a trap, never a reinterpret
        unsafe {
            let err = err_of(<Opaque<u64> as HostParam>::read(&vm, slot));
            assert!(err.msg.contains("not `u64`"), "{}", err.msg);
            let ok = <Opaque<i64> as HostParam>::read(&vm, slot).unwrap();
            assert_eq!(ok.with(|v| *v).unwrap(), 7i64);
        }
    }
}

// ---- the any-lane tests (nmap-hostvals P3): the arms are additive, so
// these pin the NEW surface end-to-end — the site-type capture, the
// origin-slot identity, the retain/release balance through both
// directions, the no-alloc answer path, the untagged bits move, and the
// §0.8 g Empty trap. `call_host` is driven DIRECTLY: funcs[0] is a plain
// caller frame whose argv pool and regs table the test types, funcs[1]
// is the joined host thunk — the arg snapshot, the site types, and the
// dst write-back are the subject, not the op stream. ----

#[cfg(test)]
mod any_lane_tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A VM with a plain caller frame (`funcs[0]`, argv pool `[0, 1]`,
    /// the regs table the test types) and the `t::probe` host thunk
    /// (`funcs[1]`, ret `TY_VAL`) joined to the registered body.
    fn vm_with(registry: HostRegistry, caller_regs: Vec<TypeId>) -> Vm {
        vm_with_opt(registry, caller_regs, None).0
    }

    /// The same frame, with an optional `?elem` type appended to the type
    /// table and (optionally) used as the dst register's declared type —
    /// the P5 `?V` write-back arms' driver.
    fn vm_with_opt(registry: HostRegistry, mut caller_regs: Vec<TypeId>, dst_opt: Option<TypeId>) -> (Vm, TypeId) {
        let mut prog = rut_core::binary::Program::default();
        prog.types = rut_core::types::TypeTable::boot();
        let opt_ty = prog.types.types.len() as u32;
        if let Some(elem) = dst_opt {
            prog.types.types.push(rut_core::types::RutType {
                name: rut_core::sym::NIL,
                kind: rut_core::types::TyKind::Opt { elem },
            });
            caller_regs[1] = opt_ty;
        }
        let host_key = prog.interner.intern("t::probe");
        let mut mk = |name: &str, regs: Vec<TypeId>, host: Option<rut_core::sym::IdentId>| rut_core::binary::FuncCode {
            name: prog.interner.intern(name),
            params: vec![],
            ret: TY_NIL,
            is_method: false,
            n_captures: 0,
            regs,
            argv: vec![0, 1],
            labels: vec![],
            code: vec![],
            spans: vec![],
            pos: vec![],
            host_id: host,
        };
        prog.funcs.push(mk("main", caller_regs, None));
        let mut thunk = mk("probe", vec![], Some(host_key));
        thunk.ret = TY_VAL;
        thunk.argv = vec![];
        prog.funcs.push(thunk);
        (Vm::new(Rc::new(prog), &Limits::default(), HostHooks::default(), registry).expect("vm"), opt_ty)
    }

    #[test]
    fn the_any_arg_hands_the_site_type_and_the_origin_slot() {
        let site = Rc::new(std::cell::Cell::new(0u32));
        let ptr = Rc::new(std::cell::Cell::new(0usize));
        let mut hosts = HostRegistry::new();
        let (site2, ptr2) = (site.clone(), ptr.clone());
        crate::register!(hosts, "t::probe", (HostVal,) -> Option<ValSlot>,
            move |_vm: &mut Vm, hv: HostVal| {
                site2.set(hv.ty);
                ptr2.set(unsafe { hv.slot.r as usize });
                None // the miss: the flat nil
            },
        );
        let mut vm = vm_with(hosts, vec![TY_STR, TY_STR]);
        let s = vm.heap.alloc_str("hello".into()).unwrap();
        vm.cur_func = 0;
        vm.cur_regs = vec![s, Slot::null()];
        vm.call_host(1, 0, 1, 1).expect("the crossing");
        assert_eq!(site.get(), TY_STR, "the site type is the CALLER's register type");
        assert_eq!(
            ptr.get(),
            unsafe { s.r } as usize,
            "the arg's OWN cell crosses — the origin slot, no copy"
        );
        assert_eq!(unsafe { vm.cur_regs[1].i }, 0, "the miss wrote the flat nil");
    }

    #[test]
    fn a_ref_stored_via_the_arg_arm_releases_exactly_once() {
        // the host's "map put": retain the arg's own cell into storage,
        // answer the miss — the crossing itself must not double-count
        let stored: Rc<RefCell<Option<Slot>>> = Rc::new(RefCell::new(None));
        let mut hosts = HostRegistry::new();
        let st = stored.clone();
        crate::register!(hosts, "t::probe", (HostVal,) -> Option<ValSlot>,
            move |vm: &mut Vm, hv: HostVal| {
                vm.heap.retain(hv.slot);
                *st.borrow_mut() = Some(hv.slot);
                None
            },
        );
        let mut vm = vm_with(hosts, vec![TY_STR, TY_STR]);
        let base = vm.heap_usage();
        let s = vm.heap.alloc_str("kept".into()).unwrap();
        let after_alloc = vm.heap_usage();
        vm.cur_func = 0;
        vm.cur_regs = vec![s, Slot::null()];
        vm.call_host(1, 0, 1, 1).expect("the crossing");
        assert_eq!(
            vm.heap_usage(),
            after_alloc,
            "the retain mints nothing (rc is a count, not a cell)"
        );
        // the map's remove, then the arg register's own release — the
        // balance must close exactly on the base (no leak, no double:
        // a double release panics the rc walk in debug)
        let taken = stored.borrow_mut().take().unwrap();
        vm.heap.release(taken);
        vm.heap.release(vm.cur_regs[0]);
        assert_eq!(vm.heap_usage(), base, "the balance closes on the base");
    }

    #[test]
    fn a_ref_answer_keeps_the_cell_alive_and_releases_exactly_once() {
        let stored: Rc<RefCell<Option<Slot>>> = Rc::new(RefCell::new(None));
        let mut hosts = HostRegistry::new();
        let st = stored.clone();
        crate::register!(hosts, "t::probe", (HostVal,) -> Option<ValSlot>,
            move |vm: &mut Vm, hv: HostVal| {
                // the plan's decode: ref kinds retain the arg's OWN cell
                vm.heap.retain(hv.slot);
                *st.borrow_mut() = Some(hv.slot);
                Some(ValSlot::Ref(hv.slot))
            },
        );
        let mut vm = vm_with(hosts, vec![TY_STR, TY_STR]);
        let base = vm.heap_usage();
        let s = vm.heap.alloc_str("kept".into()).unwrap();
        let old = vm.heap.alloc_str("gone!".into()).unwrap();
        let after_alloc = vm.heap_usage();
        vm.cur_func = 0;
        vm.cur_regs = vec![s, old];
        vm.call_host(1, 0, 1, 1).expect("the crossing");
        assert!(
            Slot::same_ref(vm.cur_regs[1], s),
            "identity: the answered word IS the stored cell"
        );
        assert!(
            vm.heap_usage() < after_alloc,
            "the displaced cell released with the write (the ArrGet shape)"
        );
        // the map's remove, the register's borrow, the arg register's own —
        // every reference released exactly once
        let taken = stored.borrow_mut().take().unwrap();
        vm.heap.release(taken);
        vm.heap.release(vm.cur_regs[1]);
        vm.heap.release(vm.cur_regs[0]);
        assert_eq!(vm.heap_usage(), base, "no leak, no double release");
    }

    #[test]
    fn the_answer_path_mints_no_cells() {
        // the pure borrow shape: a Ref answer into an any-typed dst —
        // no retain, no release, no box, no copy: zero cells minted
        let ptr = Rc::new(std::cell::Cell::new(0usize));
        let mut hosts = HostRegistry::new();
        let ptr2 = ptr.clone();
        crate::register!(hosts, "t::probe", (HostVal,) -> Option<ValSlot>,
            move |_vm: &mut Vm, hv: HostVal| {
                ptr2.set(unsafe { hv.slot.r } as usize);
                Some(ValSlot::Ref(hv.slot))
            },
        );
        let mut vm = vm_with(hosts, vec![TY_STR, TY_VAL]);
        let base = vm.heap_usage();
        let s = vm.heap.alloc_str("hello".into()).unwrap();
        let after_alloc = vm.heap_usage();
        vm.cur_func = 0;
        vm.cur_regs = vec![s, Slot::null()];
        vm.call_host(1, 0, 1, 1).expect("the crossing");
        assert_eq!(vm.heap_usage(), after_alloc, "zero cells minted on the answer path");
        assert_eq!(ptr.get(), unsafe { s.r } as usize, "the origin slot crossed");
        vm.heap.release(vm.cur_regs[0]);
        assert_eq!(vm.heap_usage(), base);
    }

    #[test]
    fn prim_bits_move_untagged_into_a_narrower_v_register() {
        // i64 in → i32-V-shaped register out: the SAME 8 bytes — the
        // answer write performs no width conversion (the survey's
        // ArrGet receipt, §0.8 h)
        let site = Rc::new(std::cell::Cell::new(0u32));
        let mut hosts = HostRegistry::new();
        let site2 = site.clone();
        crate::register!(hosts, "t::probe", (HostVal,) -> Option<ValSlot>,
            move |_vm: &mut Vm, hv: HostVal| {
                site2.set(hv.ty);
                Some(ValSlot::Bits(hv.slot))
            },
        );
        let mut vm = vm_with(hosts, vec![TY_I64, TY_I32]);
        let bits: i64 = 0x0123_4567_89AB_CDEF;
        vm.cur_func = 0;
        vm.cur_regs = vec![Slot::int(bits), Slot::int(0)];
        vm.call_host(1, 0, 1, 1).expect("the crossing");
        assert_eq!(site.get(), TY_I64, "the prim site's static type rode the arg");
        assert_eq!(
            unsafe { vm.cur_regs[1].i },
            bits,
            "the 8 bytes move as-is — the bytecode types them"
        );
    }

    #[test]
    fn an_empty_answer_traps_loudly() {
        // §0.8 g: the h-family placeholder is a caller bug, never a
        // silent nil — and the trap fires BEFORE the dst write
        let mut hosts = HostRegistry::new();
        crate::register!(hosts, "t::probe", (HostVal,) -> Option<ValSlot>,
            move |_vm: &mut Vm, _hv: HostVal| Some(ValSlot::Empty),
        );
        let mut vm = vm_with(hosts, vec![TY_STR, TY_STR]);
        vm.cur_func = 0;
        vm.cur_regs = vec![Slot::null(), Slot::int(777)];
        let err = vm.call_host(1, 0, 1, 1).expect_err("Empty traps");
        assert!(err.msg.contains("placeholder"), "{}", err.msg);
        assert_eq!(unsafe { vm.cur_regs[1].i }, 777, "the trap fired before the dst write");
    }

    #[test]
    fn a_view_arg_crosses_as_its_own_cell() {
        // aliasing IS the view: an array window arg crosses the VIEW's
        // cell — never the parent's, never a copy
        let ptr = Rc::new(std::cell::Cell::new(0usize));
        let mut hosts = HostRegistry::new();
        let ptr2 = ptr.clone();
        crate::register!(hosts, "t::probe", (HostVal,) -> Option<ValSlot>,
            move |_vm: &mut Vm, hv: HostVal| {
                ptr2.set(unsafe { hv.slot.r } as usize);
                Some(ValSlot::Ref(hv.slot))
            },
        );
        let mut vm = vm_with(hosts, vec![TY_I32, TY_VAL]);
        let arr = vm
            .heap
            .alloc_array_filled(TY_I32, 4, Slot::int(0), &vm.prog.types)
            .unwrap();
        let view = vm.heap.alloc_arr_view(arr, 1, 2).unwrap();
        vm.cur_func = 0;
        vm.cur_regs = vec![view, Slot::null()];
        vm.call_host(1, 0, 1, 1).expect("the crossing");
        assert_eq!(ptr.get(), unsafe { view.r } as usize, "the view's OWN cell crossed");
        assert_ne!(ptr.get(), unsafe { arr.r } as usize, "never the parent");
        // balance: the register borrowed (any-typed dst), the arg register
        // owns — its release frees view + parent
        vm.heap.release(vm.cur_regs[0]);
    }

    // ---- the ?V write-back arms (nmap-hostvals P5) ----------------------

    /// A Some(Bits) answer into a `?prim` V register MINTS the one-slot
    /// opt value (the ArrGet{OptPrim} shape): the register is a fresh
    /// cell, never the flat null — a stored ZERO must read as `some(0)`,
    /// not as the miss — and the mint balances exactly (register release
    /// closes the heap on the base).
    #[test]
    fn a_bits_answer_into_a_qprim_register_mints_the_opt_value() {
        let mut hosts = HostRegistry::new();
        crate::register!(hosts, "t::probe", (HostVal,) -> Option<ValSlot>,
            move |_vm: &mut Vm, hv: HostVal| {
                let _ = hv;
                Some(ValSlot::Bits(Slot::int(0))) // the stored ZERO
            },
        );
        let (mut vm, opt_ty) = vm_with_opt(hosts, vec![TY_I32, TY_I32], Some(TY_I32));
        let base = vm.heap_usage();
        vm.cur_func = 0;
        vm.cur_regs = vec![Slot::int(0), Slot::int(0)];
        vm.call_host(1, 0, 1, 1).expect("the crossing");
        let reg = vm.cur_regs[1];
        assert!(!unsafe { reg.r.is_null() }, "some(0) must mint, never answer the flat nil");
        // the mint is a one-slot ?i32 cell whose payload IS the zero
        assert_eq!(vm.prog.types.type_at(opt_ty).kind, rut_core::types::TyKind::Opt { elem: TY_I32 });
        // balance: the register owns the mint; releasing it closes the heap
        vm.heap.release(reg);
        assert_eq!(vm.heap_usage(), base, "the mint released exactly once");
    }

    /// The miss into a `?prim` V register is the flat null — nil means
    /// absent, and no opt cell is minted for it.
    #[test]
    fn the_miss_into_a_qprim_register_is_the_flat_null() {
        let mut hosts = HostRegistry::new();
        crate::register!(hosts, "t::probe", (HostVal,) -> Option<ValSlot>,
            move |_vm: &mut Vm, _hv: HostVal| None,
        );
        let (mut vm, _) = vm_with_opt(hosts, vec![TY_I32, TY_I32], Some(TY_I32));
        let base = vm.heap_usage();
        vm.cur_func = 0;
        vm.cur_regs = vec![Slot::int(0), Slot::int(0)];
        vm.call_host(1, 0, 1, 1).expect("the crossing");
        assert!(unsafe { vm.cur_regs[1].r.is_null() }, "the miss is the flat nil");
        assert_eq!(vm.heap_usage(), base, "no mint on the miss");
    }

    /// A Some(Ref) answer into a `?ref` V register arrives INSIDE the
    /// fresh `?T` box (the RFC 0044 T → ?T law): the box aliases the
    /// stored cell (field 0 IS the cell), the store keeps its own
    /// reference, and the counts balance — the box's release refunds its
    /// own cell reference, not the store's.
    #[test]
    fn a_ref_answer_into_a_qref_register_arrives_inside_the_opt_box() {
        let mut hosts = HostRegistry::new();
        crate::register!(hosts, "t::probe", (HostVal,) -> Option<ValSlot>,
            move |vm: &mut Vm, hv: HostVal| {
                vm.heap.retain(hv.slot); // the store's own reference
                Some(ValSlot::Ref(hv.slot))
            },
        );
        let (mut vm, _) = vm_with_opt(hosts, vec![TY_STR, TY_STR], Some(TY_STR));
        let base = vm.heap_usage();
        let s = vm.heap.alloc_str("kept".into()).unwrap();
        vm.cur_func = 0;
        vm.cur_regs = vec![s, Slot::null()];
        vm.call_host(1, 0, 1, 1).expect("the crossing");
        let reg = vm.cur_regs[1];
        assert!(!unsafe { reg.r.is_null() });
        assert!(!Slot::same_ref(reg, s), "the register holds the ?str BOX, never the bare cell");
        // the box aliases the stored cell: field 0 IS it
        let cell = crate::heap::cell_of(reg);
        if let crate::heap::CellData::Record { fields } = &cell.data {
            let f0 = fields.borrow().get(0).unwrap_or(Slot::null());
            assert!(Slot::same_ref(f0, s), "the box aliases the stored cell");
        } else {
            panic!("the ?str value is a one-slot box");
        }
        // balance: the register's box (owning its aliased-cell ref), then
        // the store's own retain (the body's), then the arg register's
        // mint ref — every count closes exactly
        vm.heap.release(reg);
        vm.heap.release(s); // the store's retained reference
        vm.heap.release(s); // the arg register's original reference
        assert_eq!(vm.heap_usage(), base, "no leak, no double release");
    }

    /// The trust law's loud edge: a `Bits` answer into a `?ref` V
    /// register is a host bug and TRAPS before the dst write.
    #[test]
    fn a_bits_answer_into_a_qref_register_traps() {
        let mut hosts = HostRegistry::new();
        crate::register!(hosts, "t::probe", (HostVal,) -> Option<ValSlot>,
            move |_vm: &mut Vm, hv: HostVal| Some(ValSlot::Bits(hv.slot)),
        );
        let (mut vm, _) = vm_with_opt(hosts, vec![TY_I32, TY_I32], Some(TY_STR));
        vm.cur_func = 0;
        vm.cur_regs = vec![Slot::int(0), Slot::int(777)];
        let err = vm.call_host(1, 0, 1, 1).expect_err("Bits into a ?ref register traps");
        assert!(err.msg.contains("trust law"), "{}", err.msg);
        assert_eq!(unsafe { vm.cur_regs[1].i }, 777, "the trap fired before the dst write");
    }
}
