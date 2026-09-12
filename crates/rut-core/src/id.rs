//! Scope-qualified ids (RFC 0035 §1).
//!
//! Every module numbers its own types/functions/traits from 0. To link
//! independently-compiled modules without rewriting each id by hand, an id
//! is `(scope, local)`: the high bits name the module's **scope**, the low
//! bits the index inside it. Scope 0 is reserved for the shared **boot**
//! table (`TypeTable::boot`), so a boot id packs to itself
//! (`pack(0, d) == d`) and every module's `i32` is the same id.
//!
//! A scope is a stable module identifier, not a link-time detail: the
//! compiler emits `(scope, local)` references, and link turns each into a
//! dense global index via a per-scope base table (one base per kind).

/// Bits reserved for the local index; the rest are the scope.
pub const LOCAL_BITS: u32 = 20;
/// Largest local index inside one scope.
pub const LOCAL_MASK: u32 = (1 << LOCAL_BITS) - 1;
/// The reserved scope of the boot/prelude table.
pub const BOOT_SCOPE: ScopeId = 0;
/// Largest representable scope.
pub const MAX_SCOPE: u32 = u32::MAX >> LOCAL_BITS;

/// A module (or the boot table) identifier.
pub type ScopeId = u16;

/// Compose `(scope, local)` into one id.
#[inline]
pub const fn pack(scope: ScopeId, local: u32) -> u32 {
    ((scope as u32) << LOCAL_BITS) | (local & LOCAL_MASK)
}

/// The scope half of an id.
#[inline]
pub const fn scope_of(id: u32) -> ScopeId {
    (id >> LOCAL_BITS) as ScopeId
}

/// The local half of an id.
#[inline]
pub const fn local_of(id: u32) -> u32 {
    id & LOCAL_MASK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_ids_pack_to_themselves() {
        for d in 0..16u32 {
            assert_eq!(pack(BOOT_SCOPE, d), d);
            assert_eq!(scope_of(d), 0);
            assert_eq!(local_of(d), d);
        }
    }

    #[test]
    fn round_trips_a_module_id() {
        let id = pack(3, 4097);
        assert_eq!(scope_of(id), 3);
        assert_eq!(local_of(id), 4097);
    }
}
