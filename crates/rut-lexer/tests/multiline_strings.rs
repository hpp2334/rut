//! The hash-leveled string literal law — `r#"..."#` and `f#"..."#`.
//!
//! The hash count is the multi-line opt-in AND the quote-embedding
//! level: level 0 (`"..."`, `f"..."`, `r"..."`) keeps the single-line
//! law byte-for-byte (a newline is an "unterminated" diagnostic), level
//! `n >= 1` lets newlines through and closes at the first `"` followed
//! by at least `n` `#`s. Every byte between the delimiters is the value
//! — no escapes (raw), no indentation stripping, no newline magic.

use rut_lexer::lexer::lex;
use rut_lexer::token::Tok;

/// The single `RawStr` token of a one-literal source, plus its diags.
fn raw_of(src: &str) -> (String, usize) {
    let (toks, diags) = lex(src);
    let strs: Vec<&String> = toks
        .iter()
        .filter_map(|t| match &t.tok {
            Tok::RawStr(s) => Some(s),
            _ => None,
        })
        .collect();
    assert_eq!(strs.len(), 1, "{src:?}: expected one raw literal");
    (strs[0].clone(), diags.len())
}

/// The single `FStr` token of a one-literal source, plus its diags.
fn fstr_of(src: &str) -> (rut_lexer::token::FStrTok, usize) {
    let (toks, diags) = lex(src);
    let fs: Vec<&rut_lexer::token::FStrTok> = toks
        .iter()
        .filter_map(|t| match &t.tok {
            Tok::FStr(f) => Some(f),
            _ => None,
        })
        .collect();
    assert_eq!(fs.len(), 1, "{src:?}: expected one format literal");
    (fs[0].clone(), diags.len())
}

