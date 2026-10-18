//! the `.rutsym` restore: read, decode, warn on sections the archive
//! does not carry — the map restores on the DECODED HANDLE
//! (`bundle.restore_symbols(..)`) BEFORE any offer, so linking and the
//! VM both see the real names and positions.

/// Decode the symbol table (None = no table given — the plain run).
/// Read failures are usage errors; decode failures are load errors.
pub(crate) fn decode(path: &str) -> rut_core::strip::SymbolMap {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("run: cannot read {path}: {e}");
            std::process::exit(2);
        }
    };
    match rut_core::strip::SymbolMap::from_bytes(&bytes) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("run: {path}: {e}");
            std::process::exit(1);
        }
    }
}

/// Warn on sections the archive does not carry — tolerated (the
/// restore skips them), never silent-by-accident: the operator asked
/// for names the artifact cannot answer.
pub(crate) fn warn_absent(archive: &rut_driver::Bundle, map: &rut_core::strip::SymbolMap) {
    let carried = archive.carried_specs();
    for section in &map.sections {
        if !carried.iter().any(|s| s == &section.spec) {
            eprintln!(
                "run: warning — the symbol table names `{}`, which this bundle does not carry",
                section.spec
            );
        }
    }
}
