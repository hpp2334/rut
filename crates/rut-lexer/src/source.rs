//! The text layer — [`Edit`], the first-class change, and
//! [`Source`], THE editable source: the normalized text every span
//! indexes plus its line geometry, maintained together through edits.
//! One splice per edit; readers borrow through named doors. The
//! `rut-ast` `Doc` pipeline owns a `Source` and rides
//! [`Source::edit`] + [`crate::splice::splice_tokens`] on the way to a
//! new tree — nothing above this crate touches the storage by hand.

/// One change to a [`Source`]: replace the byte range
/// `[start, old_end)` with `text` (an insertion has `start ==
/// old_end`, a deletion has `text == ""`). Positions are byte offsets
/// into the PRE-edit (normalized) text and must sit on char
/// boundaries — [`Edit::between`] guarantees that; a wire adapter
/// constructing edits by hand owes the same.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edit {
    /// the replaced range's first byte (old-text coordinates)
    pub start: u32,
    /// the replaced range's one-past-last byte (old-text coordinates)
    pub old_end: u32,
    /// the replacement text (spliced in at `start`)
    pub text: String,
}

impl Edit {
    /// The diff helper: the minimal edit turning `old` into `new` —
    /// the common prefix and suffix (snapped to char boundaries;
    /// byte-equal prefixes may stop mid-character) leave one replaced
    /// middle. This is the wire adapter's lane: host text arrives, the
    /// shell diffs, ONE edit carries the change.
    pub fn between(old: &str, new: &str) -> Edit {
        let (ob, nb) = (old.as_bytes(), new.as_bytes());
        let min = ob.len().min(nb.len());
        // the common prefix, snapped down to a char boundary on both
        // sides (the bytes agree up to `start`, so a boundary in one is
        // a boundary in the other)
        let mut start = 0usize;
        while start < min && ob[start] == nb[start] {
            start += 1;
        }
        while start > 0 && (!old.is_char_boundary(start) || !new.is_char_boundary(start)) {
            start -= 1;
        }
        // the common suffix, capped so it never eats the prefix
        // region, then snapped down the same way
        let mut suffix = 0usize;
        while suffix < min - start && ob[ob.len() - 1 - suffix] == nb[nb.len() - 1 - suffix] {
            suffix += 1;
        }
        while suffix > 0
            && (!old.is_char_boundary(ob.len() - suffix)
                || !new.is_char_boundary(nb.len() - suffix))
        {
            suffix -= 1;
        }
        Edit {
            start: start as u32,
            old_end: (ob.len() - suffix) as u32,
            text: new[start..nb.len() - suffix].to_string(),
        }
    }
}

/// The editable source — the normalized text every span indexes (LF
/// only; the lex lane's normalization is the law) plus the line
/// geometry over it: `line_starts[i]` is the byte where line `i`
/// begins, `0` first, one entry per newline (a text ending in `\n`
/// opens an empty last line — matching the lexer's own table).
/// Constructed from the lex lane (`lex_normalized` returns the table
/// beside the tokens — no extra pass), maintained by [`Source::edit`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    text: String,
    line_starts: Vec<u32>,
}

impl Source {
    /// Assemble from the lex lane's parts: the normalized text and the
    /// line table that scan produced.
    pub fn new(text: String, line_starts: Vec<u32>) -> Source {
        debug_assert!(
            !line_starts.is_empty() && line_starts[0] == 0,
            "the line table anchors at 0"
        );
        debug_assert!(
            line_starts.windows(2).all(|w| w[0] < w[1]),
            "the line table is strictly increasing"
        );
        Source { text, line_starts }
    }

    /// The text every span indexes.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The line geometry — byte offset where each line starts.
    pub fn line_starts(&self) -> &[u32] {
        &self.line_starts
    }

    /// The start of the line CONTAINING byte `pos`. Binary search over
    /// the table (`line_starts[0] == 0` anchors it).
    pub fn line_start_of(&self, pos: u32) -> u32 {
        let ls = &self.line_starts;
        ls[ls.partition_point(|&p| p <= pos) - 1]
    }

    /// Apply ONE edit: the text splices, the line geometry moves with
    /// it — line starts at or before `start` hold (the text before the
    /// edit is untouched), starts inside the replaced range die (their
    /// newlines did), starts past it shift by the length delta, and
    /// the edit's own newlines open new lines. Incremental, never a
    /// recount.
    pub fn edit(&mut self, e: &Edit) {
        let start = e.start as usize;
        let old_end = e.old_end as usize;
        debug_assert!(start <= old_end, "an edit replaces a forward range");
        debug_assert!(old_end <= self.text.len(), "the range is in-file");
        debug_assert!(
            self.text.is_char_boundary(start) && self.text.is_char_boundary(old_end),
            "an edit lands on char boundaries"
        );
        self.text.replace_range(start..old_end, &e.text);
        // geometry: keep the prefix (last entry = the line containing
        // `start`), drop the entries the replaced range killed, shift
        // the survivors, splice in the edit text's own line starts
        let anchor = self.line_starts.partition_point(|&p| p <= e.start);
        let killed = self.line_starts.partition_point(|&p| p <= e.old_end);
        let delta = e.text.len() as i64 - (old_end - start) as i64;
        let mut fresh: Vec<u32> = e
            .text
            .bytes()
            .enumerate()
            .filter_map(|(i, b)| (b == b'\n').then_some((start + i + 1) as u32))
            .collect();
        fresh.extend(
            self.line_starts[killed..]
                .iter()
                .map(|&p| (p as i64 + delta) as u32),
        );
        self.line_starts.splice(anchor.., fresh);
    }
}

