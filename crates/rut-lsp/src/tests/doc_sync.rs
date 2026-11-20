//! The sync lane — the shell's one edit door. Pins: `sync(text)` ≡ a
//! cold parse of the same text (tokens, tree, line view), the lazy
//! views drop (a change re-derives them), and a no-op sync is a
//! no-op.

use crate::semantic::Document;
use rut_ast::parser::Mode;
use rut_lexer::source::Edit;
use rut_lexer::span::Span;

#[test]
fn sync_equals_a_cold_parse() {
    let src0 = "fn area(w: i32, h: i32) -> i32 {\n    w * h\n}\n\nfn main() -> nil {\n    let x = area(2, 3);\n    print(x);\n}\n";
    let mut d = Document::parse(Mode::Impl, src0.to_string());
    let new = "fn area(w: i32, h: i32) -> i32 {\n    w * h\n}\n\nfn main() -> nil {\n    let x = area(20, 30);\n    print(x);\n}\n";
    // the lazy views pay once before the sync — then drop
    let _ = d.spans();
    let _ = d.symbols();
    d.sync(new);
    let cold = Document::parse(Mode::Impl, new.to_string());
    assert_eq!(d.toks(), cold.toks(), "tokens ≡ cold lex after the sync");
    assert_eq!(
        format!("{:?}", d.ast().root().items(d.ast())),
        format!("{:?}", cold.ast().root().items(cold.ast())),
        "the tree survived the edit (shape ≡ cold)"
    );
    assert_eq!(d.diags(), cold.diags(), "the flat panel ≡ cold");
    // the UTF-16 view rebuilt over the FRESH text: the edited literals
    // land where the new text puts them
    let (l, c) = d.position(new.find("20").unwrap() as u32);
    assert_eq!((l, c), (5, 17), "the position view rides the new text");
    // the dropped lazies re-derive coherently (the fresh spans cover
    // the new text's tokens — a stale span would run past the text)
    for (sp, _) in d.spans() {
        assert!(
            sp.hi <= new.len() as u32,
            "a stale span survived the sync: {sp:?}"
        );
    }
}

#[test]
fn a_noop_sync_is_a_noop() {
    let src = "let ok = 1;\n";
    let mut d = Document::parse(Mode::Impl, src.to_string());
    let before = format!("{:?}", d.toks());
    d.sync(src);
    assert_eq!(format!("{:?}", d.toks()), before, "tokens unmoved");
    assert_eq!(d.position(d.src().len() as u32), (1, 0), "lines unmoved");
}

#[test]
fn the_span_join_answers_over_the_synced_text() {
    // the decl layer's verbatim slices ride the ONE text door — after
    // a sync, a span join reads the NEW text (never yesterday's)
    let src0 = "let a = 1;\nlet b = 2;\n";
    let mut d = Document::parse(Mode::Impl, src0.to_string());
    let new = "let a = 1;\nlet b = 22;\n";
    d.sync(new);
    assert_eq!(
        d.text(Span::new(15, 16)),
        "b",
        "the surviving prefix's span"
    );
    assert_eq!(
        d.text(Span::new(19, 21)),
        "22",
        "the edited literal's span joins the new text"
    );
    // `Edit::between` diffing is the shell's only input — the common
    // prefix ends at the edit, the splice rides it
    let e = Edit::between(src0, new);
    assert_eq!(e.start, 20, "the common prefix ends at the edit");
}
