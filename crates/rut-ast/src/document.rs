//! Doc — THE document pipeline: text, tokens, tree, and the ONE
//! mutator. A `Doc` owns a [`Source`] (the normalized text every span
//! indexes + its line geometry), the token stream over it, and the
//! capsule [`Ast`]; every span in the tree indexes `Doc`'s own text,
//! and every text question rides [`Doc::text`] (the span×src join).
//!
//! THE LAWS THIS FILE OWNS:
//! - **one grammar engine** — cold parse and edit reparse both ride
//!   [`parser::parse_item_window`]; the cold parse is just the window
//!   that covers everything. `parse`/`parse_with_recover` are wrappers
//!   over [`Doc::parse`].
//! - **one mutator** — [`Doc::edit`] is the only way a document
//!   changes. Inside: `src.edit` → token splice (`TokenChange` onward)
//!   → the dirty reuse unit (v1 granularity: the root item) → the
//!   window reparse into fresh slots → splice under the surviving
//!   parent → the flat panel rebuilt → the tree index invalidated.
//!   Everything between is private: nothing outside `edit` maintains
//!   anything.
//! - **edit ≡ cold** — after every step, the edited document equals a
//!   cold `Doc::parse` of the same text: tokens, printed tree, spans,
//!   flat errors, the tree index. Property-tested over random edit
//!   sequences on the corpus (`tests/doc_edit_props.rs`).
//!
//! Doors are questions, not storage handouts: `text(span)` joins, the
//! line doors answer line questions, `tokens_in` is the classifier's
//! window, `name_at` the fence residue. No raw
//! `src()`/`toks()`/`line_starts()` — the pipeline's own lanes read the
//! `Source` directly, nothing above this crate touches the storage.

use rut_lexer::diag::Diag;
use rut_lexer::lexer::{lex_canonical, lex_normalized};
use rut_lexer::source::{Edit, Source};
use rut_lexer::span::Span;
use rut_lexer::splice::{splice_tokens, TokenChange};
use rut_lexer::token::{Tok, Token};

use crate::parser::{graft_table, parse_item_window, Mode};
use crate::{children_of, remove_any, Ast, ErrorNode, ItemRef, ModuleNode};

/// The parser's once-only nesting verdict — both budgets (NEST_MAX and
/// EXPR_MAX) emit exactly this message, once per parse. The edit lane
/// greps the surviving rows with it to keep the flat panel ≡ cold.
const DEEP_MSG: &str = "nesting too deep";

/// The document: the pipeline's owner and the only mutable artifact
/// above the lexer. Constructed by [`Doc::parse`], repaired by
/// [`Doc::edit`], read through the doors below.
pub struct Doc {
    /// the normalized text every span indexes + its line geometry
    src: Source,
    /// the token stream over `src` — the cold lex, maintained by the
    /// splice (the edit lane's storage; `tokens_in` is the read face)
    toks: Vec<Token>,
    /// the tree (the capsule store — nodes, interner, errors, index)
    ast: Ast,
    /// the grammar mode the document parses in (the edit lane's
    /// reparse rides the same mode the parse chose)
    mode: Mode,
    /// how many of the module's leading items are the lex tier's
    /// pre-seeded fences (the edit lane rebuilds that prefix wholesale)
    n_lex_items: usize,
    /// where the flat error panel's PARSE tier begins (the lex tier is
    /// everything before it — rebuilt wholesale on every edit)
    parse_from: usize,
}

