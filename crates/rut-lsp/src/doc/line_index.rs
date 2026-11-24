//! Line index — byte offsets into the **normalized** source (CRLF→LF,
//! the exact text `rut_lexer::lexer::lex` produces spans over) ⇄ LSP
//! positions (line, UTF-16 code-unit column). LSP positions are UTF-16 by
//! default; spans are bytes — this is the only place that converts.
//!
//! ONE line table per artifact: the geometry comes from the rut-ast
//! [`Doc`]'s own line doors (`line_span` — the `Source` table the parse
//! lexed once), never a second byte scan.

use rut_ast::Doc;

#[derive(Debug, Clone)]
pub struct LineIndex {
    /// byte offset of each line start; `line_starts[0] == 0` always
    line_starts: Vec<u32>,
    len: u32,
}

impl LineIndex {
    /// the doc's own line geometry — one walk over the line doors
    /// (`line_of` names the last line, `line_span` yields each start),
    /// no second byte scan. Byte-identical to the old `\n` byte scan:
    /// a trailing newline registers the empty final line.
    pub fn of(doc: &Doc) -> LineIndex {
        let last = doc.line_of(doc.len());
        let mut line_starts = Vec::with_capacity(last as usize + 1);
        for line in 0..=last {
            line_starts.push(doc.line_span(line).lo);
        }
        LineIndex {
            line_starts,
            len: doc.len(),
        }
    }

    /// build over raw text — the lanes that hold text but no `Doc`
    /// (the check lane's per-file rows) pay their own scan here
    pub fn new(src: &str) -> LineIndex {
        let mut line_starts = vec![0u32];
        for (i, &b) in src.as_bytes().iter().enumerate() {
            if b == b'\n' {
                line_starts.push((i + 1) as u32);
            }
        }
        LineIndex {
            line_starts,
            len: src.len() as u32,
        }
    }

    /// byte offset -> (line, UTF-16 column); clamps to file end
    pub fn position(&self, src: &str, byte: u32) -> (u32, u32) {
        let byte = byte.min(self.len);
        let line = match self.line_starts.binary_search(&byte) {
            Ok(l) => l,
            Err(l) => l - 1,
        };
        let start = self.line_starts[line] as usize;
        let mut col = 0u32;
        for c in src[start..byte as usize].chars() {
            col += c.len_utf16() as u32;
        }
        (line as u32, col)
    }

    /// (line, UTF-16 column) -> byte offset; clamps to the line/file end
    pub fn byte(&self, src: &str, line: u32, character: u32) -> u32 {
        let Some(&start) = self.line_starts.get(line as usize) else {
            return self.len;
        };
        let start = start as usize;
        let end = self
            .line_starts
            .get(line as usize + 1)
            .map(|&e| e as usize)
            .unwrap_or(src.len());
        let mut units = 0u32;
        for (off, c) in src[start..end].char_indices() {
            if units >= character {
                return (start + off) as u32;
            }
            units += c.len_utf16() as u32;
        }
        end as u32
    }
}
