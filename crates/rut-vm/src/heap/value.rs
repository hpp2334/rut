//! The tagged `Value` for the host boundary (RFC 0023) and the untagged
//! 8-byte `Slot` bytecode moves (RFC 0015 §5).
use super::*;

// ---- the tagged Value exists only at the host boundary (RFC 0023) ----

#[derive(Clone)]
pub enum Value {
    Nil,
    I64(i64),
    F64(f64),
    Bool(bool),
    Str(String),
    /// `Vec<u8>` buffer crossing (RFC 0023 §2)
    Bytes(Vec<u8>),
    /// an `Opaque` box (RFC 0014) — the one cell the host may hold and
    /// pass back; the handle owns one arena reference
    Opaque(OpaqueRef),
    /// a tuple crossing (RFC 0007 v1.1) — records with numeric fields
    Tuple(Vec<Value>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::I64(a), Value::I64(b)) => a == b,
            (Value::F64(a), Value::F64(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Bytes(a), Value::Bytes(b)) => a == b,
            // boxes compare by identity — the payload's type is erased
            (Value::Opaque(a), Value::Opaque(b)) => a == b,
            (Value::Tuple(a), Value::Tuple(b)) => a == b,
            _ => false,
        }
    }
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Nil => write!(f, "Nil"),
            Value::I64(v) => write!(f, "I64({v})"),
            Value::F64(v) => write!(f, "F64({v})"),
            Value::Bool(v) => write!(f, "Bool({v})"),
            Value::Str(v) => write!(f, "Str({v:?})"),
            Value::Bytes(v) => write!(f, "Bytes(len {})", v.len()),
            Value::Opaque(_) => write!(f, "Opaque(<cell>)"),
            Value::Tuple(v) => write!(f, "Tuple({v:?})"),
        }
    }
}

impl Value {
    /// Boundary diagnostics: what a `Value` is, in words (`an opaque`, ...).
    pub fn kind_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::I64(_) => "an integer",
            Value::F64(_) => "a float",
            Value::Bool(_) => "a bool",
            Value::Str(_) => "a string",
            Value::Bytes(_) => "bytes",
            Value::Opaque(_) => "an opaque",
            Value::Tuple(_) => "a tuple",
        }
    }
}

// ---- slots (RFC 0015 §5): untagged 8 bytes; bytecode is typed ----

#[derive(Clone, Copy)]
pub union Slot {
    pub i: i64,
    pub f: f64,
    pub b: bool,
    /// cell handle; null = "no cell". A raw pointer (not `Option<*const _>`)
    /// keeps `Slot` 8 bytes — `Option<*const T>` has no null niche and would
    /// double the slot (and every register file / cell payload with it).
    pub r: *const CellVal,
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
    pub fn as_bool(&self) -> bool {
        unsafe { self.i != 0 }
    }
    /// Safe float read: sound whenever the slot holds an f64 — the
    /// verifier guarantees registers hold their declared types (RFC 0015
    /// §5), the same trust the float ops run on.
    pub fn as_f64(&self) -> f64 {
        unsafe { self.f }
    }
    pub fn null() -> Slot {
        Slot { r: std::ptr::null() }
    }
    /// SAFETY: caller guarantees the slot is a ref slot (verifier-checked).
    pub unsafe fn get_ref(&self) -> Option<*const CellVal> {
        let r = unsafe { self.r };
        if r.is_null() {
            None
        } else {
            Some(r)
        }
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

/// A slot is exactly one machine word (RFC 0015 §5). Guarded at compile
/// time: `Option<*const T>` has no null niche, so wrapping `r` in `Option`
/// silently doubled every register file and cell payload.
const _: () = assert!(std::mem::size_of::<Slot>() == 8);
