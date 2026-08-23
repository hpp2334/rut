//! Diagnostics — RFC 0030 §6: `Diag { span, msg, labels, notes }`,
//! one renderer for lexer/parser/resolve/typecheck, byte offsets.

use crate::span::Span;

#[derive(Clone, Debug)]
pub struct Diag {
    pub span: Span,
    pub msg: String,
    pub labels: Vec<(Span, String)>,
    pub notes: Vec<String>,
}

impl Diag {
    pub fn new(span: Span, msg: impl Into<String>) -> Diag {
        Diag {
            span,
            msg: msg.into(),
            labels: Vec::new(),
            notes: Vec::new(),
        }
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
    let mut out = String::new();
    for d in diags {
        let (line_no, col, line, caret_len) = locate(src, d.span);
        out.push_str(&format!("error: {}\n", d.msg));
        out.push_str(&format!("  --> line {}, byte {}\n", line_no, d.span.lo));
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
