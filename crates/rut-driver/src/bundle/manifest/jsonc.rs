//! The JSONC front stage — comments and trailing commas leave the text
//! BEFORE `serde_json` sees it, so the value parser and its error
//! currency stay exactly what they were.
//!
//! The law this module implements: the manifest is **JSONC** — `//`
//! line comments, `/* */` block comments, and trailing commas are all
//! legal, everywhere the grammar allows a value — and `serde_json`
//! remains THE value parser. Instead of a second parser (whose error
//! wording would fork serde_json's), this is a **stripper**: one pass
//! that rewrites the text into plain JSON without moving a byte.
//!
//! THE POSITION LAW: the stripped text has the SAME LENGTH as the
//! input — every comment byte and every elided trailing comma becomes
//! a space (`b' '`), and every `\n` is preserved (a line comment ends
//! at its newline; a block comment's newlines are kept). So
//! `serde_json`'s `line N:` always names the ORIGINAL file's line —
//! there is no remapping layer to trust, because nothing moved.
//! (Columns may shift left by the count of elided commas earlier on
//! the line; the syntax-error shape drops columns, it keeps `line N`
//! only.)
//!
//! Why hand-rolled (the crate survey, 2026-10): `jsonc-parser` and the
//! `serde_jsonc` forks are PARSERS — they would displace `serde_json`
//! as the value parser and fork the `line N:` error shape; `json5` is
//! a wider grammar than the ruling; `json_comments` is the right
//! SHAPE (a stripper) but has no trailing-comma support at all and its
//! block-comment handler blanks newlines — which would shift every
//! subsequent line number. None satisfies the ruling, so this private
//! state machine (~100 lines) does, and its table below pins every
//! corner: comments in every position, strings immune, first `*/`
//! closes, CRLF, and the original-line law.

/// Strip JSONC syntax: comment bytes and trailing-comma bytes become
/// spaces, everything else (including every newline) rides verbatim.
/// The output is always the same length as the input and always valid
/// UTF-8 (blanked bytes are ASCII spaces; multibyte characters outside
/// comments are never split, and inside comments a half-blanked
/// character is still just spaces).
pub(super) fn strip(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = bytes.to_vec();
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            match b {
                b'\\' => escaped = !escaped,
                b'"' if !escaped => in_string = false,
                _ => escaped = false,
            }
            i += 1;
            continue;
        }
        match b {
            b'"' => {
                in_string = true;
                i += 1;
            }
            // `// …` — a line comment ends at its newline (kept, so the
            // line count never moves); every other byte becomes a space
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    out[i] = b' ';
                    i += 1;
                }
            }
            // `/* … */` — block comment; NOT nestable: the first `*/`
            // closes it. Newlines inside are kept (the line law), all
            // else is space. An unterminated comment blanks to EOF and
            // `serde_json` reports the EOF it then sees.
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                out[i] = b' ';
                out[i + 1] = b' ';
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                        out[i] = b' ';
                        out[i + 1] = b' ';
                        i += 2;
                        break;
                    }
                    if bytes[i] != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
            }
            // a trailing comma — the comma whose next SIGNIFICANT byte
            // (skipping whitespace and comments) is a `}` or `]` — is
            // elided (a space); every real comma rides
            b',' if trailing(bytes, i + 1) => {
                out[i] = b' ';
                i += 1;
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).expect("the stripper only writes ASCII spaces over ASCII bytes")
}

