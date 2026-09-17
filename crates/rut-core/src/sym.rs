//! The symbol table (RFC 0030 §5): a string interner whose ids are the
//! compiler's universal name representation.
//!
//! [`IdentId`] replaces name `String`s everywhere a name is *compared* or
//! *keyed* — type names, field names, trait-method names, function names,
//! exports. Text survives only at the boundaries: module specifiers, host
//! binding names (`FuncCode.host`), string *values* (`ConstVal::Str`), and
//! the serialized name table (RFC 0033 §1).
//!
//! The interner pre-interns a fixed **well-known table** (RFC 0002 §4), so
//! ids `0..WELL_KNOWN.len()` mean the same name in every interner instance.
//! Special names (`self`, `Self`, `nil`, `main`, the builtin members, the
//! primitives) compare as `IdentId` equality — never by text:
//!
//! ```
//! use rut_core::{Interner, sym};
//! let mut i = Interner::default();
//! assert_eq!(i.intern("Self"), sym::SELF_TY); // already interned
//! assert_eq!(i.name(sym::MAIN), "main");
//! ```

/// An interned name — an index into an [`Interner`]. Ids below
/// [`WELL_KNOWN.len()`] are the pre-interned well-known table and are
/// valid in *every* interner; ids above are instance-local.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IdentId(pub u32);

/// String interner — names are indices, not `String` keys (RFC 0030 §5).
/// Append-only: a [`&str`] borrowed from [`Interner::name`] stays valid
/// for the interner's lifetime.
#[derive(Clone, Debug)]
pub struct Interner {
    names: Vec<Box<str>>,
    map: std::collections::HashMap<Box<str>, IdentId>,
}

impl Default for Interner {
    fn default() -> Self {
        Interner::new()
    }
}

impl Interner {
    /// A fresh interner with the well-known table pre-interned at ids
    /// `0..WELL_KNOWN.len()` — every instance agrees on those ids.
    pub fn new() -> Interner {
        let mut i = Interner { names: Vec::new(), map: std::collections::HashMap::new() };
        for s in WELL_KNOWN {
            i.intern(s);
        }
        i
    }

    /// The first instance-local id (the well-known table's length).
    pub fn well_known_len(&self) -> u32 {
        WELL_KNOWN.len() as u32
    }

    pub fn intern(&mut self, s: &str) -> IdentId {
        if let Some(id) = self.map.get(s) {
            return *id;
        }
        let id = IdentId(self.names.len() as u32);
        let owned: Box<str> = s.into();
        self.names.push(owned.clone());
        self.map.insert(owned, id);
        id
    }

    /// Lookup — find an existing name's id; `None` when not interned.
    pub fn lookup(&self, s: &str) -> Option<IdentId> {
        self.map.get(s).copied()
    }

    pub fn name(&self, id: IdentId) -> &str {
        &self.names[id.0 as usize]
    }

    /// The interned names, in id order. Ids below [`Interner::well_known_len`]
    /// are the well-known table (never serialized — RFC 0033 §1).
    pub fn names(&self) -> &[Box<str>] {
        &self.names
    }

    /// Restore the instance-local tail interned by [`Interner::intern`]
    /// after [`Interner::well_known_len`] — the decode side of the binary
    /// name table (RFC 0033 §1).
    pub fn extend_tail<I: IntoIterator<Item = Box<str>>>(&mut self, tail: I) {
        for s in tail {
            self.intern(&s);
        }
    }
}

/// The well-known table — **append-only, never reorder** (ids are baked
/// into every compiled module and every `sym::` constant below).
pub const WELL_KNOWN: &[&str] = &[
    "self",       // SELF
    "Self",       // SELF_TY
    "nil",        // NIL
    "main",       // MAIN
    "__iterate",  // ITERATE
    "len",        // LEN
    "set",        // SET
    "new",        // NEW
    "code",       // CODE
    "encode",     // ENCODE
    "decode",     // DECODE
    "slice",      // SLICE
    "bool",       // BOOL
    "char",       // CHAR
    "str",        // STR
    "bytes",      // BYTES
    "f32",        // F32
    "f64",        // F64
    "i8",         // I8
    "i16",        // I16
    "i32",        // I32
    "i64",        // I64
    "u8",         // U8
    "u16",        // U16
    "u32",        // U32
    "u64",        // U64
    "Array",      // ARRAY
    "Opaque",     // OPAQUE
    "Iterator",   // ITERATOR
    "downcast",   // DOWNCAST
    "assert",     // ASSERT
    "panic",      // PANIC
    "make_ptr",   // MAKE_PTR
    "on_drop",    // ON_DROP
    "string_join", // STRING_JOIN
    "type_id",    // TYPE_ID
    "from",       // FROM
    "zeroed",     // ZEROED
    "from_code",  // FROM_CODE
    "buf",        // BUF
];

