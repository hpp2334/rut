//! Diagnostics — `Diag { span, msg, labels, notes, file }`,
//! one renderer for lexer/parser/resolve/typecheck, byte offsets.

use crate::span::Span;

#[derive(Clone, Debug)]
pub struct Diag {
    pub span: Span,
    pub msg: String,
    pub labels: Vec<(Span, String)>,
    pub notes: Vec<String>,
    /// The file the span indexes, as the producer's module name — a pkg
    /// spec, `spec/mod-path`, or a formatter's real path; never a
    /// filesystem path (loaded pkgs deliberately carry none). `None` =
    /// no position to show (a graph-level structural refusal) or the
    /// producer did not label: the renderer falls back to `line/column`.
    pub file: Option<String>,
}

impl Diag {
    pub fn new(span: Span, msg: impl Into<String>) -> Diag {
        Diag {
            span,
            msg: msg.into(),
            labels: Vec::new(),
            notes: Vec::new(),
            file: None,
        }
    }
    /// Attribute the diag to a file — the label the renderer spells
    /// after the arrow (`--> hello:2:13`).
    pub fn in_file(mut self, f: impl Into<String>) -> Diag {
        self.file = Some(f.into());
        self
    }
    pub fn note(mut self, n: impl Into<String>) -> Diag {
        self.notes.push(n.into());
        self
    }
}

/// Render a diag list against source text: primary line with a caret span,
/// secondary labels, notes. Used by the CLI, tests, and the demo (the wasm
/// surface returns structured diags; this is the pretty form).
pub fn render_diags(src: &str, diags: &[Diag]) -> String {
    let mut sources = std::collections::BTreeMap::new();
    sources.insert(String::new(), src);
    render_diags_grouped(diags, &sources, None)
}

/// Render a diag list across FILES: diags sort and group by (file,
/// span start) — deterministic multi-file output — and each group
/// renders against its own source text. A diag whose file has no text
/// here renders position-only (the arrow, no snippet/caret). Diags
/// with no file of their own take `default_label` when given (`rut
/// fmt`'s real path); without it they spell `line/column` against the
/// `""` source a single-file caller passed ([`render_diags`]).
pub fn render_diags_grouped(
    diags: &[Diag],
    sources: &std::collections::BTreeMap<String, &str>,
    default_label: Option<&str>,
) -> String {
    // the effective label: the diag's own file, else the default
    // label, else "" — the single-file lane's key
    fn label_of<'a>(d: &'a Diag, default_label: Option<&'a str>) -> &'a str {
        match (&d.file, default_label) {
            (Some(f), _) => f.as_str(),
            (None, Some(l)) => l,
            (None, None) => "",
        }
    }
    let mut out = String::new();
    // stable sort by (file, span start): emission order is
    // compile-phase order (parse → collect → bodies), so an unsorted
    // render interleaves files' errors
    let mut sorted: Vec<&Diag> = diags.iter().collect();
    sorted.sort_by(|a, b| {
        label_of(a, default_label)
            .cmp(label_of(b, default_label))
            .then(a.span.lo.cmp(&b.span.lo))
    });
    for d in sorted {
        let file = label_of(d, default_label);
        out.push_str(&format!("error: {}\n", d.msg));
        match sources.get(file).copied() {
            None => {
                // no text for this file: position-only — the arrow
                // names the file, no snippet/caret (labels would
                // locate nowhere). A label-less diag has no position
                // to show at all: the bare error line is the whole
                // rendering (the graph-level refusals' shape).
                if !file.is_empty() {
                    out.push_str(&format!("  --> {file}\n"));
                }
            }
            Some(src) => {
                let (line_no, col, line, caret_len) = locate(src, d.span);
                if file.is_empty() {
                    out.push_str(&format!("  --> line {}, column {}\n", line_no, col + 1));
                } else {
                    out.push_str(&format!("  --> {file}:{line_no}:{}\n", col + 1));
                }
                out.push_str(&format!("{:4} | {}\n", line_no, line));
                out.push_str(&format!(
                    "{:4} | {}{}\n",
                    "",
                    " ".repeat(col),
                    "^".repeat(caret_len.max(1))
                ));
                for (span, label) in &d.labels {
                    let (l2, c2, _, _) = locate(src, *span);
                    out.push_str(&format!("  note@line {} col {}: {}\n", l2, c2 + 1, label));
                }
            }
        }
        for n in &d.notes {
            out.push_str(&format!("  note: {}\n", n));
        }
    }
    out
}

fn locate(src: &str, span: Span) -> (usize, usize, String, usize) {
    let bytes = src.as_bytes();
    let lo = (span.lo as usize).min(bytes.len());
    let hi = (span.hi as usize).min(bytes.len()).max(lo);
    let mut line_start = 0usize;
    let mut line_no = 1usize;
    for (i, &b) in bytes.iter().enumerate().take(lo) {
        if b == b'\n' {
            line_no += 1;
            line_start = i + 1;
        }
    }
    let line_end = bytes[line_start..]
        .iter()
        .position(|&b| b == b'\n')
        .map(|p| line_start + p)
        .unwrap_or(bytes.len());
    let line = String::from_utf8_lossy(&bytes[line_start..line_end]).to_string();
    let col = src[line_start..lo].chars().count();
    let caret = src[lo..hi].chars().count();
    (line_no, col, line, caret)
}
