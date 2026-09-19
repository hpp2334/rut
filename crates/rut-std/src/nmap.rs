//! `nmap` — the native key-table experiment (the mapset-host plan, H2):
//! an open-addressing hash table whose state lives Rust-side behind an
//! `Opaque` payload box (RFC 0023), so rut meets it only through the
//! `pub host fn` surface bound by [`install_std_nmap`]. This is the HOST
//! half of the experiment; the pure-rut `mapset` package stays untouched
//! as the reference and the general-key implementation.
//!
//! Design (plan §0/§1):
//! - the native side owns KEYS only — owned Rust data (integer bits /
//!   `String` / `Vec<u8>` copies). Values stay rut-side in the wrapper's
//!   parallel array, so `get -> *V` aliasing semantics match mapset
//!   exactly and the payload box stays free of rut cells (no release
//!   hazards — the rc-0 `Drop` is plain Rust, RFC 0016 §3);
//! - the key set is CLOSED (i8..i64, u8..u64, bool, str, bytes). Keys
//!   hash wrapper-side and cross in `h` — the table only RECORDS hashes;
//!   the payload box crosses once per call and is read through the H1
//!   accessor (`Vm::opaque_key_payload`). Anything else — floats, chars,
//!   user records, host boxes — traps loudly, pointing at `mapset`. No
//!   re-entrant `hash_eq` callbacks (RFC 0023 §2: callbacks cross by
//!   name and cannot reach impl methods);
//! - probe semantics are mapset.rut transplanted verbatim: open
//!   addressing with linear probing, power-of-two capacity (≥ 4),
//!   tombstones (`states` 0 EMPTY · 1 FULL · 2 DEAD, insertion reuses
//!   the first DEAD slot), the load-factor law
//!   `(count + tomb + 1) × 10 >= cap × 7` widened to u64, and RECORDED
//!   hashes — `grow` re-slots from them and never re-hashes a key;
//! - growth: the native table rehashes its own keys and builds the
//!   old→new relocation list; the wrapper relocates its value array by
//!   draining [`NativeTable::take_reloc`] until the `-1` sentinel
//!   (plan §0.6). `grow` clears any stale, undrained list first — the
//!   protocol is drain-after-every-grow, and a wrapper that skipped a
//!   drain gets the latest mapping only (mapset's per-rehash map has the
//!   same honesty).
//!
//! Crossing shapes (one scalar each — the `.d.rut` decl crossing set has
//! no tuples): `map_remove` answers the freed slot or `-1` (absent);
//! `map_take_reloc` answers `(old << 32) | new` or `-1` (drained). Same
//! facts the plan's `(bool, i32)` / `(i32, i32)` pairs carried, packed.
//!
//! Storage note: the plan sketch's `keys_bits` / `keys_str` parallel
//! vecs fold into one `Vec<KeyVal>` — the key kind fixes on the table's
//! first insert and never mixes, so per-slot variants are the same data
//! with one allocation per grow instead of three.

use std::collections::VecDeque;

use rut_vm::interp::{HostRegistry, KeyPayload, Vm};
use rut_vm::{OpaqueBox, OpaqueRef, Trap, TrapKind};

/// Which payload flavor the table stores — fixed by the first inserted
/// key and admitted thereafter (the wrapper is generic over `K`, so keys
/// are homogeneous per map by construction; the check is defense, and a
/// mismatch is a trap rather than a silent absent).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    /// nothing stored yet
    Unset,
    /// integer primitives and `bool` — equality is the payload bits
    Bits,
    /// `str` — owned copy, equality is octet equality (RFC 0012 §4)
    Str,
    /// `bytes` — owned copy, equality is octet equality
    Bytes,
}

/// A stored key: the closed native set as owned Rust data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyVal {
    /// the slot word's raw bits (a negative integer crosses
    /// sign-extended, exactly as it hashed wrapper-side)
    Bits(u64),
    /// an owned `str` copy — short, once per call
    Str(String),
    /// an owned `bytes` copy — short, once per call
    Bytes(Vec<u8>),
}

/// The native table: mapset.rut's `RawHashTable` core with the keys
/// owned natively. Pure Rust data — no rut cells inside — so the
/// payload's `Drop` (which frees every key) runs deterministically when
/// the box's rc hits 0.
pub struct NativeTable {
    kind: KeyKind,
    /// each stored key's hash, recorded at insert: the probe's u64
    /// pre-filter and the grow's re-slot source — a key is hashed
    /// exactly once per lifetime (wrapper-side)
    hashes: Vec<u64>,
    keys: Vec<KeyVal>,
    /// one byte per slot: 0 EMPTY · 1 FULL · 2 DEAD (tombstone)
    states: Vec<u8>,
    count: u32,
    tomb: u32,
    /// power of two, >= 4 — the index mask is `cap - 1`
    cap: u32,
    /// the last grow's old→new slot pairs, drained by `take_reloc`
    reloc: VecDeque<(i32, i32)>,
}

