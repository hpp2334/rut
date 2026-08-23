//! Tokens — RFC 0030 §1: one flat exhaustive enum. Keywords are `Ident`s
//! (the parser matches them by interner text — RFC 0002 §4/§5); reserved
//! words are rejected by the LEXER with a "rut does not have X" message.

use crate::span::Span;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    // literals
    Int(u64, Option<IntSuffix>),
    Float(u64 /*f64 bits*/, Option<FloatSuffix>),
    Str(String),           // decoded UTF-8, escapes resolved
    RawStr(String),        // no escape processing
    FStr(FStrTok),         // RFC 0030 §1.1 — parts + lexed holes
    Char(char),
    Bool(bool),
    Ident(String),         // includes keywords after the reservation check

    // punctuation & operators — exhaustive, single flat enum
    LParen, RParen, LBrace, RBrace, LBracket, RBracket,
    Comma, Semi, Colon, Dot, DotDot, Arrow, FatArrow, // . .. -> =>
    Plus, Minus, Star, Slash, Percent,               // + - * / %
    AmpAmp, PipePipe, Bang,                          // && || !
    Eq, EqEq, NotEq, Lt, Gt, LtEq, GtEq,             // = == != < > <= >=
    PlusEq, MinusEq, StarEq, SlashEq, PercentEq,     // += -= *= /= %=
    Amp, Pipe, Caret, Tilde, Shl, Shr,               // & | ^ ~ << >>
    AmpEq, PipeEq, CaretEq, ShlEq, ShrEq,            // &= |= ^= <<= >>=
    AmpAmpEq, PipePipeEq,                            // &&= ||=
    // wrapping-arith (RFC 0004 §3): dedicated digraphs, no maximal-munch ambiguity
    AmpPlus, AmpMinus, AmpStar, AmpShl,              // &+ &- &* &<<
    AmpPlusEq, AmpMinusEq, AmpStarEq, AmpShlEq,      // &+= &-= &*= &<<=
    Question, At,

    Eof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntSuffix {
    U8, U16, U32, U64, I8, I16, I32, I64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FloatSuffix {
    F32, F64,
}

/// `f"..."` — interleaved chunks and holes; holes are FULLY LEXED token
/// streams (RFC 0030 §1.1), `}`-terminated, with real spans inside the
/// literal's span.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FStrTok {
    pub parts: Vec<FPart>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FPart {
    Lit(String),      // decoded like a plain string; {{ }} -> { }
    Hole(Vec<Token>), // balanced-brace token stream, ready for the parser
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

impl Token {
    /// Human-readable description for diagnostics ("expected X, found Y").
    pub fn describe(&self) -> String {
        use Tok::*;
        match &self.tok {
            Int(v, sfx) => match sfx {
                Some(s) => format!("integer literal `{v:?}{s:?}`"),
                None => format!("integer literal `{v}`"),
            },
            Float(v, _) => format!("float literal `{}`", f64::from_bits(*v)),
            Str(s) | RawStr(s) => format!("string literal {s:?}"),
            FStr(_) => "format string".to_string(),
            Char(c) => format!("char literal '{c}'"),
            Bool(b) => format!("`{b}`"),
            Ident(s) => format!("`{s}`"),
            LParen => "`(`".into(), RParen => "`)`".into(),
            LBrace => "`{`".into(), RBrace => "`}`".into(),
            LBracket => "`[`".into(), RBracket => "`]`".into(),
            Comma => "`,`".into(), Semi => "`;`".into(), Colon => "`:`".into(),
            Dot => "`.`".into(), DotDot => "`..`".into(),
            Arrow => "`->`".into(), FatArrow => "`=>`".into(),
            Plus => "`+`".into(), Minus => "`-`".into(),
            Star => "`*`".into(), Slash => "`/`".into(), Percent => "`%`".into(),
            AmpAmp => "`&&`".into(), PipePipe => "`||`".into(), Bang => "`!`".into(),
            Eq => "`=`".into(), EqEq => "`==`".into(), NotEq => "`!=`".into(),
            Lt => "`<`".into(), Gt => "`>`".into(),
            LtEq => "`<=`".into(), GtEq => "`>=`".into(),
            PlusEq => "`+=`".into(), MinusEq => "`-=`".into(),
            StarEq => "`*=`".into(), SlashEq => "`/=`".into(), PercentEq => "`%=`".into(),
            Amp => "`&`".into(), Pipe => "`|`".into(), Caret => "`^`".into(),
            Tilde => "`~`".into(), Shl => "`<<`".into(), Shr => "`>>`".into(),
            AmpEq => "`&=`".into(), PipeEq => "`|=`".into(), CaretEq => "`^=`".into(),
            ShlEq => "`<<=`".into(), ShrEq => "`>>=`".into(),
            AmpAmpEq => "`&&=`".into(), PipePipeEq => "`||=`".into(),
            AmpPlus => "`&+`".into(), AmpMinus => "`&-`".into(),
            AmpStar => "`&*`".into(), AmpShl => "`&<<`".into(),
            AmpPlusEq => "`&+=`".into(), AmpMinusEq => "`&-=`".into(),
            AmpStarEq => "`&*=`".into(), AmpShlEq => "`&<<=`".into(),
            Question => "`?`".into(), At => "`@`".into(),
            Eof => "end of file".into(),
        }
    }
}
