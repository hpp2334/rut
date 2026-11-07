//! Spans — byte ranges into the normalized source; the shared nesting
//! budget.

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
    /// THE JOIN (the artifact law v3): the text a span indexes, sliced
    /// out of the CALLER's src — the exact normalized text the parse
    /// produced (`Parsed.src`), never a raw input.
    pub fn text(self, src: &str) -> &str {
        &src[self.lo as usize..self.hi as usize]
    }
    pub fn to(self, other: Span) -> Span {
        Span {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }
}

/// Nesting budget shared by the lexer's bracket depth and the parser's
/// frame depth (single NEST_MAX = 1024). Exceeding it is a
/// normal Diag, never a host stack overflow (contract C3).
pub const NEST_MAX: u32 = 1024;
