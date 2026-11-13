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
//!
//! **Spans (v2)**: node spans are EXACT — a node ends at its own last
//! token (`prev_end`), no next-token tail — so `span.hi` IS the
//! element's content end (`content_end`). TWO survivors of the v1
//! overshoot: the when-ARM's span (built from the bare current token
//! after its comma = the NEXT arm's opener — the arm walk keeps its
//! `floor` correction) and the BLOCK's (built from the token AFTER the
//! `}` — `block_end` walks the token stream back onto the `}`).

use rut_ast::*;
use rut_lexer::token::{FloatSuffix, IntSuffix};

use crate::comments::CommentRun;
use crate::style::Style;

// operator levels — the parser's reduce levels, mined from
// rut-ast/src/parser/expr.rs:247-316 (the fmt survey §2 table)
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
        // (rut-ast/src/parser/expr.rs:266-276)
        _ => "=",
    }
}

fn vis_str(v: Vis) -> &'static str {
    match v {
        Vis::Pub => "pub",
        Vis::Super => "pub(super)",
        Vis::Pkg => "pub(pkg)",
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
fn escape_str(body: &str, in_fstring: bool, literal_newline: bool) -> String {
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
            '\n' => {
                if literal_newline {
                    // the hash-leveled multi-line forms emit the newline
                    // for real — that is their point
                    s.push('\n');
                } else {
                    s.push_str("\\n");
                }
            }
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

/// Canonical raw-literal spelling: `r"..."` while the value fits (no
/// newline, no quote); otherwise the hash-leveled multi-line form at the
/// MINIMAL level — one past the longest `#` run that follows any `"` in
/// the value (that run is what a close scan would trip on). A pure
/// function of the value: parse∘print round-trips and fmt stays
/// idempotent.
fn raw_repr(s: &str) -> String {
    if !s.contains('\n') && !s.contains('"') {
        return format!("r\"{s}\"");
    }
    let b = s.as_bytes();
    let mut level = 1usize;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' {
            let mut run = 0usize;
            let mut j = i + 1;
            while j < b.len() && b[j] == b'#' {
                run += 1;
                j += 1;
            }
            level = level.max(run + 1);
            i = j;
        } else {
            i += 1;
        }
    }
    let hashes = "#".repeat(level);
    format!("r{hashes}\"{s}\"{hashes}")
}

#[derive(Clone, Copy)]
struct Mark {
    len: usize,
    ri: usize,
    passed: u32,
    /// the literal-newline counter at stamp time — `over_on_flat`
    /// discounts newlines that came from multi-line literal content
    lit_nl: usize,
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
    /// `block_end` reads it (block spans are the one v2 overshoot:
    /// built from the token AFTER the `}`, so the block's true end
    /// comes from the tokens, not the span)
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
    /// newlines emitted INSIDE multi-line literal content (`r#"..."#`,
    /// `f#"..."#`) — the flat-attempt probe discounts them: a multi-line
    /// literal is an atom that fits, not a child break
    lit_nl: usize,
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
            lit_nl: 0,
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

    /// the element's TRUE content end. v2 node spans are EXACT (the
    /// item-span law: a node ends at its OWN last consumed token —
    /// `prev_end`, no next-token tail), so `span.hi` IS the content
    /// end; the v1 token-walk that peeled the overshoot off is gone
    /// from this lane (it survives, block-only, as [`Self::block_end`]).
    fn content_end(&self, span: Span) -> u32 {
        span.hi
    }

