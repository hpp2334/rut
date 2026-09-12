//! The tagged `Value` for the host boundary (RFC 0023) and the untagged
//! 8-byte `Slot` bytecode moves (RFC 0015 §5).
use super::*;

// ---- the tagged Value exists only at the host boundary (RFC 0023) ----

#[derive(Clone)]
pub enum Value {
    Unit,
    I64(i64),
    F64(f64),
    Bool(bool),
    Char(char),
    Str(String),
    /// `Vec<u8>` buffer crossing (RFC 0023 §2)
    Bytes(Vec<u8>),
    /// `Option<T>` — payload converted when `Some`
    Opt(Option<Box<Value>>),
    /// `Result<T, E>` — payloads converted in both arms
    Res(Result<Box<Value>, Box<Value>>),
    /// an `Opaque` box (RFC 0014) — the one cell the host may hold and
    /// pass back; the handle owns one arena reference
    Opaque(OpaqueRef),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Unit, Value::Unit) => true,
            (Value::I64(a), Value::I64(b)) => a == b,
            (Value::F64(a), Value::F64(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Char(a), Value::Char(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a == b,
            (Value::Opt(a), Value::Opt(b)) => a == b,
            (Value::Res(a), Value::Res(b)) => a == b,
            // boxes compare by identity — the payload's type is erased
            (Value::Opaque(a), Value::Opaque(b)) => a == b,
            _ => false,
        }
    }
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Unit => write!(f, "Unit"),
            Value::I64(v) => write!(f, "I64({v})"),
            Value::F64(v) => write!(f, "F64({v})"),
            Value::Bool(v) => write!(f, "Bool({v})"),
            Value::Char(v) => write!(f, "Char({v:?})"),
            Value::Str(v) => write!(f, "Str({v:?})"),
            Value::Bytes(v) => write!(f, "Bytes(len {})", v.len()),
            Value::Opt(None) => write!(f, "Opt(None)"),
            Value::Opt(Some(v)) => write!(f, "Opt(Some({v:?}))"),
            Value::Res(Ok(v)) => write!(f, "Res(Ok({v:?}))"),
            Value::Res(Err(v)) => write!(f, "Res(Err({v:?}))"),
            Value::Opaque(_) => write!(f, "Opaque(<cell>)"),
        }
    }
}

// ---- slots (RFC 0015 §5): untagged 8 bytes; bytecode is typed ----

#[derive(Clone, Copy)]
pub union Slot {
    pub i: i64,
    pub f: f64,
    pub b: bool,
    pub c: char,
    pub r: Option<*const CellVal>,
}

impl Slot {
    pub fn int(v: i64) -> Slot {
        Slot { i: v }
    }
    pub fn float(v: f64) -> Slot {
        Slot { f: v }
    }
    /// NOTE: every constructor writes the FULL 8 bytes — unions leave
    /// stale bytes otherwise, and ops must be able to read `.i` from any
    /// slot (RFC 0015 §5 untagged discipline).
    pub fn bool(v: bool) -> Slot {
        Slot { i: v as i64 }
    }
    pub fn ch(v: char) -> Slot {
        Slot { i: v as u32 as i64 }
    }
    pub fn as_bool(&self) -> bool {
        unsafe { self.i != 0 }
    }
    pub fn as_char(&self) -> char {
        char::from_u32(unsafe { self.i } as u32).unwrap_or('\0')
    }
    pub fn null() -> Slot {
        Slot { r: None }
    }
    /// SAFETY: caller guarantees the slot is a ref slot (verifier-checked).
    pub unsafe fn get_ref(&self) -> Option<*const CellVal> {
        unsafe { self.r }
    }
    pub fn same_ref(a: Slot, b: Slot) -> bool {
        unsafe { a.r == b.r } // cell identity (RFC 0012 §4)
    }
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "slot({:#x})", unsafe { self.i as u64 })
    }
}
