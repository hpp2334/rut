//! Lexer — RFC 0030 §1: mode-stack tokenizer. Pure function `&str ->
//! (Vec<Token>, Vec<Diag>)`. Handles `f"..."` interpolation holes (fully
//! lexed token streams, brace-balanced — §1.1), `r"..."` raw strings,
//! maximal munch with a longest-match table, reserved-word rejection with
//! rut-specific messages (RFC 0002 §4), and the bracket depth budget
//! (C3 — exceeding it is a Diag, never a host crash).

use crate::diag::Diag;
use crate::span::{Span, NEST_MAX};
use crate::token::{FPart, FStrTok, FloatSuffix, IntSuffix, Tok, Token};

/// CRLF -> LF normalization (RFC 0002 §1). Spans index the normalized text,
/// so every consumer that maps spans back to positions (the LSP's line
/// index, diag renderers) must run the source through THIS function —
/// parity with `lex` by construction.
pub fn normalize(src: &str) -> String {
    if src.contains('\r') {
        src.replace("\r\n", "\n").replace('\r', "\n")
    } else {
        src.to_string()
    }
}

pub fn lex(src: &str) -> (Vec<Token>, Vec<Diag>) {
    let normalized = normalize(src);
    let mut lx = Lexer::new(&normalized);
    lx.run();
    (lx.toks, lx.diags)
}

struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    pub toks: Vec<Token>,
    pub diags: Vec<Diag>,
    depth: u32,
    depth_reported: bool,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Lexer<'a> {
        let mut lx = Lexer {
            src: src.as_bytes(),
            pos: 0,
            toks: Vec::new(),
            diags: Vec::new(),
            depth: 0,
            depth_reported: false,
        };
        // skip BOM (RFC 0002 §1)
        if lx.src.starts_with(&[0xEF, 0xBB, 0xBF]) {
            lx.pos = 3;
        }
        lx
    }

    fn span(&self, lo: usize) -> Span {
        Span::new(lo as u32, self.pos as u32)
    }

    fn peek(&self) -> u8 {
        *self.src.get(self.pos).unwrap_or(&0)
    }
    fn peek2(&self) -> u8 {
        *self.src.get(self.pos + 1).unwrap_or(&0)
    }
    fn peek3(&self) -> u8 {
        *self.src.get(self.pos + 2).unwrap_or(&0)
    }
    fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn push(&mut self, tok: Tok, lo: usize) {
        self.toks.push(Token {
            tok,
            span: self.span(lo),
        });
    }

    fn err(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diag::new(span, msg));
    }

    fn run(&mut self) {
        while !self.at_end() {
            let lo = self.pos;
            let c = self.peek();
            match c {
                b' ' | b'\t' | b'\n' => {
                    self.pos += 1;
                }
                b'/' if self.peek2() == b'/' => {
                    while !self.at_end() && self.peek() != b'\n' {
                        self.pos += 1;
                    }
                }
                b'/' if self.peek2() == b'*' => {
                    self.pos += 2;
                    let mut closed = false;
                    while self.pos + 1 < self.src.len() + 1 && !self.at_end() {
                        if self.peek() == b'*' && self.peek2() == b'/' {
                            self.pos += 2;
                            closed = true;
                            break;
                        }
                        self.pos += 1;
                    }
                    if !closed {
                        self.err(self.span(lo), "unterminated block comment");
                    }
                }
                b'"' => {
                    self.pos += 1;
                    let s = self.lex_plain_string_body(lo);
                    self.push(Tok::Str(s), lo);
                }
                b'\'' => {
                    self.lex_char(lo);
                }
                b'0'..=b'9' => self.lex_number(lo),
                c if is_ident_start(c) || c >= 0x80 => self.lex_word(lo),
                _ => self.lex_punct(lo),
            }
        }
        let lo = self.pos;
        self.push(Tok::Eof, lo);
    }

    /// Body after the opening `"`; consumes through the closing `"`.
    fn lex_plain_string_body(&mut self, lo: usize) -> String {
        let mut out = String::new();
        loop {
            if self.at_end() || self.peek() == b'\n' {
                self.err(self.span(lo), "unterminated string literal");
                return out;
            }
            let b = self.peek();
            self.pos += 1;
            match b {
                b'"' => return out,
                b'\\' => self.lex_escape(lo, &mut out),
                _ => {
                    // copy the full UTF-8 sequence
                    let start = self.pos - 1;
                    let len = utf8_len(b);
                    self.pos = start + len;
                    if self.pos > self.src.len() {
                        self.pos = self.src.len();
                        self.err(self.span(lo), "invalid UTF-8 in string literal");
                        return out;
                    }
                    out.push_str(&String::from_utf8_lossy(&self.src[start..self.pos]));
                }
            }
        }
    }

    fn lex_escape(&mut self, lo: usize, out: &mut String) {
        if self.at_end() {
            self.err(self.span(lo), "unterminated escape");
            return;
        }
        let b = self.peek();
        self.pos += 1;
        match b {
            b't' => out.push('\t'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b'b' => out.push('\u{8}'),
            b'f' => out.push('\u{c}'),
            b'\\' => out.push('\\'),
            b'"' => out.push('"'),
            b'\'' => out.push('\''),
            b'0' => out.push('\0'),
            b'u' => {
                // \u{XXXX}
                if self.peek() != b'{' {
                    self.err(self.span(lo), "expected `{` after \\u");
                    return;
                }
                self.pos += 1;
                let start = self.pos;
                while !self.at_end() && self.peek() != b'}' {
                    self.pos += 1;
                }
                if self.at_end() {
                    self.err(self.span(lo), "unterminated \\u{...} escape");
                    return;
                }
                let hex = String::from_utf8_lossy(&self.src[start..self.pos]).to_string();
                self.pos += 1; // }
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(c) => out.push(c),
                    None => self.err(self.span(lo), "invalid \\u{...} escape"),
                }
            }
            other => self.err(
                self.span(lo),
                format!("unknown escape `\\{}`", other as char),
            ),
        }
    }

    fn lex_char(&mut self, lo: usize) {
        self.pos += 1; // opening '
        if self.at_end() || self.peek() == b'\n' {
            self.err(self.span(lo), "unterminated char literal");
            return;
        }
        let c: char;
        if self.peek() == b'\\' {
            let mut buf = String::new();
            self.pos += 1;
            self.lex_escape(lo, &mut buf);
            c = buf.chars().next().unwrap_or('\0');
        } else {
            let start = self.pos;
            let len = utf8_len(self.peek());
            self.pos += len;
            c = String::from_utf8_lossy(&self.src[start..self.pos.min(self.src.len())])
                .chars()
                .next()
                .unwrap_or('\0');
        }
        if self.peek() == b'\'' {
            self.pos += 1;
        } else {
            self.err(self.span(lo), "unterminated char literal");
        }
        self.push(Tok::Char(c), lo);
    }

    fn lex_number(&mut self, lo: usize) {
        let mut is_float = false;
        if self.peek() == b'0' && matches!(self.peek2(), b'x' | b'X' | b'b' | b'B' | b'o' | b'O') {
            let radix = match self.peek2() {
                b'x' | b'X' => 16,
                b'o' | b'O' => 8,
                _ => 2,
            };
            self.pos += 2;
            let start = self.pos;
            while !self.at_end() && (self.peek().is_ascii_alphanumeric() || self.peek() == b'_') {
                self.pos += 1;
            }
            let digits: String = self.src[start..self.pos]
                .iter()
                .filter(|&&b| b != b'_')
                .map(|&b| b as char)
                .collect();
            if digits.is_empty() {
                self.err(self.span(lo), "missing digits after base prefix");
                self.push(Tok::Int(0, None), lo);
                return;
            }
            let value = match u64::from_str_radix(&digits, radix) {
                Ok(v) => v,
                Err(_) => {
                    self.err(self.span(lo), "integer literal out of range");
                    0
                }
            };
            let sfx = self.try_int_suffix();
            if sfx.is_none() && self.next_is_ident_continue() {
                self.err(self.span(lo), "invalid numeric suffix");
            }
            self.push(Tok::Int(value, sfx), lo);
            return;
        }
        // decimal
        while !self.at_end() && (self.peek().is_ascii_digit() || self.peek() == b'_') {
            self.pos += 1;
        }
        if self.peek() == b'.' && self.peek2().is_ascii_digit() {
            is_float = true;
            self.pos += 1;
            while !self.at_end() && (self.peek().is_ascii_digit() || self.peek() == b'_') {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), b'e' | b'E')
            && (self.peek2().is_ascii_digit()
                || (matches!(self.peek2(), b'+' | b'-') && self.peek3().is_ascii_digit()))
        {
            is_float = true;
            self.pos += 1;
            if matches!(self.peek(), b'+' | b'-') {
                self.pos += 1;
            }
            while !self.at_end() && (self.peek().is_ascii_digit() || self.peek() == b'_') {
                self.pos += 1;
            }
        }
        let raw: String = self.src[lo..self.pos]
            .iter()
            .filter(|&&b| b != b'_')
            .map(|&b| b as char)
            .collect();
        if is_float {
            let f: f64 = raw.parse().unwrap_or(f64::NAN);
            let sfx = self.try_float_suffix();
            if sfx.is_none() && self.next_is_ident_continue() {
                self.err(self.span(lo), "invalid numeric suffix");
            }
            self.push(Tok::Float(f.to_bits(), sfx), lo);
        } else {
            let v = match raw.parse::<u64>() {
                Ok(v) => v,
                Err(_) => {
                    self.err(self.span(lo), "integer literal out of range");
                    0
                }
            };
            let sfx = self.try_int_suffix();
            if sfx.is_none() && self.next_is_ident_continue() {
                self.err(self.span(lo), "invalid numeric suffix");
            }
            self.push(Tok::Int(v, sfx), lo);
        }
    }

    fn next_is_ident_continue(&self) -> bool {
        !self.at_end() && (is_ident_continue(self.peek()) || self.peek() >= 0x80)
    }

    fn try_int_suffix(&mut self) -> Option<IntSuffix> {
        for (text, sfx) in [
            ("u8", IntSuffix::U8), ("u16", IntSuffix::U16), ("u32", IntSuffix::U32), ("u64", IntSuffix::U64),
            ("i8", IntSuffix::I8), ("i16", IntSuffix::I16), ("i32", IntSuffix::I32), ("i64", IntSuffix::I64),
        ] {
            if self.src[self.pos..].starts_with(text.as_bytes()) {
                self.pos += text.len();
                return Some(sfx);
            }
        }
        None
    }

    fn try_float_suffix(&mut self) -> Option<FloatSuffix> {
        for (text, sfx) in [("f32", FloatSuffix::F32), ("f64", FloatSuffix::F64)] {
            if self.src[self.pos..].starts_with(text.as_bytes()) {
                self.pos += text.len();
                return Some(sfx);
            }
        }
        None
    }

    fn lex_word(&mut self, lo: usize) {
        // identifiers (incl. keywords), raw strings `r"..."`, f-strings `f"..."`
        let start = self.pos;
        while !self.at_end() && (is_ident_continue(self.peek()) || self.peek() >= 0x80) {
            let len = utf8_len(self.peek());
            self.pos += len;
        }
        let word = String::from_utf8_lossy(&self.src[start..self.pos]).to_string();
        match word.as_str() {
            "r" if self.peek() == b'"' => {
                self.pos += 1;
                let s = self.lex_raw_string_body(lo);
                self.push(Tok::RawStr(s), lo);
            }
            "f" if self.peek() == b'"' => {
                self.pos += 1;
                let f = self.lex_fstring(lo);
                self.push(Tok::FStr(f), lo);
            }
            "true" => self.push(Tok::Bool(true), lo),
            "false" => self.push(Tok::Bool(false), lo),
            w => {
                if let Some(msg) = reserved_word_msg(w) {
                    self.err(self.span(lo), msg);
                }
                self.push(Tok::Ident(w.to_string()), lo);
            }
        }
    }

    fn lex_raw_string_body(&mut self, lo: usize) -> String {
        // RFC 0007 §2: every byte is literal; `\"` does NOT close the string.
        let mut out = Vec::new();
        loop {
            if self.at_end() || self.peek() == b'\n' {
                self.err(self.span(lo), "unterminated raw string literal");
                return String::from_utf8_lossy(&out).to_string();
            }
            let b = self.peek();
            self.pos += 1;
            if b == b'"' {
                return String::from_utf8_lossy(&out).to_string();
            }
            out.push(b);
        }
    }

    /// `f"..."` — literal chunks with `{{`/`}}` escapes, holes fully lexed
    /// (RFC 0030 §1.1). The lexer mode-stacks into hole lexing.
    fn lex_fstring(&mut self, lo: usize) -> FStrTok {
        let mut parts: Vec<FPart> = Vec::new();
        let mut lit = String::new();
        loop {
            if self.at_end() || self.peek() == b'\n' {
                self.err(self.span(lo), "unterminated format string");
                break;
            }
            let b = self.peek();
            match b {
                b'"' => {
                    self.pos += 1;
                    break;
                }
                b'{' if self.peek2() == b'{' => {
                    self.pos += 2;
                    lit.push('{');
                }
                b'}' if self.peek2() == b'}' => {
                    self.pos += 2;
                    lit.push('}');
                }
                b'{' => {
                    self.pos += 1;
                    if !lit.is_empty() {
                        parts.push(FPart::Lit(std::mem::take(&mut lit)));
                    }
                    let hole = self.lex_hole(lo);
                    parts.push(FPart::Hole(hole));
                }
                b'\\' => {
                    self.pos += 1;
                    self.lex_escape(lo, &mut lit);
                }
                _ => {
                    let start = self.pos;
                    let len = utf8_len(b);
                    self.pos += len;
                    lit.push_str(&String::from_utf8_lossy(
                        &self.src[start..self.pos.min(self.src.len())],
                    ));
                }
            }
        }
        if !lit.is_empty() {
            parts.push(FPart::Lit(lit));
        }
        FStrTok { parts }
    }

    /// Hole body: lex a balanced-brace token stream through the closing `}`.
    /// A string literal inside a hole is a lex error (RFC 0007 §2): "bind it
    /// to a name first".
    fn lex_hole(&mut self, lo: usize) -> Vec<Token> {
        let mut toks = Vec::new();
        let mut depth = 1u32;
        loop {
            if self.at_end() || self.peek() == b'\n' {
                self.err(self.span(lo), "unterminated format-string hole");
                break;
            }
            let c = self.peek();
            match c {
                b' ' | b'\t' => {
                    self.pos += 1;
                    continue;
                }
                b'}' => {
                    self.pos += 1;
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    let hlo = self.pos - 1;
                    self.push_hole_tok(&mut toks, Tok::RBrace, hlo);
                    continue;
                }
                b'{' => {
                    self.pos += 1;
                    depth += 1;
                    let hlo = self.pos - 1;
                    self.push_hole_tok(&mut toks, Tok::LBrace, hlo);
                    continue;
                }
                b'"' => {} // string literal — allowed (hole termination is
                          // brace-based; the RFC 0007 §2 note stays as a
                          // style rule, not a lexer error)
                b'\'' => {} // char literal — legal in holes (RFC 0007 §2)
                _ => {}
            }
            // lex one token into the hole
            let hlo = self.pos;
            let save_toks = std::mem::take(&mut self.toks);
            let before = self.pos;
            match c {
                b'/' if self.peek2() == b'/' => {
                    while !self.at_end() && self.peek() != b'\n' && self.peek() != b'}' {
                        self.pos += 1;
                    }
                }
                b'/' if self.peek2() == b'*' => {
                    while self.pos + 1 < self.src.len()
                        && !(self.peek() == b'*' && self.peek2() == b'/')
                    {
                        self.pos += 1;
                    }
                    self.pos = (self.pos + 2).min(self.src.len());
                }
                b'0'..=b'9' => self.lex_number(hlo),
                b'\'' => self.lex_char(hlo),
                b'"' => {
                    self.pos += 1; // opening quote
                    let s = self.lex_plain_string_body(hlo);
                    self.push(Tok::Str(s), hlo);
                }
                _ if is_ident_start(c) || c >= 0x80 => self.lex_word(hlo),
                _ => self.lex_punct(hlo),
            }
            debug_assert_eq!(before, hlo);
            // restore the main token stream, keeping the freshly lexed token
            let mut fresh = std::mem::replace(&mut self.toks, save_toks);
            if let Some(t) = fresh.pop() {
                if !matches!(t.tok, Tok::Eof) {
                    toks.push(t);
                }
            }
        }
        toks.push(Token {
            tok: Tok::Eof,
            span: self.span(lo),
        });
        toks
    }

    fn push_hole_tok(&mut self, toks: &mut Vec<Token>, tok: Tok, lo: usize) {
        toks.push(Token {
            tok,
            span: self.span(lo),
        });
    }

    fn lex_punct(&mut self, lo: usize) {
        let rest = &self.src[self.pos..];
        let mut m: Option<(&[u8], Tok)> = None;
        for (text, tok) in LONGEST_MATCH {
            if rest.starts_with(text) {
                m = Some((text, tok.clone()));
                break;
            }
        }
        let Some((text, tok)) = m else {
            let bad = String::from_utf8_lossy(&rest[..utf8_len(rest[0]).min(rest.len())]).to_string();
            self.err(self.span(lo), format!("unexpected character `{bad}`"));
            self.pos += 1;
            return;
        };
        self.pos += text.len();
        // bracket depth budget (C3)
        match tok {
            Tok::LParen | Tok::LBrace | Tok::LBracket => {
                self.depth += 1;
                if self.depth > NEST_MAX {
                    if !self.depth_reported {
                        self.depth_reported = true;
                        self.err(self.span(lo), "nesting too deep");
                    }
                }
            }
            Tok::RParen | Tok::RBrace | Tok::RBracket => {
                self.depth = self.depth.saturating_sub(1);
            }
            _ => {}
        }
        self.push(tok, lo);
    }
}

