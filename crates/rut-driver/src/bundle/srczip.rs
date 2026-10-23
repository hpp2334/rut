//! The `.zip` source transport — the directory grammar over the wire.
//! A source zip is a package directory packed for travel: `rut.jsonc`
//! at the archive root, the module tree under `src/`, and every dep
//! riding embedded under `deps/<name>@<fp16>/` — a source dep as its
//! own tree (`rut.jsonc` + `src/`), a compiled dep as its `.rutbundle`
//! file verbatim. The published manifest's `[deps]` rows all spell
//! `path` into that `deps/` area ([`rewrite_for_zip`]); the consumer
//! unpacks once and mounts an ordinary directory package.
//!
//! The laws this module owns:
//!
//! - **Placement** ([`validate_grammar`]): `.rut` code lives ONLY
//!   under a `src/` shell; `rut.jsonc` only at a package root
//!   (the archive root or `deps/<identity>/`); a `.rutbundle` only as
//!   a dep's IR file. Everything else rides inert (README, LICENSE,
//!   tooling droppings — nothing reads them).
//! - **Slip + hygiene**: no absolute paths, no `..`, no backslashes,
//!   no empty components, no dot entries; explicit directory entries
//!   (trailing `/`) are skipped.
//! - **Determinism** ([`write_srczip`]): STORE entries, fixed order as
//!   given, the DOS epoch — the bundle writer's knobs, shared. Same
//!   input ⇒ byte-identical zip, so published zips are cacheable and
//!   pin-stable.
//! - **Reader subset**: the zip essentials (CRC verified in-read,
//!   zip64 refused like the bundle lane) plus DEFLATE — user-made zips
//!   compress; ours don't have to.
//!
//! The zip64 refusal and the CRC law ride the same mechanical layer
//! the bundle policy uses; this module is that layer's second
//! consumer, with its own grammar.

use std::collections::BTreeMap;

use super::identity::{is_identity, DEPS_DIR};
use super::zip::{self, ZipLayerError};
use super::MANIFEST_NAME;

/// The source shell every package carries — the only place `.rut`
/// code may live.
pub const SRC_DIR: &str = "src";

/// A malformed source zip — a load error, never a runtime trap.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SrcZipError(pub String);

