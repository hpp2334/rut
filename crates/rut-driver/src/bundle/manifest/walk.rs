//! The section walkers: top-level keys, `entry`, the dep tables,
//! `style` — plus the syntax boundary (serde_json's diagnostics become
//! the module's single-line `line N:` shape; value laws are
//! path-targeted instead) and the `_`-key retirement.

use serde_json::{Map, Value};

use super::descriptor::{check_dep_key, parse_deps_descriptor, parse_peer_descriptor};
use super::expect::{expect_field_string, expect_field_string_array, expect_string, expect_u64};
use super::{valid_spec, Manifest, ManifestError, PkgType};

// ---- the syntax boundary: serde_json's diagnostics become the
// module's single-line `line N:` shape. serde_json's Display appends
// `at line L column C` to its message; the location moves into the
// prefix, the parser's own wording carries the rest. ----

/// A JSON syntax error as one `line N: <message>` diagnostic.
pub(super) fn syntax_error(err: &serde_json::Error) -> ManifestError {
    let line = err.line().max(1);
    let raw = err.to_string();
    let message = strip_location(&raw).unwrap_or(raw.as_str());
    ManifestError(format!("line {line}: {message}"))
}

/// Drop serde_json's `at line L column C` tail (it is already the
/// prefix); a message that merely ends with a similar phrase (a url in
/// an error?) keeps its text whole.
fn strip_location(message: &str) -> Option<&str> {
    let at = message.rfind(" at line ")?;
    let rest = &message[at + " at line ".len()..];
    let (line, column) = rest.split_once(" column ")?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if digits(line) && digits(column) {
        Some(&message[..at])
    } else {
        None
    }
}

/// The value as spelled — the "found X" text of the type refusals,
/// JSON-spelled by serde_json's own Display.
pub(super) fn raw(value: &Value) -> String {
    value.to_string()
}

/// A table value must be an object — `{table}: expected an object`.
pub(super) fn expect_object<'a>(table: &str, value: &'a Value) -> Result<&'a Map<String, Value>, ManifestError> {
    value
        .as_object()
        .ok_or_else(|| ManifestError(format!("{table}: expected an object, found {}", raw(value))))
}

/// The `_`-prefixed prose lane RETIRED with the JSONC cutover: the
/// manifest speaks comments now (`// …`, `/* … */`), so an `_`-key
/// (`"_comment"`, any `_`-prefixed name) refuses loudly naming the fix
/// — an old manifest keeps failing LOUDLY instead of silently
/// dropping its prose. `path` is the key's walk path (`entry`,
/// `deps.pouch`, …) — the value laws' targeting.
pub(crate) fn underscore_refused(path: &str, key: &str) -> ManifestError {
    ManifestError(format!(
        "{path}: `{key}` — the `_`-prefixed prose lane retired; JSONC comments are the prose now (write a `//` comment above the key)"
    ))
}

// ---- the sections ----

/// Top-level keys. Unknown keys ride (forward compatibility, any
/// shape), `host_scope` refuses loudly (the retirement law).
pub(super) fn walk_top_lane(
    key: &str,
    value: &Value,
    declared_type: &mut bool,
    m: &mut Manifest,
    compat: bool,
) -> Result<(), ManifestError> {
    match key {
        "name" => {
            let name = expect_string(key, value)?;
            if !valid_spec(&name) {
                return Err(ManifestError(format!(
                    "name: '{name}' is not a bare package name — expected [a-zA-Z0-9_]+"
                )));
            }
            m.name = Some(name);
        }
        // the declared kind: `lib` (the ordinary source package)
        // or `host` (a pure declaration surface). Absent ⇒ lib —
        // a host pkg SPELLS itself
        "type" => {
            *declared_type = true;
            match expect_string(key, value)?.as_str() {
                "lib" => m.pkg_type = PkgType::Lib,
                "host" => m.pkg_type = PkgType::Host,
                other => {
                    return Err(ManifestError(format!(
                        "type: expected 'lib' or 'host', found '{other}'"
                    )));
                }
            }
        }
        // bundle-shaped manifests
        "format" => m.format = Some(expect_string(key, value)?),
        "format_version" => m.format_version = Some(expect_u64(key, value)?),
        // `host_scope` (the retired registration-prefix override)
        // refuses LOUDLY: the key was load-bearing for
        // registration names, so a silent ignore would surface as
        // a confusing boot panic later — the error names the fix
        // (the registration scope is the package name)
        "host_scope" => {
            return Err(ManifestError(
                "host_scope: retired — the registration scope is the package name; delete the key (rename the pkg if its scope must change)"
                    .into(),
            ));
        }
        // the qualified-access head (`calc`'s `Math`): the manifest's
        // spelling of what the surface grammar cannot say — the
        // namespace `use <pkg>::{ Head }` binds and qualified member
        // access routes through
        "namespace" => m.namespace = Some(expect_string(key, value)?),
        // the host body's constants (`calc`'s `Math.PI`): name → f64,
        // compiler-materialized — the surface grammar has no `static`
        // field form, so the manifest is their only spelling
        "consts" => walk_consts(value, m)?,
        "entry" => walk_entry(value, m, compat)?,
        "deps" => walk_deps(value, m)?,
        "peer-deps" => walk_peers(value, "peer-deps", m)?,
        "dev-deps" => walk_peers(value, "dev-deps", m)?,
        "style" => walk_style(value, m)?,
        _ => {} // forward-compatible: ignore unknown top-level keys
    }
    Ok(())
}