impl Doc {
    /// The cold open: one lex (`lex_canonical` — line starts ride the
    /// same byte scan), then the window-reparse engine over the full
    /// range. Lex rows with no grammar home pre-seed as root-level
    /// `ErrorNode`s (the recovery ladder's LEX tier); docs graft at
    /// allocation; the flat panel is `[lex rows…, parse rows…]`.
    pub fn parse(mode: Mode, src: String) -> Doc {
        let (normalized, mut toks, lex_diags, comments, line_starts) = lex_canonical(src, true);
        let source = Source::new(normalized, line_starts);
        let mut ast = Ast::new();
        let n_lex_items = lex_diags.len();
        let mut pre_items = Vec::with_capacity(n_lex_items);
        for d in &lex_diags {
            let h = ErrorNode {
                span: d.span,
                diags: vec![d.clone()],
            }
            .alloc_plain(&mut ast);
            pre_items.push(ItemRef::Error(h));
        }
        let grafts = graft_table(source.text(), comments);
        let (items, parse_diags) =
            parse_item_window(mode, &mut toks, &mut ast, grafts, pre_items, 0, false);
        let eof_hi = toks.last().map(|t| t.span.hi).unwrap_or(0);
        let root = ModuleNode {
            span: Span::new(0, eof_hi),
            items,
        }
        .alloc_plain(&mut ast);
        ast.set_root(root);
        let parse_from = lex_diags.len();
        let mut all = lex_diags;
        all.extend(parse_diags);
        ast.set_errors(all);
        Doc {
            src: source,
            toks,
            ast,
            mode,
            n_lex_items,
            parse_from,
        }
    }

    /// THE mutator. One edit in, a coherent document out — the internal
    /// stages, in order:
    ///
    /// 1. `src.edit` — text + line geometry move together;
    /// 2. the token splice — a windowed relex with resync/convergence;
    ///    the dirty token range (`TokenChange`) reports onward;
    /// 3. the lex tier re-derived over the new text (v1 price: one
    ///    linear scan for the diags + the doc-graft table; the PARSE
    ///    below stays windowed);
    /// 4. the lex-fence prefix rebuilt (the module items' leading
    ///    `ErrorNode`s — yesterday's die, fresh ones pre-seed);
    /// 5. the dirty reuse unit found from `TokenChange` — v1
    ///    granularity: the ROOT ITEM containing the first dirty token;
    ///    everything before it is byte-identical text and survives
    ///    slot-stable, everything from it on is reparsed to EOF;
    /// 6. the dropped subtrees punched into holes (stale handles die
    ///    loudly on read, never read yesterday's nodes);
    /// 7. the window reparse into FRESH SLOTS of the live store — the
    ///    same engine a cold parse rides;
    /// 8. the splice under the surviving parent (the root module — its
    ///    slot repaired in place, the root handle stable) and the flat
    ///    panel rebuilt `[lex…, surviving parse…, fresh parse…]`;
    /// 9. the tree index invalidated — the next read rebuilds it (the
    ///    same deterministic walk a cold parse runs).
    ///
    /// v1 notes: the root-item granularity makes the span shift a
    /// no-op (survivors are the unchanged prefix; the tail is
    /// reparsed wholesale) and a full tree-index rebuild stands in for
    /// a patch — both are the later knobs the plan defers.
    pub fn edit(&mut self, e: Edit) {
        // 1. text + line geometry
        self.src.edit(&e);
        // 2. the token splice — the dirty range's start is the window's
        //    landing index in the spliced stream, always a live token
        //    (the stream never loses its Eof, so `change.start` stays
        //    in bounds even for the whitespace-at-EOF corners, which
        //    ride the append branch below)
        let change = splice_tokens(&mut self.toks, &self.src, &e);
        // 3. the lex tier + the doc-graft table, re-derived
        let (_, lex_diags, comments, _) = lex_normalized(self.src.text(), true);
        let grafts = graft_table(self.src.text(), comments);
        // 4. the lex-fence prefix: yesterday's fences die, fresh ones
        //    pre-seed in the same order a cold parse seeds them
        let n_old_fences = self.n_lex_items;
        let old_items: Vec<ItemRef> = self.ast.root().items(&self.ast).to_vec();
        for f in &old_items[..n_old_fences] {
            remove_subtree(&mut self.ast, f.any());
        }
        let mut fences = Vec::with_capacity(lex_diags.len());
        for d in &lex_diags {
            let h = ErrorNode {
                span: d.span,
                diags: vec![d.clone()],
            }
            .alloc_plain(&mut self.ast);
            fences.push(ItemRef::Error(h));
        }
        self.n_lex_items = fences.len();
        // 5. the dirty reuse unit
        let (drop_from, window_from, bnd) = self.dirty_unit(&e, change, &old_items, n_old_fences);
        // 6. the dropped subtrees die loudly
        let parsed_old = &old_items[n_old_fences..];
        for it in &parsed_old[drop_from..] {
            remove_subtree(&mut self.ast, it.any());
        }
        // the survivors' parse rows: everything before the boundary
        // re-rides the surviving subtrees verbatim (byte-identical
        // text ⇒ byte-identical rows); the boundary is the first
        // dropped item's start (a dropped item's rows can start before
        // the first dirty token, never before its own item)
        let survivors: Vec<Diag> = self.ast.errors()[self.parse_from..]
            .iter()
            .filter(|d| d.span.lo < bnd)
            .cloned()
            .collect();
        let deep_already = survivors.iter().any(|d| d.msg == DEEP_MSG);
        // 7. the window reparse into fresh slots
        let mut window: Vec<Token> = self.toks[window_from..].to_vec();
        let prev_end = window.first().map(|t| t.span.lo).unwrap_or(0);
        let (fresh, parse_diags) = parse_item_window(
            self.mode,
            &mut window,
            &mut self.ast,
            grafts,
            Vec::new(),
            prev_end,
            deep_already,
        );
        // 8. splice under the surviving parent + the flat panel
        let mut items = fences;
        items.extend_from_slice(&parsed_old[..drop_from]);
        items.extend(fresh);
        let eof_hi = self.toks.last().map(|t| t.span.hi).unwrap_or(0);
        self.ast.reseal_root(items, eof_hi);
        let mut all = lex_diags;
        self.parse_from = all.len();
        all.extend(survivors);
        all.extend(parse_diags);
        self.ast.set_errors(all);
        // 9. the tree index — invalidated; the next read rebuilds
        self.ast.index_invalidate();
    }

