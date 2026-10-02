//! The printer — the AST reprint (`pretty(ast)`).
//!
//! Structure is source-order (the parser is monotone — §4.2 — so a
//! clean traversal walks nodes in the order they were written); layout
//! is canonical house style, colored by the `[style]` knobs.
//!
//! **Comment drains are positional.** Runs (the gap scan's product, in
//! source byte order) drain with a `passed` cursor at guarded points:
//! a TRAILING run (scan-classified — it starts on the same source line
//! its preceding token ends) attaches right after the element it
//! trails, on that output line; an OWN-LINE run drains just before the
//! element it precedes, at the enclosing indent. Blank lines derive
//! from source-line arithmetic (≥ 2 source lines between the consumed
//! cursor and the incoming content = one blank), so runs of blanks
//! collapse and re-formatting is idempotent.
//!
//! **Break points are two-mode**: try flat (one line); if the produced
//! line would exceed `max_width` — or a comment flush inside the flat
//! attempt emitted a newline — roll back to the mark and print one
//! element per line with trailing commas. Marks snapshot (output
//! length, comment cursor, passed cursor), so rollbacks also
//! un-consume comments; nested break points compose.
//!
//! **Parens**: the AST has no paren nodes; the printer re-parens a
//! child only where flat precedence would invert the parse's grouping
//! (the child-level table mirrors the parser's reduce levels). The
//! VM-verified suite (`tests/semantic_equiv.rs`) referees the table.
//!
//! **Flat mode** (`in_flat`): when-arm INLINE bodies print
//! their statements without newline hooks — drains are suppressed (the
//! enclosing arm's end drain catches the runs; the block's own lines
//! flush the comments the scanner places on the arm's line; the comment
//! positions were trailing by scan).

use rut_ast::ast::*;
use rut_lexer::span::Span;
use rut_lexer::token::{FloatSuffix, IntSuffix};

use crate::comments::CommentRun;
use crate::style::Style;

// operator levels — the parser's reduce levels, mined from
// rut-parser/src/expr.rs:247-316 (the fmt survey §2 table)
const L_OR: u8 = 3;
const L_AND: u8 = 4;
const L_EQ: u8 = 5;
const L_CMP: u8 = 6; // comparisons + `is` (non-associative)
const L_BOR: u8 = 7;
const L_BAND: u8 = 8;
const L_SHIFT: u8 = 9;
const L_ADD: u8 = 10;
const L_MUL: u8 = 11;
const L_CAST: u8 = 12;
const L_UNARY: u8 = 13;
const L_POSTFIX: u8 = 14; // call/index/field/try — children never anchor

fn binop_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::Le => "<=",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
    }
}

fn binop_level(op: BinOp) -> u8 {
    match op {
        BinOp::Or => L_OR,
        BinOp::And => L_AND,
        BinOp::Eq | BinOp::Ne => L_EQ,
        BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => L_CMP,
        BinOp::BitOr | BinOp::BitXor => L_BOR,
        BinOp::BitAnd => L_BAND,
        BinOp::Shl | BinOp::Shr => L_SHIFT,
        BinOp::Add | BinOp::Sub => L_ADD,
        BinOp::Mul | BinOp::Div | BinOp::Mod => L_MUL,
    }
}

fn unop_str(op: UnOp) -> &'static str {
    match op {
        UnOp::Neg => "-",
        UnOp::Not => "!",
        UnOp::BitNot => "~",
    }
}

fn assign_str(op: Option<BinOp>) -> &'static str {
    let Some(o) = op else { return "=" };
    match o {
        BinOp::Add => "+=",
        BinOp::Sub => "-=",
        BinOp::Mul => "*=",
        BinOp::Div => "/=",
        BinOp::Mod => "%=",
        BinOp::BitAnd => "&=",
        BinOp::BitOr => "|=",
        BinOp::BitXor => "^=",
        BinOp::Shl => "<<=",
        BinOp::Shr => ">>=",
        // unreachable family: Eq/Ne/Lt/Gt/Le/Ge/And/Or never compound
        // — shift_assign only arms the arithmetic + bitwise table
        // (rut-parser/src/expr.rs:266-276)
        _ => "=",
    }
}

fn vis_str(v: Vis) -> &'static str {
    match v {
        Vis::Pub => "pub",
        Vis::Mod => "pub(mod)",
        Vis::Super => "pub(super)",
        Vis::Self_ => "",
    }
}

fn vis_opt(v: Vis) -> Option<&'static str> {
    match v {
        Vis::Self_ => None,
        other => Some(vis_str(other)),
    }
}

fn int_suffix(s: Option<IntSuffix>) -> &'static str {
    match s {
        None => "",
        Some(IntSuffix::U8) => "u8",
        Some(IntSuffix::U16) => "u16",
        Some(IntSuffix::U32) => "u32",
        Some(IntSuffix::U64) => "u64",
        Some(IntSuffix::I8) => "i8",
        Some(IntSuffix::I16) => "i16",
        Some(IntSuffix::I32) => "i32",
        Some(IntSuffix::I64) => "i64",
    }
}

fn float_suffix(s: Option<FloatSuffix>) -> &'static str {
    match s {
        None => "",
        Some(FloatSuffix::F32) => "f32",
        Some(FloatSuffix::F64) => "f64",
    }
}

/// The escape inverse of the lexer's `lex_escape` (`lexer.rs:171-212`),
/// plus the f-string's `{{`/`}}` doubling (the fmt survey §2): every
/// escape the lexer DECODES must re-encode so bytes re-lex identically.
fn escape_str(body: &str, in_fstring: bool) -> String {
    let mut s = String::with_capacity(body.len() + 8);
    for c in body.chars() {
        if in_fstring {
            match c {
                '{' => {
                    s.push_str("{{");
                    continue;
                }
                '}' => {
                    s.push_str("}}");
                    continue;
                }
                _ => {}
            }
        }
        match c {
            '\t' => s.push_str("\\t"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\u{8}' => s.push_str("\\b"),
            '\u{c}' => s.push_str("\\f"),
            '\\' => s.push_str("\\\\"),
            '"' => s.push_str("\\\""),
            '\0' => s.push_str("\\0"),
            '\'' => s.push_str("\\'"),
            c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
                s.push_str(&format!("\\u{{{:x}}}", c as u32));
            }
            c => s.push(c),
        }
    }
    s
}

#[derive(Clone, Copy)]
struct Mark {
    len: usize,
    ri: usize,
    passed: u32,
}

/// the begin-hook's drain mode:
/// - `Stmt` — statement/item separators: the full law (blank rules,
///   trailing runs wait for the element's own end drain);
/// - `List` — element lists: comments drain own-line, never blanks;
/// - `Expr` — expression children: mid-expression trailing comments
///   attach own-line here (no later drain can match their line).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Begin {
    Stmt,
    List,
    Expr,
}

pub(super) struct P<'a> {
    a: &'a Ast,
    style: &'a Style,
    /// the NORMALIZED source — comment text slices out of it verbatim
    src: &'a str,
    /// the lexed token stream — spans index the same normalized text;
    /// the drains' line anchors read the token ends (parser spans
    /// OVERSHOOT to the next token — the statement-span convention —
    /// so the element's true content end comes from the tokens)
    toks: &'a [rut_lexer::token::Token],
    line_starts: Vec<u32>,
    runs: Vec<CommentRun>,
    /// index of the next unconsumed run
    ri: usize,
    /// the source byte the last consumed content ended at — the
    /// comment/blank arithmetic's cursor
    passed: u32,
    out: String,
    /// current indent level (times `indent_width` spaces on fresh lines)
    level: usize,
    /// pending indent for the next `text()` on a fresh line
    pending: Option<usize>,
    /// the inline-flat depth (arm bodies): drains suppress, breaks skip
    in_flat: usize,
}