/// The `entry` object. The post-repeal grammar keeps only `type` (the
/// `.d.rut` surface): the root module is `mod.rut` beside the manifest
/// and the body keys refuse LOUDLY, naming the fix. `compat` is the
/// bundle reader's lane — a PUBLISHED bundle's manifest is forever old
/// (the pinned CDN tags), so there the keys parse onto
/// [`Manifest::legacy_entry`] under the old value laws instead. The
/// retired `entry.ir` rides either way; unknown keys are refused (the
/// entry strictness).
pub(super) fn walk_entry(value: &Value, m: &mut Manifest, compat: bool) -> Result<(), ManifestError> {
    let table = expect_object("entry", value)?;
    for (key, v) in table {
        if key.starts_with('_') {
            return Err(underscore_refused("entry", key));
        }
        match key.as_str() {
            "type" => m.entry.type_path = Some(expect_field_string("entry", "type", v)?),
            "lib" if compat => {
                m.legacy_entry.lib = Some(expect_field_string("entry", "lib", v)?);
            }
            "libs" if compat => {
                m.legacy_entry.libs = expect_field_string_array("entry", "libs", v)?;
            }
            "lib" => {
                return Err(ManifestError(
                    "entry.lib: repealed — the root module is `mod.rut` beside the manifest: \
                     rename the file, drop the key"
                        .into(),
                ));
            }
            "libs" => {
                return Err(ManifestError(
                    "entry.libs: repealed — structure lives in `mod` directories: each file \
                     becomes `NAME/mod.rut` and the root declares `mod NAME;`"
                        .into(),
                ));
            }
            "ir" => {}
            other => {
                return Err(ManifestError(format!("entry: unknown key '{other}'")));
            }
        }
    }
    Ok(())
}

/// The `deps` object: exact specifier → descriptor; the descriptor
/// laws are [`parse_deps_descriptor`]'s.
pub(super) fn walk_deps(value: &Value, m: &mut Manifest) -> Result<(), ManifestError> {
    let table = expect_object("deps", value)?;
    for (spec, v) in table {
        if spec.starts_with('_') {
            return Err(underscore_refused("deps", spec));
        }
        check_dep_key("deps", spec)?;
        m.deps.insert(spec.clone(), parse_deps_descriptor("deps", spec, v)?);
    }
    Ok(())
}

/// The `peer-deps` / `dev-deps` objects: same descriptor walk with the
/// `optional` flag and the `lib` group key.
pub(super) fn walk_peers(
    value: &Value,
    kind: &str,
    m: &mut Manifest,
) -> Result<(), ManifestError> {
    let table = expect_object(kind, value)?;
    for (spec, v) in table {
        if spec.starts_with('_') {
            return Err(underscore_refused(kind, spec));
        }
        check_dep_key(kind, spec)?;
        let desc = parse_peer_descriptor(kind, spec, v)?;
        match kind {
            "peer-deps" => {
                m.peer_deps.insert(spec.clone(), desc);
            }
            _ => {
                m.dev_deps.insert(spec.clone(), desc);
            }
        }
    }
    Ok(())
}

/// The `style` object: `key: "value"` rows, string-typed, schema-free
/// — the fmt crate validates keys and parses values (the manifest
/// stays schema-free, the deps-table law); unknown keys ride, refused
/// only by the tool that knows its schema.
pub(super) fn walk_style(value: &Value, m: &mut Manifest) -> Result<(), ManifestError> {
    let table = expect_object("style", value)?;
    for (key, v) in table {
        if key.starts_with('_') {
            return Err(underscore_refused("style", key));
        }
        m.style.insert(key.clone(), expect_field_string("style", key, v)?);
    }
    Ok(())
}

/// The `consts` object: name → f64 — the host body's compiler-
/// materialized constants (`Math.PI`). Each value is a JSON number;
/// the non-finite pair has no JSON spelling, so the strings `"inf"` /
/// `"-inf"` (and `"nan"`) name it. f64 is the only width the manifest
/// carries.
fn walk_consts(value: &Value, m: &mut Manifest) -> Result<(), ManifestError> {
    let table = expect_object("consts", value)?;
    for (key, v) in table {
        if key.starts_with('_') {
            return Err(underscore_refused("consts", key));
        }
        let n = match v {
            Value::Number(_) => v.as_f64().ok_or_else(|| {
                ManifestError(format!("consts.{key}: expected a number, found {}", raw(v)))
            })?,
            Value::String(s) => match s.as_str() {
                "inf" | "+inf" => f64::INFINITY,
                "-inf" => f64::NEG_INFINITY,
                "nan" => f64::NAN,
                other => {
                    return Err(ManifestError(format!(
                        "consts.{key}: `{other}` is not a number — spell a JSON number (the \
                         non-finite spellings are \"inf\", \"-inf\", \"nan\")"
                    )));
                }
            },
            other => {
                return Err(ManifestError(format!(
                    "consts.{key}: expected a number, found {}",
                    raw(other)
                )));
            }
        };
        m.consts.insert(key.clone(), n);
    }
    Ok(())
}
