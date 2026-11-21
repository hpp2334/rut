//! The line index's gate — the byte ⇄ (line, UTF-16 column) walk the
//! server runs on every request, driven through the envelopes: CRLF
//! normalization keeps LSP positions over the editor's original text,
//! an astral-plane character counts two UTF-16 units, and the clamps
//! degrade honestly (out-of-range positions answer null/empty, never
//! a panic). The byte-scan equality itself is pinned by every range
//! assert in the tree (the decode half of each span check).

mod common;

use common::*;

/// the UTF-16 column a byte offset sits at over `src` (the encode half
/// of the LSP position contract)
fn utf16_col(src: &str, byte: usize) -> u32 {
    src[..byte].chars().map(|c| c.len_utf16() as u32).sum()
}

#[test]
fn crlf_normalization_keeps_positions() {
    // CR removal shifts byte offsets but never line/UTF-16 positions —
    // ranges reported against the editor's ORIGINAL text stay correct:
    // the diag range decodes onto the same text slice in the CRLF
    // source the client holds
    let mut s = S::boot();
    let original = "entry fn main() -> nil {\n    let x = ;\n}\r\nlet b = 2;\r\n";
    let uri = "file:///t/crlf.rut";
    let a = s.analyze(uri, original);
    let diags = a["diags"].as_array().unwrap();
    assert!(!diags.is_empty(), "{a}");
    let d = &diags[0];
    // decode the range against the ORIGINAL text: the positions land
    // inside line 1 (`    let x = ;`), whose columns are untouched by
    // the CR removal (it lives on later lines)
    let start = pos_pair(&d["range"]["start"]);
    let end = pos_pair(&d["range"]["end"]);
    assert_eq!(start.0, 1, "the broken line: {d}");
    let line1_start = original.find("    let x").unwrap();
    let lo = byte_at(original, start.0 as u32, start.1 as u32);
    let hi = byte_at(original, end.0 as u32, end.1 as u32);
    assert!(
        lo >= line1_start && hi <= line1_start + "    let x = ;".len(),
        "the diag decodes onto the original line: {start:?}..{end:?}"
    );
    // and the CR-carrying line's positions ride the SAME convention:
    // hover a name on the CRLF line at its UTF-16 column
    let h = s.hover_pos(uri, 3, 4); // `b` of `let b = 2;` (line 3 after \r\n)
    let md = markdown(h);
    assert!(
        md.contains("let b = 2"),
        "the hover decodes the CR-carrying line: {md}"
    );
}

#[test]
fn astral_plane_counts_two_units() {
    // a wide char before the cursor shifts UTF-16 columns by 2 — the
    // hover position a client computes must land on the ident the
    // server's own index decodes
    let mut s = S::boot();
    let src = "let x = \"𝕏\"; let n = 1;\n";
    let uri = "file:///t/wide.rut";
    s.analyze(uri, src);
    // the char after the astral one sits 2 UTF-16 units later: `n`'s
    // column is its byte column + 2
    let n_byte = byte_of(src, "let n", 1) + 4;
    let col = utf16_col(src, n_byte as usize);
    // a client that counted the astral char as ONE unit would ask for
    // col - 1 — the server's walk must NOT land there
    let h = s.hover_pos(uri, 0, col);
    let md = markdown(h);
    assert!(
        md.contains("let n = 1"),
        "the hover lands on `n` at the UTF-16 column: {md}"
    );
    let h = s.hover_pos(uri, 0, col - 1);
    assert!(
        no_hover(&h) || !markdown(h).contains("let n = 1"),
        "one unit short is not `n` — the astral char counts two"
    );
}

#[test]
fn clamps_degrade_honestly() {
    // out-of-range positions clamp on the server's own walk — the
    // answers are the honest null/empty shapes, never a panic
    let mut s = S::boot();
    let src = "entry fn main() -> nil {\n}\n";
    let uri = "file:///t/clamp.rut";
    s.analyze(uri, src);
    let h = s.hover_pos(uri, 999, 0);
    assert!(no_hover(&h), "a line past EOF: {h}");
    let h = s.hover_pos(uri, 0, 999);
    assert!(no_hover(&h), "a column past the line end: {h}");
    let d = s.def_pos(uri, 9, 0);
    assert!(d.is_empty(), "a line past EOF defines nothing: {d:?}");
    let items = s.complete_pos(uri, src, 0, 999);
    assert!(
        !items.is_empty(),
        "a clamped column at the line end still completes the bare tier: {:?}",
        labels(&items)
    );
    let r = s.refs_pos(uri, (999, 0), true);
    assert!(r.is_empty(), "a line past EOF references nothing: {r:?}");
    let h = s.sig_pos(uri, (999, 0));
    assert!(h.is_null(), "a line past EOF helps nobody: {h}");
}