    // ---- text — questions, not storage ----

    /// the text `span` indexes — the verbatim join (the ONE text door;
    /// every span in the tree indexes this document's own text)
    pub fn text(&self, span: Span) -> &str {
        &self.src.text()[span.lo as usize..span.hi as usize]
    }

    /// the whole-document span — `[0, len)`, the range every full-text
    /// view joins over (a formatter's comment scan, a shell's views)
    pub fn span(&self) -> Span {
        Span::new(0, self.len())
    }

    /// the text's byte length
    pub fn len(&self) -> u32 {
        self.src.text().len() as u32
    }

    /// an empty document?
    pub fn is_empty(&self) -> bool {
        self.src.text().is_empty()
    }

    /// the line `byte` sits on — 0-based, the newline closing a line
    /// counts as its own end (the `Source` table's convention)
    pub fn line_of(&self, byte: u32) -> u32 {
        let starts = self.src.line_starts();
        (starts.partition_point(|&p| p <= byte) - 1) as u32
    }

    /// the span of line `line` — from its first byte to the next
    /// line's first byte (the trailing newline included); the last
    /// line ends at the text's length
    pub fn line_span(&self, line: u32) -> Span {
        let starts = self.src.line_starts();
        let lo = starts[line as usize];
        let hi = starts.get(line as usize + 1).copied().unwrap_or(self.len());
        Span::new(lo, hi)
    }

    // ---- tokens ----

    /// the tokens intersecting `span` — the classifier's window and
    /// the one public token face (the top-level stream; an f-string's
    /// hole tokens ride their `FStr`). The `Eof` is the stream's
    /// `[len, len]` terminator: it rides any window that reaches the
    /// text's end.
    pub fn tokens_in(&self, span: Span) -> &[Token] {
        let first = self.toks.partition_point(|t| t.span.hi <= span.lo);
        let mut last = self.toks.partition_point(|t| t.span.lo < span.hi);
        if span.hi >= self.len() && self.toks.get(last).is_some_and(|t| t.tok == Tok::Eof) {
            last += 1;
        }
        &self.toks[first..last.max(first)]
    }

    /// the lexeme under the cursor — THE FENCE RESIDUE: when the tree
    /// has a real node at `pos` the tree owns the answer (consumers
    /// walk it; `None` here), and when `node_at` landed on an
    /// `ErrorNode` the token under the cursor is the residue.
    pub fn name_at(&self, pos: u32) -> Option<(Span, &str)> {
        let h = self.ast.node_at(pos)?;
        if !h.is::<ErrorNode>() {
            return None;
        }
        let t = self.tok_at(pos)?;
        Some((t.span, self.text(t.span)))
    }

