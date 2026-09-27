//! Style — the `[style]` manifest block, resolved. The manifest layer
//! stays schema-free (rows are strings); THIS is where keys are known
//! and values are parsed, so an unknown-but-manifest-legal key never
//! traps the loader and a malformed value only trips the formatter
//! tool, never a compile.
//!
//! Defaults are the house corpus's own conventions (the fmt batch's
//! survey §5): 4-space indent — zero tab-indented corpus lines — and a
//! 100-column soft width.

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Style {
    /// spaces per indent level (`indent_width`)
    pub indent_width: usize,
    /// the soft line width (`max_width`): break points try flat, spill
    /// to one-element-per-line when the line would exceed this
    pub max_width: usize,
}

impl Default for Style {
    fn default() -> Self {
        Style { indent_width: 4, max_width: 100 }
    }
}

/// Parse a `[style]` manifest block. Unknown keys are IGNORED (the
/// forward-compatibility rule the top-level manifest keys already
/// hold); known keys with malformed values are a formatter error
/// naming the key — never a loader error.
pub fn from_manifest(map: &BTreeMap<String, String>) -> Result<Style, String> {
    let mut s = Style::default();
    for (k, v) in map {
        match k.as_str() {
            "indent_width" => {
                let n: u64 = v
                    .parse()
                    .map_err(|_| format!("[style] `indent_width` must be an integer, found `{v}`"))?;
                if !(1..=8).contains(&n) {
                    return Err(format!(
                        "[style] `indent_width` must be 1..=8 (spaces per level), found {n}"
                    ));
                }
                s.indent_width = n as usize;
            }
            "max_width" => {
                let n: u64 = v
                    .parse()
                    .map_err(|_| format!("[style] `max_width` must be an integer, found `{v}`"))?;
                if n < 20 {
                    return Err(format!(
                        "[style] `max_width` must be at least 20 columns, found {n}"
                    ));
                }
                s.max_width = n as usize;
            }
            // forward-compat: unknown keys ride (RFC 0041 §5's rule)
            _ => {}
        }
    }
    Ok(s)
}
