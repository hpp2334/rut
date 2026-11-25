//! Shared helpers for the fmt test targets.

/// blank the `[lo, hi)` spans out of a rendered dump — the formatted
/// text is legitimately a different LENGTH (whitespace collapses,
/// comments move), so the round-trip compares SHAPE, not spans
pub fn span_free(dump: &str) -> String {
    let mut out = String::with_capacity(dump.len());
    let mut chars = dump.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '[' {
            // a span opens: `[digits, digits)` — blank it
            let rest = &dump[i + 1..];
            let is_span = {
                let mut it = rest.split_inclusive(')');
                it.next().is_some_and(|head| {
                    let inner = head.trim_end_matches(')');
                    let mut parts = inner.splitn(2, ',');
                    match (parts.next(), parts.next()) {
                        (Some(a), Some(b)) => {
                            !a.is_empty()
                                && !b.is_empty()
                                && a.trim().chars().all(|c| c.is_ascii_digit())
                                && b.trim().chars().all(|c| c.is_ascii_digit())
                        }
                        _ => false,
                    }
                })
            };
            if is_span {
                out.push_str("[..)");
                for (_, c) in chars.by_ref() {
                    if c == ')' {
                        break;
                    }
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}