impl<'a> P<'a> {
    pub(super) fn new(
        a: &'a Ast,
        src: &'a str,
        toks: &'a [rut_lexer::token::Token],
        style: &'a Style,
        runs: Vec<CommentRun>,
    ) -> P<'a> {
        let mut line_starts: Vec<u32> = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        P {
            a,
            style,
            src,
            toks,
            line_starts,
            runs,
            ri: 0,
            passed: 0,
            out: String::new(),
            level: 0,
            pending: Some(0),
            in_flat: 0,
        }
    }
}

// ---- the writer ----
//
// The line discipline: a newline is pushed only by `nl`/`nl_band`/
// `blank`/`emit_run`, and the NEXT line's indent materializes lazily
// when the first text of that line writes (`pending`). `open_line()`
// answers whether the just-printed line is still appendable (trailing
// comments ride it).

impl<'a> P<'a> {
    fn line_of(&self, off: u32) -> usize {
        match self.line_starts.partition_point(|&s| s <= off) {
            0 => 0,
            n => n - 1,
        }
    }

    /// the element's TRUE content end. The parser's node spans
    /// OVERSHOOT: span.hi is the NEXT token's hi (the parser absorbs at
    /// the current token), so the
    /// last token at-or-before span.hi may be the next element's
    /// opener. The law: walk back while a token ends EXACTLY at
    /// span.hi and starts after span.lo — those ride the overshoot;
    /// the element's own last token ends strictly inside.
    fn content_end(&self, span: Span) -> u32 {
        let n = self.toks.partition_point(|t| t.span.hi <= span.hi);
        let mut j = n;
        while j > 0 {
            let t = &self.toks[j - 1];
            if t.span.hi == span.hi && t.span.lo > span.lo {
                j -= 1;
                continue;
            }
            return t.span.hi;
        }
        span.lo
    }

    /// chars on the currently-open line (0 when the line is fresh)
    fn col(&self) -> usize {
        let start = self.out.rfind('\n').map_or(0, |i| i + 1);
        self.out[start.min(self.out.len())..].chars().count()
    }

    fn open_line(&self) -> bool {
        self.pending.is_none() && !self.out.is_empty() && !self.out.ends_with('\n')
    }

    fn text(&mut self, s: &str) {
        if s.is_empty() {
            return; // the pending indent stays pending for the next text
        }
        if let Some(l) = self.pending.take() {
            self.out.push_str(&" ".repeat(l * self.style.indent_width));
        }
        self.out.push_str(s);
    }

    /// one space on the open line (never at a fresh line — the pending
    /// indent IS the pad there)
    fn sp(&mut self) {
        if self.pending.is_none() && !self.out.is_empty() && !self.out.ends_with(' ') {
            self.out.push(' ');
        }
    }

    fn trim_tail(&mut self) {
        while self.out.ends_with(' ') {
            self.out.pop();
        }
    }

    /// finish the open line; the NEXT line re-indents at `self.level`
    fn nl(&mut self) {
        self.trim_tail();
        if !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        self.pending = Some(self.level);
    }

    /// one blank line between the finished content and the coming one
    /// (runs of source blanks collapse to one)
    fn blank(&mut self) {
        self.trim_tail();
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        self.out.push('\n');
        self.pending = Some(self.level);
    }

    // ---- marks (a rollback un-emits comments too) ------------------

    fn stamp(&self) -> Mark {
        Mark { len: self.out.len(), ri: self.ri, passed: self.passed }
    }

    fn rollback(&mut self, m: Mark) {
        self.out.truncate(m.len);
        self.ri = m.ri;
        self.passed = m.passed;
        self.pending = if self.out.is_empty() || self.out.ends_with('\n') {
            Some(self.level)
        } else {
            None
        };
    }

    /// flat attempt failed: a newline appeared inside it (a comment or
    /// a child break), or the resulting line exceeds the width
    fn over_on_flat(&self, m: Mark) -> bool {
        if self.out[m.len..].contains('\n') {
            return true;
        }
        self.col() > self.style.max_width
    }
}

// ---- comment machinery + break-list machinery ----

impl<'a> P<'a> {
    // ---- comment helpers ----

    fn run_text(&self, r: &CommentRun) -> String {
        self.src[r.lo as usize..r.hi as usize].to_string()
    }

    /// BEGIN: drain own-line comments pending before the element, then
    /// — at statement/item/field/arm separators (`sep`) — the blank
    /// rule (source gap ≥ 2 lines between the cursor and the element =
    /// one blank). Inside expressions, comments carry no blank rules,
    /// and a trailing run attaches as its own line here (there is no
    /// later statement-boundary drain that matches its line).
    fn begin_el(&mut self, span: Span, mode: Begin) {
        loop {
            let Some(r) = self.runs.get(self.ri).cloned() else { break };
            if r.lo >= span.lo {
                break;
            }
            if mode == Begin::Stmt && r.trailing {
                break; // the element it trails owns it (its end hook)
            }
            // own-line emission; the blank-before rule rides only the
            // statement separators (lists and expressions never blank)
            if mode == Begin::Stmt && self.line_of(r.lo) >= self.line_of(self.passed) + 2 {
                self.blank();
            }
            if self.open_line() {
                self.nl();
            }
            self.emit_run(&r);
            self.ri += 1;
            self.passed = r.hi;
        }
        // the element's own blank rule: a source gap of >= 2 lines
        // between the cursor and this element = one blank line (runs of
        // source blanks collapse; no comments needed for the law to fire)
        if mode == Begin::Stmt
            && !self.out.is_empty()
            && self.line_of(span.lo) >= self.line_of(self.passed) + 2
        {
            self.blank();
        }
    }

    /// write one comment run: each of its lines at the current indent
    /// (block comments re-indented line by line, blank lines bare)
    fn emit_run(&mut self, r: &CommentRun) {
        let body = self.run_text(r);
        for line in body.lines() {
            if line.trim_start().is_empty() {
                self.text(""); // keeps the blank line a blank line
            } else {
                self.text(line.trim_start());
            }
            self.nl();
        }
    }

    /// END: drain the trailing line comments of THIS element's last
    /// source line onto the just-printed line (one pad, verbatim),
    /// then finish the line.
    fn end_el(&mut self, span: Span) {
        self.drain_trailing_bare(self.content_end(span));
        self.nl();
        self.passed = self.passed.max(self.content_end(span));
    }

    /// the trailing-drain without the newline (for broken-list members
    /// whose comma prints first). Returns whether a run was consumed.
    fn drain_trailing_bare(&mut self, content_hi: u32) -> bool {
        if self.in_flat > 0 {
            // an inline-flat context cannot host a line comment (the
            // comment would swallow the tokens after it) — leave it for
            // the enclosing statement's end drain
            return false;
        }
        let end_line = self.line_of(content_hi);
        let mut consumed = false;
        while let Some(r) = self.runs.get(self.ri).cloned() {
            if !r.trailing || self.line_of(r.lo) != end_line {
                break;
            }
            self.sp();
            let t = self.run_text(&r);
            self.text(t.trim_start());
            self.ri += 1;
            self.passed = r.hi;
            consumed = true;
        }
        consumed
    }