fn lit_chunks(f: &rut_lexer::token::FStrTok) -> Vec<&str> {
    f.parts
        .iter()
        .filter_map(|p| match p {
            rut_lexer::token::FPart::Lit(s) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------- level 0
// frozen: every pre-hash-form behavior is asserted here so a future
// edit cannot silently relax the single-line law.

#[test]
fn level0_laws_are_unchanged() {
    // plain / raw / format still close at the first `"`
    assert_eq!(raw_of(r#"r"a"b""#).0, "a");
    // ... still diagnose a newline inside the body (sources end at the
    // newline so the case yields exactly the one diagnostic)
    assert_eq!(raw_of("r\"a\nb").1, 1);
    assert_eq!(fstr_of("f\"a{x}b\n").1, 1);
    // ... and the plain string still terminates on its own line
    let (toks, diags) = lex("\"a\nb");
    assert_eq!(diags.len(), 1, "plain multi-line body must diagnose");
    assert!(matches!(toks[0].tok, Tok::Str(ref s) if s == "a"));
}

#[test]
fn level0_raw_embeds_nothing() {
    // a `"` at level 0 always closes — value can never contain one
    assert_eq!(raw_of(r#"r"x""#).0, "x");
    assert_eq!(raw_of(r#"r"""#).0, "");
}

// ---------------------------------------------------------------- raw n=1

#[test]
fn raw1_spans_lines_verbatim() {
    let (v, d) = raw_of("r#\"a\nb\"#");
    assert_eq!(d, 0);
    assert_eq!(v, "a\nb");
    // the leading newline after the opener is part of the value
    let (v, d) = raw_of("r#\"\n  select id\n  from users\n\"#");
    assert_eq!(d, 0);
    assert_eq!(v, "\n  select id\n  from users\n");
}

#[test]
fn raw1_embeds_quotes() {
    let (v, d) = raw_of(r##"r#"she said "never""#"##);
    assert_eq!(d, 0);
    assert_eq!(v, "she said \"never\"");
    // a `"` with a SHORT `#` run is content, run included
    let (v, d) = raw_of(r###"r##"flag = "# done"##"###);
    assert_eq!(d, 0);
    assert_eq!(v, "flag = \"# done");
}

#[test]
fn raw1_embeds_backslash_and_quotes_together() {
    // raw: `\` is an ordinary byte at every level
    let (v, d) = raw_of(r##"r#"C:\temp\"x""#"##);
    assert_eq!(d, 0);
    assert_eq!(v, "C:\\temp\\\"x\"");
}

#[test]
fn raw_levels_embed_hash_runs() {
    // level 2 embeds `"#`; level 3 embeds `"##`
    let (v, _) = raw_of(r###"r##"a"#b"##"###);
    assert_eq!(v, "a\"#b");
    let (v, _) = raw_of(r####"r###"a"##b"###"####);
    assert_eq!(v, "a\"##b");
}

#[test]
fn raw_multiline_one_liner() {
    // hashes without newlines are legal too — embedding is their other
    // job. `r#""#` is the EMPTY string at level 1; one content `"` needs
    // one more
    let (v, d) = raw_of(r##"r#""#"##);
    assert_eq!(d, 0);
    assert_eq!(v, "");
    let (v, d) = raw_of("r#\"\"\"#");
    assert_eq!(d, 0);
    assert_eq!(v, "\"");
}

#[test]
fn raw_multiline_utf8_across_lines() {
    let (v, d) = raw_of("r#\"héllo\nwörld\"#");
    assert_eq!(d, 0);
    assert_eq!(v, "h\u{e9}llo\nw\u{f6}rld");
}

#[test]
fn raw_multiline_crlf_normalizes_to_lf() {
    // the pre-lex CRLF normalization reaches inside the literal:
    // multi-line raw values are LF-only by construction
    let (v, d) = raw_of("r#\"a\r\nb\"#");
    assert_eq!(d, 0);
    assert_eq!(v, "a\nb");
}

#[test]
fn raw_multiline_eof_diagnoses_at_opener() {
    let src = "let x = r#\"abc\ndef";
    let (toks, diags) = lex(src);
    assert_eq!(diags.len(), 1);
    assert!(diags[0].msg.contains("unterminated raw string"));
    // the partial literal still tokens (same law as level 0), its span
    // — and the diag's — anchored at the OPENING quote even though the
    // body swallowed the rest of the file
    assert_eq!(toks.len(), 5); // let x = <raw> Eof
    assert!(matches!(&toks[3].tok, Tok::RawStr(s) if s == "abc\ndef"));
    assert_eq!(toks[3].span.lo as usize, 8);
    assert_eq!(diags[0].span.lo, toks[3].span.lo);
}

// ------------------------------------------------------------- malformed

#[test]
fn hashes_without_quote_diagnose() {
    let (_, diags) = lex("let x = r#abc");
    assert_eq!(diags.len(), 1);
    assert!(diags[0].msg.contains("expected `\"` after `r#`"));
    let (_, diags) = lex("let x = f## y");
    assert_eq!(diags.len(), 1);
    assert!(diags[0].msg.contains("expected `\"` after `f##`"));
}

#[test]
fn surplus_hashes_after_close_are_stray() {
    // close consumes exactly n of the run — the surplus `#` lexes alone
    let (_, diags) = lex(r###"r#"x"##"###);
    assert!(
        diags
            .iter()
            .any(|d| d.msg.contains("unexpected character `#`")),
        "surplus `#` after the close must be a stray-character diag"
    );
}

// -------------------------------------------------------------- f-string

#[test]
fn fstr1_spans_lines_and_keeps_holes() {
    let (f, d) = fstr_of("f#\"a\n{x}\nb\"#");
    assert_eq!(d, 0);
    assert_eq!(lit_chunks(&f), vec!["a\n", "\nb"]);
    match &f.parts[1] {
        rut_lexer::token::FPart::Hole(hole) => {
            // the hole stream: Ident(x), Eof
            assert!(matches!(&hole[0].tok, Tok::Ident(s) if s == "x"));
        }
        other => panic!("expected a hole, got {other:?}"),
    }
}

#[test]
fn fstr1_embeds_quotes_and_braces() {
    // `"` not followed by n `#`s is a chunk byte; {{ }} stay literal
    let (f, d) = fstr_of(r##"f#"say "hi" {{x}} {y}"#"##);
    assert_eq!(d, 0);
    assert_eq!(lit_chunks(&f), vec!["say \"hi\" {x} "]);
}

#[test]
fn fstr1_escapes_still_work() {
    let (f, d) = fstr_of("f#\"a\\tb\\nc {1}\"#");
    assert_eq!(d, 0);
    assert_eq!(lit_chunks(&f), vec!["a\tb\nc "]);
}

#[test]
fn fstr_holes_stay_single_line() {
    // a hole spilling past its line diagnoses, multi-line body or not
    let (_, diags) = lex("f#\"a {\nx} b\"#");
    assert_eq!(diags.len(), 1);
    assert!(diags[0].msg.contains("unterminated format-string hole"));
}

#[test]
fn fstr1_eof_diagnoses() {
    let (_, diags) = lex("f#\"abc\ndef");
    assert_eq!(diags.len(), 1);
    assert!(diags[0].msg.contains("unterminated format string"));
}

#[test]
fn raw_and_format_still_do_not_combine() {
    // `rf#..."#` is the word `rf`, two stray `#`s, and a plain string —
    // never a special token
    let (toks, diags) = lex("rf#\"x\"#");
    assert!(matches!(&toks[0].tok, Tok::Ident(s) if s == "rf"));
    assert!(matches!(&toks[1].tok, Tok::Str(s) if s == "x"));
    assert_eq!(
        diags
            .iter()
            .filter(|d| d.msg.contains("unexpected character `#`"))
            .count(),
        2
    );
}

// ------------------------------------------------------------- patterns

#[test]
fn hash_levels_do_not_leak_into_words() {
    // `r` and `f` stay ordinary identifiers when not glued to a literal
    let (toks, diags) = lex("let r = 1; let f = 2;");
    assert_eq!(diags.len(), 0);
    assert!(toks
        .iter()
        .any(|t| matches!(&t.tok, Tok::Ident(s) if s == "r")));
    assert!(toks
        .iter()
        .any(|t| matches!(&t.tok, Tok::Ident(s) if s == "f")));
}