/// Longest-match table (RFC 0030 §1): 4+ char ops first.
const LONGEST_MATCH: &[(&[u8], Tok)] = &[
    (b"&&=", Tok::AmpAmpEq),
    (b"||=", Tok::PipePipeEq),
    (b"&<<=", Tok::AmpShlEq),
    (b"&<<", Tok::AmpShl),
    (b"<<=", Tok::ShlEq),
    (b">>=", Tok::ShrEq),
    (b"&+=", Tok::AmpPlusEq),
    (b"&-=", Tok::AmpMinusEq),
    (b"&*=", Tok::AmpStarEq),
    (b"&+", Tok::AmpPlus),
    (b"&-", Tok::AmpMinus),
    (b"&*", Tok::AmpStar),
    (b"<<", Tok::Shl),
    (b">>", Tok::Shr),
    (b"->", Tok::Arrow),
    (b"=>", Tok::FatArrow),
    (b"==", Tok::EqEq),
    (b"!=", Tok::NotEq),
    (b"<=", Tok::LtEq),
    (b">=", Tok::GtEq),
    (b"&&", Tok::AmpAmp),
    (b"||", Tok::PipePipe),
    (b"+=", Tok::PlusEq),
    (b"-=", Tok::MinusEq),
    (b"*=", Tok::StarEq),
    (b"/=", Tok::SlashEq),
    (b"%=", Tok::PercentEq),
    (b"&=", Tok::AmpEq),
    (b"|=", Tok::PipeEq),
    (b"^=", Tok::CaretEq),
    (b"..", Tok::DotDot),
    (b"(", Tok::LParen),
    (b")", Tok::RParen),
    (b"{", Tok::LBrace),
    (b"}", Tok::RBrace),
    (b"[", Tok::LBracket),
    (b"]", Tok::RBracket),
    (b",", Tok::Comma),
    (b";", Tok::Semi),
    (b":", Tok::Colon),
    (b".", Tok::Dot),
    (b"+", Tok::Plus),
    (b"-", Tok::Minus),
    (b"*", Tok::Star),
    (b"/", Tok::Slash),
    (b"%", Tok::Percent),
    (b"&", Tok::Amp),
    (b"|", Tok::Pipe),
    (b"^", Tok::Caret),
    (b"~", Tok::Tilde),
    (b"!", Tok::Bang),
    (b"=", Tok::Eq),
    (b"<", Tok::Lt),
    (b">", Tok::Gt),
    (b"?", Tok::Question),
    (b"@", Tok::At),
];