    /// comments pending between the last inner element and a CLOSING
    /// bracket: own-line at the inner level, then the closer sits fresh
    fn drain_close(&mut self, node_hi: u32) {
        while let Some(r) = self.runs.get(self.ri).cloned() {
            if r.lo >= node_hi {
                break;
            }
            if self.open_line() {
                self.nl();
            }
            self.emit_run(&r);
            self.ri += 1;
            self.passed = r.hi;
        }
    }

    /// the file's tail: everything pending (the EOF group), own-line at
    /// level 0; output ends with exactly one newline
    fn drain_tail(&mut self) {
        while self.ri < self.runs.len() {
            let r = self.runs[self.ri].clone();
            if self.open_line() {
                self.sp();
                let t = self.run_text(&r);
                self.text(t.trim_start());
                self.nl();
            } else {
                self.emit_run(&r);
            }
            self.ri += 1;
            self.passed = r.hi;
        }
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.nl();
        }
        while self.out.ends_with("\n\n") {
            self.out.pop();
        }
    }

    // ---- two-mode lists --------------------------------------------

    /// flat/broken element lists: ", " separators flat, or one element
    /// per line at the continuation band with trailing commas. Each
    /// element's own break points re-run fresh in broken mode (the
    /// band's fresh indent gives them room). NOT for when arms —
    /// those are always one-per-line.
    ///
    /// `spans` is the ELEMENT spans parallel to `items` (the drain and
    /// trailing anchors); `whole` is the enclosing node's span (the
    /// close-bracket drain's cursor bound).
    fn break_list<T>(
        &mut self,
        items: &[T],
        spans: &[Span],
        trailing_comma: bool,
        mut one: impl FnMut(&mut Self, &T),
    ) {
        debug_assert_eq!(items.len(), spans.len());
        if items.is_empty() {
            return;
        }
        let m = self.stamp();
        let mut commented = false;
        for (i, (it, sp)) in items.iter().zip(spans.iter()).enumerate() {
            if i > 0 {
                self.text(", ");
            }
            self.begin_el(*sp, Begin::List);
            one(self, it);
            // the element's trailing comment rides the flat line too —
            // otherwise it leaks to the NEXT element's mid-expression
            // drain and migrates on the re-format. But a LINE comment
            // owns the rest of its line: the list must break so the
            // separators/closers never print after a `//`.
            commented |= self.drain_trailing_bare(self.content_end(*sp));
        }
        if self.in_flat > 0 {
            return; // inline-flat context: no two-mode decision, no break
        }
        if std::env::var_os("FMT_DEBUG").is_some() {
            eprintln!("[list] commented={} over={} col={} chunk={:?}",
                commented, self.over_on_flat(m), self.col(),
                self.out[m.len..].chars().take(46).collect::<String>());
        }
        if commented || self.over_on_flat(m) {
            self.rollback(m);
            self.nested_break(items, spans, trailing_comma, one);
        }
    }

    /// the broken form itself (called after the rollback), one element
    /// per continuation line + trailing commas; leaves the output at a
    /// fresh line whose next content sits at the OUTER level
    fn nested_break<T>(
        &mut self,
        items: &[T],
        spans: &[Span],
        trailing_comma: bool,
        mut one: impl FnMut(&mut Self, &T),
    ) {
        // `trailing_comma`: calls/arrays/struct-literals take one (the
        // parser eats it); TUPLES do not — `(a, b,)` demands an element
        // after every comma (paren_top pushes an Expr frame per comma)
        self.nl_band();
        for (i, (it, sp)) in items.iter().zip(spans.iter()).enumerate() {
            if i > 0 {
                self.nl();
            }
            self.begin_el(*sp, Begin::List);
            one(self, it);
            // the SEPARATOR rides every element but the last; the
            // TRAILING comma re-adds one after the last (calls/arrays/
            // struct literals take it — tuples never do)
            let last = i == items.len() - 1;
            if !last || trailing_comma {
                self.text(",");
            }
            self.drain_trailing_bare(self.content_end(*sp));
        }
        self.level -= 1;
        self.nl();
    }

    /// newline whose next line hangs one level deeper — and whose
    /// SUCCEEDING fresh lines (until the level restores) live at that
    /// deeper band
    fn nl_band(&mut self) {
        self.trim_tail();
        self.out.push('\n');
        self.level += 1;
        self.pending = Some(self.level);
    }
}


// ---- the module, items, members ----

impl<'a> P<'a> {
    pub(super) fn print_module(mut self) -> String {
        let items = self.a.module_items(self.a.root).to_vec();
        for it in items {
            self.item(it);
        }
        self.drain_tail();
        self.out
    }

