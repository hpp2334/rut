//! The line-table law — one entry per newline, on terminated input
//! and unterminated input alike (`source.rs`'s contract: the table
//! matches a cold recount exactly). The entries ride the SAME byte
//! scan as the tokens (`lex_normalized` returns them beside them), so
//! a newline the scan consumes must count wherever it sits — the last
//! byte of an unterminated block comment inside an f-string hole
//! included.

use rut_lexer::lexer::lex_normalized;

#[test]
fn the_line_table_counts_the_last_newline_inside_a_hole_comment() {
    // the hole's comment is unterminated and swallows the rest of the
    // file; its LAST byte is the final newline, and the empty last
    // line it opens must be in the table ([0, 10, 21] — newlines at 9
    // and 20)
    let src = "f\"{/*;\"};\nlet a = 1;\n";
    let (_, _, _, lines) = lex_normalized(src, false);
    assert_eq!(lines, vec![0, 10, 21], "{src:?}: one entry per newline");
}