/// Unpack a source zip: every entry CRC-verified in-read, DEFLATE
/// accepted, zip64 refused, the placement grammar enforced. Explicit
/// directory entries (trailing `/`) are skipped; what remains is the
/// file set the unpacker lays down verbatim.
pub fn read_srczip(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, SrcZipError> {
    // the zip64 gate is OURS — the same sentinel scan the bundle
    // policy runs: the crate reads zip64 transparently, v1 refuses
    if !has_eocd(data) {
        return Err(SrcZipError(
            "not a zip archive (no end-of-central-directory)".into(),
        ));
    }
    let mut eocd = None;
    let min = 22usize;
    let start = data.len().saturating_sub(min + 0xFFFF);
    for i in (start..=data.len() - min).rev() {
        if data[i..i + 4] == [0x50, 0x4b, 0x05, 0x06] {
            eocd = Some(i);
            break;
        }
    }
    if let Some(at) = eocd {
        let n = get16(data, at + 10).unwrap_or(0) as usize;
        let cd_size = get32(data, at + 12).unwrap_or(0);
        let cd_offset = get32(data, at + 16).unwrap_or(0);
        if n == 0xFFFF || cd_size == u32::MAX || cd_offset == u32::MAX {
            return Err(SrcZipError(
                "zip64 source zips are not supported in v1".into(),
            ));
        }
    }
    // the method gate runs BEFORE any payload decodes: this reader
    // carries STORE (0) and DEFLATE (8) — anything else names its
    // entry, never a corruption report
    for (name, method) in zip::entry_methods(data).map_err(policy)? {
        if method != 0 && method != 8 {
            return Err(SrcZipError(format!(
                "source-zip entry `{name}` uses compression method {method} — this reader \
                 carries STORE and DEFLATE only"
            )));
        }
    }
    let entries = zip::read_archive(data).map_err(policy)?;
    let files: Vec<(String, Vec<u8>)> = entries
        .into_iter()
        .filter(|e| !e.name.ends_with('/'))
        .map(|e| (e.name, e.bytes))
        .collect();
    validate_grammar(&files)?;
    Ok(files)
}
/// Pack `(name, bytes)` entries into a source zip — deterministically
/// (the bundle writer's knobs: STORE, the DOS epoch, no extra fields,
/// order as given). The placement grammar is enforced here too: the
/// writer never emits what the reader would refuse.
pub fn write_srczip(entries: &[(String, Vec<u8>)]) -> Result<Vec<u8>, SrcZipError> {
    for (name, _) in entries {
        if name.is_empty() || name.len() > 0xFFFF || name.starts_with('/') || name.contains('\\') {
            return Err(SrcZipError(format!("bad source-zip entry name `{name}`")));
        }
    }
    validate_grammar(entries)?;
    zip::write_archive(entries).map_err(policy)
}

/// The placement grammar: exactly one `rut.jsonc` at the archive root;
/// every entry either rides a package root's three legal shapes
/// (`rut.jsonc`, `src/…`, `deps/<identity>/…`) or is inert. A `.rut`
/// outside any `src/` shell is THE refusal (the whole point of the
/// law: rut code lives under `src/`, never beside a manifest).
fn validate_grammar(entries: &[(String, Vec<u8>)]) -> Result<(), SrcZipError> {
    let roots = entries.iter().filter(|(n, _)| n == MANIFEST_NAME).count();
    if roots != 1 {
        return Err(SrcZipError(format!(
            "a source zip carries exactly one `{MANIFEST_NAME}` at the archive root (found {roots})"
        )));
    }
    for (name, _) in entries {
        check_entry(name)?;
    }
    Ok(())
}

/// One entry's placement, walked component by component: a package
/// root (the archive root, or inside some `deps/<identity>/`) may
/// carry `rut.jsonc`, descend into `src/` (anything legal under it),
/// descend into `deps/<identity>/` (recursively a package root), or an
/// inert file. A `.rutbundle` is legal only as a dep's IR file.
fn check_entry(name: &str) -> Result<(), SrcZipError> {
    let comps: Vec<&str> = name.split('/').collect();
    for c in &comps {
        if c.is_empty() || c.starts_with('.') || *c == ".." {
            return Err(SrcZipError(format!(
                "source-zip entry `{name}` — component `{c}` is not a legal path part \
                 (no empty, dot, or `..` components)"
            )));
        }
    }
    let mut at_pkg_root = true;
    let mut depth = 0usize; // how many `deps/<identity>/` levels we are inside
    let mut i = 0usize;
    while i < comps.len() {
        let c = comps[i];
        let last = i + 1 == comps.len();
        if c == MANIFEST_NAME {
            if !at_pkg_root {
                return Err(SrcZipError(format!(
                    "source-zip entry `{name}` — `{MANIFEST_NAME}` rides only at a package root"
                )));
            }
            if !last {
                return Err(SrcZipError(format!(
                    "source-zip entry `{name}` — `{MANIFEST_NAME}` is a file, not a directory"
                )));
            }
            return Ok(());
        }
        if c == SRC_DIR && !last {
            // the source shell: everything under it rides — rut
            // code's ONE home
            return Ok(());
        }
        if c == DEPS_DIR && !last && at_pkg_root {
            let ident = comps[i + 1];
            if ident.is_empty() || !is_identity(ident) {
                return Err(SrcZipError(format!(
                    "source-zip entry `{name}` — dep directories ride at \
                     `{DEPS_DIR}/<name>@<16 hex>/`"
                )));
            }
            at_pkg_root = true;
            depth += 1;
            i += 2;
            continue;
        }
        // an inert file at a package root — or a violation: a `.rut`
        // beside a manifest, a `.rutbundle` at the ARCHIVE root (a
        // dep's own IR file is the one legal spelling), a directory
        // shaped where a file was due
        if c.ends_with(".rut") {
            return Err(SrcZipError(format!(
                "source-zip entry `{name}` — rut code rides only under a `{SRC_DIR}/` shell"
            )));
        }
        if c.ends_with(".rutbundle") && depth == 0 {
            return Err(SrcZipError(format!(
                "source-zip entry `{name}` — a `.rutbundle` rides only at \
                 `{DEPS_DIR}/<name>@<16 hex>/`"
            )));
        }
        if !last {
            return Err(SrcZipError(format!(
                "source-zip entry `{name}` — inert files ride only at a package root \
                 (unexpected directory `{c}/`)"
            )));
        }
        return Ok(());
    }
    Ok(())
}

/// Rewrite a manifest for publication inside a source zip: every
/// `[deps]` row's descriptor becomes `{ "path": "<rel>" }` — the
/// `url` and external `path` values drop (the dep rides embedded at
/// that relative spot), the row keys stay, and every comment outside
/// the replaced descriptor blocks rides. The `format`/`format_version`
/// rows drop too: a source zip's manifest does not claim the compiled
/// wire. The surgery is textual and comment-aware; the result is
/// RE-PARSED and compared against the original (fields the rewrite
/// does not touch must survive byte-for-byte in meaning) — any drift
/// refuses loudly: normalize the manifest's dep rows and re-pack.
pub fn rewrite_for_zip(
    text: &str,
    dep_paths: &BTreeMap<String, String>,
) -> Result<String, SrcZipError> {
    let original = super::manifest::parse_manifest(text)
        .map_err(|e| SrcZipError(format!("rut.jsonc: {e}")))?;
    let mut out = text.to_string();
    // the wire rows drop, line-wise: `"format": …` and
    // `"format_version": …` at the object's top level
    for key in ["format", "format_version"] {
        while let Some(span) = find_scalar_row(&out, key) {
            let line_span = line_span_of(&out, span);
            out.replace_range(line_span, "");
        }
    }
    // the deps block: each row's descriptor block becomes the one
    // canonical path spelling. A manifest with no `deps` object and no
    // rows to rewrite rides as-is (minus the wire rows) — a leaf dep's
    // manifest has none.
    if !dep_paths.is_empty() {
        let deps_span = find_object_row(&out, "deps").ok_or_else(|| {
            SrcZipError(format!(
                "rut.jsonc has no `deps` object but the closure carries {} dep(s)",
                dep_paths.len()
            ))
        })?;
        let deps_text = &out[deps_span.clone()];
        let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        for (spec, rel) in dep_paths {
            let desc = find_object_row(deps_text, spec).ok_or_else(|| {
                SrcZipError(format!(
                    "rut.jsonc lost its `{spec}` dep row — the manifest and the closure disagree"
                ))
            })?;
            let abs = deps_span.start + desc.start..deps_span.start + desc.end;
            edits.push((abs, format!("{{ \"path\": \"{rel}\" }}")));
        }
        edits.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
        for (span, replacement) in edits {
            out.replace_range(span, &replacement);
        }
    }
    // rows the closure did not collect: a descriptor that would keep
    // pointing outside the archive
    let parsed_deps = super::manifest::parse_manifest(text)
        .map_err(|e| SrcZipError(format!("rut.jsonc: {e}")))?
        .deps;
    for spec in parsed_deps.keys() {
        if !dep_paths.contains_key(spec) {
            return Err(SrcZipError(format!(
                "rut.jsonc's `{spec}` dep row has no embedded tree — the closure and the \
                 manifest disagree"
            )));
        }
    }
    // the re-parse gate: everything the rewrite does not touch must
    // survive (the deps rows are checked separately below — they are
    // the rewrite's own product)
    let result = super::manifest::parse_manifest(&out).map_err(|e| {
        SrcZipError(format!(
            "the rewritten rut.jsonc does not parse: {e} — \
             normalize the manifest's dep rows and re-pack"
        ))
    })?;
    let mut check = original.clone();
    check.format = None;
    check.format_version = None;
    check.deps = Default::default();
    let mut expect = result.clone();
    expect.format = None;
    expect.format_version = None;
    expect.deps = Default::default();
    if check != expect {
        return Err(SrcZipError(
            "the rewritten rut.jsonc drifted from the original — normalize the manifest's \
             dep rows and re-pack"
                .into(),
        ));
    }
    if result.deps.len() != dep_paths.len() {
        return Err(SrcZipError(
            "the rewritten rut.jsonc's dep rows and the closure disagree".into(),
        ));
    }
    for (spec, rel) in dep_paths {
        let row = result.deps.get(spec).ok_or_else(|| {
            SrcZipError(format!("the rewritten rut.jsonc lost its `{spec}` dep row"))
        })?;
        if row.get("path").map(String::as_str) != Some(rel.as_str()) || row.len() != 1 {
            return Err(SrcZipError(format!(
                "the rewritten `{spec}` row does not spell `{{ \"path\": \"{rel}\" }}`"
            )));
        }
    }
    Ok(out)
}

// ---- the JSONC scanner -------------------------------------------------

/// The span of a top-level scalar row (`"key": value`) — the object's
/// first `{` to its matching `}`.
fn find_object_row(text: &str, key: &str) -> Option<std::ops::Range<usize>> {
    let b = text.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        skip_ws_comments(b, &mut i);
        if i >= b.len() {
            break;
        }
        if b[i] == b'{' {
            i += 1;
            // scan the object's member keys at this depth
            loop {
                skip_ws_comments(b, &mut i);
                if i >= b.len() || b[i] == b'}' {
                    return None;
                }
                if b[i] != b'"' {
                    return None; // not a member key — malformed; let the parser refuse
                }
                let key_tok = read_string(b, &mut i)?;
                skip_ws_comments(b, &mut i);
                if i >= b.len() || b[i] != b':' {
                    return None;
                }
                i += 1;
                skip_ws_comments(b, &mut i);
                if key_tok == key {
                    // the value's object: brace-match from here
                    if i >= b.len() || b[i] != b'{' {
                        return None;
                    }
                    let end = match_brace(b, i)?;
                    return Some(i..end + 1);
                }
                skip_value(b, &mut i)?;
                skip_ws_comments(b, &mut i);
                if i < b.len() && b[i] == b',' {
                    i += 1;
                }
            }
        }
        i += 1;
    }
    None
}