    /// one item: BEGIN drain (+ separator blank rules), the tokens, END
    fn item(&mut self, h: NodeHandle<AnyItem>) {
        let span = self.a.span(h.id());
        self.begin_el(span, Begin::Stmt);
        match self.a.item(h).clone() {
            ItemKind::Module { items } => {
                // reachable only via the root — walk it the same way
                for it in items {
                    self.item(it);
                }
            }
            ItemKind::Use { pkg, names } => {
                self.text("use ");
                self.text(self.a.name(pkg));
                if names.is_empty() {
                    self.text(";");
                } else {
                    self.text("::{");
                    self.sp();
                    let nm: Vec<String> = names.iter().map(|&n| self.a.name(n).to_string()).collect();
                    let dummies: Vec<Span> = std::iter::repeat(span).take(nm.len()).collect();
                    self.ident_list(&nm, &dummies, span);
                    self.sp();
                    self.text("};");
                }
            }
            ItemKind::Alias(d) => {
                if let Some(v) = vis_opt(d.vis) {
                    self.text(v);
                    self.sp();
                }
                self.text("type ");
                self.text(self.a.name(d.name));
                self.text(" = ");
                self.ty(d.target);
                self.text(";");
            }
            ItemKind::ModuleLet { vis, name, ty, init } => {
                if let Some(v) = vis_opt(vis) {
                    self.text(v);
                    self.sp();
                }
                self.text("let ");
                self.text(self.a.name(name));
                if let Some(t) = ty {
                    self.text(": ");
                    self.ty(t);
                }
                self.rhs_eq(init);
                self.text(";");
            }
            ItemKind::Enum { vis, name, members } => {
                if let Some(v) = vis_opt(vis) {
                    self.text(v);
                    self.sp();
                }
                self.text("enum ");
                self.text(self.a.name(name));
                self.text(" {");
                let names: Vec<String> = members
                    .iter()
                    .map(|(m, v)| match v {
                        None => self.a.name(*m).to_string(),
                        Some(x) => format!("{} = {}", self.a.name(*m), x),
                    })
                    .collect();
                if !names.is_empty() {
                    self.sp();
                    let dummies: Vec<Span> = std::iter::repeat(span).take(names.len()).collect();
                    self.ident_list(&names, &dummies, span);
                }
                self.sp();
                self.text("}");
            }
            ItemKind::Struct { vis, name, generics, fields, methods } => {
                if let Some(v) = vis_opt(vis) {
                    self.text(v);
                    self.sp();
                }
                self.text("struct ");
                self.ty_decl(name, &generics, &Vec::new());
                self.body_of_fields_methods(fields, methods, Some(span));
            }
            ItemKind::Class { vis, name, generics, requires, newtype, fields, methods } => {
                if let Some(v) = vis_opt(vis) {
                    self.text(v);
                    self.sp();
                }
                self.text("class ");
                self.ty_decl(name, &generics, &requires);
                // the positional one-field decl prints back in its
                // spelling — never as the desugared braced body
                if newtype {
                    if let Some(f) = fields.first() {
                        let d = self.a.field_decl(*f);
                        self.text("(");
                        self.ty(d.ty);
                        self.text(");");
                        return;
                    }
                }
                self.body_of_fields_methods(fields, methods, Some(span));
            }
            ItemKind::Trait { vis, name, generics, requires, methods } => {
                if let Some(v) = vis_opt(vis) {
                    self.text(v);
                    self.sp();
                }
                self.text("trait ");
                self.text(self.a.name(name));
                if !generics.is_empty() {
                    self.text("<");
                    let gnames = generics.iter().map(|&g| self.a.name(g).to_string()).collect::<Vec<_>>().join(", ");
                    self.text(&gnames);
                    self.text(">");
                }
                if !requires.is_empty() {
                    self.sp();
                    self.text("requires");
                    self.sp();
                    for (i, t) in requires.iter().enumerate() {
                        if i > 0 {
                            self.text(" | ");
                        }
                        self.ty(*t);
                    }
                }
                self.trait_body(methods, span);
            }
            ItemKind::Impl { generics, trait_ref, target, methods, .. } => {
                self.text("impl");
                if !generics.is_empty() {
                    self.gen_only(&generics);
                    self.sp();
                } else {
                    self.sp();
                }
                if let Some(tr) = trait_ref {
                    self.ty(tr);
                    self.sp();
                    self.text("for ");
                }
                self.ty(target);
                self.methods_body(methods, span);
            }
            ItemKind::Fn(f) => {
                self.fn_header(&f, true);
                self.sp();
                self.block(f.body);
            }
            ItemKind::SurfaceFn { vis, linkage, is_async, name, generics, params, ret } => {
                if let Some(v) = vis_opt(vis) {
                    self.text(v);
                    self.sp();
                }
                self.text(linkage_str(linkage));
                self.sp();
                if is_async {
                    self.text("async ");
                }
                self.text("fn ");
                self.text(self.a.name(name));
                self.gen_only(&generics);
                self.params_list(&params, span);
                if let Some(r) = ret {
                    self.text(" -> ");
                    self.ty(r);
                }
                self.text(";");
            }
            ItemKind::SurfaceStruct { vis, name, fields } => {
                if let Some(v) = vis_opt(vis) {
                    self.text(v);
                    self.sp();
                }
                self.text("host struct ");
                self.text(self.a.name(name));
                self.fields_body(fields, Some(span));
            }
            ItemKind::BuiltinTy { vis: _, ambient, name, generics, members } => {
                // the type-shaped builtin items carry the strict spelling
                // they were declared with (`prelude builtin` ambient /
                // `pub builtin` import-gated)
                self.text(if ambient { "prelude builtin class " } else { "pub builtin class " });
                self.text(self.a.name(name));
                self.gen_only(&generics);
                self.trait_body(members, span);
            }
            ItemKind::BuiltinPrimitive { ambient, name, members } => {
                self.text(if ambient { "prelude builtin primitive " } else { "pub builtin primitive " });
                self.text(self.a.name(name));
                self.trait_body(members, span);
            }
            ItemKind::BuiltinImpl { vis, prim, methods } => {
                let _ = vis;
                self.text("prelude builtin impl ");
                self.text(self.a.name(prim));
                self.trait_body(methods, span);
            }
        }
        self.end_el(span);
    }

    fn ty_decl(&mut self, name: IdentId, gens: &[IdentId], requires: &[(IdentId, NodeHandle<AnyTy>)]) {
        self.text(self.a.name(name));
        if gens.is_empty() {
            return;
        }
        self.text("<");
        for (i, g) in gens.iter().enumerate() {
            if i > 0 {
                self.text(", ");
            }
            self.text(self.a.name(*g));
            if let Some((_, t)) = requires.iter().find(|(b, _)| *b == *g) {
                self.sp();
                self.text("requires");
                self.sp();
                self.ty(*t);
            }
        }
        self.text(">");
    }

    fn gen_only(&mut self, gens: &[IdentId]) {
        if gens.is_empty() {
            return;
        }
        self.text("<");
        let gs = gens.iter().map(|&g| self.a.name(g).to_string()).collect::<Vec<_>>().join(", ");
        self.text(&gs);
        self.text(">");
    }

    fn body_of_fields_methods(
        &mut self,
        fields: Vec<NodeHandle<FieldDeclNode>>,
        methods: Vec<NodeHandle<MethodDeclNode>>,
        enclosing: Option<Span>,
    ) {
        self.sp();
        self.text("{");
        if let Some(e) = enclosing {
            self.passed = self.passed.max(e.lo);
        }
        self.level += 1;
        self.nl();
        for f in &fields {
            self.member_field(*f);
        }
        for m in &methods {
            self.member_method(*m);
        }
        if let Some(e) = enclosing {
            self.drain_close(self.content_end(e));
        }
        self.level -= 1;
        self.nl();
        self.text("}");
    }

    fn fields_body(&mut self, fields: Vec<NodeHandle<FieldDeclNode>>, enclosing: Option<Span>) {
        self.sp();
        self.text("{");
        if let Some(e) = enclosing {
            self.passed = self.passed.max(e.lo);
        }
        self.level += 1;
        self.nl();
        for f in &fields {
            self.member_field(*f);
        }
        if let Some(e) = enclosing {
            self.drain_close(self.content_end(e));
        }
        self.level -= 1;
        self.nl();
        self.text("}");
    }

    fn trait_body(&mut self, methods: Vec<NodeHandle<MethodDeclNode>>, enclosing: Span) {
        self.sp();
        self.text("{");
        self.passed = self.passed.max(enclosing.lo);
        self.level += 1;
        self.nl();
        for m in &methods {
            self.member_method(*m);
        }
        self.drain_close(self.content_end(enclosing));
        self.level -= 1;
        self.nl();
        self.text("}");
    }

    fn methods_body(&mut self, methods: Vec<NodeHandle<MethodDeclNode>>, enclosing: Span) {
        self.sp();
        self.text("{");
        self.passed = self.passed.max(enclosing.lo);
        self.level += 1;
        self.nl();
        for m in &methods {
            self.member_method(*m);
        }
        self.drain_close(self.content_end(enclosing));
        self.level -= 1;
        self.nl();
        self.text("}");
    }

    fn member_field(&mut self, h: NodeHandle<FieldDeclNode>) {
        let span = self.a.span(h.id());
        self.begin_el(span, Begin::Stmt);
        {
            let d = self.a.field_decl(h).clone();
            if let Some(v) = d.vis.and_then(vis_opt) {
                self.text(v);
                self.sp();
            }
            if d.is_static {
                self.text("static ");
            }
            self.text(self.a.name(d.name));
            self.text(": ");
            self.ty(d.ty);
            if let Some(i) = d.init {
                self.text(" = ");
                self.expr(i);
            }
            self.text(";");
        }
        self.end_el(span);
    }

    fn member_method(&mut self, h: NodeHandle<MethodDeclNode>) {
        let span = self.a.span(h.id());
        self.begin_el(span, Begin::Stmt);
        self.method_decl(h);
        self.end_el(span);
    }
}

