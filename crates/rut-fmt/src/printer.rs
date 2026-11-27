//! The printer — the AST reprint (`pretty(ast)`).
//!
//! Structure is source-order (the parser is monotone, so a
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
//! **Parens**: the AST has no paren nodes; the printer keeps a child's
//! group exactly where the child's bare spelling would not re-parse to
//! the same node in the role it sits in (law 1 — the role table in
//! `Role`). The matrix gate (`tests/paren_matrix.rs`) referees the
//! table cell by cell; the VM-verified suite
//! (`tests/semantic_equiv.rs`) referees the programs.
//!
//! **Literals print verbatim**: every `LitNode` carries its exact
//! token span, so atoms slice their spelling out of the normalized
//! source (`P::src_slice`) — radix, underscores, exponent shapes,
//! escape spellings, and raw-string hash levels survive byte-exact
//! (the LSP verbatim law's fmt twin). The f-string composite slices
//! its delimiters and chunk text and reprints only the hole
//! expressions (live subtrees — fmt's whole job); the walk is driven
//! by the hole expr spans, never by brace counting. Nothing is
//! re-derived: the `Lit` value payloads stay in the AST for the
//! compiler and fmt simply stops reading them.
//!
//! **Flat mode** (`in_flat`): when-arm INLINE bodies print
//! their statements without newline hooks — drains are suppressed (the
//! enclosing arm's end drain catches the runs; the block's own lines
//! flush the comments the scanner places on the arm's line; the comment
//! positions were trailing by scan).
//!
//! **The hole zone** (`hole_depth`): an f-string hole is a SINGLE-LINE
//! ZONE — the language's own law (`lex_hole` errors on a newline in a
//! hole at every hash level). The zone is hard: no layout newline may
//! fire inside (`nl`/`nl_band`/`blank` assert it), no try-flat site
//! can flip to broken while it is up (`over_on_flat`), and the
//! block-carrying atoms print the source's own flat spellings
//! (`{ p -> b, p -> b }`, `async { .. }`). A hole wider than the line
//! stays on one line; the enclosing statement breaks instead.
//!
//! **Spans (v2)**: node spans are EXACT — a node ends at its own last
//! token (`prev_end`), no next-token tail — so `span.hi` IS the
//! element's content end (`content_end`).

use rut_ast::*;

use crate::comments::CommentRun;
use crate::style::Style;

// operator levels — the parser's reduce levels, mined from
// rut-ast/src/parser/expr.rs:290-352 (the fmt survey §2 table)
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
const L_POSTFIX: u8 = 14; // call/index/field — children never anchor

/// the side of a binary operator a child sits on
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Lhs,
    Rhs,
}

/// the role a child expression sits in — the position decides whether
/// its bare spelling would re-parse to the same node
#[derive(Clone, Copy)]
enum Role {
    /// the callee of a call — an operand of the postfix loop
    Callee,
    /// the receiver of a `.field` / `[index]` / `.method()` postfix —
    /// a chain re-associates left, so a chain receiver drops its group
    Receiver,
    /// the target of an assignment — the parser takes whatever operand
    /// stood left of `=`, so the group never carries meaning
    AssignTarget,
    /// an operand of a binary operator
    Bin { op: BinOp, side: Side },
    /// the operand of a prefix `-` / `!`
    Unary,
    /// the operand of `as`
    CastOperand,
    /// the operand of `is`
    IsOperand,
    /// the operand of `await`
    AwaitOperand,
    /// the value of an assignment
    AssignValue,
}

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
        // (rut-ast/src/parser/expr.rs:364-376)
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
    /// the pipeline owner — `line_of` reads its own line geometry
    /// (the lex rode the same byte scan; no second table here)
    doc: &'a rut_ast::Doc,
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
    /// the hard-flat depth (an f-string hole): a hole is a SINGLE-LINE
    /// ZONE — no layout newline may fire inside (`nl`/`nl_band`/
    /// `blank` assert it), no try-flat site can flip to broken while
    /// it is up (`over_on_flat`), and the break machinery honors it
    /// through [`P::flat`]
    hole_depth: usize,
    /// newlines emitted INSIDE multi-line literal content (`r#"..."#`,
    /// `f#"..."#`) — the flat-attempt probe discounts them: a multi-line
    /// literal is an atom that fits, not a child break
    lit_nl: usize,
}