/// The well-known symbols — fixed ids into [`WELL_KNOWN`], meaningful in
/// every interner instance. Removed names (`Option`, `Result`,
/// `Weak`) are deliberately absent: their diagnostics stay text-based.
///
/// Used as `rut_core::SELF` etc.
pub const SELF: IdentId = IdentId(0); // `self`
pub const SELF_TY: IdentId = IdentId(1); // `Self`
pub const NIL: IdentId = IdentId(2);
pub const MAIN: IdentId = IdentId(3);
pub const ITERATE: IdentId = IdentId(4); // `__iterate`
pub const LEN: IdentId = IdentId(5);
pub const SET: IdentId = IdentId(6);
pub const NEW: IdentId = IdentId(7);
pub const CODE: IdentId = IdentId(8);
pub const ENCODE: IdentId = IdentId(9);
pub const DECODE: IdentId = IdentId(10);
pub const SLICE: IdentId = IdentId(11);
pub const BOOL: IdentId = IdentId(12);
pub const CHAR: IdentId = IdentId(13);
pub const STR: IdentId = IdentId(14);
pub const BYTES: IdentId = IdentId(15);
pub const F32: IdentId = IdentId(16);
pub const F64: IdentId = IdentId(17);
pub const I8: IdentId = IdentId(18);
pub const I16: IdentId = IdentId(19);
pub const I32: IdentId = IdentId(20);
pub const I64: IdentId = IdentId(21);
pub const U8: IdentId = IdentId(22);
pub const U16: IdentId = IdentId(23);
pub const U32: IdentId = IdentId(24);
pub const U64: IdentId = IdentId(25);
pub const ARRAY: IdentId = IdentId(26);
pub const OPAQUE: IdentId = IdentId(27);
pub const ITERATOR: IdentId = IdentId(28);
pub const DOWNCAST: IdentId = IdentId(29);
pub const ASSERT: IdentId = IdentId(30);
pub const PANIC: IdentId = IdentId(31);
pub const MAKE_PTR: IdentId = IdentId(32);
pub const ON_DROP: IdentId = IdentId(33);
pub const STRING_JOIN: IdentId = IdentId(34);
pub const TYPE_ID: IdentId = IdentId(35);
pub const FROM: IdentId = IdentId(36);
pub const ZEROED: IdentId = IdentId(37);
pub const FROM_CODE: IdentId = IdentId(38);
pub const BUF: IdentId = IdentId(39);

/// The text of a well-known id, if it is one — the bridge back to text at
/// host-facing boundaries (e.g. mounting `std:core` into a `Session`).
pub fn text(id: IdentId) -> Option<&'static str> {
    WELL_KNOWN.get(id.0 as usize).copied()
}

/// The type a primitive name denotes (`sym::I32` -> `TY_I32`), or
/// `None` for non-type names. Mirrors the resolver's primitive table.
pub fn primitive_ty(id: IdentId) -> Option<crate::types::TypeId> {
    use crate::types::*;
    Some(match id {
        NIL => TY_NIL,
        BOOL => TY_BOOL,
        STR => TY_STR,
        BYTES => TY_BYTES,
        F32 => TY_F32,
        F64 => TY_F64,
        I8 => TY_I8,
        I16 => TY_I16,
        I32 => TY_I32,
        I64 => TY_I64,
        U8 => TY_U8,
        U16 => TY_U16,
        U32 => TY_U32,
        U64 => TY_U64,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `sym::` constant must address its documented text — the
    /// table and the constants may never drift apart.
    #[test]
    fn well_known_consts_match_the_table() {
        let cases: &[(&str, IdentId)] = &[
            ("self", SELF),
            ("Self", SELF_TY),
            ("nil", NIL),
            ("main", MAIN),
            ("__iterate", ITERATE),
            ("len", LEN),
            ("set", SET),
            ("new", NEW),
            ("code", CODE),
            ("encode", ENCODE),
            ("decode", DECODE),
            ("slice", SLICE),
            ("bool", BOOL),
            ("char", CHAR),
            ("str", STR),
            ("bytes", BYTES),
            ("f32", F32),
            ("f64", F64),
            ("i8", I8),
            ("i16", I16),
            ("i32", I32),
            ("i64", I64),
            ("u8", U8),
            ("u16", U16),
            ("u32", U32),
            ("u64", U64),
            ("Array", ARRAY),
            ("Opaque", OPAQUE),
            ("Iterator", ITERATOR),
            ("downcast", DOWNCAST),
            ("assert", ASSERT),
            ("panic", PANIC),
            ("make_ptr", MAKE_PTR),
            ("on_drop", ON_DROP),
            ("string_join", STRING_JOIN),
            ("type_id", TYPE_ID),
            ("from", FROM),
            ("zeroed", ZEROED),
            ("from_code", FROM_CODE),
            ("buf", BUF),
        ];
        for (text, id) in cases {
            assert_eq!(WELL_KNOWN.get(id.0 as usize), Some(text), "id {id:?}");
            let mut i = Interner::new();
            assert_eq!(i.name(*id), *text);
            assert_eq!(i.lookup(text), Some(*id));
            assert_eq!(i.intern(text), *id, "intern must not re-assign");
        }
        assert_eq!(WELL_KNOWN.len(), cases.len(), "table and cases disagree");
    }

    #[test]
    fn interning_is_content_addressed_and_append_only() {
        let mut i = Interner::new();
        let a = i.intern("Point");
        let b = i.intern("Point");
        let c = i.intern("other");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.0 >= i.well_known_len(), "user names start after the table");
        assert_eq!(i.name(a), "Point");
    }

    #[test]
    fn every_interner_agrees_on_well_known_ids() {
        let mut i = Interner::new();
        // user names interned on top do not move the well-known range
        let point = i.intern("Point");
        assert_eq!(i.lookup("self"), Some(SELF));
        assert_eq!(i.lookup("Point"), Some(point));
        assert_eq!(i.intern("Self"), SELF_TY);
    }

    #[test]
    fn tail_round_trips() {
        let mut i = Interner::new();
        i.intern("Point");
        i.intern("Vec");
        let tail: Vec<Box<str>> =
            i.names()[i.well_known_len() as usize..].to_vec();
        let mut j = Interner::new();
        j.extend_tail(tail);
        assert_eq!(j.lookup("Point"), i.lookup("Point"));
        assert_eq!(j.lookup("Vec"), i.lookup("Vec"));
        assert_eq!(j.well_known_len(), j.names().len() as u32 - 2);
    }
}