impl NativeTable {
    /// mapset's `with_capacity` law: round up to a power of two, never
    /// below 4. `cap` crosses as the `i64` the wrapper passes.
    pub fn new(cap: i64) -> NativeTable {
        let mut c: u32 = 4;
        if cap > 0 {
            let want = cap.min(i32::MAX as i64) as u32;
            while c < want {
                c *= 2;
            }
        }
        NativeTable {
            kind: KeyKind::Unset,
            hashes: vec![0; c as usize],
            keys: vec![KeyVal::Bits(0); c as usize],
            states: vec![0; c as usize],
            count: 0,
            tomb: 0,
            cap: c,
            reloc: VecDeque::new(),
        }
    }

    /// The put-path crossing: probe, and on a miss INSERT the owned key.
    /// Answers the found slot (≥ 0 — the wrapper replaces its value
    /// there) or `-(slot + 1)` for the freshly occupied slot — the same
    /// encoding mapset's `probe` hands its owners. The caller grows past
    /// the load factor first (`needs_grow` / `grow`), so an EMPTY slot
    /// always exists and the probe pass ends.
    pub fn entry(&mut self, kind: KeyKind, key: KeyVal, h: u64) -> Result<i32, Trap> {
        self.admit(kind)?;
        let at = self.probe(&key, h);
        if at >= 0 {
            return Ok(at); // found — replace in place, nothing stored
        }
        if self.kind == KeyKind::Unset {
            self.kind = kind;
        }
        let slot = -(at + 1);
        if self.states[slot as usize] == 2 {
            self.tomb -= 1; // the DEAD slot leaves the tomb count
        }
        self.states[slot as usize] = 1;
        self.keys[slot as usize] = key;
        self.hashes[slot as usize] = h;
        self.count += 1;
        Ok(-(slot + 1))
    }

    /// The lookup crossing: probe only. The found slot, or `-1` — the
    /// wrapper answers `nil` / `false` from the sign.
    pub fn find(&self, kind: KeyKind, key: &KeyVal, h: u64) -> Result<i32, Trap> {
        self.admit(kind)?;
        Ok(match self.probe(key, h) {
            at if at >= 0 => at,
            _ => -1,
        })
    }

    /// The remove crossing: tombstone the found slot (the owned key
    /// drops with the store) and answer it, or `-1` when absent.
    pub fn remove(&mut self, kind: KeyKind, key: &KeyVal, h: u64) -> Result<i32, Trap> {
        self.admit(kind)?;
        let at = self.probe(key, h);
        if at < 0 {
            return Ok(-1);
        }
        self.keys[at as usize] = KeyVal::Bits(0);
        self.states[at as usize] = 2;
        self.count -= 1;
        self.tomb += 1;
        Ok(at)
    }

    /// The load-factor law, mapset's constants: `(count + tomb + 1) / cap
    /// >= 0.7`, widened to u64 — u32 `×10` would overflow near 429M.
    pub fn needs_grow(&self) -> bool {
        (self.count as u64 + self.tomb as u64 + 1) * 10 >= self.cap as u64 * 7
    }

    /// Double the table and re-slot every FULL entry from its RECORDED
    /// hash — no key is hashed again. The tombstones are gone (`tomb`
    /// resets, `count` is unchanged) and the old→new slot pairs queue in
    /// `reloc` for the wrapper's value-array relocation. Answers the new
    /// capacity so the wrapper sizes its parallel array in one crossing.
    pub fn grow(&mut self) -> i32 {
        let mut old_keys = std::mem::take(&mut self.keys);
        let old_hashes = std::mem::take(&mut self.hashes);
        let old_states = std::mem::take(&mut self.states);
        let new_cap = self.cap * 2;
        self.keys = vec![KeyVal::Bits(0); new_cap as usize];
        self.hashes = vec![0; new_cap as usize];
        self.states = vec![0; new_cap as usize];
        self.cap = new_cap;
        self.tomb = 0;
        self.reloc.clear();
        for (i, st) in old_states.iter().enumerate() {
            if *st != 1 {
                continue;
            }
            let h = old_hashes[i];
            let at = self.first_free(h);
            self.keys[at as usize] = std::mem::replace(&mut old_keys[i], KeyVal::Bits(0));
            self.hashes[at as usize] = h;
            self.states[at as usize] = 1;
            self.reloc.push_back((i as i32, at));
        }
        new_cap as i32
    }

