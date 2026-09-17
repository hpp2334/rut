//! Line index — byte offsets into the **normalized** source (CRLF→LF,
//! the exact text `rut_lexer::lexer::lex` produces spans over) ⇄ LSP
//! positions (line, UTF-16 code-unit column). LSP positions are UTF-16 by
//! default; spans are bytes — this is the only place that converts.

pub struct LineIndex {
    /// byte offset of each line start; `line_starts[0] == 0` always
    line_starts: Vec<u32>,
    len: u32,
}

impl LineIndex {
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

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_round_trip() {
        let src = "fn main() -> nil {\n    let x = 1;\n}\n";
        let ix = LineIndex::new(src);
        assert_eq!(ix.line_count(), 4);
        for (byte, _) in src.bytes().enumerate() {
            let byte = byte as u32;
            let (l, c) = ix.position(src, byte);
            assert_eq!(ix.byte(src, l, c), byte, "round trip failed at {byte}");
        }
    }

    #[test]
    fn crlf_normalization_keeps_positions() {
        // CR removal shifts byte offsets but never line/UTF-16 positions —
        // ranges reported against the editor's original text stay correct.
        let original = "let a = 1;\r\nlet b = 2;\r\n";
        let normalized = rut_lexer::lexer::normalize(original);
        assert_eq!(normalized, "let a = 1;\nlet b = 2;\n");
        let ix = LineIndex::new(&normalized);
        assert_eq!(ix.position(&normalized, 11), (1, 0)); // `let b` line start
        assert_eq!(ix.byte(&normalized, 1, 0), 11);
    }

    #[test]
    fn astral_plane_counts_two_units() {
        let src = "let x = \"𝕏\"; // wide";
        let ix = LineIndex::new(src);
        let wide = src.find('𝕏').unwrap() as u32;
        let (l, c) = ix.position(src, wide);
        assert_eq!((l, c), (0, 9)); // `let x = "` is 9 UTF-16 units
        // the char after the astral one sits 2 UTF-16 units later
        let after = wide + '𝕏'.len_utf8() as u32;
        assert_eq!(ix.position(src, after), (0, 11));
        assert_eq!(ix.byte(src, 0, 11), after);
    }

    #[test]
    fn clamps() {
        let src = "ab\ncd";
        let ix = LineIndex::new(src);
        assert_eq!(ix.position(src, 999), (1, 2));
        assert_eq!(ix.byte(src, 9, 0), 5);
        assert_eq!(ix.byte(src, 0, 99), 3); // clamps to line end (incl. \n)
    }
}