fn linkage_str(l: Linkage) -> &'static str {
    match l {
        Linkage::Host => "host",
        // the two strict spellings: ambient and import-gated
        Linkage::Builtin { ambient: true } => "prelude builtin",
        Linkage::Builtin { ambient: false } => "pub builtin",
    }
}

// ---- types, params, fn/method headers ----

impl<'a> P<'a> {
    /// one type, flat (types never wrap — a type spelling is a naming
    /// position, never a value flow)
    fn ty(&mut self, h: NodeHandle<AnyTy>) {
        match self.a.ty(h).clone() {
            TypeKind::TyPath { segs } => {
                for (i, seg) in segs.iter().enumerate() {
                    if i > 0 {
                        self.text(".");
                    }
                    self.text(self.a.name(seg.name));
                    if !seg.generics.is_empty() {
                        self.text("<");
                        let gs: Vec<NodeHandle<AnyTy>> = seg.generics.clone();
                        for (j, g) in gs.iter().enumerate() {
                            if j > 0 {
                                self.text(", ");
                            }
                            self.ty(*g);
                        }
                        self.text(">");
                    }
                }
            }
            TypeKind::TyFn { params, ret } => {
                self.text("fn(");
                let ps: Vec<NodeHandle<AnyTy>> = params.clone();
                for (i, p) in ps.iter().enumerate() {
                    if i > 0 {
                        self.text(", ");
                    }
                    self.ty(*p);
                }
                self.text(") -> ");
                self.ty(ret);
            }
            TypeKind::TyOpt { inner } => {
                self.text("?");
                self.ty(inner);
            }
            TypeKind::TyArray { elem } => {
                self.text("[");
                self.ty(elem);
                self.text("]");
            }
            TypeKind::TyTuple { elems } => {
                // >= 2 always — the one-element form is a grouping and
                // never becomes a TyTuple node (the parser unfolds it)
                self.text("(");
                let es: Vec<NodeHandle<AnyTy>> = elems.clone();
                for (i, e) in es.iter().enumerate() {
                    if i > 0 {
                        self.text(", ");
                    }
                    self.ty(*e);
                }
                self.text(")");
            }
            TypeKind::TyConst(e) => {
                self.expr(e);
            }
            TypeKind::TyUnion { elems } => {
                let es: Vec<NodeHandle<AnyTy>> = elems.clone();
                for (i, e) in es.iter().enumerate() {
                    if i > 0 {
                        self.text(" | ");
                    }
                    self.ty(*e);
                }
            }
        }
    }

    /// one parameter: `mut self` / `self` / `mut x: T` / `x: T` / `x`
    fn param(&mut self, h: NodeHandle<AnyParam>) {
        match self.a.param(h).clone() {
            MemberKind::SelfParam(d) => {
                if d.is_mut {
                    self.text("mut ");
                }
                self.text("self");
            }
            MemberKind::Param(d) => {
                if d.is_mut {
                    self.text("mut ");
                }
                self.text(self.a.name(d.name));
                if let Some(t) = d.ty {
                    self.text(": ");
                    self.ty(t);
                }
            }
            MemberKind::FieldDecl(_) | MemberKind::MethodDecl(_) => {}
        }
    }

    fn params_list(&mut self, params: &[NodeHandle<AnyParam>], _enclosing: Span) {
        self.text("(");
        if params.is_empty() {
            self.text(")");
            return;
        }
        let spans: Vec<Span> = params.iter().map(|&p| self.a.span(p.id())).collect();
        self.break_list(params, &spans, true, |p, it| p.param(*it));
        self.text(")");
    }

    /// the generic list with per-name bounds: `<K requires H, V>`
    fn gen_bounds(&mut self, gens: &[IdentId], bounds: &[(IdentId, NodeHandle<AnyTy>)]) {
        if gens.is_empty() {
            return;
        }
        self.text("<");
        for (i, g) in gens.iter().enumerate() {
            if i > 0 {
                self.text(", ");
            }
            self.text(self.a.name(*g));
            if let Some((_, t)) = bounds.iter().find(|(b, _)| *b == *g) {
                self.sp();
                self.text("requires");
                self.sp();
                self.ty(*t);
            }
        }
        self.text(">");
    }

    fn fn_header(&mut self, f: &FnData, with_vis: bool) {
        if with_vis {
            if let Some(v) = vis_opt(f.vis) {
                self.text(v);
                self.sp();
            }
        }
        if f.is_async {
            self.text("async ");
        }
        if f.entry {
            self.text("entry ");
        }
        self.text("fn ");
        self.text(self.a.name(f.name));
        self.gen_bounds(&f.generics, &f.bounds);
        self.text("(");
        let spans: Vec<Span> = f.params.iter().map(|&p| self.a.span(p.id())).collect();
        let ps: Vec<NodeHandle<AnyParam>> = f.params.clone();
        self.break_list(&ps, &spans, true, |p, it| p.param(*it));
        self.text(")");
        if let Some(r) = f.ret {
            self.text(" -> ");
            self.ty(r);
        }
    }

    fn method_decl(&mut self, h: NodeHandle<MethodDeclNode>) {
        let d = self.a.method_decl(h).clone();
        // the bracket marker comes FIRST (marker-before-visibility)
        if let Some(mk) = d.marker {
            self.text("[");
            self.text(self.a.name(mk));
            self.text("]");
            self.sp();
        }
        if let Some(v) = d.vis.and_then(vis_opt) {
            self.text(v);
            self.sp();
        }
        if d.is_async {
            self.text("async ");
        }
        self.text("fn ");
        self.text(self.a.name(d.name));
        self.gen_bounds(&d.generics, &d.bounds);
        self.text("(");
        let spans: Vec<Span> = d.params.iter().map(|&p| self.a.span(p.id())).collect();
        let ps: Vec<NodeHandle<AnyParam>> = d.params.clone();
        self.break_list(&ps, &spans, true, |p, it| p.param(*it));
        self.text(")");
        if let Some(r) = d.ret {
            self.text(" -> ");
            self.ty(r);
        }
        match d.body {
            Some(b) => {
                self.sp();
                self.block(b);
            }
            None => {
                self.text(";");
            }
        }
    }

    /// a bare rhs (return's) — the two-mode law without the `=`
    fn rhs_value(&mut self, value: NodeHandle<AnyExpr>) {
        let m = self.stamp();
        self.in_flat += 1;
        self.sp();
        self.expr(value);
        self.in_flat -= 1;
        if self.over_on_flat(m) {
            self.rollback(m);
            self.nl_band();
            self.expr(value);
            self.level -= 1;
        }
    }

    /// `= <expr>` with the two-mode law: flat try (inner breaks
    /// suppressed), rollback to `=` then the value on a continuation
    /// line whose break points are armed
    fn rhs_eq(&mut self, value: NodeHandle<AnyExpr>) {
        let m = self.stamp();
        self.in_flat += 1;
        self.sp();
        self.text("=");
        self.sp();
        self.expr(value);
        self.in_flat -= 1;
        if self.over_on_flat(m) {
            self.rollback(m);
            self.sp(); // pad off `>` — a flush `>` + `=` re-lexes as `>=`
            self.text("=");
            self.nl_band();
            self.expr(value);
            self.level -= 1;
        }
    }
}