/// The span of a top-level scalar row (`"key": <scalar>`) — used for
/// the wire rows the rewrite drops.
fn find_scalar_row(text: &str, key: &str) -> Option<std::ops::Range<usize>> {
    let b = text.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        skip_ws_comments(b, &mut i);
        if i >= b.len() {
            break;
        }
        if b[i] == b'{' {
            i += 1;
            loop {
                skip_ws_comments(b, &mut i);
                if i >= b.len() || b[i] == b'}' {
                    return None;
                }
                if b[i] != b'"' {
                    return None;
                }
                let key_tok = read_string(b, &mut i)?;
                skip_ws_comments(b, &mut i);
                if i >= b.len() || b[i] != b':' {
                    return None;
                }
                i += 1;
                skip_ws_comments(b, &mut i);
                if key_tok == key {
                    // the scalar value's span, to its end
                    let vstart = i;
                    skip_value(b, &mut i)?;
                    return Some(vstart..i);
                }
                skip_value(b, &mut i)?;
                skip_ws_comments(b, &mut i);
                if i < b.len() && b[i] == b',' {
                    i += 1;
                }
            }
        }
        i += 1;
    }
    None
}

/// Expand a value span to cover its whole line (the line the row sits
/// on — trailing comma INCLUDED — plus its newline), so the deletion
/// leaves no residue.
fn line_span_of(text: &str, span: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let b = text.as_bytes();
    let mut start = span.start;
    while start > 0 && b[start - 1] != b'\n' {
        start -= 1;
    }
    let mut end = span.end.min(b.len());
    while end < b.len() && b[end] != b'\n' {
        end += 1;
    }
    // swallow the newline itself so no blank line remains
    if end < b.len() && b[end] == b'\n' {
        end += 1;
    }
    start..end
}

