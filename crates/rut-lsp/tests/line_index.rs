use rut_lsp::line_index::*;

#[test]
fn ascii_round_trip() {
    let src = "entry fn main() -> nil {\n    let x = 1;\n}\n";
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