impl<'a> P<'a> {
    /// `a, b` / broken one-per-line — the use/enum/ident list (no
    /// comment drains: those lists carry no anchors; the enclosing
    /// item's end drain owns any stray run)
    fn ident_list(&mut self, names: &[String], _spans: &[Span], _at: Span) {
        let m = self.stamp();
        for (i, n) in names.iter().enumerate() {
            if i > 0 {
                self.text(", ");
            }
            self.text(n);
        }
        if self.in_flat > 0 {
            return;
        }
        if self.over_on_flat(m) {
            self.rollback(m);
            self.nl_band();
            for (i, n) in names.iter().enumerate() {
                if i > 0 {
                    self.nl();
                }
                self.text(n);
                self.text(",");
            }
            self.level -= 1;
            self.nl();
        }
    }
}

// ---- statements + blocks + arms ----

impl<'a> P<'a> {
    /// one statement: BEGIN drain (+ separator blank law), the tokens,
    /// END (trailing comments ride the statement's line, then newline).
    /// In inline-flat contexts (arm/lambda bodies) the drains and the
    /// end hooks are suppressed — the enclosing arm/lambda's own end
    /// drain owns the comment queue there.
    fn stmt(&mut self, h: NodeHandle<AnyStmt>) {
        let span = self.a.span(h.id());
        if self.in_flat > 0 {
            self.stmt_tokens(h);
            return;
        }
        self.begin_el(span, Begin::Stmt);
        self.stmt_tokens(h);
        self.end_el(span);
    }

    fn stmt_tokens(&mut self, h: NodeHandle<AnyStmt>) {
        match self.a.stmt(h).clone() {
            StmtKind::LetStmt { is_mut, name, destructure, ty, init } => {
                self.text("let");
                if is_mut {
                    self.sp();
                    self.text("mut");
                }
                match destructure {
                    Some(names) => {
                        self.sp();
                        self.text("(");
                        let ns: Vec<String> = names.iter().map(|&n| self.a.name(n).to_string()).collect();
                        for (i, n) in ns.iter().enumerate() {
                            if i > 0 {
                                self.text(", ");
                            }
                            self.text(n);
                        }
                        self.text(")");
                    }
                    None => {
                        self.sp();
                        self.text(self.a.name(name));
                    }
                }
                if let Some(t) = ty {
                    self.text(": ");
                    self.ty(t);
                }
                self.rhs_eq(init);
                self.text(";");
            }
            StmtKind::If { cond, then, els } => {
                self.if_stmt(cond, then, els);
            }
            StmtKind::While { cond, body } => {
                self.text("while (");
                self.expr(cond);
                self.text(")");
                self.sp();
                self.block(body);
            }
            StmtKind::ForOf { var, iter, body } => {
                self.text("for (let ");
                self.text(self.a.name(var));
                self.text(" of ");
                self.expr(iter);
                self.text(")");
                self.sp();
                self.block(body);
            }
            StmtKind::ForC { var, init, cond, update, body } => {
                self.text("for (let ");
                self.text(self.a.name(var));
                self.text(" = ");
                self.expr(init);
                self.text("; ");
                self.expr(cond);
                self.text("; ");
                self.expr(update);
                self.text(")");
                self.sp();
                self.block(body);
            }
            StmtKind::Return { value } => {
                self.text("return");
                if let Some(v) = value {
                    self.rhs_value(v);
                }
                self.text(";");
            }
            StmtKind::Break => {
                self.text("break;");
            }
            StmtKind::Continue => {
                self.text("continue;");
            }
            StmtKind::WhenStmt { scrut, arms } => {
                self.text("when (");
                self.expr(scrut);
                self.text(")");
                self.sp();
                self.arms(&arms, self.a.span(h.id()));
            }
            StmtKind::ExprStmt(e) => {
                self.expr(e);
                self.text(";");
            }
        }
    }

    fn if_stmt(
        &mut self,
        cond: NodeHandle<AnyExpr>,
        then: NodeHandle<BlockNode>,
        els: Option<ElseBranch>,
    ) {
        self.text("if (");
        self.expr(cond);
        self.text(")");
        self.sp();
        self.block(then);
        if let Some(e) = els {
            self.sp();
            self.text("else ");
            match e {
                ElseBranch::If(h2) => {
                    if let StmtKind::If { cond: c2, then: t2, els: e2 } = self.a.stmt(h2.into()).clone() {
                        self.if_stmt(c2, t2, e2);
                    }
                }
                ElseBranch::Block(b) => {
                    self.block(b);
                }
            }
        }
    }

    /// the multiline block: ` {` statements ` }` — the drains ride the
    /// per-statement begin/end hooks; the close drains the runs pending
    /// between the last statement and the bracket.
    fn block(&mut self, h: NodeHandle<BlockNode>) {
        if self.in_flat > 0 {
            // the inline-flat shape (arm/lambda bodies inside flat contexts)
            self.text("{");
            for s in self.a.block(h) {
                self.sp();
                self.stmt_tokens(*s);
            }
            self.text(" }");
            return;
        }
        self.text("{");
        self.passed = self.passed.max(self.a.span(h.id()).lo);
        self.level += 1;
        self.nl();
        let stmts = self.a.block(h).to_vec();
        for s in &stmts {
            self.stmt(*s);
        }
        self.drain_close(self.content_end(self.a.span(h.id())));
        self.level -= 1;
        self.nl();
        self.text("}");
    }

    /// the arm list — ALWAYS one per line (the shape): pattern
    /// alternatives, `->`, the body (flat attempt with rollback to a
    /// broken block), trailing comma, then the line's own comments.
    fn arms(&mut self, arms: &[NodeHandle<AnyArm>], node: Span) {
        self.text("{");
        self.level += 1;
        self.nl();
        // the WHEN-arm's node span is DISPLACED (built from the bare
        // current token = the NEXT arm's opener), so each arm's begin
        // bound is the PREVIOUS arm's displaced span.lo — which IS this
        // arm's real first token
        let mut floor = self.passed;
        for a in arms {
            let span = self.a.span(a.id());
            self.begin_el(Span::new(floor, floor), Begin::Stmt);
            match self.a.arm(*a).clone() {
                ArmKind::WhenArm { pats, body } => {
                    for (i, p) in pats.iter().enumerate() {
                        if i > 0 {
                            self.text(", ");
                        }
                        self.pat(*p);
                    }
                    self.sp();
                    self.text("->");
                    self.sp();
                    self.arm_body(body, span);
                }
            }
            self.text(",");
            // the drain anchors on the cursor — the body's expr() just
            // boosted `passed` to the body's last token
            self.drain_trailing_bare(self.passed);
            self.nl();
            floor = span.lo;
        }
        self.drain_close(self.content_end(node));
        self.level -= 1;
        self.nl();
        self.text("}");
    }

    /// the arm body: a Block tries inline-flat (`{ x(); }`, the corpus's
    /// short-arm shape) and falls back to the multiline block; anything
    /// else prints as an expression.
    fn arm_body(&mut self, body: NodeHandle<AnyExpr>, _arm_span: Span) {
        let Some(b) = self.a.narrow_block(body) else {
            self.expr(body);
            return;
        };
        let stmts = self.a.block(b).to_vec();
        if self.flat_safe(&stmts) {
            let m = self.stamp();
            self.in_flat += 1;
            self.text("{");
            for s in &stmts {
                self.sp();
                self.stmt_tokens(*s);
            }
            self.text(" }");
            self.in_flat -= 1;
            if !self.over_on_flat(m) {
                self.passed = self.passed.max(self.content_end(self.a.span(b.id())));
                return;
            }
            self.rollback(m);
        }
        self.block(b);
        self.passed = self.passed.max(self.content_end(self.a.span(b.id())));
    }