fn skip_ws_comments(b: &[u8], i: &mut usize) {
    loop {
        while *i < b.len() && (b[*i] as char).is_ascii_whitespace() {
            *i += 1;
        }
        if *i + 1 < b.len() && b[*i] == b'/' && b[*i + 1] == b'/' {
            while *i < b.len() && b[*i] != b'\n' {
                *i += 1;
            }
            continue;
        }
        if *i + 1 < b.len() && b[*i] == b'/' && b[*i + 1] == b'*' {
            *i += 2;
            while *i + 1 < b.len() && !(b[*i] == b'*' && b[*i + 1] == b'/') {
                *i += 1;
            }
            *i = (*i + 2).min(b.len());
            continue;
        }
        break;
    }
}

/// Read one JSON string token — the UNQUOTED contents (the scanner
/// compares keys against bare names).
fn read_string(b: &[u8], i: &mut usize) -> Option<String> {
    if b.get(*i) != Some(&b'"') {
        return None;
    }
    let start = *i;
    *i += 1;
    while *i < b.len() {
        match b[*i] {
            b'\\' => *i += 2,
            b'"' => {
                *i += 1;
                return String::from_utf8(b[start + 1..*i - 1].to_vec()).ok();
            }
            _ => *i += 1,
        }
    }
    None
}

