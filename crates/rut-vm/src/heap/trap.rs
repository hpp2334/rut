//! Traps (RFC 0034 §2) — `Err(Trap)`, never a Rust panic.
// ---- traps (RFC 0034 §2) ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrapKind {
    OutOfFuel,
    OutOfMemory,
    Interrupted,
    Overflow,
    DivByZero,
    UnwrapNone,
    IndexOutOfBounds,
    Assert,
    Panic,
    BadUnbox,
    Invalid,
}

#[derive(Clone, Debug)]
pub struct Trap {
    pub kind: TrapKind,
    pub msg: String,
}

impl Trap {
    pub fn new(kind: TrapKind, msg: impl Into<String>) -> Trap {
        Trap { kind, msg: msg.into() }
    }
    pub fn name(&self) -> String {
        match self.kind {
            TrapKind::OutOfFuel => "OutOfFuel".into(),
            TrapKind::OutOfMemory => "OutOfMemory".into(),
            TrapKind::Interrupted => "Interrupted".into(),
            TrapKind::Overflow => "Overflow".into(),
            TrapKind::DivByZero => "DivByZero".into(),
            TrapKind::UnwrapNone => "UnwrapNone".into(),
            TrapKind::IndexOutOfBounds => "IndexOutOfBounds".into(),
            TrapKind::Assert => "Assert".into(),
            TrapKind::Panic => "Panic".into(),
            TrapKind::BadUnbox => "BadUnbox".into(),
            TrapKind::Invalid => "Invalid".into(),
        }
    }
}