impl<'a> P<'a> {
    pub(super) fn new(
        a: &'a Ast,
        src: &'a str,
        style: &'a Style,
        runs: Vec<CommentRun>,
        doc: &'a rut_ast::Doc,
    ) -> P<'a> {
        P {
            a,
            style,
            src,
            doc,
            runs,
            ri: 0,
            passed: 0,
            out: String::new(),
            level: 0,
            pending: Some(0),
            in_flat: 0,
            hole_depth: 0,
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
        self.doc.line_of(off) as usize
    }

    /// the element's TRUE content end. v2 node spans are EXACT (the
    /// item-span law: a node ends at its OWN last consumed token —
    /// `prev_end`, no next-token tail), so `span.hi` IS the content
    /// end.
    fn content_end(&self, span: Span) -> u32 {
        span.hi
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
        assert_eq!(
            self.hole_depth, 0,
            "a layout newline inside an f-string hole — a hole is a \
             single-line zone"
        );
        self.trim_tail();
        if !self.out.ends_with('\n') {
            self.out.push('\n');
        }
        self.pending = Some(self.level);
    }

    /// one blank line between the finished content and the coming one
    /// (runs of source blanks collapse to one)
    fn blank(&mut self) {
        assert_eq!(
            self.hole_depth, 0,
            "a layout newline inside an f-string hole — a hole is a \
             single-line zone"
        );
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
    /// an atom that fits wherever its first line does. Inside a hole
    /// the attempt can never fail: the hard-flat zone has no broken
    /// fallback, so the probe stays false while it is up and the hole
    /// prints on one line however wide it is.
    fn over_on_flat(&self, m: Mark) -> bool {
        if self.hole_depth > 0 {
            return false;
        }
        let nls = self.out[m.len..].matches('\n').count();
        if nls > self.lit_nl - m.lit_nl {
            return true;
        }
        self.col() > self.style.max_width
    }

    /// the flat test: a soft inline-flat context (arm/lambda bodies,
    /// the rhs try-flats) OR the hole's hard-flat zone — the drains
    /// suppress and the two-mode break decisions skip
    fn flat(&self) -> bool {
        self.in_flat > 0 || self.hole_depth > 0
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
        if self.flat() {
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
        if self.flat() {
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
        assert_eq!(
            self.hole_depth, 0,
            "a layout newline inside an f-string hole — a hole is a \
             single-line zone"
        );
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
                self.text(self.a.name(d.name.ident));
                self.text(";");
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
                    self.text(self.a.name(seg.ident));
                }
                if d.names.is_empty() && d.group.is_none() {
                    self.text(";");
                } else {
                    self.text("::{");
                    self.sp();
                    let nm: Vec<String> = d
                        .names
                        .iter()
                        .map(|n| self.a.name(n.ident).to_string())
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
                self.text(self.a.name(d.name.ident));
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
                self.text(self.a.name(d.name.ident));
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
                self.text(self.a.name(d.name.ident));
                self.text(" {");
                let names: Vec<String> = d
                    .members
                    .iter()
                    .filter_map(|m| m.as_member())
                    .map(|mh| self.a.name(mh.get(self.a).name.ident).to_string())
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
                self.ty_decl(d.name.ident, &d.generics, &[]);
                self.body_of_fields_methods(&d.fields, &d.methods, Some(span));
            }
            ItemRef::Class(h) => {
                let d = h.get(self.a);
                if let Some(v) = vis_opt(d.vis()) {
                    self.text(v);
                    self.sp();
                }
                self.text("class ");
                self.ty_decl(d.name.ident, &d.generics, &d.requires);
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
                self.text(self.a.name(d.name.ident));
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
                self.text(self.a.name(d.name.ident));
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
                self.text(self.a.name(d.name.ident));
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
                self.text(self.a.name(d.name.ident));
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
                self.text(self.a.name(d.name.ident));
                self.iface_body(&d.members, span);
            }
            ItemRef::BuiltinImpl(h) => {
                let d = h.get(self.a);
                self.text("prelude builtin impl ");
                self.text(self.a.name(d.prim.ident));
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
            self.text(self.a.name(d.name.ident));
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
                    self.text(self.a.name(seg.name.ident));
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
            TyRef::Self_(_) => {
                self.text("Self");
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

    /// one parameter: `self` / `x: T` / `x` — params are mutable by
    /// default (no mutability spell exists on a param)
    fn param(&mut self, pr: ParamRef) {
        match pr {
            ParamRef::SelfParam(_) => {
                self.text("self");
            }
            ParamRef::Param(h) => {
                let d = h.get(self.a);
                self.text(self.a.name(d.name.ident));
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
        self.text(self.a.name(f.name.ident));
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
        self.text(self.a.name(d.name.ident));
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
        if self.flat() {
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
        if self.flat() {
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
                if d.is_const {
                    self.text("const");
                } else {
                    self.text("let");
                }
                match &d.destructure {
                    Some(names) => {
                        self.sp();
                        self.text("(");
                        for (i, n) in names.iter().enumerate() {
                            if i > 0 {
                                self.text(", ");
                            }
                            self.text(self.a.name(n.ident));
                        }
                        self.text(")");
                    }
                    None => {
                        self.sp();
                        self.text(self.a.name(d.name.ident));
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
                if d.is_const {
                    self.text("for (const ");
                } else {
                    self.text("for (let ");
                }
                self.text(self.a.name(d.var.ident));
                self.text(" of ");
                self.expr(d.iter);
                self.text(")");
                self.sp();
                self.block(d.body);
            }
            StmtRef::ForC(h) => {
                let d = h.get(self.a);
                self.text("for (let ");
                self.text(self.a.name(d.var.ident));
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
        if self.flat() {
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
        self.drain_close(h.span(self.a).hi);
        self.level -= 1;
        self.nl();
        self.text("}");
    }

    /// the arm list — ALWAYS one per line (the shape): pattern
    /// alternatives, `->`, the body (flat attempt with rollback to a
    /// broken block), trailing comma, then the line's own comments.
    /// Inside a hole the shape is the source's own flat spelling
    /// `{ p -> b, p -> b }` — the arm list's newlines would tear the
    /// hole (law 2).
    fn arms(&mut self, arms: &[Handle<WhenArmNode>], node: Span) {
        if self.hole_depth > 0 {
            self.text("{");
            self.sp();
            for (i, a) in arms.iter().enumerate() {
                if i > 0 {
                    self.text(", ");
                }
                let arm = a.get(self.a);
                for (j, p) in arm.pats.iter().enumerate() {
                    if j > 0 {
                        self.text(", ");
                    }
                    self.pat(*p);
                }
                self.sp();
                self.text("->");
                self.sp();
                self.arm_body(arm.body);
            }
            self.text(" }");
            return;
        }
        self.text("{");
        self.level += 1;
        self.nl();
        for a in arms {
            let span = a.span(self.a);
            self.begin_el(span, Begin::Stmt);
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
                self.passed = self.passed.max(b.span(self.a).hi);
                return;
            }
            self.rollback(m);
        }
        self.block(b);
        self.passed = self.passed.max(b.span(self.a).hi);
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
            self.text(self.a.name(seg.name.ident));
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

    /// the child-paren law: a paren is kept IFF the child's bare
    /// spelling would not re-parse to the same node in this role. One
    /// exhaustive match, role by role, no wildcard arm — a new
    /// `ExprRef` variant fails to compile here and joins the matrix
    /// gate (`tests/paren_matrix.rs`), the lane's referee.
    fn needs_parens(&self, h: ExprRef, role: Role) -> bool {
        // a lambda's body is a full expression and an assignment takes
        // whatever operand stood left of `=` — either bare spelling
        // re-nests the node in every role BUT the statement's value
        // position, where the statement already prints it bare (the
        // `let`'s own shape): there the group drops
        if matches!(h, ExprRef::Lambda(_) | ExprRef::Assign(_))
            && !matches!(role, Role::AssignValue)
        {
            return true;
        }
        match role {
            Role::Callee => match h {
                // a call's callee is an operand: a chain names its
                // value through the postfix loop (`(self.drive)(emit)`,
                // `(xs[i])(a)`), a composite literal leaves the atom
                // frame (`(P { a: 1 })(a)`), and a prefixed, awaited or
                // operator expression re-binds (`(-a)(1)`,
                // `(b as u64)(x)`)
                ExprRef::Field(_)
                | ExprRef::Index(_)
                | ExprRef::Struct(_)
                | ExprRef::Tuple(_)
                | ExprRef::ArrayLit(_)
                | ExprRef::ArrayRepeat(_)
                | ExprRef::Unary(_)
                | ExprRef::Await(_)
                | ExprRef::Binary(_)
                | ExprRef::Is(_)
                | ExprRef::Cast(_) => true,
                // a call or method chain, a literal, a path, `self`, an
                // f-string, a `when`/`if`/`async` atom — the bare
                // spelling re-parses to the same call
                ExprRef::Call(_)
                | ExprRef::Method(_)
                | ExprRef::Lit(_)
                | ExprRef::Path(_)
                | ExprRef::Self_(_)
                | ExprRef::FStr(_)
                | ExprRef::WhenExpr(_)
                | ExprRef::If(_)
                | ExprRef::AsyncBlock(_) => false,
                // unreachable (the early return above), spelled so the
                // match stays exhaustive with no wildcard arm
                ExprRef::Lambda(_) | ExprRef::Assign(_) => true,
                ExprRef::Block(_) | ExprRef::Error(_) => false,
            },
            Role::Receiver | Role::AssignTarget => match h {
                // a composite literal leaves the atom frame
                // (`(P { a: 1 }).a = b;`), a prefixed or awaited
                // expression re-nests (`(-a).x`, `(await f()).x`), and
                // an operator expression re-nests
                // (`(h ^ b as u32).wrapping_mul(..)`)
                ExprRef::Struct(_)
                | ExprRef::Tuple(_)
                | ExprRef::ArrayLit(_)
                | ExprRef::ArrayRepeat(_)
                | ExprRef::Unary(_)
                | ExprRef::Await(_)
                | ExprRef::Binary(_)
                | ExprRef::Is(_)
                | ExprRef::Cast(_) => true,
                // a chain re-associates left: `q.b.c` and `(q.b).c` are
                // the same tree, and so are `k[i].m()` and
                // `(k[i]).m()` — the group drops; so does a literal, a
                // path, a call, an f-string, a `when`/`if`/`async` atom
                ExprRef::Field(_)
                | ExprRef::Index(_)
                | ExprRef::Call(_)
                | ExprRef::Method(_)
                | ExprRef::Lit(_)
                | ExprRef::Path(_)
                | ExprRef::Self_(_)
                | ExprRef::FStr(_)
                | ExprRef::WhenExpr(_)
                | ExprRef::If(_)
                | ExprRef::AsyncBlock(_) => false,
                // unreachable (the early return above), spelled so the
                // match stays exhaustive with no wildcard arm
                ExprRef::Lambda(_) | ExprRef::Assign(_) => true,
                ExprRef::Block(_) | ExprRef::Error(_) => false,
            },
            Role::Bin { op, side } => {
                let parent = binop_level(op);
                let child = self.reduce_level(h);
                child < parent
                    || (child == parent && side == Side::Rhs)
                    // level 6 is not associative: a level-6 left operand
                    // keeps its group (`(a < b) < c`, `(a is i32) < b`,
                    // `(a < b) is i32`)
                    || (child == L_CMP && parent == L_CMP)
                    // in type position `<` opens generic arguments, so a
                    // cast's bare spelling on the left of `<` feeds the
                    // type parse (`(a as i64) < b`)
                    || (child == L_CAST && side == Side::Lhs && op == BinOp::Lt)
            }
            Role::Unary | Role::AwaitOperand => self.reduce_level(h) < L_UNARY,
            Role::CastOperand => self.reduce_level(h) < L_CAST,
            Role::IsOperand => {
                // level 6 is not associative: a level-6 operand keeps
                // its group (`(a < b) is i32`, `(a is i32) is i32`)
                self.reduce_level(h) <= L_CMP
            }
            // the value side takes the whole expression: no reduce
            // level anchors through it, so the bare spelling re-parses
            // whole — the statement's value position prints a lambda or
            // assignment bare (the `let`'s own shape)
            Role::AssignValue => false,
        }
    }

    /// the reduce level an operator child anchors — a non-operator
    /// child never anchors a reduce, so its bare spelling re-parses
    /// whole under every reduce-level operator
    fn reduce_level(&self, h: ExprRef) -> u8 {
        match h {
            ExprRef::Binary(bh) => binop_level(bh.get(self.a).op),
            ExprRef::Is(_) => L_CMP,
            ExprRef::Cast(_) => L_CAST,
            ExprRef::Block(_)
            | ExprRef::Lit(_)
            | ExprRef::Path(_)
            | ExprRef::Self_(_)
            | ExprRef::Call(_)
            | ExprRef::Method(_)
            | ExprRef::Field(_)
            | ExprRef::Index(_)
            | ExprRef::Unary(_)
            | ExprRef::Lambda(_)
            | ExprRef::Assign(_)
            | ExprRef::FStr(_)
            | ExprRef::Struct(_)
            | ExprRef::Tuple(_)
            | ExprRef::ArrayLit(_)
            | ExprRef::ArrayRepeat(_)
            | ExprRef::WhenExpr(_)
            | ExprRef::If(_)
            | ExprRef::Await(_)
            | ExprRef::AsyncBlock(_)
            | ExprRef::Error(_) => L_POSTFIX,
        }
    }

    /// a child in a constrained position (operand, callee, receiver,
    /// index): parens where the law asks
    fn operand(&mut self, h: ExprRef, role: Role) {
        let needs = self.needs_parens(h, role);
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
                if self.flat() {
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
                self.drain_close(b.span(self.a).hi);
                self.level -= 1;
                self.nl();
                self.text("}");
            }
            ExprRef::Lit(h) => {
                self.lit(h.get(self.a));
            }
            ExprRef::AsyncBlock(h) => {
                let d = h.get(self.a);
                // `async { .. }` — the async primitive. Inside a hole
                // the mint prints the source's own flat spelling (a
                // hole is a single-line zone); outside one the block
                // always breaks (a mint is a statement-sized thing).
                // The tail prints like a statement (the comment drains
                // ride it), minus the `;` — the whole mint prints,
                // every time.
                let blk = d.body.get(self.a);
                let stmts: Vec<StmtRef> = blk.stmts.to_vec();
                let tail = blk.tail;
                if self.hole_depth > 0 {
                    self.text("async ");
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
                self.text("async ");
                self.text("{");
                self.passed = self.passed.max(d.body.span(self.a).lo);
                self.level += 1;
                self.nl();
                for s in stmts {
                    self.stmt(s);
                }
                if let Some(t) = tail {
                    let span = t.span(self.a);
                    self.begin_el(span, Begin::Stmt);
                    self.expr_tokens(t);
                    self.end_el(span);
                }
                self.drain_close(d.body.span(self.a).hi);
                self.level -= 1;
                self.nl();
                self.text("}");
            }
            ExprRef::Path(h) => {
                self.path(&h.get(self.a).segs);
            }
            ExprRef::Self_(h) => {
                let d = h.get(self.a);
                self.text(if d.is_ty { "Self" } else { "self" });
            }
            ExprRef::Call(h) => {
                let d = h.get(self.a);
                self.operand(d.callee, Role::Callee);
                self.text("(");
                let spans: Vec<Span> = d.args.iter().map(|&e| e.span(self.a)).collect();
                self.break_list(&d.args, &spans, true, |p, e| p.expr(*e));
                self.drain_close(h.span(self.a).hi);
                self.text(")");
            }
            ExprRef::Method(h) => {
                let d = h.get(self.a);
                self.operand(d.recv, Role::Receiver);
                self.text(".");
                if let Some(m) = d.member {
                    self.text(self.a.name(m.ident));
                }
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
                self.operand(d.recv, Role::Receiver);
                self.text(".");
                self.text(self.a.name(d.name.ident));
            }
            ExprRef::Index(h) => {
                let d = h.get(self.a);
                self.operand(d.recv, Role::Receiver);
                self.text("[");
                self.expr(d.idx);
                self.drain_close(h.span(self.a).hi);
                self.text("]");
            }
            ExprRef::Unary(h) => {
                let d = h.get(self.a);
                self.text(unop_str(d.op));
                self.operand(d.expr, Role::Unary);
            }
            ExprRef::Binary(h) => {
                let d = h.get(self.a);
                let op = d.op;
                // flat try with the children's own breaks suppressed
                let m = self.stamp();
                self.in_flat += 1;
                self.operand(
                    d.lhs,
                    Role::Bin {
                        op,
                        side: Side::Lhs,
                    },
                );
                self.sp();
                self.text(binop_str(op));
                self.sp();
                self.operand(
                    d.rhs,
                    Role::Bin {
                        op,
                        side: Side::Rhs,
                    },
                );
                self.in_flat -= 1;
                if self.over_on_flat(m) {
                    self.rollback(m);
                    self.operand(
                        d.lhs,
                        Role::Bin {
                            op,
                            side: Side::Lhs,
                        },
                    );
                    self.sp();
                    self.text(binop_str(op));
                    self.nl_band();
                    self.operand(
                        d.rhs,
                        Role::Bin {
                            op,
                            side: Side::Rhs,
                        },
                    );
                    self.level -= 1;
                }
            }
            ExprRef::Assign(h) => {
                let d = h.get(self.a);
                self.operand(d.target, Role::AssignTarget);
                let m = self.stamp();
                self.in_flat += 1;
                self.sp();
                self.text(assign_str(d.op));
                self.sp();
                self.operand(d.value, Role::AssignValue);
                self.in_flat -= 1;
                if self.over_on_flat(m) {
                    self.rollback(m);
                    self.sp();
                    self.text(assign_str(d.op));
                    self.nl_band();
                    self.operand(d.value, Role::AssignValue);
                    self.level -= 1;
                }
            }
            ExprRef::Lambda(h) => {
                let d = h.get(self.a);
                self.lambda(&d.params, d.ret, d.body);
            }
            ExprRef::FStr(h) => {
                let d = h.get(self.a);
                // The composite prints VERBATIM: delimiters and chunk
                // text slice out of the source (hash levels, escape
                // spellings, the `{{`/`}}` doublings survive byte-exact
                // — no multiline promotion); only the hole expressions
                // reprint (live subtrees — fmt's whole job). The walk
                // is driven by the hole EXPR spans, never by brace
                // counting: a chunk's braces exist only as `{{`/`}}`
                // pairs, so the nearest `{` behind a hole expr is its
                // opener and the first `}` at/after its end is its
                // closer. `FPartAst::Lit` payloads are IGNORED here
                // (the compiler keeps the decoded chunks); `f""` has an
                // EMPTY parts vec — the delimiter slicing must not
                // depend on parts.
                let sp = d.span;
                let b = self.src.as_bytes();
                // open delimiter: `f` + n `#`s + `"` (lex_str_prefix's
                // shape)
                let mut i = sp.lo as usize;
                assert!(b[i] == b'f', "f-string span must open at its `f` prefix");
                i += 1;
                while b.get(i) == Some(&b'#') {
                    i += 1;
                }
                assert!(
                    b.get(i) == Some(&b'"'),
                    "f-string open delimiter must end at `\"` on a clean parse"
                );
                let open_end = i + 1;
                // close delimiter: scan back from span.hi over the
                // `#`s, land on the `"`
                let mut j = sp.hi as usize;
                while j > open_end && b[j - 1] == b'#' {
                    j -= 1;
                }
                j -= 1;
                assert!(
                    b[j] == b'"',
                    "f-string close delimiter must start at `\"` on a clean parse"
                );
                let close_start = j;
                self.text_lit(&self.src[sp.lo as usize..open_end]);
                let mut cursor = open_end;
                for part in &d.parts {
                    if let FPartAst::Hole(e) = part {
                        let he = e.span(self.a);
                        // the chunk gap before the hole: scan back
                        // from the expr to the hole's `{`
                        let mut k = he.lo as usize;
                        while k > cursor && b[k - 1] != b'{' {
                            k -= 1;
                        }
                        assert!(
                            k > cursor && b[k - 1] == b'{',
                            "f-string hole opener `{{` not found behind its expression"
                        );
                        self.text_lit(&self.src[cursor..k - 1]);
                        self.text("{");
                        self.hole_depth += 1;
                        self.expr(*e);
                        self.hole_depth -= 1;
                        // the hole's `}`: the first one at/after the
                        // expr's end
                        let mut m = he.hi as usize;
                        while m < close_start && b[m] != b'}' {
                            m += 1;
                        }
                        assert!(
                            b.get(m) == Some(&b'}'),
                            "f-string hole closer `}}` not found after its expression"
                        );
                        self.text("}");
                        cursor = m + 1;
                    }
                }
                // the tail chunk between the last hole (or the open
                // delimiter) and the close delimiter
                self.text_lit(&self.src[cursor..close_start]);
                self.text_lit(&self.src[close_start..sp.hi as usize]);
            }
            ExprRef::Struct(h) => {
                let d = h.get(self.a);
                self.ty(d.ty);
                self.sp();
                self.text("{");
                let m = self.stamp();
                self.in_flat += 1;
                self.sp();
                for (i, fi) in d.fields.iter().enumerate() {
                    if i > 0 {
                        self.text(", ");
                    }
                    self.text(self.a.name(fi.name.ident));
                    self.text(": ");
                    self.expr(fi.value);
                }
                self.in_flat -= 1;
                if self.over_on_flat(m) {
                    self.rollback(m);
                    self.nl_band();
                    for fi in d.fields.iter() {
                        self.text(self.a.name(fi.name.ident));
                        self.text(": ");
                        self.expr(fi.value);
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
                self.operand(d.expr, Role::AwaitOperand);
            }
            ExprRef::Is(h) => {
                let d = h.get(self.a);
                self.operand(d.expr, Role::IsOperand);
                self.sp();
                self.text("is");
                self.sp();
                self.ty(d.ty);
            }
            ExprRef::Cast(h) => {
                let d = h.get(self.a);
                self.operand(d.expr, Role::CastOperand);
                self.sp();
                self.text("as");
                self.sp();
                self.ty(d.ty);
            }
            ExprRef::Error(_) => {}
        }
    }

    /// the lambda's two spellings (the parser's two forms):
    /// - one type-less param, no ret → `x => body`;
    /// - a Block body otherwise → `fn (params) (-> ret)? { .. }` (the
    ///   anon form, block-bodied by grammar);
    /// - an expr body otherwise → `(params) (-> ret)? => body` (the
    ///   paren form, `=>`-connected by grammar).
    fn lambda(&mut self, params: &[ParamRef], ret: Option<TyRef>, body: ExprRef) {
        if params.len() == 1 && ret.is_none() {
            if let ParamRef::Param(ph) = params[0] {
                let d = ph.get(self.a);
                if d.ty.is_none() {
                    self.text(self.a.name(d.name.ident));
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

    /// a literal's source slice — the span×src join. The AST stores
    /// literal VALUES (semantics — the compiler consumes the decoded
    /// strings, the u64 values, the f64 bits), but every `LitNode`
    /// carries its exact token span, so fmt reprints the SPELLING from
    /// the source: radix, underscores, exponent shapes, escape
    /// spellings, and raw-string hash levels survive byte-exact.
    /// `true`/`false`/`nil` are words — the slice IS the word. A clean
    /// parse token-backs every literal, so an empty slice is a broken
    /// invariant and fails LOUDLY (release `text("")` would silently
    /// DROP the literal — `text` no-ops on empty).
    fn src_slice(src: &str, sp: Span) -> &str {
        let s = &src[sp.lo as usize..sp.hi as usize];
        assert!(
            !s.is_empty(),
            "literal span must be token-backed on a clean parse"
        );
        s
    }

    fn lit(&mut self, node: &LitNode) {
        let s = Self::src_slice(self.src, node.span);
        // text_lit: raw strings carry REAL newlines — the count rides
        // `lit_nl` for the flat-attempt probe
        self.text_lit(s);
    }
}