    /// the token containing `pos` (the residue's private helper)
    fn tok_at(&self, pos: u32) -> Option<&Token> {
        let i = self
            .toks
            .partition_point(|t| t.span.lo <= pos)
            .checked_sub(1)?;
        let t = self.toks.get(i)?;
        (t.span.lo <= pos && pos < t.span.hi).then_some(t)
    }

    // ---- the structural read ----

    /// the tree — the capsule store (structure, interner, errors, the
    /// derived tree index)
    pub fn ast(&self) -> &Ast {
        &self.ast
    }

    // ---- the edit lane's privates ----

    /// The dirty reuse unit from `TokenChange` — v1 granularity: the
    /// ROOT ITEM. Returns `(drop_from, window_from, bnd)`:
    ///
    /// - `drop_from` — the first parsed item to drop, in the module's
    ///   PARSED region (the lex-fence prefix excluded): the first item
    ///   reaching past the splice's window start. Tokens before the
    ///   window are byte-identical text, so an item built purely from
    ///   them survives slot-stable; every item owning a relexed token
    ///   is reparsed (to EOF — item boundaries are not trusted past
    ///   damage). The window start sits at/before the edit's start, so
    ///   old-text item spans compare against it directly.
    /// - `window_from` — the spliced stream's index the reparse window
    ///   starts at: the dropped item's OWN first token (the first dirty
    ///   token can sit mid-item — the window must open at the item
    ///   boundary, or the fresh item would start mid-shape).
    /// - `bnd` — the old-coordinate boundary the survivors' parse rows
    ///   must precede (the dropped item's start; `u32::MAX` when the
    ///   fresh items append past the last item).
    ///
    /// The token→old-coordinate map: positions before the edit are
    /// identical in old and new text; positions at/after map back by
    /// the length delta, clamped to the edit's start (a token inside
    /// the inserted region occupies the replaced range). The map is
    /// monotone along the stream, so the partition point is exact.
    fn dirty_unit(
        &self,
        e: &Edit,
        change: TokenChange,
        items: &[ItemRef],
        n_old_fences: usize,
    ) -> (usize, usize, u32) {
        let parsed = &items[n_old_fences..];
        let delta = e.text.len() as i64 - (e.old_end - e.start) as i64;
        let old_of = |lo: u32| -> u32 {
            if lo <= e.start {
                lo
            } else {
                (lo as i64 - delta).max(e.start as i64) as u32
            }
        };
        let mut drop_from = parsed.partition_point(|it| it.span(&self.ast).hi <= change.window_lo);
        // The boundary must settle between two CLEAN items — recovery is
        // NOT item-local: a failed frame's sync skim and its expects read
        // tokens past the item's own span, and a clean item's recovery
        // rows land just past it (at the following fence's start). So:
        //
        // (pair) a dropped side opening with a fence backtracks one item
        // — the clean item before it peeked there, and only a window
        // opening at THAT item re-derives its rows;
        //
        // (tail) a survivor tail ending with a fence backtracks one item
        // — the fence's extent keys on the token its sync stopped at,
        // and past the boundary that token changed.
        //
        // Backtracking is always sound: an earlier window only re-derives
        // more, and the parse state at a clean item boundary of the
        // survivor prefix is exactly cold's.
        loop {
            // (pair) the dropped side opens with a fence, or (tail) the
            // survivor tail ends with one — either way one item of
            // backtrack re-derives the recovery with the new context
            let pair = drop_from > 0
                && drop_from < parsed.len()
                && matches!(parsed[drop_from], ItemRef::Error(_));
            let tail = drop_from > 0 && matches!(parsed[drop_from - 1], ItemRef::Error(_));
            if !pair && !tail {
                break;
            }
            drop_from -= 1;
        }
        if drop_from == parsed.len() {
            // past the last item: the fresh items append; every parse
            // row survives
            return (drop_from, change.start, u32::MAX);
        }
        let d_lo = parsed[drop_from].span(&self.ast).lo;
        let window_from = self
            .toks
            .partition_point(|t| old_of(t.span.lo) < d_lo)
            // the window can never start PAST the first dirty token: a
            // token the relex created (a word born from an edit) maps
            // back to a meaningless old position, but every dirty token
            // is the reparse's input — skipping one would parse a shape
            // the cold lex never sees
            .min(change.start);
        (drop_from, window_from, d_lo)
    }
}

