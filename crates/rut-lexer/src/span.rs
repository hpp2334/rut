//! Spans — byte ranges into the normalized source; the shared nesting
//! budget (RFC 0030 OQ-3).

/// Byte range into the (normalized) source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub lo: u32,
    pub hi: u32,
}

impl Span {
    pub fn new(lo: u32, hi: u32) -> Span {
        Span { lo, hi }
    }
    pub fn to(self, other: Span) -> Span {
        Span {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }
}

/// Nesting budget shared by the lexer's bracket depth and the parser's
/// frame depth (RFC 0030 OQ-3: single NEST_MAX = 1024). Exceeding it is a
/// normal Diag, never a host stack overflow (contract C3).
pub const NEST_MAX: u32 = 1024;
