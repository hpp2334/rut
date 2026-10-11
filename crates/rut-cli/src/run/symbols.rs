//! the `.rutsym` restore: read, decode, warn on sections the bundle
//! does not carry — the map mounts on the chain BEFORE the graph
//! compiles, so linking and the VM both see the real names and
//! positions.

/// Restore the symbol table for a stripped artifact (None = no table
/// given — the plain run). Read failures are usage errors; decode
/// failures are load errors; sections naming modules the bundle does
/// not carry warn.
pub(crate) fn restore(
    loaded: &rut_driver::Loaded,
    path: Option<&str>,
) -> Option<rut_core::strip::SymbolMap> {
    let sym = path?;
    let bytes = match std::fs::read(sym) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("run: cannot read {sym}: {e}");
            std::process::exit(2);
        }
    };
    let map = match rut_core::strip::SymbolMap::from_bytes(&bytes) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("run: {sym}: {e}");
            std::process::exit(1);
        }
    };
    for section in &map.sections {
        if loaded.pkg(&section.spec).is_none() {
            eprintln!("run: warning — the symbol table names `{}`, which this bundle does not carry", section.spec);
        }
    }
    Some(map)
}