/// Punch every node of a subtree into holes — the dropped region's
/// loud death. An explicit worklist (no host recursion — the same
/// contract as the parser's frame machine); children are collected
/// while the parent slot is still live.
fn remove_subtree(ast: &mut Ast, root: crate::AnyHandle) {
    let mut stack = vec![root];
    while let Some(h) = stack.pop() {
        // children first onto the worklist (they read the live slots),
        // then this slot's hole
        let kids = children_of(ast, h);
        stack.extend(kids);
        remove_any(ast, h);
    }
}

#[cfg(test)]
mod tests {
    use super::{Doc, DEEP_MSG};
    use crate::parser::Mode;
    use rut_lexer::source::Edit;
    use rut_lexer::span::Span;
    use rut_lexer::token::Tok;

    fn doc(src: &str) -> Doc {
        Doc::parse(Mode::Impl, src.to_string())
    }

    /// edit ≡ cold, the compact form: tokens, printed tree, flat panel
    fn assert_edit_cold(doc: &mut Doc, old: &str, new: &str) {
        doc.edit(Edit::between(old, new));
        let cold = Doc::parse(Mode::Impl, new.to_string());
        assert_eq!(
            doc.tokens_in(doc.span()),
            cold.tokens_in(cold.span()),
            "tokens ≡ cold lex: {old:?} -> {new:?}"
        );
        assert_eq!(
            crate::AstPrinter::json_of(doc.ast()),
            crate::AstPrinter::json_of(cold.ast()),
            "tree ≡ cold parse: {old:?} -> {new:?}"
        );
        assert_eq!(
            doc.ast().errors(),
            cold.ast().errors(),
            "flat panel ≡ cold: {old:?} -> {new:?}"
        );
    }

    const TWO_FNS: &str = "\
fn area(w: i32, h: i32) -> i32 {
    w * h
}