    /// a BLOCK's true content end. Block spans are the one v2
    /// overshoot that survives: `BlockFrame` builds them from
    /// `p.span().hi` AFTER the `}` is eaten — the NEXT token's hi (the
    /// zero-width Eof token's at the file end) — so the `}` is the
    /// token just before span.hi. The walk: skip the one token ending
    /// EXACTLY at span.hi that starts after span.lo (tokens are
    /// disjoint — at most one can end at any byte), landing on the
    /// block's own `}`.
    fn block_end(&self, span: Span) -> u32 {
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

    /// emit literal content that may carry REAL newlines (the
    /// hash-leveled multi-line forms); the count rides `lit_nl` so the
    /// flat-attempt probe can discount them
    fn text_lit(&mut self, s: &str) {
        self.lit_nl += s.matches('\n').count();
        self.text(s);
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
        Mark {
            len: self.out.len(),
            ri: self.ri,
            passed: self.passed,
            lit_nl: self.lit_nl,
        }
    }

    fn rollback(&mut self, m: Mark) {
        self.out.truncate(m.len);
        self.ri = m.ri;
        self.passed = m.passed;
        self.lit_nl = m.lit_nl;
        self.pending = if self.out.is_empty() || self.out.ends_with('\n') {
            Some(self.level)
        } else {
            None
        };
    }

    /// flat attempt failed: a newline appeared inside it (a comment or
    /// a child break), or the resulting line exceeds the width. Newlines
    /// inside multi-line literal content do NOT count — the literal is
    /// an atom that fits wherever its first line does.
    fn over_on_flat(&self, m: Mark) -> bool {
        let nls = self.out[m.len..].matches('\n').count();
        if nls > self.lit_nl - m.lit_nl {
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
        while let Some(r) = self.runs.get(self.ri).cloned() {
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
            eprintln!(
                "[list] commented={} over={} col={} chunk={:?}",
                commented,
                self.over_on_flat(m),
                self.col(),
                self.out[m.len..].chars().take(46).collect::<String>()
            );
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
        let items: Vec<ItemRef> = self.a.root().items(self.a).to_vec();
        for it in items {
            self.item(it);
        }
        self.drain_tail();
        self.out
    }

    /// one item: BEGIN drain (+ separator blank rules), the tokens, END
    fn item(&mut self, it: ItemRef) {
        let span = it.span(self.a);
        self.begin_el(span, Begin::Stmt);
        match it {
            ItemRef::Module(h) => {
                // reachable only via the root — walk it the same way
                let items: Vec<ItemRef> = h.get(self.a).items.to_vec();
                for it in items {
                    self.item(it);
                }
            }
            ItemRef::ModDecl(h) => {
                let d = h.get(self.a);
                if let Some(v) = vis_opt(d.vis()) {
                    self.text(v);
                    self.sp();
                }
                self.text("mod ");
                self.text(self.a.name(d.name));
                self.text(";");
            }
            ItemRef::ModInline(h) => {
                let d = h.get(self.a);
                // the declaration file's inline module (wire v12): the
                // nested declarations print through the item walk
                if let Some(v) = vis_opt(d.vis()) {
                    self.text(v);
                    self.sp();
                }
                self.text("mod ");
                self.text(self.a.name(d.name));
                self.text(" {");
                let items: Vec<ItemRef> = d.items.to_vec();
                for it in items {
                    self.item(it);
                }
                self.text("}");
            }
            ItemRef::Use(h) => {
                let d = h.get(self.a);
                // the full path — package, modules, in order — then the
                // leaf names in braces; `pub use` re-exports print their
                // prefix
                if let Some(v) = vis_opt(d.vis()) {
                    self.text(v);
                    self.sp();
                }
                self.text("use ");
                for (i, seg) in d.path.iter().enumerate() {
                    if i > 0 {
                        self.text("::");
                    }
                    self.text(self.a.name(*seg));
                }
                if d.names.is_empty() {
                    self.text(";");
                } else {
                    self.text("::{");
                    self.sp();
                    let nm: Vec<String> = d
                        .names
                        .iter()
                        .map(|&n| self.a.name(n).to_string())
                        .collect();
                    let dummies: Vec<Span> = std::iter::repeat_n(span, nm.len()).collect();
                    self.ident_list(&nm, &dummies, span);
                    self.sp();
                    self.text("};");
                }
            }
            ItemRef::Alias(h) => {
                let d = h.get(self.a);
                if let Some(v) = vis_opt(d.vis()) {
                    self.text(v);
                    self.sp();
                }
                self.text("type ");
                self.text(self.a.name(d.name));
                self.text(" = ");
                self.ty(d.target);
                self.text(";");
            }
            ItemRef::ModuleLet(h) => {
                let d = h.get(self.a);
                if let Some(v) = vis_opt(d.vis()) {
                    self.text(v);
                    self.sp();
                }
                self.text("let ");
                self.text(self.a.name(d.name));
                if let Some(t) = d.ty {
                    self.text(": ");
                    self.ty(t);
                }
                self.rhs_eq(d.init);
                self.text(";");
            }
            ItemRef::Enum(h) => {
                let d = h.get(self.a);
                if let Some(v) = vis_opt(d.vis()) {
                    self.text(v);
                    self.sp();
                }
                self.text("enum ");
                self.text(self.a.name(d.name));
                self.text(" {");
                let names: Vec<String> = d
                    .members
                    .iter()
                    .filter_map(|m| m.as_member())
                    .map(|mh| self.a.name(mh.get(self.a).name).to_string())
                    .collect();
                if !names.is_empty() {
                    self.sp();
                    let dummies: Vec<Span> = std::iter::repeat_n(span, names.len()).collect();
                    self.ident_list(&names, &dummies, span);
                }
                self.sp();
                self.text("}");
            }
            ItemRef::Struct(h) => {
                let d = h.get(self.a);
                if let Some(v) = vis_opt(d.vis()) {
                    self.text(v);
                    self.sp();
                }
                self.text("struct ");
                self.ty_decl(d.name, &d.generics, &[]);
                self.body_of_fields_methods(&d.fields, &d.methods, Some(span));
            }
            ItemRef::Class(h) => {
                let d = h.get(self.a);
                if let Some(v) = vis_opt(d.vis()) {
                    self.text(v);
                    self.sp();
                }
                self.text("class ");
                self.ty_decl(d.name, &d.generics, &d.requires);
                // the positional one-field decl prints back in its
                // spelling — never as the desugared braced body
                if d.newtype {
                    if let Some(f) = d.fields.first().and_then(|f| f.as_field()) {
                        let fd = f.get(self.a);
                        self.text("(");
                        self.ty(fd.ty);
                        self.text(");");
                        return;
                    }
                }
                self.body_of_fields_methods(&d.fields, &d.methods, Some(span));
            }
            ItemRef::Interface(h) => {
                let d = h.get(self.a);
                if let Some(v) = vis_opt(d.vis()) {
                    self.text(v);
                    self.sp();
                }
                self.text("interface ");
                self.text(self.a.name(d.name));
                if !d.generics.is_empty() {
                    self.text("<");
                    let gnames = d
                        .generics
                        .iter()
                        .map(|&g| self.a.name(g).to_string())
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.text(&gnames);
                    self.text(">");
                }
                self.iface_body(&d.methods, span);
            }
            ItemRef::Impl(h) => {
                let d = h.get(self.a);
                self.text("impl");
                if !d.generics.is_empty() {
                    self.gen_only(&d.generics);
                    self.sp();
                } else {
                    self.sp();
                }
                self.ty(d.target);
                self.methods_body(&d.methods, span);
            }
            ItemRef::Fn(h) => {
                let f = h.get(self.a);
                self.fn_header(f, true);
                self.sp();
                self.block(f.body);
            }
            ItemRef::SurfaceFn(h) => {
                let d = h.get(self.a);
                if let Some(v) = vis_opt(d.vis()) {
                    self.text(v);
                    self.sp();
                }
                self.text(linkage_str(d.linkage()));
                self.sp();
                if d.is_async() {
                    self.text("async ");
                }
                self.text("fn ");
                self.text(self.a.name(d.name));
                self.gen_only(&d.generics);
                self.params_list(&d.params, span);
                if let Some(r) = d.ret {
                    self.text(" -> ");
                    self.ty(r);
                }
                self.text(";");
            }
            ItemRef::SurfaceStruct(h) => {
                let d = h.get(self.a);
                if let Some(v) = vis_opt(d.vis()) {
                    self.text(v);
                    self.sp();
                }
                self.text("host struct ");
                self.text(self.a.name(d.name));
                self.fields_body(&d.fields, Some(span));
            }
            ItemRef::BuiltinTy(h) => {
                let d = h.get(self.a);
                // the type-shaped builtin items carry the strict spelling
                // they were declared with (`prelude builtin` ambient /
                // `pub builtin` import-gated)
                self.text(if d.ambient() {
                    "prelude builtin class "
                } else {
                    "pub builtin class "
                });
                self.text(self.a.name(d.name));
                self.gen_only(&d.generics);
                self.iface_body(&d.members, span);
            }
            ItemRef::BuiltinPrimitive(h) => {
                let d = h.get(self.a);
                self.text(if d.ambient() {
                    "prelude builtin primitive "
                } else {
                    "pub builtin primitive "
                });
                self.text(self.a.name(d.name));
                self.iface_body(&d.members, span);
            }
            ItemRef::BuiltinImpl(h) => {
                let d = h.get(self.a);
                self.text("prelude builtin impl ");
                self.text(self.a.name(d.prim));
                self.iface_body(&d.methods, span);
            }
            // the refusal gate (`is_clean`) means no Error slot can
            // reach the printer; the arm is the exhaustiveness law
            ItemRef::Error(_) => {}
        }
        self.end_el(span);
    }

    fn ty_decl(&mut self, name: IdentId, gens: &[IdentId], requires: &[(IdentId, TyRef)]) {
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
        let gs = gens
            .iter()
            .map(|&g| self.a.name(g).to_string())
            .collect::<Vec<_>>()
            .join(", ");
        self.text(&gs);
        self.text(">");
    }

    fn body_of_fields_methods(
        &mut self,
        fields: &[FieldRef],
        methods: &[MethodRef],
        enclosing: Option<Span>,
    ) {
        self.sp();
        self.text("{");
        if let Some(e) = enclosing {
            self.passed = self.passed.max(e.lo);
        }
        self.level += 1;
        self.nl();
        for f in fields {
            self.member_field(*f);
        }
        for m in methods {
            self.member_method(*m);
        }
        if let Some(e) = enclosing {
            self.drain_close(self.content_end(e));
        }
        self.level -= 1;
        self.nl();
        self.text("}");
    }

    fn fields_body(&mut self, fields: &[FieldRef], enclosing: Option<Span>) {
        self.sp();
        self.text("{");
        if let Some(e) = enclosing {
            self.passed = self.passed.max(e.lo);
        }
        self.level += 1;
        self.nl();
        for f in fields {
            self.member_field(*f);
        }
        if let Some(e) = enclosing {
            self.drain_close(self.content_end(e));
        }
        self.level -= 1;
        self.nl();
        self.text("}");
    }

    fn iface_body(&mut self, methods: &[MethodRef], enclosing: Span) {
        self.sp();
        self.text("{");
        self.passed = self.passed.max(enclosing.lo);
        self.level += 1;
        self.nl();
        for m in methods {
            self.member_method(*m);
        }
        self.drain_close(self.content_end(enclosing));
        self.level -= 1;
        self.nl();
        self.text("}");
    }

    fn methods_body(&mut self, methods: &[MethodRef], enclosing: Span) {
        self.sp();
        self.text("{");
        self.passed = self.passed.max(enclosing.lo);
        self.level += 1;
        self.nl();
        for m in methods {
            self.member_method(*m);
        }
        self.drain_close(self.content_end(enclosing));
        self.level -= 1;
        self.nl();
        self.text("}");
    }

    fn member_field(&mut self, m: FieldRef) {
        let span = m.span(self.a);
        self.begin_el(span, Begin::Stmt);
        if let FieldRef::Field(h) = m {
            let d = h.get(self.a);
            if let Some(v) = d.vis().and_then(vis_opt) {
                self.text(v);
                self.sp();
            }
            if d.is_static() {
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

    fn member_method(&mut self, m: MethodRef) {
        let span = m.span(self.a);
        self.begin_el(span, Begin::Stmt);
        if let MethodRef::Method(h) = m {
            self.method_decl(h);
        }
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
    fn ty(&mut self, t: TyRef) {
        match t {
            TyRef::Path(h) => {
                let d = h.get(self.a);
                for (i, seg) in d.segs.iter().enumerate() {
                    if i > 0 {
                        self.text("::");
                    }
                    self.text(self.a.name(seg.name));
                    if !seg.generics.is_empty() {
                        self.text("<");
                        for (j, g) in seg.generics.iter().enumerate() {
                            if j > 0 {
                                self.text(", ");
                            }
                            self.ty(*g);
                        }
                        self.text(">");
                    }
                }
            }
            TyRef::Fn(h) => {
                let d = h.get(self.a);
                self.text("fn(");
                for (i, p) in d.params.iter().enumerate() {
                    if i > 0 {
                        self.text(", ");
                    }
                    self.ty(*p);
                }
                self.text(") -> ");
                self.ty(d.ret);
            }
            TyRef::Opt(h) => {
                self.text("?");
                self.ty(h.get(self.a).inner);
            }
            TyRef::Array(h) => {
                self.text("[");
                self.ty(h.get(self.a).elem);
                self.text("]");
            }
            TyRef::Tuple(h) => {
                let d = h.get(self.a);
                // >= 2 always — the one-element form is a grouping and
                // never becomes a TyTuple node (the parser unfolds it)
                self.text("(");
                for (i, e) in d.elems.iter().enumerate() {
                    if i > 0 {
                        self.text(", ");
                    }
                    self.ty(*e);
                }
                self.text(")");
            }
            TyRef::Const(h) => {
                self.expr(h.get(self.a).expr);
            }
            TyRef::Union(h) => {
                let d = h.get(self.a);
                for (i, e) in d.elems.iter().enumerate() {
                    if i > 0 {
                        self.text(" | ");
                    }
                    self.ty(*e);
                }
            }
            TyRef::Error(_) => {}
        }
    }

    /// one parameter: `mut self` / `self` / `mut x: T` / `x: T` / `x`
    fn param(&mut self, pr: ParamRef) {
        match pr {
            ParamRef::SelfParam(h) => {
                let d = h.get(self.a);
                if d.is_mut {
                    self.text("mut ");
                }
                self.text("self");
            }
            ParamRef::Param(h) => {
                let d = h.get(self.a);
                if d.is_mut {
                    self.text("mut ");
                }
                self.text(self.a.name(d.name));
                if let Some(t) = d.ty {
                    self.text(": ");
                    self.ty(t);
                }
            }
            ParamRef::Error(_) => {}
        }
    }

    fn params_list(&mut self, params: &[ParamRef], _enclosing: Span) {
        self.text("(");
        if params.is_empty() {
            self.text(")");
            return;
        }
        let spans: Vec<Span> = params.iter().map(|&p| p.span(self.a)).collect();
        self.break_list(params, &spans, true, |p, it| p.param(*it));
        self.text(")");
    }

    /// the generic list with per-name bounds: `<K requires H, V>`
    fn gen_bounds(&mut self, gens: &[IdentId], bounds: &[(IdentId, TyRef)]) {
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

    fn fn_header(&mut self, f: &FnNode, with_vis: bool) {
        if with_vis {
            if let Some(v) = vis_opt(f.vis()) {
                self.text(v);
                self.sp();
            }
        }
        if f.is_async() {
            self.text("async ");
        }
        if f.entry() {
            self.text("entry ");
        }
        self.text("fn ");
        self.text(self.a.name(f.name));
        self.gen_bounds(&f.generics, &f.bounds);
        self.text("(");
        let spans: Vec<Span> = f.params.iter().map(|&p| p.span(self.a)).collect();
        self.break_list(&f.params, &spans, true, |p, it| p.param(*it));
        self.text(")");
        if let Some(r) = f.ret {
            self.text(" -> ");
            self.ty(r);
        }
    }

    fn method_decl(&mut self, h: Handle<MethodDeclNode>) {
        let d = h.get(self.a);
        // the bracket marker comes FIRST (marker-before-visibility)
        if let Some(mk) = d.marker() {
            self.text("[");
            self.text(self.a.name(mk));
            self.text("]");
            self.sp();
        }
        if let Some(v) = d.vis().and_then(vis_opt) {
            self.text(v);
            self.sp();
        }
        if d.is_async() {
            self.text("async ");
        }
        self.text("fn ");
        self.text(self.a.name(d.name));
        self.gen_bounds(&d.generics, &d.bounds);
        self.text("(");
        let spans: Vec<Span> = d.params.iter().map(|&p| p.span(self.a)).collect();
        self.break_list(&d.params, &spans, true, |p, it| p.param(*it));
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
    fn rhs_value(&mut self, value: ExprRef) {
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
    fn rhs_eq(&mut self, value: ExprRef) {
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
    fn stmt(&mut self, s: StmtRef) {
        let span = s.span(self.a);
        if self.in_flat > 0 {
            self.stmt_tokens(s);
            return;
        }
        self.begin_el(span, Begin::Stmt);
        self.stmt_tokens(s);
        self.end_el(span);
    }

    fn stmt_tokens(&mut self, s: StmtRef) {
        match s {
            StmtRef::Let(h) => {
                let d = h.get(self.a);
                self.text("let");
                if d.is_mut {
                    self.sp();
                    self.text("mut");
                }
                match &d.destructure {
                    Some(names) => {
                        self.sp();
                        self.text("(");
                        for (i, n) in names.iter().enumerate() {
                            if i > 0 {
                                self.text(", ");
                            }
                            self.text(self.a.name(*n));
                        }
                        self.text(")");
                    }
                    None => {
                        self.sp();
                        self.text(self.a.name(d.name));
                    }
                }
                if let Some(t) = d.ty {
                    self.text(": ");
                    self.ty(t);
                }
                self.rhs_eq(d.init);
                self.text(";");
            }
            StmtRef::If(h) => {
                let d = h.get(self.a);
                self.if_stmt(d.cond, d.then, d.els);
            }
            StmtRef::While(h) => {
                let d = h.get(self.a);
                self.text("while (");
                self.expr(d.cond);
                self.text(")");
                self.sp();
                self.block(d.body);
            }
            StmtRef::ForOf(h) => {
                let d = h.get(self.a);
                self.text("for (let ");
                if d.is_mut {
                    self.text("mut ");
                }
                self.text(self.a.name(d.var));
                self.text(" of ");
                self.expr(d.iter);
                self.text(")");
                self.sp();
                self.block(d.body);
            }
            StmtRef::ForC(h) => {
                let d = h.get(self.a);
                self.text("for (let ");
                if d.is_mut {
                    self.text("mut ");
                }
                self.text(self.a.name(d.var));
                self.text(" = ");
                self.expr(d.init);
                self.text("; ");
                self.expr(d.cond);
                self.text("; ");
                self.expr(d.update);
                self.text(")");
                self.sp();
                self.block(d.body);
            }
            StmtRef::Return(h) => {
                let d = h.get(self.a);
                self.text("return");
                if let Some(v) = d.value {
                    self.rhs_value(v);
                }
                self.text(";");
            }
            StmtRef::Break(_) => {
                self.text("break;");
            }
            StmtRef::Continue(_) => {
                self.text("continue;");
            }
            StmtRef::When(h) => {
                let d = h.get(self.a);
                self.text("when (");
                self.expr(d.scrut);
                self.text(")");
                self.sp();
                self.arms(&d.arms, d.span);
            }
            StmtRef::Expr(e) => {
                self.expr_tokens(e);
                self.text(";");
            }
            StmtRef::Error(_) => {}
        }
    }

    fn if_stmt(&mut self, cond: ExprRef, then: Handle<BlockNode>, els: Option<ElseBranch>) {
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
                    let d = h2.get(self.a);
                    self.if_stmt(d.cond, d.then, d.els);
                }
                ElseBranch::Block(b) => {
                    self.block(b);
                }
            }
        }
    }

    /// the multiline block: ` {` statements [` ` tail] ` }` — the
    /// drains ride the per-statement begin/end hooks (the tail gets the
    /// same element hooks, minus the `;`); the close drains the runs
    /// pending between the last element and the bracket.
    fn block(&mut self, h: Handle<BlockNode>) {
        let b = h.get(self.a);
        let stmts: Vec<StmtRef> = b.stmts.to_vec();
        let tail = b.tail;
        if self.in_flat > 0 {
            // the inline-flat shape (arm/lambda bodies inside flat contexts)
            self.text("{");
            for s in &stmts {
                self.sp();
                self.stmt_tokens(*s);
            }
            if let Some(t) = tail {
                self.sp();
                self.expr_tokens(t);
            }
            self.text(" }");
            return;
        }
        self.text("{");
        self.passed = self.passed.max(h.span(self.a).lo);
        self.level += 1;
        self.nl();
        for s in &stmts {
            self.stmt(*s);
        }
        if let Some(t) = tail {
            // the block's value: printed like a statement (the comment
            // drains ride it) but without the `;`
            let span = t.span(self.a);
            self.begin_el(span, Begin::Stmt);
            self.expr_tokens(t);
            self.end_el(span);
        }
        self.drain_close(self.block_end(h.span(self.a)));
        self.level -= 1;
        self.nl();
        self.text("}");
    }

    /// the arm list — ALWAYS one per line (the shape): pattern
    /// alternatives, `->`, the body (flat attempt with rollback to a
    /// broken block), trailing comma, then the line's own comments.
    fn arms(&mut self, arms: &[Handle<WhenArmNode>], node: Span) {
        self.text("{");
        self.level += 1;
        self.nl();
        // the WHEN-arm's node span is DISPLACED (built from the bare
        // current token = the NEXT arm's opener), so each arm's begin
        // bound is the PREVIOUS arm's displaced span.lo — which IS this
        // arm's real first token
        let mut floor = self.passed;
        for a in arms {
            let span = a.span(self.a);
            self.begin_el(Span::new(floor, floor), Begin::Stmt);
            let arm = a.get(self.a);
            for (i, p) in arm.pats.iter().enumerate() {
                if i > 0 {
                    self.text(", ");
                }
                self.pat(*p);
            }
            self.sp();
            self.text("->");
            self.sp();
            self.arm_body(arm.body);
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
    /// else prints as an expression. A tail rides the flat attempt (no
    /// `;`) — a block-holding tail forces the broken shape.
    fn arm_body(&mut self, body: ExprRef) {
        let ExprRef::Block(b) = body else {
            self.expr(body);
            return;
        };
        let blk = b.get(self.a);
        let stmts: Vec<StmtRef> = blk.stmts.to_vec();
        let tail = blk.tail;
        let flat_ok = self.flat_safe(&stmts) && tail.is_none_or(|t| self.expr_flat(t));
        if flat_ok {
            let m = self.stamp();
            self.in_flat += 1;
            self.text("{");
            for s in &stmts {
                self.sp();
                self.stmt_tokens(*s);
            }
            if let Some(t) = tail {
                self.sp();
                self.expr_tokens(t);
            }
            self.text(" }");
            self.in_flat -= 1;
            if !self.over_on_flat(m) {
                self.passed = self.passed.max(self.block_end(b.span(self.a)));
                return;
            }
            self.rollback(m);
        }
        self.block(b);
        self.passed = self.passed.max(self.block_end(b.span(self.a)));
    }

    /// inline-flat eligibility: only terminal statements whose
    /// expressions contain no block-holding constructs (when
    /// trees, nested blocks, block-bodied lambdas)
    fn flat_safe(&self, stmts: &[StmtRef]) -> bool {
        stmts.iter().all(|s| match *s {
            StmtRef::Return(h) => h.get(self.a).value.is_none_or(|v| self.expr_flat(v)),
            StmtRef::Expr(e) => self.expr_flat(e),
            StmtRef::Break(_) | StmtRef::Continue(_) => true,
            StmtRef::Let(h) => self.expr_flat(h.get(self.a).init),
            _ => false,
        })
    }

    fn expr_flat(&self, h: ExprRef) -> bool {
        match h {
            ExprRef::Block(_) | ExprRef::WhenExpr(_) | ExprRef::If(_) => false,
            ExprRef::Lambda(lh) => !matches!(lh.get(self.a).body, ExprRef::Block(_)),
            _ => true,
        }
    }

    // ---- patterns ----

    fn pat(&mut self, p: PatRef) {
        match p {
            PatRef::Lit(h) => {
                self.expr(h.get(self.a).expr);
            }
            PatRef::Path(h) => {
                self.path(&h.get(self.a).segs);
            }
            PatRef::Ctor(h) => {
                let d = h.get(self.a);
                self.path(&d.segs);
                if !d.args.is_empty() {
                    self.text("(");
                    for (i, arg) in d.args.iter().enumerate() {
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
            PatRef::Wild(_) => {
                self.text("_");
            }
            PatRef::Else(_) => {
                self.text("else");
            }
            PatRef::Error(_) => {}
        }
    }

    fn path(&mut self, segs: &[PathSeg]) {
        for (i, seg) in segs.iter().enumerate() {
            if i > 0 {
                self.text("::");
            }
            self.text(self.a.name(seg.name));
            if !seg.generics.is_empty() {
                self.text("<");
                for (j, g) in seg.generics.iter().enumerate() {
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
    fn expr(&mut self, h: ExprRef) {
        self.begin_el(h.span(self.a), Begin::Expr);
        self.expr_tokens(h);
        self.passed = self.passed.max(self.content_end(h.span(self.a)));
    }

    /// the child-paren law: a child parenthesizes only where flat
    /// precedence would invert the parse's grouping (the fmt survey
    /// §2's table; the VM pins referee)
    fn needs_parens(&self, h: ExprRef, parent: u8, rhs: bool) -> bool {
        match h {
            ExprRef::Binary(bh) => {
                let d = bh.get(self.a);
                let l = binop_level(d.op);
                l < parent || (l == parent && rhs)
            }
            ExprRef::Is(_) => L_CMP < parent || (L_CMP == parent && rhs),
            ExprRef::Cast(_) => L_CAST < parent || (L_CAST == parent && rhs),
            ExprRef::Assign(_) => true,
            // a FIELD/INDEX chain as a CALLEE names the value through the
            // chain — the call grammar takes an operand, so the chain
            // needs its parens back (`(self.drive)(emit)`, `(xs[i])(a)`)
            ExprRef::Field(_) | ExprRef::Index(_) if parent == L_POSTFIX => true,
            _ => false,
        }
    }

    /// a child in a constrained position (operand, callee, receiver,
    /// index): parens where the law asks
    fn operand(&mut self, h: ExprRef, parent: u8, rhs: bool) {
        let needs = self.needs_parens(h, parent, rhs);
        if needs {
            self.text("(");
        }
        self.expr(h);
        if needs {
            self.text(")");
        }
    }

    fn expr_tokens(&mut self, h: ExprRef) {
        match h {
            ExprRef::Block(b) => {
                // expression-position block — a lambda/anon body or a
                // grouped block; multiline always at statement level.
                // The tail prints without the `;` (the block's value).
                let blk = b.get(self.a);
                let stmts: Vec<StmtRef> = blk.stmts.to_vec();
                let tail = blk.tail;
                if self.in_flat > 0 {
                    self.text("{");
                    for s in &stmts {
                        self.sp();
                        self.stmt_tokens(*s);
                    }
                    if let Some(t) = tail {
                        self.sp();
                        self.expr_tokens(t);
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
                if let Some(t) = tail {
                    let span = t.span(self.a);
                    self.begin_el(span, Begin::Stmt);
                    self.expr_tokens(t);
                    self.end_el(span);
                }
                self.drain_close(self.block_end(b.span(self.a)));
                self.level -= 1;
                self.nl();
                self.text("}");
            }
            ExprRef::Lit(h) => {
                self.lit(&h.get(self.a).lit);
            }
            ExprRef::AsyncBlock(h) => {
                let d = h.get(self.a);
                // `async { .. }` — the async primitive; the block always
                // breaks (a mint is a statement-sized thing)
                let blk = d.body.get(self.a);
                let stmts: Vec<StmtRef> = blk.stmts.to_vec();
                self.text("async ");
                self.text("{");
                self.level += 1;
                self.nl();
                for s in stmts {
                    self.stmt(s);
                }
                self.drain_close(d.body.span(self.a).hi);
                self.level -= 1;
                self.nl();
                self.text("}");
            }
            ExprRef::Path(h) => {
                self.path(&h.get(self.a).segs);
            }
            ExprRef::Call(h) => {
                let d = h.get(self.a);
                self.operand(d.callee, L_POSTFIX, false);
                self.text("(");
                let spans: Vec<Span> = d.args.iter().map(|&e| e.span(self.a)).collect();
                self.break_list(&d.args, &spans, true, |p, e| p.expr(*e));
                self.drain_close(h.span(self.a).hi);
                self.text(")");
            }
            ExprRef::Method(h) => {
                let d = h.get(self.a);
                self.operand(d.recv, L_POSTFIX, false);
                self.text(".");
                self.text(self.a.name(d.name));
                if !d.generics.is_empty() {
                    self.text("<");
                    for (i, g) in d.generics.iter().enumerate() {
                        if i > 0 {
                            self.text(", ");
                        }
                        self.ty(*g);
                    }
                    self.text(">");
                }
                self.text("(");
                let spans: Vec<Span> = d.args.iter().map(|&e| e.span(self.a)).collect();
                self.break_list(&d.args, &spans, true, |p, e| p.expr(*e));
                self.drain_close(h.span(self.a).hi);
                self.text(")");
            }
            ExprRef::Field(h) => {
                let d = h.get(self.a);
                self.operand(d.recv, L_POSTFIX, false);
                self.text(".");
                self.text(self.a.name(d.name));
            }
            ExprRef::Index(h) => {
                let d = h.get(self.a);
                self.operand(d.recv, L_POSTFIX, false);
                self.text("[");
                self.expr(d.idx);
                self.drain_close(h.span(self.a).hi);
                self.text("]");
            }
            ExprRef::Unary(h) => {
                let d = h.get(self.a);
                self.text(unop_str(d.op));
                self.operand(d.expr, L_UNARY, false);
            }
            ExprRef::Binary(h) => {
                let d = h.get(self.a);
                let op = d.op;
                let lvl = binop_level(op);
                // flat try with the children's own breaks suppressed
                let m = self.stamp();
                self.in_flat += 1;
                self.operand(d.lhs, lvl, false);
                self.sp();
                self.text(binop_str(op));
                self.sp();
                self.operand(d.rhs, lvl, true);
                self.in_flat -= 1;
                if self.over_on_flat(m) {
                    self.rollback(m);
                    self.operand(d.lhs, lvl, false);
                    self.sp();
                    self.text(binop_str(op));
                    self.nl_band();
                    self.operand(d.rhs, lvl, true);
                    self.level -= 1;
                }
            }
            ExprRef::Assign(h) => {
                let d = h.get(self.a);
                self.operand(d.target, L_POSTFIX, false);
                let m = self.stamp();
                self.in_flat += 1;
                self.sp();
                self.text(assign_str(d.op));
                self.sp();
                self.operand(d.value, 0, false);
                self.in_flat -= 1;
                if self.over_on_flat(m) {
                    self.rollback(m);
                    self.sp();
                    self.text(assign_str(d.op));
                    self.nl_band();
                    self.operand(d.value, 0, false);
                    self.level -= 1;
                }
            }
            ExprRef::Lambda(h) => {
                let d = h.get(self.a);
                self.lambda(&d.params, d.ret, d.body);
            }
            ExprRef::Try(h) => {
                let d = h.get(self.a);
                self.operand(d.expr, L_POSTFIX, false);
                self.text("?");
            }
            ExprRef::FStr(h) => {
                let d = h.get(self.a);
                // a newline in any chunk promotes the literal to the
                // hash-leveled multi-line form — the template's line
                // structure survives formatting
                let multiline = d
                    .parts
                    .iter()
                    .any(|p| matches!(p, FPartAst::Lit(s) if s.contains('\n')));
                let (open, close) = if multiline {
                    ("f#\"", "\"#")
                } else {
                    ("f\"", "\"")
                };
                self.text_lit(open);
                for p in &d.parts {
                    match p {
                        FPartAst::Lit(s) => {
                            let esc = escape_str(s, true, multiline);
                            self.text_lit(&esc);
                        }
                        FPartAst::Hole(e) => {
                            self.text("{");
                            self.expr(*e);
                            self.text("}");
                        }
                    }
                }
                self.text_lit(close);
            }
            ExprRef::Struct(h) => {
                let d = h.get(self.a);
                self.ty(d.ty);
                self.sp();
                self.text("{");
                let m = self.stamp();
                self.in_flat += 1;
                self.sp();
                for (i, (name, v)) in d.fields.iter().enumerate() {
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
                    for (name, v) in d.fields.iter() {
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
            ExprRef::Tuple(h) => {
                let d = h.get(self.a);
                self.text("(");
                let spans: Vec<Span> = d.elems.iter().map(|&e| e.span(self.a)).collect();
                self.break_list(&d.elems, &spans, false, |p, e| p.expr(*e));
                self.drain_close(h.span(self.a).hi);
                self.text(")");
            }
            ExprRef::ArrayLit(h) => {
                let d = h.get(self.a);
                self.text("[");
                let spans: Vec<Span> = d.elems.iter().map(|&e| e.span(self.a)).collect();
                self.break_list(&d.elems, &spans, true, |p, e| p.expr(*e));
                self.drain_close(h.span(self.a).hi);
                self.text("]");
            }
            ExprRef::ArrayRepeat(h) => {
                let d = h.get(self.a);
                self.text("[");
                self.expr(d.value);
                self.text("; ");
                self.expr(d.count);
                self.text("]");
            }
            ExprRef::WhenExpr(h) => {
                let d = h.get(self.a);
                self.text("when (");
                self.expr(d.scrut);
                self.text(")");
                self.sp();
                self.arms(&d.arms, d.span);
            }
            ExprRef::If(h) => {
                let d = h.get(self.a);
                // the expression form prints like the statement form (one
                // spelling); `else if` links hold if STATEMENTS — the
                // same recursion if_stmt already speaks
                self.if_stmt(d.cond, d.then, d.els);
            }
            ExprRef::Await(h) => {
                let d = h.get(self.a);
                self.text("await ");
                self.operand(d.expr, L_UNARY, false);
            }
            ExprRef::Is(h) => {
                let d = h.get(self.a);
                self.operand(d.expr, L_CMP, false);
                self.sp();
                self.text("is");
                self.sp();
                self.ty(d.ty);
            }
            ExprRef::Cast(h) => {
                let d = h.get(self.a);
                self.operand(d.expr, L_CAST, false);
                self.sp();
                self.text("as");
                self.sp();
                self.ty(d.ty);
            }
            ExprRef::Error(_) => {}
        }
    }

    /// the lambda's two spellings (the parser's two forms):
    /// - one type-less non-mut param, no ret → `x => body`;
    /// - a Block body otherwise → `fn (params) (-> ret)? { .. }` (the
    ///   anon form, block-bodied by grammar);
    /// - an expr body otherwise → `(params) (-> ret)? => body` (the
    ///   paren form, `=>`-connected by grammar).
    fn lambda(&mut self, params: &[ParamRef], ret: Option<TyRef>, body: ExprRef) {
        if params.len() == 1 && ret.is_none() {
            if let ParamRef::Param(ph) = params[0] {
                let d = ph.get(self.a);
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
        let block_body = matches!(body, ExprRef::Block(_));
        if block_body {
            self.text("fn ");
        }
        self.text("(");
        let spans: Vec<Span> = params.iter().map(|&p| p.span(self.a)).collect();
        self.break_list(params, &spans, true, |p, it| p.param(*it));
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
    /// `fn (x: i32) -> i32 { return x + 1; }` shape), broken fallback.
    /// The tail rides the flat attempt without the `;`.
    fn lambda_block(&mut self, body: ExprRef) {
        let ExprRef::Block(b) = body else {
            self.expr(body);
            return;
        };
        let blk = b.get(self.a);
        let stmts: Vec<StmtRef> = blk.stmts.to_vec();
        let tail = blk.tail;
        let flat_ok = self.flat_safe(&stmts) && tail.is_none_or(|t| self.expr_flat(t));
        if flat_ok {
            let m = self.stamp();
            self.in_flat += 1;
            self.text("{");
            for s in &stmts {
                self.sp();
                self.stmt_tokens(*s);
            }
            if let Some(t) = tail {
                self.sp();
                self.expr_tokens(t);
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
                // `.0` rides whenever the digits came out integral.
                // The NON-FINITE family reprints BARE: the digits are
                // `inf`/`NaN` (the ident-fold literal's shape), no
                // suffix spelling exists for them (`inff64` does not
                // lex), and the parser's ident lane re-reads the bare
                // word exactly.
                let val = f64::from_bits(*bits);
                let digits = match *sfx {
                    Some(FloatSuffix::F32) => {
                        format!("{}", f32::from_bits((val as f32).to_bits()))
                    }
                    _ => format!("{}", val),
                };
                let s = if digits.contains("inf") || digits.contains("NaN") {
                    digits
                } else if digits.contains('.') || digits.contains('e') {
                    format!("{digits}{}", float_suffix(*sfx))
                } else {
                    format!("{digits}.0{}", float_suffix(*sfx))
                };
                self.text(&s);
            }
            Lit::Str(s) => {
                let esc = escape_str(s, false, false);
                self.text(&format!("\"{}\"", esc));
            }
            Lit::RawStr(s) => {
                // hash-leveled repr — level 0 while the value fits (no
                // newline, no quote), else the minimal multi-line level
                // (`raw_repr`); content prints verbatim, never re-indented
                self.text_lit(&raw_repr(s));
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