/// Skip one JSON value (string, number, literal, object, array) —
/// string- and comment-aware brace/bracket matching for containers.
fn skip_value(b: &[u8], i: &mut usize) -> Option<()> {
    skip_ws_comments(b, i);
    match b.get(*i)? {
        b'"' => {
            read_string(b, i)?;
            Some(())
        }
        b'{' | b'[' => {
            let end = match_brace(b, *i)?;
            *i = end + 1;
            Some(())
        }
        _ => {
            // a scalar: to the next delimiter
            while *i < b.len() && !matches!(b[*i], b',' | b'}' | b']' | b'\n') {
                *i += 1;
            }
            Some(())
        }
    }
}

/// At an opening `{` or `[`: the index of its matching closer,
/// string- and comment-aware.
fn match_brace(b: &[u8], open: usize) -> Option<usize> {
    let (o, c) = match b[open] {
        b'{' => (b'{', b'}'),
        b'[' => (b'[', b']'),
        _ => return None,
    };
    let mut depth = 0usize;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'"' => {
                read_string(b, &mut i)?;
                continue;
            }
            _ if b[i] == b'/' && i + 1 < b.len() && (b[i + 1] == b'/' || b[i + 1] == b'*') => {
                skip_ws_comments(b, &mut i);
                continue;
            }
            _ if b[i] == o => depth += 1,
            _ if b[i] == c => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn get16(d: &[u8], at: usize) -> Option<u16> {
    d.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
}
fn get32(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Does the tail of the data carry an end-of-central-directory
/// signature? (Scans back over a possible zip comment, ≤ 64 KiB.)
fn has_eocd(data: &[u8]) -> bool {
    const EOCD: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    if data.len() < 22 {
        return false;
    }
    let start = data.len().saturating_sub(22 + 0xFFFF);
    (start..=data.len() - 22)
        .rev()
        .any(|i| data[i..i + 4] == EOCD)
}

/// The mechanical layer's failures become the format's diagnostics.
fn policy(e: ZipLayerError) -> SrcZipError {
    match e {
        ZipLayerError::NotAZip => {
            SrcZipError("not a zip archive (no end-of-central-directory)".into())
        }
        ZipLayerError::Checksum { name } => SrcZipError(format!(
            "corrupt source zip: entry `{name}` fails its CRC-32 check"
        )),
        ZipLayerError::UnsupportedMethod { name, method } => SrcZipError(format!(
            "source-zip entry `{name}` uses compression method {method} — this reader \
             carries STORE and DEFLATE only"
        )),
        ZipLayerError::Corrupt(what) => SrcZipError(format!("corrupt source zip: {what}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(entries: &[(&str, &str)]) -> Vec<(String, Vec<u8>)> {
        entries
            .iter()
            .map(|(n, b)| (n.to_string(), b.as_bytes().to_vec()))
            .collect()
    }

    #[test]
    fn round_trip_and_determinism() {
        let entries = stage(&[
            ("rut.jsonc", "{\"name\": \"app\"}"),
            ("src/mod.rut", "entry fn main() {}\n"),
            ("src/util/mod.rut", "pub fn one() -> i64 { return 1; }\n"),
            (
                "deps/pouch@e3b0c44298fc1c14/rut.jsonc",
                "{\"name\": \"pouch\"}",
            ),
            (
                "deps/pouch@e3b0c44298fc1c14/src/mod.rut",
                "pub fn bag() {}\n",
            ),
            ("README.md", "hello\n"),
        ]);
        let a = write_srczip(&entries).unwrap();
        let b = write_srczip(&entries).unwrap();
        assert_eq!(a, b, "same input ⇒ byte-identical zip");
        let parsed = read_srczip(&a).unwrap();
        assert_eq!(parsed, entries);
    }

    #[test]
    fn deflate_reads_back() {
        // a DEFLATE entry: user-made zips compress; the reader carries
        // the codec even though our writer is STORE-only
        use std::io::{Cursor, Write as _};
        // the EXTERNAL zip crate — the internal `zip` module is this
        // file's mechanical layer
        use ::zip as zipc;
        use zipc::write::SimpleFileOptions;
        use zipc::{CompressionMethod, DateTime, ZipWriter};
        let mut buf = Cursor::new(Vec::new());
        {
            let mut w = ZipWriter::new(&mut buf);
            let opts = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .last_modified_time(DateTime::from_date_and_time(1980, 1, 1, 0, 0, 0).unwrap());
            w.start_file("rut.jsonc", opts).unwrap();
            w.write_all(b"{\"name\": \"app\"}").unwrap();
            w.start_file("src/mod.rut", opts).unwrap();
            w.write_all(b"entry fn main() {}\n").unwrap();
            w.finish().unwrap();
        }
        let parsed = read_srczip(&buf.into_inner()).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].0, "src/mod.rut");
    }

    #[test]
    fn grammar_refusals() {
        let ok = stage(&[
            ("rut.jsonc", "{}"),
            ("src/mod.rut", "x"),
            ("deps/u@0123456789abcdef/rut.jsonc", "{}"),
        ]);
        assert!(validate_grammar(&ok).is_ok());
        // rut code beside the manifest — THE refusal
        let stray = stage(&[("rut.jsonc", "{}"), ("mod.rut", "x")]);
        assert!(validate_grammar(&stray).is_err());
        // two manifests: the archive root's PLUS a dep's own — legal
        // (each package root carries one; the count law is about the
        // ROOT manifest existing)
        let two = stage(&[
            ("rut.jsonc", "{}"),
            ("deps/u@0123456789abcdef/rut.jsonc", "{}"),
        ]);
        assert!(validate_grammar(&two).is_ok());
        // a dep dir that is not an identity
        let bare = stage(&[("rut.jsonc", "{}"), ("deps/pouch/rut.jsonc", "{}")]);
        assert!(validate_grammar(&bare).is_err());
        // slip shapes
        let slip = stage(&[("rut.jsonc", "{}"), ("../evil.rut", "x")]);
        assert!(validate_grammar(&slip).is_err());
        // a rutbundle at the root
        let stray_bundle = stage(&[("rut.jsonc", "{}"), ("x.rutbundle", "ir")]);
        assert!(validate_grammar(&stray_bundle).is_err());
    }

    const MANIFEST: &str = r#"{
  // ink — the logger package: rut source wrapping the `ink_host`
  // host surface in the `Logger` class. Publishes COMPILED.
  "format": "rutbundle",
  "format_version": 12,
  "name": "ink",
    "deps": {
    "ink_host": {
      "path": "../ink_host"
    },
    "calc": { "url": "https://cdn.example/calc.rutbundle" }
  }
}"#;

    #[test]
    fn rewrite_drops_wire_rows_and_paths_every_dep() {
        let mut idents = BTreeMap::new();
        idents.insert(
            "ink_host".to_string(),
            "deps/ink_host@a1b2c3d4e5f60718".to_string(),
        );
        idents.insert("calc".to_string(), "deps/calc@b1b2c3d4e5f60718".to_string());
        let out = rewrite_for_zip(MANIFEST, &idents).unwrap();
        assert!(!out.contains("rutbundle\""), "the wire rows drop: {out}");
        assert!(!out.contains("format_version"), "{out}");
        assert!(
            out.contains("// ink — the logger package"),
            "comments ride: {out}"
        );
        assert!(
            out.contains(r#""path": "deps/ink_host@a1b2c3d4e5f60718""#),
            "{out}"
        );
        assert!(
            out.contains(r#""path": "deps/calc@b1b2c3d4e5f60718""#),
            "{out}"
        );
        assert!(!out.contains("../ink_host"), "{out}");
        assert!(!out.contains("cdn.example"), "{out}");
        // the rewritten manifest parses and the fields survive
        let m = super::super::manifest::parse_manifest(&out).unwrap();
        assert_eq!(m.name.as_deref(), Some("ink"));
        assert_eq!(m.deps.len(), 2);
    }

    #[test]
    fn rewrite_refuses_drift() {
        let mut idents = BTreeMap::new();
        idents.insert(
            "ink_host".to_string(),
            "deps/ink_host@a1b2c3d4e5f60718".to_string(),
        );
        // a dep the closure did not collect stays behind → refuse
        assert!(rewrite_for_zip(MANIFEST, &idents).is_err());
    }
}