fn main() -> nil {
    let x = area(2, 3);
    print(x);
}
";

    #[test]
    fn text_door_joins_and_line_questions_answer() {
        let d = doc("let a = 1;\nlet b = 22;\n");
        assert_eq!(d.len(), 23);
        assert_eq!(d.text(Span::new(4, 5)), "a");
        assert_eq!(d.text(d.span()), "let a = 1;\nlet b = 22;\n");
        assert_eq!(d.line_of(0), 0);
        assert_eq!(d.line_of(10), 0, "the newline closes its own line");
        assert_eq!(d.line_of(11), 1);
        assert_eq!(d.line_of(23), 2, "the empty last line");
        assert_eq!(d.line_span(0), Span::new(0, 11));
        assert_eq!(d.line_span(1), Span::new(11, 23));
        assert_eq!(d.line_span(2), Span::new(23, 23));
    }

    #[test]
    fn tokens_in_is_a_window_not_a_harvest() {
        let d = doc("let a = 1;\nlet b = 22;\n");
        // one line's tokens
        let line1 = d.tokens_in(Span::new(11, 22));
        let kinds: Vec<&Tok> = line1.iter().map(|t| &t.tok).collect();
        assert_eq!(kinds.len(), 5, "{kinds:?}");
        assert_eq!(line1[1].span, Span::new(15, 16), "the `b` token");
        // the token's own span yields it
        assert_eq!(d.tokens_in(Span::new(15, 16)).len(), 1);
        // a whitespace gap yields none
        assert_eq!(d.tokens_in(Span::new(10, 11)).len(), 0);
        // the whole span is the whole stream (the Eof included)
        assert_eq!(d.tokens_in(d.span()).last().unwrap().tok, Tok::Eof);
    }

    #[test]
    fn name_at_is_the_fence_residue() {
        let d = doc("let ok = 1;\noops here\n");
        // the fence swallows `oops here` — the residue is the word under it
        let bad = Span::new(12, 21);
        assert_eq!(d.text(bad), "oops here");
        let (span, name) = d.name_at(13).expect("the fence residue");
        assert_eq!(name, "oops");
        assert_eq!(span, Span::new(12, 16));
        // a real node at the cursor: the tree owns the answer (None here)
        assert_eq!(d.name_at(4), None, "the `a` ident sits in a ModuleLet");
        // dead space (whitespace) is no fence either
        assert_eq!(d.name_at(11), None);
    }

    #[test]
    fn an_edit_inside_a_fn_reparses_the_tail_not_the_file() {
        let mut d = doc(TWO_FNS);
        // the survivor keeps its SLOT — `area`'s handle is stable, the
        // tree below the module is untouched
        let before = d.ast().root().items(d.ast()).len();
        let old = d.text(d.span()).to_string();
        let new = old.replace("let x = area(2, 3);", "let x = area(20, 30);");
        assert_edit_cold(&mut d, &old, &new);
        let items = d.ast().root().items(d.ast());
        assert_eq!(items.len(), before, "two fns before, two after");
        assert_eq!(
            d.text(Span::new(0, 38)),
            TWO_FNS[..38.min(TWO_FNS.len())].to_string().as_str(),
            "the surviving head is byte-identical"
        );
    }

    #[test]
    fn item_split_delete_and_append_stay_cold_equal() {
        let mut d = doc("let a = 1;\nlet b = 2;\n");
        let old = d.text(d.span()).to_string();
        // split: a new item between the two
        assert_edit_cold(&mut d, &old, "let a = 1;\nlet mid = 9;\nlet b = 2;\n");
        // delete the middle item
        let old = d.text(d.span()).to_string();
        assert_edit_cold(&mut d, &old, "let a = 1;\nlet b = 2;\n");
        // append at EOF
        let old = d.text(d.span()).to_string();
        assert_edit_cold(&mut d, &old, "let a = 1;\nlet b = 2;\nlet c = 3;\n");
        // delete everything (the tokens vanish; the Eof stays)
        let old = d.text(d.span()).to_string();
        assert_edit_cold(&mut d, &old, "\n");
        // and back from the empty document
        let old = d.text(d.span()).to_string();
        assert_edit_cold(&mut d, &old, "let z = 0;\n");
    }

    #[test]
    fn an_edit_inside_a_literal_reanchors_the_window() {
        let mut d = doc("let s = r#\"\nline one\nline two\n\"#;\nlet n = 2;\n");
        let old = d.text(d.span()).to_string();
        assert_edit_cold(
            &mut d,
            &old,
            "let s = r#\"\nline one\nline TWO\n\"#;\nlet n = 2;\n",
        );
    }

    #[test]
    fn the_eof_whitespace_corner_moves_only_the_module_span() {
        // inserting whitespace at EOF after a trailing newline: the
        // splice converges with an empty dirty range — no token, no
        // item changed, the module's span rides the longer Eof
        let mut d = doc("let a = 1;\n");
        let old = d.text(d.span()).to_string();
        let new = format!("{old}   ");
        d.edit(Edit::between(&old, &new));
        let cold = doc(&new);
        assert_eq!(d.tokens_in(d.span()), cold.tokens_in(cold.span()));
        assert_eq!(
            crate::AstPrinter::json_of(d.ast()),
            crate::AstPrinter::json_of(cold.ast()),
            "only the module span moved"
        );
        // a pure no-op edit is a pure no-op
        let before = crate::AstPrinter::json_of(d.ast());
        d.edit(Edit::between(&new, &new));
        assert_eq!(crate::AstPrinter::json_of(d.ast()), before);
    }

    #[test]
    fn doc_chains_regraft_after_an_edit_above() {
        let mut d = doc("/// adds one\nfn inc(x: i64) -> i64 { x + 1 }\n");
        let old = d.text(d.span()).to_string();
        // the head changed: the fresh fn must carry the chain, verbatim
        assert_edit_cold(
            &mut d,
            &old,
            "/// adds one\nfn inc(x: i64) -> i64 { x * 2 }\n",
        );
        let ast = d.ast();
        let items = ast.root().items(ast);
        let crate::ItemRef::Fn(h) = items[items.len() - 1] else {
            panic!("the fn survives")
        };
        let text = d.text(d.span());
        assert_eq!(
            h.get(ast).doc.lines(text).collect::<Vec<_>>(),
            vec!["adds one"],
            "the Go-convention chain re-grafted from the refreshed table"
        );
    }

    #[test]
    fn broken_regions_repair_and_refence_like_cold() {
        // a fence between two good items; the edit REPLACES the garbage
        let mut d = doc("let ok = 1;\noops here\nlet tail = 3;\n");
        assert!(!d.ast().is_clean());
        let old = d.text(d.span()).to_string();
        assert_edit_cold(&mut d, &old, "let ok = 1;\nlet fine = 2;\nlet tail = 3;\n");
        assert!(
            d.ast().is_clean(),
            "the fence is gone, the repair parsed clean"
        );
        // and the other direction: good source gains garbage mid-file
        let old = d.text(d.span()).to_string();
        assert_edit_cold(
            &mut d,
            &old,
            "let ok = 1;\n%%% garbage %%%\nlet tail = 3;\n",
        );
        assert!(!d.ast().is_clean());
    }

    #[test]
    fn the_nesting_verdict_stays_the_global_first_site() {
        // a surviving deep site BEFORE the boundary and a fresh one
        // after: the flat panel must report exactly the earlier row
        // (the once-only law, ≡ cold)
        let deep = |body: &str| {
            let mut inner = body.to_string();
            for _ in 0..70 {
                inner = format!("({inner})");
            }
            inner
        };
        let src0 = format!("let deep = {};\nlet later = 1;\n", deep("1"));
        let mut d = doc(&src0);
        assert!(
            d.ast().errors().iter().any(|e| e.msg == DEEP_MSG),
            "the first parse trips the budget once"
        );
        let old = d.text(d.span()).to_string();
        let new = format!("let deep = {};\nlet later = {};\n", deep("1"), deep("2"));
        assert_edit_cold(&mut d, &old, &new);
        assert_eq!(
            d.ast()
                .errors()
                .iter()
                .filter(|e| e.msg == DEEP_MSG)
                .count(),
            1,
            "one nesting row across the whole panel, not one per parse"
        );
    }

    #[test]
    fn stale_handles_die_loudly_not_lie() {
        let mut d = doc("let a = 1;\nlet b = 2;\n");
        let ast = d.ast();
        let items = ast.root().items(ast);
        let crate::ItemRef::ModuleLet(h) = items[1] else {
            panic!()
        };
        // the edit deletes item b — its slot becomes a hole
        let old = d.text(d.span()).to_string();
        d.edit(Edit::between(&old, "let a = 1;\n"));
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.get(d.ast())));
        assert!(
            err.is_err(),
            "a stale handle must panic, never read yesterday"
        );
    }

    #[test]
    fn decl_mode_edits_reparse_in_the_same_mode() {
        // a `.d.rut` document: bodies are forbidden — the edit lane
        // must ride the parse's own mode
        let mut d = Doc::parse(Mode::Decl, "pub host fn f(x: i64) -> i64;\n".to_string());
        assert!(d.ast().is_clean(), "{:?}", d.ast().errors());
        let old = d.text(d.span()).to_string();
        let new = "pub host fn g(y: i64) -> i64;\n".to_string();
        d.edit(Edit::between(&old, &new));
        let cold = Doc::parse(Mode::Decl, new.clone());
        assert_eq!(
            d.ast().errors(),
            cold.ast().errors(),
            "same mode, same verdicts"
        );
        assert_eq!(
            crate::AstPrinter::json_of(d.ast()),
            crate::AstPrinter::json_of(cold.ast())
        );
        // and the mode's refusal fires identically through an edit
        let old = new;
        let new = "pub host fn h(z: i64) -> i64 { z }\n".to_string();
        d.edit(Edit::between(&old, &new));
        let cold = Doc::parse(Mode::Decl, new);
        assert_eq!(
            d.ast().errors(),
            cold.ast().errors(),
            "bodies stay forbidden"
        );
    }
}