    /// The relocation iterator's next pair, packed for the single-scalar
    /// crossing: `-1` = drained; else `(old << 32) | new` — both are
    /// in-bounds slot indices, so the low word never reaches the sign.
    /// A fresh table (no grow yet) drains immediately.
    pub fn take_reloc(&mut self) -> i64 {
        match self.reloc.pop_front() {
            None => -1,
            Some((old, new)) => ((old as i64) << 32) | (new as i64),
        }
    }

    /// The capacity, at the i32 boundary (`len()` and indexing are i32).
    pub fn cap(&self) -> i32 {
        self.cap as i32
    }

    /// The FULL slot count.
    pub fn len(&self) -> i32 {
        self.count as i32
    }

    /// The ONE three-stage probe pass (mapset.rut verbatim): state byte,
    /// recorded hash, then the native key compare (only on a FULL slot
    /// whose recorded hash matches — most steps never compare). `>= 0` —
    /// FULL match at that slot; `< 0` — the insertion slot encoded
    /// `-(at + 1)`, with the first DEAD slot on the sequence winning.
    fn probe(&self, key: &KeyVal, h: u64) -> i32 {
        let mask = (self.cap - 1) as i32;
        let mut at = ((h as u32) & (self.cap - 1)) as i32;
        let mut dead = -1;
        loop {
            let st = self.states[at as usize];
            if st == 0 {
                if dead >= 0 {
                    return -(dead + 1);
                }
                return -(at + 1);
            }
            if st == 1 && self.hashes[at as usize] == h && self.keys[at as usize] == *key {
                return at;
            }
            if st == 2 && dead < 0 {
                dead = at;
            }
            at = (at + 1) & mask;
        }
    }

    /// The first EMPTY slot on `h`'s probe sequence — fresh-table
    /// insertion (a new or just-rehashed table has no DEAD slots).
    fn first_free(&self, h: u64) -> i32 {
        let mask = (self.cap - 1) as i32;
        let mut at = ((h as u32) & (self.cap - 1)) as i32;
        while self.states[at as usize] != 0 {
            at = (at + 1) & mask;
        }
        at
    }

    /// Kind admission: homogeneous per table by construction; a mismatch
    /// is a host/wrapper bug and traps instead of silently missing.
    fn admit(&self, kind: KeyKind) -> Result<(), Trap> {
        if self.kind != KeyKind::Unset && self.kind != kind {
            return Err(Trap::new(
                TrapKind::Invalid,
                format!("nmap: mixed key kinds in one table (table holds {kind:?} keys)"),
            ));
        }
        Ok(())
    }
}

/// The crossing-side key read: the `Opaque` box's payload classified
/// against the closed native key set (the H1 accessor), as the table's
/// owned [`KeyVal`]. An `Unsupported` payload is the loud trap — the
/// message names the type and points at `mapset`, which admits any
/// `Hashable` key.
fn key_val(vm: &Vm, k: &OpaqueRef) -> Result<(KeyKind, KeyVal), Trap> {
    match vm.opaque_key_payload(k)? {
        KeyPayload::Bits { val, .. } => Ok((KeyKind::Bits, KeyVal::Bits(val))),
        KeyPayload::Str(s) => Ok((KeyKind::Str, KeyVal::Str(s))),
        KeyPayload::Bytes(b) => Ok((KeyKind::Bytes, KeyVal::Bytes(b))),
        KeyPayload::Unsupported(ty) => Err(Trap::new(
            TrapKind::Invalid,
            format!(
                "nmap: key type `{}` is not natively supported — use mapset (the native key set is the integer primitives, `bool`, `str`, and `bytes`)",
                vm.prog.type_name(ty)
            ),
        )),
    }
}