#[cfg(test)]
mod tests {
    use super::{Edit, Source};

    /// build a Source the way the lex lane does — table from a recount
    fn src_of(text: &str) -> Source {
        let mut ls = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                ls.push(i as u32 + 1);
            }
        }
        Source::new(text.to_string(), ls)
    }

    fn assert_edits(old: &str, new: &str) {
        let e = Edit::between(old, new);
        let mut check = old.to_string();
        check.replace_range(e.start as usize..e.old_end as usize, &e.text);
        assert_eq!(check, new, "between({old:?}, {new:?}) must reproduce");
    }

    #[test]
    fn between_reproduces_on_shape_pairs() {
        for (old, new) in [
            ("", ""),
            ("", "x"),
            ("x", ""),
            ("abc", "abc"),
            ("abc", "abcd"),
            ("abc", "ab"),
            ("aab", "ab"),
            ("let a = 1;\nlet b = 2;\n", "let a = 1;\nlet b = 23;\n"),
            (
                "let a = 1;\nlet b = 2;\n",
                "let a = 1;\nlet b = 2;\nlet c = 3;\n",
            ),
            ("a\nb\nc", "a\nb\ncd"),
            ("héllo", "hell"),
            ("héllo", "héllo"),
            ("é", "ê"),
            ("é", "e"),
            ("xé", "yx"),
        ] {
            assert_edits(old, new);
        }
    }

    #[test]
    fn between_is_minimal_where_bytes_allow() {
        let e = Edit::between("let a = 1;\nlet b = 2;\n", "let a = 1;\nlet b = 23;\n");
        assert_eq!(e.start, 20);
        assert_eq!(e.old_end, 20, "appending after `2` is a pure insertion");
        assert_eq!(e.text, "3");
        // pure insertion: start == old_end
        let e = Edit::between("ab", "abx");
        assert_eq!((e.start, e.old_end), (2, 2));
        assert_eq!(e.text, "x");
        // pure deletion: empty replacement
        let e = Edit::between("abx", "ab");
        assert_eq!((e.start, e.old_end), (2, 3));
        assert_eq!(e.text, "");
        // identical input: a no-op edit at EOF
        let e = Edit::between("same", "same");
        assert_eq!((e.start, e.old_end), (4, 4));
        assert_eq!(e.text, "");
    }

    #[test]
    fn source_edit_maintains_the_line_table() {
        let mut s = src_of("a\nb\nc");
        assert_eq!(s.line_starts(), [0, 2, 4]);
        // insertion carrying newlines
        s.edit(&Edit::between("a\nb\nc", "a\nx\nyb\nc"));
        assert_eq!(s.text(), "a\nx\nyb\nc");
        assert_eq!(s.line_starts(), [0, 2, 4, 7]);
        // deletion of a newline
        s.edit(&Edit::between("a\nx\nyb\nc", "a\nx\nybc"));
        assert_eq!(s.text(), "a\nx\nybc");
        assert_eq!(s.line_starts(), [0, 2, 4]);
        // multi-line replacement collapses and reopens lines
        s.edit(&Edit::between("a\nx\nybc", "a\nZ\nW\nybc"));
        assert_eq!(s.text(), "a\nZ\nW\nybc");
        assert_eq!(s.line_starts(), [0, 2, 4, 6]);
        // append at EOF without a newline: no new line
        s.edit(&Edit::between("a\nZ\nW\nybc", "a\nZ\nW\nybc!"));
        assert_eq!(s.line_starts(), [0, 2, 4, 6]);
        // append with a trailing newline: the empty last line opens
        s.edit(&Edit::between("a\nZ\nW\nybc!", "a\nZ\nW\nybc!\n"));
        assert_eq!(s.text(), "a\nZ\nW\nybc!\n");
        assert_eq!(s.line_starts(), [0, 2, 4, 6, 11]);
    }

    #[test]
    fn source_edit_matches_a_cold_recount_over_random_shapes() {
        // the incremental table ≡ a recount, on every hand shape the
        // lane can produce (the corpus-scale property test rides the
        // splice suite)
        let shapes = [
            ("", "x"),
            ("a\nb", "a\nb\nc\n"),
            ("a\nb\nc", ""),
            ("line one\nline two\nline three\n", "line one\nline three\n"),
            ("x = \"a\\nb\"\ny = 2\n", "x = \"a\\nb\"\nz = 3\ny = 2\n"),
            (
                "fn f() {\n    r#\"\nbody\n\"#\n}\n",
                "fn f() {\n    r#\"\nbody2\n\"#\n}\n",
            ),
        ];
        for (old, new) in shapes {
            let mut s = src_of(old);
            s.edit(&Edit::between(old, new));
            assert_eq!(s.text(), new);
            assert_eq!(s.line_starts(), src_of(new).line_starts(), "{new:?}");
        }
    }

    #[test]
    fn line_start_of_finds_the_containing_line() {
        // "ab\ncd\n": the newline byte itself closes its line — line 2
        // starts at 3, the empty last line at 6
        let s = src_of("ab\ncd\n");
        assert_eq!(s.line_start_of(0), 0);
        assert_eq!(s.line_start_of(2), 0);
        assert_eq!(s.line_start_of(3), 3);
        assert_eq!(s.line_start_of(4), 3);
        assert_eq!(s.line_start_of(5), 3);
        assert_eq!(s.line_start_of(6), 6);
    }
}