/// Reserved words (RFC 0002 §4): hard errors naming the rut replacement.
/// NOTE: `super` and `as` are NOT here — they are contextual (`pub(super)`,
/// RFC 0003 §2; `as` binds select arms, RFC 0019 §3); the parser rejects
/// them in every other position.
fn reserved_word_msg(w: &str) -> Option<String> {
    let repl: &str = match w {
        "switch" => "rut does not have `switch`; use `when`",
        "case" => "rut does not have `case`; `when` arms are `pattern -> body`",
        "void" => "rut spells the unit type `unit`",
        "extends" => "rut has no inheritance (`extends`); compose instead (RFC 0010 §3)",
        "trait" => "rut spells this `interface`",
        "type" => "`type` members are not available — the element is a type argument (`interface Iter<T>`, RFC 0012)",
        "struct" => "rut does not have `struct`; use `dataclass`",
        "match" => "rut does not have `match`; use `when`",
        "null" => "rut has no `null`; absence is `Option<T>` (RFC 0005)",
        "undefined" => "rut has no `undefined`; absence is `Option<T>` (RFC 0005)",
        "any" => "rut has no `any`; use an interface type or `Opaque` (RFC 0012, RFC 0014)",
        "typeof" => "rut has no `typeof`; types are static — `x is T` tests at runtime",
        "instanceof" => "rut has no `instanceof`; use `is`",
        "delete" => "rut has no `delete`; there are no dynamic properties",
        "in" => "`in` is not an operator; iteration is `for (let x of ...)`",
        "with" => "rut does not have `with`",
        "var" => "rut does not have `var`; use `let` / `let mut` (RFC 0003 §1)",
        "const" => "rut does not have `const`; use `let` (RFC 0003 §1)",
        "private" => {
            "members are private by default —add `pub` (RFC 0003 §2, RFC 0010 §2)"
        }
        _ => return None,
    };
    Some(repl.to_string())
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}
fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}
fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}
