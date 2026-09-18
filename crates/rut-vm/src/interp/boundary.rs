//! The typed host boundary (RFC 0023, revised): Rust types are the
//! currency of `Vm::call`/`Vm::resume` and (Phase 2) of host-fn bodies;
//! `Value`/`Slot` are the internal marshaling formats, invisible outside
//! the crate. Owned impls (`String`, `Vec<u8>`) are the explicit "I keep
//! this data" copy; `&str`/`&[u8]` host params (Phase 2) borrow the block
//! store zero-copy. No `Option`/`Result` impls — optionals cross as v1.1
//! tuples (RFC 0007 §7).

use rut_core::types::{PrimTy, TypeId, TyKind};
use rut_core::types::{
    TY_BOOL, TY_CHAR, TY_F32, TY_F64, TY_I16, TY_I32, TY_I64, TY_I8, TY_NIL, TY_OPAQUE,
    TY_U16, TY_U32, TY_U8,
};

use super::*;
use crate::arena::OpaqueRef;
use crate::heap::OpaqueBox;

/// A rut → Rust conversion: read a slot under its declared type.
/// `declared` is program-relative (the export's/field's own id), so the
/// check catches an embedder passing the wrong Rust shape — a trap naming
/// both sides, never UB.
pub(crate) trait Ret: Sized {
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
        TyKind::Prim(PrimTy::Char) if rust == "char" => Ok(()),
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

impl Ret for f64 {
    const TY: TypeId = TY_F64;
    fn rust_name() -> &'static str { "f64" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "f64")?;
        Ok(slot.as_f64())
    }
}

impl Ret for f32 {
    const TY: TypeId = TY_F32;
    fn rust_name() -> &'static str { "f32" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "f32")?;
        Ok(slot.as_f64() as f32)
    }
}

impl Ret for bool {
    const TY: TypeId = TY_BOOL;
    fn rust_name() -> &'static str { "bool" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "bool")?;
        Ok(slot.as_bool())
    }
}

impl Ret for char {
    const TY: TypeId = TY_CHAR;
    fn rust_name() -> &'static str { "char" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "char")?;
        Ok(slot.as_char())
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
}

/// owned copy — the explicit "I keep this data" (RFC 0023 §2)
impl Ret for String {
    fn rust_name() -> &'static str { "String" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "String")?;
        Ok(cell_of(slot).as_str().to_string())
    }
}

/// owned copy — the explicit "I keep this data" (RFC 0023 §2)
impl Ret for Vec<u8> {
    fn rust_name() -> &'static str { "Vec<u8>" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "Vec<u8>")?;
        Ok(cell_of(slot).bytes_copy())
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
    fn into_slot(self, vm: &mut Vm) -> Result<Slot, Trap> {
        let _ = vm;
        // TRANSFER: the handle's reference count becomes the slot's —
        // `mem::forget` skips the Drop release (crossing-ownership law)
        let p = self.ptr();
        std::mem::forget(self);
        Ok(Slot { r: p })
    }
}

impl<T: 'static> Ret for OpaqueBox<T> {
    fn rust_name() -> &'static str { "OpaqueBox" }
    fn from_slot(vm: &Vm, slot: Slot, declared: TypeId) -> Result<Self, Trap> {
        expect_kind(vm, slot, declared, "OpaqueBox")?;
        let p = unsafe { slot.r };
        if p.is_null() {
            return Err(Trap::new(TrapKind::NilDeref, "boundary: nil does not bind `OpaqueBox`"));
        }
        OpaqueBox::from_handle(&vm.heap.opaque_handle(p))
    }
    fn into_slot(self, vm: &mut Vm) -> Result<Slot, Trap> {
        self.handle().clone().into_slot(vm)
    }
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

/// A Rust → rut argument for `Vm::call_typed` (owned values; `&str`/`&[u8]`
/// copy — the embedder's data, an explicit copy is the honest shape).
pub(crate) trait CallArg {
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

impl CallArg for char {
    const NAME: &'static str = "char";
    fn into_value(self) -> Value { Value::Char(self) }
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

impl<T: 'static> CallArg for OpaqueBox<T> {
    const NAME: &'static str = "OpaqueBox";
    fn into_value(self) -> Value { Value::Opaque(self.handle().clone()) }
}

/// the argument bundle of a typed `call` — tuples up to arity 8
pub(crate) trait CallArgs {
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