/// Is the comma at `comma` trailing? Scan forward from `from`, skipping
/// whitespace and whole comments (a comment may sit between the comma
/// and its closer); `true` iff the first significant byte is the `}`
/// or `]` that closes the comma's object/array. Strings never start
/// that closer, so the scan does not enter one.
fn trailing(bytes: &[u8], mut from: usize) -> bool {
    while from < bytes.len() {
        match bytes[from] {
            b' ' | b'\t' | b'\r' | b'\n' => from += 1,
            b'/' if bytes.get(from + 1) == Some(&b'/') => {
                from += 2;
                while from < bytes.len() && bytes[from] != b'\n' {
                    from += 1;
                }
            }
            b'/' if bytes.get(from + 1) == Some(&b'*') => {
                from += 2;
                loop {
                    if from >= bytes.len() {
                        return false; // unterminated: not a trailing comma
                    }
                    if bytes[from] == b'*' && bytes.get(from + 1) == Some(&b'/') {
                        from += 2;
                        break;
                    }
                    from += 1;
                }
            }
            b'}' | b']' => return true,
            _ => return false,
        }
    }
    false // a comma at EOF is a syntax error, unchanged law
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- the invariants: length and newlines never move ----

    #[test]
    fn plain_json_rides_byte_for_byte() {
        // no comments, no trailing commas: the stripper is identity
        let text = r#"{"name": "pouch", "entry": {"lib": "./pouch.rut"}}"#;
        assert_eq!(strip(text), text);
    }

    #[test]
    fn the_output_never_moves_a_byte() {
        // same length, same newlines — THE position law
        let text = "{\n  // header\n  \"name\": \"x\", /* mid */\n  \"entry\": {\"lib\": \"./x.rut\"},\n}\n";
        let out = strip(text);
        assert_eq!(out.len(), text.len());
        assert_eq!(out.bytes().filter(|&b| b == b'\n').count(), 5);
        // and it now parses as plain JSON
        assert!(serde_json::from_str::<serde_json::Value>(&out).is_ok());
    }

    // ---- comments in every position; strings are immune ----

    #[test]
    fn comments_in_every_table_position() {
        let text = r#"{
  // before the root's first key
  "name": "x", // after a value
  /* a block comment on its own line */
  "entry": {
    // inside entry
    "lib": "./x.rut", /* beside a value */
  },
  "deps": {
    // inside the deps table
    "core": {
      // inside a descriptor
      "path": "rut/core", // trailing prose
    },
  },
  "style": {
    // inside style
    "indent": "2",
  },
}
// after the root"#;
        let v: serde_json::Value = serde_json::from_str(&strip(text)).unwrap();
        assert_eq!(v["name"], "x");
        assert_eq!(v["entry"]["lib"], "./x.rut");
        assert_eq!(v["deps"]["core"]["path"], "rut/core");
        assert_eq!(v["style"]["indent"], "2");
    }

    #[test]
    fn comments_inside_strings_are_not_stripped() {
        // the string state machine: urls, comment marks, escaped quotes
        let text = r#"{
  "name": "a//b", /* real comment */
  "entry": {"lib": "./x.rut"},
  "url": "https://example.com/p.rutbundle // not a comment /* neither */",
  "prose": "he said \"hi\" // and the quote kept the string open",
  "block": "a/*b*/c"
}"#;
        let v: serde_json::Value = serde_json::from_str(&strip(text)).unwrap();
        assert_eq!(v["name"], "a//b");
        assert_eq!(
            v["url"],
            "https://example.com/p.rutbundle // not a comment /* neither */"
        );
        assert_eq!(v["prose"], "he said \"hi\" // and the quote kept the string open");
        assert_eq!(v["block"], "a/*b*/c");
    }

    #[test]
    fn comment_marks_in_keys_ride() {
        let text = r#"{"na//me": "x", "en/*try*/": {"lib": "./x.rut"}}"#;
        let v: serde_json::Value = serde_json::from_str(&strip(text)).unwrap();
        assert_eq!(v["na//me"], "x");
        assert_eq!(v["en/*try*/"]["lib"], "./x.rut");
    }

    // ---- trailing commas at every nesting level ----

    #[test]
    fn trailing_commas_at_every_nesting_level() {
        let text = r#"{
  "name": "x",
  "entry": {"lib": "./x.rut", "libs": ["./a.rut", "./b.rut",]},
  "deps": {"core": {"path": "rut/core",},},
  "peer-deps": {"p": {"path": "..", "optional": true,}},
  "style": {"indent": "2",},
}"#;
        let v: serde_json::Value = serde_json::from_str(&strip(text)).unwrap();
        assert_eq!(v["entry"]["libs"].as_array().unwrap().len(), 2);
        assert_eq!(v["deps"]["core"]["path"], "rut/core");
        assert_eq!(v["peer-deps"]["p"]["optional"], true);
        assert_eq!(v["style"]["indent"], "2");
    }

    #[test]
    fn a_comment_may_sit_between_the_comma_and_its_closer() {
        let text = "{\n  \"a\": [1, /* why */ ],\n  \"b\": 1, // why\n}";
        let v: serde_json::Value = serde_json::from_str(&strip(text)).unwrap();
        assert_eq!(v["a"].as_array().unwrap().len(), 1);
        assert_eq!(v["b"], 1);
    }

    #[test]
    fn real_commas_ride() {
        // a comma followed by another member is NOT trailing — the
        // elider must not touch it (the whole document parses only if
        // the real separators survived)
        let text = r#"{"name": "x", "entry": {"libs": ["./a.rut", "./b.rut"], "lib": "./x.rut"}}"#;
        assert_eq!(strip(text), text);
        let v: serde_json::Value = serde_json::from_str(&strip(text)).unwrap();
        assert_eq!(v["entry"]["libs"].as_array().unwrap().len(), 2);
    }

    // ---- CRLF ----

    #[test]
    fn crlf_lines_survive_comments_and_trailing_commas() {
        let text = "{\r\n  // header\r\n  \"name\": \"x\", /* mid */\r\n  \"entry\": {\"lib\": \"./x.rut\",},\r\n}\r\n";
        let out = strip(text);
        assert_eq!(out.len(), text.len());
        assert_eq!(out.matches("\r\n").count(), 4);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["entry"]["lib"], "./x.rut");
    }

    // ---- block comments: first `*/` closes; no nesting ----

    #[test]
    fn the_first_close_ends_a_block_comment() {
        // `/* a /* b */` is ONE comment: the inner `/*` is text
        let text = r#"/* a /* b */ {"name": "x"}"#;
        let v: serde_json::Value = serde_json::from_str(&strip(text)).unwrap();
        assert_eq!(v["name"], "x");
    }

    #[test]
    fn a_late_close_is_code_and_refuses() {
        // after the first `*/` the rest is code — `*/` out here is the
        // syntax error it always was
        let text = r#"{"name": "x"} /* outer /* inner */ still-open */"#;
        let err = serde_json::from_str::<serde_json::Value>(&strip(text)).unwrap_err();
        assert!(err.to_string().contains("trailing characters"), "{err}");
    }

    #[test]
    fn multiline_block_comments_keep_the_line_count() {
        let text = "{\n  /* one\n     two\n     three */\n  \"name\": \"x\"\n}";
        let out = strip(text);
        assert_eq!(out.matches('\n').count(), text.matches('\n').count());
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["name"], "x");
    }

    // ---- the error lanes: serde_json's own wording, original lines ----

    #[test]
    fn unterminated_block_comment_is_serde_jsons_eof() {
        let text = "{\n  /* never closed\n  \"name\": \"x\"\n}";
        let err = serde_json::from_str::<serde_json::Value>(&strip(text)).unwrap_err();
        assert!(err.to_string().contains("EOF while parsing"), "{err}");
    }

    #[test]
    fn a_bare_slash_is_the_syntax_error_it_always_was() {
        // a lone `/` starts nothing — leave it for serde_json to refuse
        let text = "{\n  \"name\": \"x\",\n  / bad\n}";
        let err = serde_json::from_str::<serde_json::Value>(&strip(text)).unwrap_err();
        assert!(err.to_string().contains("line 3"), "{err}");
    }

    #[test]
    fn syntax_errors_name_the_original_line_after_jsonc_syntax() {
        // THE position law, end to end: comments and an elided comma
        // above the mistake do not move the line serde_json reports
        let text = "{\n  // the header\n  /* multi\n     line */\n  \"name\": \"x\",\n  \"entry\": {\"lib\": six},\n}";
        let err = serde_json::from_str::<serde_json::Value>(&strip(text)).unwrap_err();
        assert!(err.to_string().contains("line 6"), "{err}");
    }
}