    /// inline-flat eligibility: only terminal statements whose
    /// expressions contain no block-holding constructs (when
    /// trees, nested blocks, block-bodied lambdas)
    fn flat_safe(&self, stmts: &[NodeHandle<AnyStmt>]) -> bool {
        stmts.iter().all(|s| match self.a.stmt(*s).clone() {
            StmtKind::Return { value } => value.map_or(true, |v| self.expr_flat(v)),
            StmtKind::ExprStmt(e) => self.expr_flat(e),
            StmtKind::Break | StmtKind::Continue => true,
            StmtKind::LetStmt { init, .. } => self.expr_flat(init),
            _ => false,
        })
    }

    fn expr_flat(&self, h: NodeHandle<AnyExpr>) -> bool {
        match self.a.expr(h) {
            ExprKind::Block { .. } | ExprKind::WhenExpr { .. } => false,
            ExprKind::Lambda { body, .. } => self.a.narrow_block(*body).is_none(),
            _ => true,
        }
    }

    // ---- patterns ----

    fn pat(&mut self, h: NodeHandle<AnyPat>) {
        match self.a.pat(h).clone() {
            PatKind::PatLit(e) => {
                self.expr(e);
            }
            PatKind::PatPath { segs } => {
                self.path(&segs);
            }
            PatKind::PatCtor { segs, args } => {
                self.path(&segs);
                if !args.is_empty() {
                    self.text("(");
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            self.text(", ");
                        }
                        match arg {
                            Some(id) => self.text(self.a.name(*id)),
                            None => self.text("_"),
                        }
                    }
                    self.text(")");
                }
            }
            PatKind::PatWild => {
                self.text("_");
            }
            PatKind::PatElse => {
                self.text("else");
            }
        }
    }

    fn path(&mut self, segs: &[PathSeg]) {
        for (i, seg) in segs.iter().enumerate() {
            if i > 0 {
                self.text(".");
            }
            self.text(self.a.name(seg.name));
            if !seg.generics.is_empty() {
                self.text("<");
                let gs: Vec<NodeHandle<AnyTy>> = seg.generics.clone();
                for (j, g) in gs.iter().enumerate() {
                    if j > 0 {
                        self.text(", ");
                    }
                    self.ty(*g);
                }
                self.text(">");
            }
        }
    }
}

// ---- expressions ----

impl<'a> P<'a> {
    fn expr(&mut self, h: NodeHandle<AnyExpr>) {
        self.begin_el(self.a.span(h.id()), Begin::Expr);
        self.expr_tokens(h);
        self.passed = self.passed.max(self.content_end(self.a.span(h.id())));
    }

    /// the child-paren law: a child parenthesizes only where flat
    /// precedence would invert the parse's grouping (the fmt survey
    /// §2's table; the VM pins referee)
    fn needs_parens(&self, h: NodeHandle<AnyExpr>, parent: u8, rhs: bool) -> bool {
        match self.a.expr(h) {
            ExprKind::Binary { op, .. } => {
                let l = binop_level(*op);
                l < parent || (l == parent && rhs)
            }
            ExprKind::Is { .. } => L_CMP < parent || (L_CMP == parent && rhs),
            ExprKind::Cast { .. } => L_CAST < parent || (L_CAST == parent && rhs),
            ExprKind::Assign { .. } => true,
            // a FIELD/INDEX chain as a CALLEE names the value through the
            // chain — the call grammar takes an operand, so the chain
            // needs its parens back (`(self.drive)(emit)`, `(xs[i])(a)`)
            ExprKind::Field { .. } | ExprKind::Index { .. } if parent == L_POSTFIX => true,
            _ => false,
        }
    }

    /// a child in a constrained position (operand, callee, receiver,
    /// index): parens where the law asks
    fn operand(&mut self, h: NodeHandle<AnyExpr>, parent: u8, rhs: bool) {
        let needs = self.needs_parens(h, parent, rhs);
        if needs {
            self.text("(");
        }
        self.expr(h);
        if needs {
            self.text(")");
        }
    }