/// Install the nine `nmap` bodies under the `nmap` scope (the `calc`
/// pattern, RFC 0023/0025): the callable's Rust shape IS the `.d.rut`
/// row, so the surface declares exactly these signatures. `map_new`'s
/// box carries the table; every other fn's first param borrows it typed
/// (`OpaqueBox<NativeTable>` — a wrong payload is a checked trap).
pub fn install_std_nmap(hosts: &mut HostRegistry) {
    rut_vm::register!(hosts, "nmap::map_new", (i64,) -> OpaqueRef, |vm: &mut Vm, cap: i64| {
        let b = OpaqueBox::alloc(vm, NativeTable::new(cap))?;
        Ok(b.handle().clone())
    });
    rut_vm::register!(
        hosts,
        "nmap::map_entry",
        (OpaqueBox<NativeTable>, OpaqueRef, i64) -> i32,
        |vm: &mut Vm, b: OpaqueBox<NativeTable>, k: OpaqueRef, h: i64| -> Result<i32, Trap> {
            let (kind, key) = key_val(vm, &k)?;
            Ok(b.with_mut(|t| t.entry(kind, key, h as u64))??)
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_find",
        (OpaqueBox<NativeTable>, OpaqueRef, i64) -> i32,
        |vm: &mut Vm, b: OpaqueBox<NativeTable>, k: OpaqueRef, h: i64| -> Result<i32, Trap> {
            let (kind, key) = key_val(vm, &k)?;
            Ok(b.with(|t| t.find(kind, &key, h as u64))??)
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_remove",
        (OpaqueBox<NativeTable>, OpaqueRef, i64) -> i32,
        |vm: &mut Vm, b: OpaqueBox<NativeTable>, k: OpaqueRef, h: i64| -> Result<i32, Trap> {
            let (kind, key) = key_val(vm, &k)?;
            Ok(b.with_mut(|t| t.remove(kind, &key, h as u64))??)
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_needs_grow",
        (OpaqueBox<NativeTable>,) -> bool,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>| b.with(|t| t.needs_grow()),
    );
    rut_vm::register!(
        hosts,
        "nmap::map_grow",
        (OpaqueBox<NativeTable>,) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>| b.with_mut(|t| t.grow()),
    );
    rut_vm::register!(
        hosts,
        "nmap::map_take_reloc",
        (OpaqueBox<NativeTable>,) -> i64,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>| b.with_mut(|t| t.take_reloc()),
    );
    rut_vm::register!(
        hosts,
        "nmap::map_cap",
        (OpaqueBox<NativeTable>,) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>| b.with(|t| t.cap()),
    );
    rut_vm::register!(
        hosts,
        "nmap::map_len",
        (OpaqueBox<NativeTable>,) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>| b.with(|t| t.len()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// mapset.rut's mix64 — the wrapper-side integer hash, mirrored here
    /// so recorded hashes land the way the real crossing produces them.
    fn mix64(bits: u64) -> u64 {
        (14695981039346656037u64 ^ bits).wrapping_mul(1099511628211u64)
    }

    fn bits(v: u64) -> KeyVal {
        KeyVal::Bits(v)
    }

    #[test]
    fn capacity_rounds_up_to_a_power_of_two_not_below_four() {
        assert_eq!(NativeTable::new(0).cap(), 4);
        assert_eq!(NativeTable::new(-5).cap(), 4);
        assert_eq!(NativeTable::new(4).cap(), 4);
        assert_eq!(NativeTable::new(5).cap(), 8);
        assert_eq!(NativeTable::new(1000).cap(), 1024);
    }

    #[test]
    fn entry_inserts_then_replaces_at_the_same_slot() {
        let mut t = NativeTable::new(8);
        let a = t.entry(KeyKind::Bits, bits(11), mix64(11)).unwrap();
        assert!(a < 0, "a fresh key inserts");
        let slot = -(a + 1);
        assert_eq!(t.len(), 1);
        // same key, same recorded hash: the found slot back, no second
        // entry — the replace answer
        let again = t.entry(KeyKind::Bits, bits(11), mix64(11)).unwrap();
        assert_eq!(again, slot);
        assert_eq!(t.len(), 1);
        assert_eq!(t.find(KeyKind::Bits, &bits(11), mix64(11)).unwrap(), slot);
        // a miss is the -1 answer
        assert_eq!(t.find(KeyKind::Bits, &bits(12), mix64(12)).unwrap(), -1);
    }

    #[test]
    fn remove_tombstones_and_the_dead_slot_is_reused() {
        let mut t = NativeTable::new(8);
        t.entry(KeyKind::Bits, bits(11), mix64(11)).unwrap();
        t.entry(KeyKind::Bits, bits(22), mix64(22)).unwrap();
        let slot = t.find(KeyKind::Bits, &bits(11), mix64(11)).unwrap();
        // remove answers the freed slot; a second remove misses
        assert_eq!(t.remove(KeyKind::Bits, &bits(11), mix64(11)).unwrap(), slot);
        assert_eq!(t.remove(KeyKind::Bits, &bits(11), mix64(11)).unwrap(), -1);
        assert_eq!(t.len(), 1);
        assert_eq!(t.tomb, 1);
        assert_eq!(t.find(KeyKind::Bits, &bits(11), mix64(11)).unwrap(), -1);
        // re-insert: the DEAD slot on the key's own probe path wins, so
        // the key lands exactly where it was, and the tomb count drops
        let a = t.entry(KeyKind::Bits, bits(11), mix64(11)).unwrap();
        assert_eq!(-(a + 1), slot);
        assert_eq!(t.len(), 2);
        assert_eq!(t.tomb, 0);
    }

    #[test]
    fn the_load_factor_law_is_mapsets_constants() {
        let mut t = NativeTable::new(16);
        assert!(!t.needs_grow(), "empty: (0+0+1)*10 < 112");
        for i in 0u64..11 {
            t.entry(KeyKind::Bits, bits(i), mix64(i)).unwrap();
        }
        // (11+0+1)*10 = 120 >= 16*7 = 112 — the first FULL count that
        // asks for a grow; (10+1)*10 = 110 did not
        assert!(t.needs_grow());
        assert_eq!(t.cap(), 16, "needs_grow observes, it never grows");
        assert_eq!(t.len(), 11);
    }

    #[test]
    fn grow_reslots_from_recorded_hashes_and_queues_relocation() {
        let mut t = NativeTable::new(4);
        let mut old_slots = Vec::new();
        for i in 0u64..3 {
            let a = t.entry(KeyKind::Bits, bits(i), mix64(i)).unwrap();
            old_slots.push(-(a + 1));
        }
        assert_eq!(t.len(), 3);

        let new_cap = t.grow();
        assert_eq!(new_cap, 8);
        assert_eq!(t.cap(), 8);
        assert_eq!(t.len(), 3, "count survives the grow");
        assert_eq!(t.tomb, 0, "tombstones are gone");

        // the iterator yields exactly one pair per FULL slot, then the
        // sentinel; a further take is -1 again
        let mut moved = std::collections::HashMap::new();
        let mut drained = 0;
        while drained < 3 {
            let packed = t.take_reloc();
            assert!(
                packed >= 0,
                "the grow must queue one pair per FULL slot (got -1 after {drained})"
            );
            let old = (packed >> 32) as i32;
            let new = (packed & 0xFFFF_FFFF) as i32;
            assert!(moved.insert(old, new).is_none(), "one pair per old slot");
            drained += 1;
        }
        assert_eq!(t.take_reloc(), -1, "the iterator drains exactly count pairs");
        // every key finds its new slot, and the mapping says so
        for (i, old) in old_slots.iter().enumerate() {
            let at = t.find(KeyKind::Bits, &bits(i as u64), mix64(i as u64)).unwrap();
            assert!(at >= 0, "key {i} lost by the grow");
            assert_eq!(moved.get(old), Some(&at), "key {i}: {old} -> {at}");
        }
    }

    #[test]
    fn key_kinds_admit_but_never_mix() {
        let mut t = NativeTable::new(8);
        t.entry(KeyKind::Bits, bits(1), mix64(1)).unwrap();
        // the same kind admits; a different one is a trap, not a miss
        t.entry(KeyKind::Bits, bits(2), mix64(2)).unwrap();
        let err = t
            .entry(KeyKind::Str, KeyVal::Str("x".into()), mix64(0))
            .unwrap_err();
        assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
        assert_eq!(t.len(), 2, "the rejected key stored nothing");
    }

    #[test]
    fn str_key_equality_is_content_equality() {
        // one table, one key kind — homogeneity is the contract
        let mut t = NativeTable::new(8);
        let a = t
            .entry(KeyKind::Str, KeyVal::Str("alpha".into()), mix64(1))
            .unwrap();
        assert!(a < 0);
        // a fresh copy of the same octets finds the stored slot
        let at = t.find(KeyKind::Str, &KeyVal::Str("alpha".into()), mix64(1)).unwrap();
        assert_eq!(at, -(a + 1));
        assert_eq!(t.find(KeyKind::Str, &KeyVal::Str("alph".into()), mix64(1)).unwrap(), -1);

        let mut t = NativeTable::new(8);
        let b = t.entry(KeyKind::Bytes, KeyVal::Bytes(vec![9, 8, 7]), mix64(2)).unwrap();
        assert!(b < 0);
        let at = t.find(KeyKind::Bytes, &KeyVal::Bytes(vec![9, 8, 7]), mix64(2)).unwrap();
        assert_eq!(at, -(b + 1));
        assert_eq!(t.find(KeyKind::Bytes, &KeyVal::Bytes(vec![9, 8]), mix64(2)).unwrap(), -1);
    }
}