    fn expr_tokens(&mut self, h: NodeHandle<AnyExpr>) {
        match self.a.expr(h).clone() {
            ExprKind::Block { stmts } => {
                // expression-position block — a lambda/anon body or a
                // grouped block; multiline always at statement level
                if self.in_flat > 0 {
                    self.text("{");
                    for s in &stmts {
                        self.sp();
                        self.stmt_tokens(*s);
                    }
                    self.text(" }");
                    return;
                }
                self.sp();
                self.text("{");
                self.level += 1;
                self.nl();
                for s in &stmts {
                    self.stmt(*s);
                }
                self.drain_close(self.content_end(self.a.span(h.id())));
                self.level -= 1;
                self.nl();
                self.text("}");
            }
            ExprKind::Lit(l) => {
                self.lit(&l);
            }
            ExprKind::AsyncBlock { body } => {
                // `async { .. }` — the async primitive; the block always
                // breaks (a mint is a statement-sized thing)
                let stmts: Vec<_> = self.a.block(body).to_vec();
                self.text("async ");
                self.text("{");
                self.level += 1;
                self.nl();
                for s in stmts {
                    self.stmt(s);
                }
                self.drain_close(self.a.span(body.id()).hi);
                self.level -= 1;
                self.nl();
                self.text("}");
            }
            ExprKind::Path { segs } => {
                self.path(&segs);
            }
            ExprKind::Call { callee, args } => {
                self.operand(callee, L_POSTFIX, false);
                self.text("(");
                let spans: Vec<Span> = args.iter().map(|&e| self.a.span(e.id())).collect();
                self.break_list(&args, &spans, true, |p, e| p.expr(*e));
                self.drain_close(self.a.span(h.id()).hi);
                self.text(")");
            }
            ExprKind::Method { recv, name, generics, args } => {
                self.operand(recv, L_POSTFIX, false);
                self.text(".");
                self.text(self.a.name(name));
                if !generics.is_empty() {
                    self.text("<");
                    for (i, g) in generics.iter().enumerate() {
                        if i > 0 {
                            self.text(", ");
                        }
                        self.ty(*g);
                    }
                    self.text(">");
                }
                self.text("(");
                let spans: Vec<Span> = args.iter().map(|&e| self.a.span(e.id())).collect();
                self.break_list(&args, &spans, true, |p, e| p.expr(*e));
                self.drain_close(self.a.span(h.id()).hi);
                self.text(")");
            }
            ExprKind::Field { recv, name } => {
                self.operand(recv, L_POSTFIX, false);
                self.text(".");
                self.text(self.a.name(name));
            }
            ExprKind::Index { recv, idx } => {
                self.operand(recv, L_POSTFIX, false);
                self.text("[");
                self.expr(idx);
                self.drain_close(self.a.span(h.id()).hi);
                self.text("]");
            }
            ExprKind::Unary { op, expr } => {
                self.text(unop_str(op));
                self.operand(expr, L_UNARY, false);
            }
            ExprKind::Binary { op, lhs, rhs } => {
                let lvl = binop_level(op);
                // flat try with the children's own breaks suppressed
                let m = self.stamp();
                self.in_flat += 1;
                self.operand(lhs, lvl, false);
                self.sp();
                self.text(binop_str(op));
                self.sp();
                self.operand(rhs, lvl, true);
                self.in_flat -= 1;
                if self.over_on_flat(m) {
                    self.rollback(m);
                    self.operand(lhs, lvl, false);
                    self.sp();
                    self.text(binop_str(op));
                    self.nl_band();
                    self.operand(rhs, lvl, true);
                    self.level -= 1;
                }
            }
            ExprKind::Assign { op, target, value } => {
                self.operand(target, L_POSTFIX, false);
                let m = self.stamp();
                self.in_flat += 1;
                self.sp();
                self.text(assign_str(op));
                self.sp();
                self.operand(value, 0, false);
                self.in_flat -= 1;
                if self.over_on_flat(m) {
                    self.rollback(m);
                    self.sp();
                    self.text(assign_str(op));
                    self.nl_band();
                    self.operand(value, 0, false);
                    self.level -= 1;
                }
            }
            ExprKind::Lambda { params, ret, body } => {
                self.lambda(params, ret, body);
            }
            ExprKind::Try { expr } => {
                self.operand(expr, L_POSTFIX, false);
                self.text("?");
            }
            ExprKind::FStr { parts } => {
                self.text("f\"");
                for p in parts {
                    match p {
                        FPartAst::Lit(s) => {
                            let esc = escape_str(&s, true);
                            self.text(&esc);
                        }
                        FPartAst::Hole(e) => {
                            self.text("{");
                            self.expr(e);
                            self.text("}");
                        }
                    }
                }
                self.text("\"");
            }
            ExprKind::Struct { ty, fields } => {
                self.ty(ty);
                self.sp();
                self.text("{");
                let m = self.stamp();
                self.in_flat += 1;
                self.sp();
                for (i, (name, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.text(", ");
                    }
                    self.text(self.a.name(*name));
                    self.text(": ");
                    self.expr(*v);
                }
                self.in_flat -= 1;
                if self.over_on_flat(m) {
                    self.rollback(m);
                    self.nl_band();
                    for (name, v) in fields.iter() {
                        self.text(self.a.name(*name));
                        self.text(": ");
                        self.expr(*v);
                        self.text(",");
                        self.nl();
                    }
                    self.level -= 1;
                    self.nl();
                }
                self.sp();
                self.text("}");
            }
            ExprKind::Tuple { elems } => {
                self.text("(");
                let spans: Vec<Span> = elems.iter().map(|&e| self.a.span(e.id())).collect();
                self.break_list(&elems, &spans, false, |p, e| p.expr(*e));
                self.drain_close(self.a.span(h.id()).hi);
                self.text(")");
            }
            ExprKind::ArrayLit { elems } => {
                self.text("[");
                let spans: Vec<Span> = elems.iter().map(|&e| self.a.span(e.id())).collect();
                self.break_list(&elems, &spans, true, |p, e| p.expr(*e));
                self.drain_close(self.a.span(h.id()).hi);
                self.text("]");
            }
            ExprKind::ArrayRepeat { value, count } => {
                self.text("[");
                self.expr(value);
                self.text("; ");
                self.expr(count);
                self.text("]");
            }
            ExprKind::WhenExpr { scrut, arms } => {
                self.text("when (");
                self.expr(scrut);
                self.text(")");
                self.sp();
                self.arms(&arms, self.a.span(h.id()));
            }
            ExprKind::Await { expr } => {
                self.text("await ");
                self.operand(expr, L_UNARY, false);
            }
            ExprKind::Is { expr, ty } => {
                self.operand(expr, L_CMP, false);
                self.sp();
                self.text("is");
                self.sp();
                self.ty(ty);
            }
            ExprKind::Cast { expr, ty } => {
                self.operand(expr, L_CAST, false);
                self.sp();
                self.text("as");
                self.sp();
                self.ty(ty);
            }
        }
    }

    /// the lambda's two spellings (the parser's two forms):
    /// - one type-less non-mut param, no ret → `x => body`;
    /// - a Block body otherwise → `fn (params) (-> ret)? { .. }` (the
    ///   anon form, block-bodied by grammar);
    /// - an expr body otherwise → `(params) (-> ret)? => body` (the
    ///   paren form, `=>`-connected by grammar).
    fn lambda(
        &mut self,
        params: Vec<NodeHandle<AnyParam>>,
        ret: Option<NodeHandle<AnyTy>>,
        body: NodeHandle<AnyExpr>,
    ) {
        if params.len() == 1 && ret.is_none() {
            if let MemberKind::Param(d) = self.a.param(params[0]) {
                let d = d.clone();
                if d.ty.is_none() && !d.is_mut {
                    self.text(self.a.name(d.name));
                    self.sp();
                    self.text("=>");
                    self.sp();
                    self.expr(body);
                    return;
                }
            }
        }
        let block_body = self.a.narrow_block(body).is_some();
        if block_body {
            self.text("fn ");
        }
        self.text("(");
        let spans: Vec<Span> = params.iter().map(|&p| self.a.span(p.id())).collect();
        self.break_list(&params, &spans, true, |p, it| p.param(*it));
        self.text(")");
        if !block_body {
            self.sp();
            self.text("=>");
            self.sp();
            self.expr(body);
            return;
        }
        if let Some(r) = ret {
            self.text(" -> ");
            self.ty(r);
        }
        self.sp();
        self.lambda_block(body);
    }

    /// the lambda's block body: inline-flat attempt (the corpus's
    /// `fn (x: i32) -> i32 { return x + 1; }` shape), broken fallback
    fn lambda_block(&mut self, body: NodeHandle<AnyExpr>) {
        let Some(b) = self.a.narrow_block(body) else {
            self.expr(body);
            return;
        };
        let stmts = self.a.block(b).to_vec();
        if self.flat_safe(&stmts) {
            let m = self.stamp();
            self.in_flat += 1;
            self.text("{");
            for s in &stmts {
                self.sp();
                self.stmt_tokens(*s);
            }
            self.text(" }");
            self.in_flat -= 1;
            if !self.over_on_flat(m) {
                return;
            }
            self.rollback(m);
        }
        self.block(b);
    }

    fn lit(&mut self, l: &Lit) {
        match l {
            Lit::Int(v, sfx) => {
                let s = format!("{}{}", v, int_suffix(*sfx));
                self.text(&s);
            }
            Lit::Float(bits, sfx) => {
                // the shortest round-trip digits; a float with no `.`
                // and no exponent would re-lex as a SUFFIXED INT, so
                // `.0` rides whenever the digits came out integral
                let val = f64::from_bits(*bits);
                let digits = match *sfx {
                    Some(FloatSuffix::F32) => {
                        format!("{}", f32::from_bits((val as f32).to_bits()))
                    }
                    _ => format!("{}", val),
                };
                let s = if digits.contains('.') || digits.contains('e') || digits.contains("inf")
                    || digits.contains("NaN")
                {
                    format!("{digits}{}", float_suffix(*sfx))
                } else {
                    format!("{digits}.0{}", float_suffix(*sfx))
                };
                self.text(&s);
            }
            Lit::Str(s) => {
                let esc = escape_str(s, false);
                self.text(&format!("\"{}\"", esc));
            }
            Lit::RawStr(s) => {
                // no escape processing, no hash-quoted form in the
                // lexer — the content can never contain `"` (the raw
                // body closes at the first one)
                self.text(&format!("r\"{}\"", s));
            }
            Lit::Bool(b) => {
                self.text(if *b { "true" } else { "false" });
            }
            Lit::Nil => {
                self.text("nil");
            }
        }
    }
}
