//! `nmap` — the native key-table experiment (the mapset-host plan, H2):
//! an open-addressing hash table whose state lives Rust-side behind an
//! `opaque` payload box (RFC 0023), so rut meets it only through the
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
//!
//! The val column (nmapset-round3 phase 1): alongside the keys the
//! table carries `vals: Vec<u64>` — raw bits, one column for every
//! primitive val kind (the monomorphized wrapper reinterprets
//! i64/u64/f64/bool client-side; no lane explosion). Presence is
//! BY-KEY (plan §0.3): a val is valid iff its key slot is FULL, so the
//! column has no nil tags and `remove` writes nothing — a put always
//! stores the val at the slot its crossing answered, and a get only
//! reads a found slot. `grow` relocates vals WITH the keys (the same
//! slot walk, one mapping — the wrapper-side relocation drain that
//! phase 2 deletes never touches this column). Slots cross as caller-
//! supplied indices and are validated like any host lane: out of
//! range is a trap, not a read.

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
    /// the val column: one raw u64 per slot, valid iff the key slot is
    /// FULL (presence-by-key — no nil tags, no writes on remove). Eager,
    /// cap-aligned: allocated zeroed at creation and reallocated by
    /// `grow`, so `vals.len() == cap` holds at every observation point
    /// and the set/get crossings are a bounds check + an index — no
    /// per-op "is it allocated" branch on the phase-2 consumer's hot
    /// path. The zero fill is the initial state only; a val under a
    /// DEAD slot stays stale by design (unreachable through the map
    /// protocol, overwritten by the next insert at that slot).
    ///
    /// Accounting note (measured, phase-1 sanity pass): the Vec header
    /// grows `size_of::<NativeTable>()` 120 → 144, and `OpaqueBox::
    /// alloc` charges that shallow size to the RFC 0040 heap budget at
    /// every `map_new` — so the VM-heap high-water reads +24 B per
    /// table box alive at peak. No rut-visible allocation changes: the
    /// column's backing memory is plain Rust, invisible to cell
    /// accounting, and every row's checksum and fuel are untouched.
    vals: Vec<u64>,
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
            vals: vec![0; c as usize],
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
    /// drops with the store) and answer it, or `-1` when absent. The
    /// val column is NOT written: presence-by-key (plan §0.3) makes the
    /// stale bits unreachable, and the slot's next insert stores a fresh
    /// val before anything can observe the old one.
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
    ///
    /// The val column moves WITH the keys, inside the same slot walk:
    /// each FULL slot's u64 lands on its key's new slot (round3 phase
    /// 1), so a column-backed wrapper needs no drain pass at all — the
    /// `reloc` queue stays for the `[?V]` sidecar path, unchanged.
    /// Slot assignment is untouched by the column, so iteration order
    /// and every existing checksum hold by construction.
    pub fn grow(&mut self) -> i32 {
        let mut old_keys = std::mem::take(&mut self.keys);
        let old_hashes = std::mem::take(&mut self.hashes);
        let old_states = std::mem::take(&mut self.states);
        let old_vals = std::mem::take(&mut self.vals);
        let new_cap = self.cap * 2;
        self.keys = vec![KeyVal::Bits(0); new_cap as usize];
        self.hashes = vec![0; new_cap as usize];
        self.states = vec![0; new_cap as usize];
        self.vals = vec![0; new_cap as usize];
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
            self.vals[at as usize] = old_vals[i];
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

    /// The val column's write crossing (round3 phase 1): store `raw` at
    /// `slot`. The slot is caller-supplied (the wrapper holds it from
    /// its entry/find/remove crossing) and validated like any host
    /// lane — negative or `>= cap` is a trap, never a read/write. Raw
    /// bits only: every primitive val kind rides the one u64 lane, the
    /// wrapper reinterprets client-side.
    pub fn val_set(&mut self, slot: i32, raw: u64) -> Result<(), Trap> {
        let at = self.val_slot(slot)?;
        self.vals[at] = raw;
        Ok(())
    }

    /// The val column's read crossing: the raw u64 at `slot` (same
    /// bounds law). Validity is the map invariant — a val is whatever
    /// the last put at this key's slot stored; reading a slot whose key
    /// is absent is a caller bug the bounds check cannot name (no nil
    /// tags by design), so this answers the stored bits.
    pub fn val_get(&self, slot: i32) -> Result<u64, Trap> {
        let at = self.val_slot(slot)?;
        Ok(self.vals[at])
    }

    /// The caller-supplied slot's bounds check: `[0, cap)`, trapping
    /// with the lane house shape (`Invalid` + "out of range") on
    /// anything else.
    fn val_slot(&self, slot: i32) -> Result<usize, Trap> {
        if slot < 0 || slot >= self.cap as i32 {
            return Err(Trap::new(
                TrapKind::Invalid,
                format!("nmap: val slot {slot} out of range (cap {})", self.cap),
            ));
        }
        Ok(slot as usize)
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

/// The crossing-side key read: the `opaque` box's payload classified
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

/// The two hash constants mapset's `Hashable` impls run (the ONE key
/// contract, nmapset.rut `mix64`/`fnv1a64`): FNV-1a 64's offset basis
/// and prime. These are the checksum law — ported BIT-FOR-BIT, so the
/// host's recorded hashes equal the wrapper's `k.hash()` bit for bit
/// and the phase-4 bench checksums (which must equal mapset's rows)
/// hold. A one-ulp difference here breaks the gate; the unit test pins
/// the exact outputs as literals.
const FNV_OFFSET: u64 = 14695981039346656037;
const FNV_PRIME: u64 = 1099511628211;

/// The typed lanes' host-side hash — the ONE shared payload hasher the
/// crossings run instead of minting an Opaque box and reading the
/// wrapper's `h`: mix64 for the integer/bool bits, FNV-1a 64 over the
/// octets for `str`/`bytes`. Same inputs, mapset's constants, mapset's
/// exact bits.
pub fn hash_payload(k: &KeyVal) -> u64 {
    let mut h = FNV_OFFSET;
    match k {
        KeyVal::Bits(v) => return (FNV_OFFSET ^ v).wrapping_mul(FNV_PRIME),
        KeyVal::Str(s) => {
            for b in s.as_bytes() {
                h = (h ^ *b as u64).wrapping_mul(FNV_PRIME);
            }
        }
        KeyVal::Bytes(b) => {
            for byte in b {
                h = (h ^ *byte as u64).wrapping_mul(FNV_PRIME);
            }
        }
    }
    h
}

/// The payload's kind — the table's fixed flavor, as the Opaque lane's
/// `key_val` classifies it. All integer primitives and `bool` share
/// `Bits` (equality is the payload bits: `255u8`, `255i64`, and
/// `255i32` collapse to one key, exactly as the wrapper's homogeneous
/// `K` guarantees), `str`/`bytes` are their own kinds.
fn kind_of(key: &KeyVal) -> KeyKind {
    match key {
        KeyVal::Bits(_) => KeyKind::Bits,
        KeyVal::Str(_) => KeyKind::Str,
        KeyVal::Bytes(_) => KeyKind::Bytes,
    }
}

/// The load-factor sentinel the fused `map_entry_*` answers when
/// inserting would breach the load factor: the wrapper grows (drains
/// the relocation queue), then retries the entry call. Collision-free
/// with the entry answers — a found slot is `>= 0`, a fresh insert is
/// `-(slot + 1) >= -2^30` (slot < cap <= 2^30, or the table is far
/// past any real allocation), so `i32::MIN` can never be mistaken for
/// either.
pub const GROW_FIRST: i32 = i32::MIN;

/// The typed entry lane: same probe core as [`NativeTable::entry`], but
/// the key crossed DIRECTLY (no Opaque mint) and the hash came from
/// [`hash_payload`]. Grow-first is FUSED into the crossing: the
/// wrapper's `map_needs_grow + map_grow + drain + retry` round trip
/// collapses to one sentinel answer (plan phase 2). `map_needs_grow` /
/// `map_grow` stay exported for the grow path itself and the tests.
fn typed_entry(t: &mut NativeTable, key: KeyVal) -> Result<i32, Trap> {
    let h = hash_payload(&key);
    let kind = kind_of(&key);
    if t.needs_grow() {
        return Ok(GROW_FIRST); // grow-first: nothing stored on this call
    }
    t.entry(kind, key, h)
}

/// The typed lookup lane: probe only — the found slot, or `-1`.
fn typed_find(t: &NativeTable, key: &KeyVal) -> Result<i32, Trap> {
    let h = hash_payload(key);
    t.find(kind_of(key), key, h)
}

/// The typed remove lane: tombstone the found slot, or `-1`.
fn typed_remove(t: &mut NativeTable, key: &KeyVal) -> Result<i32, Trap> {
    let h = hash_payload(key);
    t.remove(kind_of(key), key, h)
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

    // ---- the typed lanes (plan phase 2) ----------------------------
    // 15 fns `map_{entry,find,remove}_{i,u,b,s,y}`: the key crosses
    // DIRECTLY — no `opaque(..)` mint, no wrapper-hash `h`, no key box —
    // and the hash is computed host-side by [`hash_payload`]. The `i`
    // lane takes `i64` (wrapper-cast from i8..i64; sign-extension is
    // the `KeyPayload::Bits` law, so the recorded bits match), the `u`
    // lane takes `u64` (raw bits — verified across the boundary in the
    // phase-2 driver tests), `b` takes `bool`, `s`/`y` borrow
    // `str`/`bytes` ZERO-COPY (the found text's octets read straight
    // out of the block store; only a fresh find copies into the probe
    // key). `map_entry_*` answers the fused grow sentinel
    // ([`GROW_FIRST`] = `i32::MIN`) before any insert.
    rut_vm::register!(
        hosts,
        "nmap::map_entry_i",
        (OpaqueBox<NativeTable>, i64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: i64| -> Result<i32, Trap> {
            b.with_mut(|t| typed_entry(t, KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_entry_u",
        (OpaqueBox<NativeTable>, u64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: u64| -> Result<i32, Trap> {
            b.with_mut(|t| typed_entry(t, KeyVal::Bits(k)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_entry_b",
        (OpaqueBox<NativeTable>, bool) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: bool| -> Result<i32, Trap> {
            b.with_mut(|t| typed_entry(t, KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_entry_s",
        (OpaqueBox<NativeTable>, &str) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &str| -> Result<i32, Trap> {
            b.with_mut(|t| typed_entry(t, KeyVal::Str(k.to_owned())))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_entry_y",
        (OpaqueBox<NativeTable>, &[u8]) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &[u8]| -> Result<i32, Trap> {
            b.with_mut(|t| typed_entry(t, KeyVal::Bytes(k.to_vec())))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_find_i",
        (OpaqueBox<NativeTable>, i64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: i64| -> Result<i32, Trap> {
            b.with(|t| typed_find(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_find_u",
        (OpaqueBox<NativeTable>, u64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: u64| -> Result<i32, Trap> {
            b.with(|t| typed_find(t, &KeyVal::Bits(k)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_find_b",
        (OpaqueBox<NativeTable>, bool) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: bool| -> Result<i32, Trap> {
            b.with(|t| typed_find(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_find_s",
        (OpaqueBox<NativeTable>, &str) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &str| -> Result<i32, Trap> {
            b.with(|t| typed_find(t, &KeyVal::Str(k.to_owned())))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_find_y",
        (OpaqueBox<NativeTable>, &[u8]) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &[u8]| -> Result<i32, Trap> {
            b.with(|t| typed_find(t, &KeyVal::Bytes(k.to_vec())))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_remove_i",
        (OpaqueBox<NativeTable>, i64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: i64| -> Result<i32, Trap> {
            b.with_mut(|t| typed_remove(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_remove_u",
        (OpaqueBox<NativeTable>, u64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: u64| -> Result<i32, Trap> {
            b.with_mut(|t| typed_remove(t, &KeyVal::Bits(k)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_remove_b",
        (OpaqueBox<NativeTable>, bool) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: bool| -> Result<i32, Trap> {
            b.with_mut(|t| typed_remove(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_remove_s",
        (OpaqueBox<NativeTable>, &str) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &str| -> Result<i32, Trap> {
            b.with_mut(|t| typed_remove(t, &KeyVal::Str(k.to_owned())))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_remove_y",
        (OpaqueBox<NativeTable>, &[u8]) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &[u8]| -> Result<i32, Trap> {
            b.with_mut(|t| typed_remove(t, &KeyVal::Bytes(k.to_vec())))?
        },
    );

    // ---- the val column (nmapset-round3, phase 1) ----------------------
    // Two crossings, raw u64 bits in/out, ONE pair serving every
    // primitive val kind (i64/u64/f64/bool — the monomorphized wrapper
    // reinterprets client-side; no lane explosion). Slots cross as
    // caller-supplied `i32` indices — the wrapper holds the slot its
    // `map_entry_*`/`map_find_*`/`map_remove_*` call answered — and are
    // bounds-checked host-side (out of range traps). The u64 lane is
    // raw-bit at the boundary in BOTH directions (the `u64` param/ret
    // read/write the slot's bit pattern, verified in the phase-2
    // driver tests), so no sign rules apply to payload bits.
    // `map_val_set_u` answers nothing (a put always stores); growth
    // relocates the column host-side, so no wrapper drain exists for
    // this path.
    rut_vm::register!(
        hosts,
        "nmap::map_val_set_u",
        (OpaqueBox<NativeTable>, i32, u64) -> (),
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, slot: i32, raw: u64| -> Result<(), Trap> {
            b.with_mut(|t| t.val_set(slot, raw))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_val_get_u",
        (OpaqueBox<NativeTable>, i32) -> u64,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, slot: i32| -> Result<u64, Trap> {
            b.with(|t| t.val_get(slot))?
        },
    );

    // ---- the f64 val lane (nmapset-round3, phase 2) ---------------------
    // The same column, read/written through an `f64`-typed crossing:
    // `f64::to_bits`/`from_bits` at the boundary, nothing else. This is
    // the plan's "minimal honest surface" for the prim-val map's float
    // lane — rut has no `f64 <-> u64` bitcast (the numeric `as` is a
    // VALUE conversion, RFC 0007 §1, and `bool` is excluded from `as`
    // entirely), so the wrapper cannot spell the reinterpret client-side
    // the way the integer lanes do with `as u64`. Raw bits in the
    // column, byte-for-byte, both directions; the slot law is the u
    // lane's (caller-supplied index, out of range traps, growth
    // relocates host-side).
    rut_vm::register!(
        hosts,
        "nmap::map_val_set_f",
        (OpaqueBox<NativeTable>, i32, f64) -> (),
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, slot: i32, v: f64| -> Result<(), Trap> {
            b.with_mut(|t| t.val_set(slot, v.to_bits()))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap::map_val_get_f",
        (OpaqueBox<NativeTable>, i32) -> f64,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, slot: i32| -> Result<f64, Trap> {
            b.with(|t| t.val_get(slot).map(f64::from_bits))?
        },
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
        assert_eq!(
            t.take_reloc(),
            -1,
            "the iterator drains exactly count pairs"
        );
        // every key finds its new slot, and the mapping says so
        for (i, old) in old_slots.iter().enumerate() {
            let at = t
                .find(KeyKind::Bits, &bits(i as u64), mix64(i as u64))
                .unwrap();
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
        let at = t
            .find(KeyKind::Str, &KeyVal::Str("alpha".into()), mix64(1))
            .unwrap();
        assert_eq!(at, -(a + 1));
        assert_eq!(
            t.find(KeyKind::Str, &KeyVal::Str("alph".into()), mix64(1))
                .unwrap(),
            -1
        );

        let mut t = NativeTable::new(8);
        let b = t
            .entry(KeyKind::Bytes, KeyVal::Bytes(vec![9, 8, 7]), mix64(2))
            .unwrap();
        assert!(b < 0);
        let at = t
            .find(KeyKind::Bytes, &KeyVal::Bytes(vec![9, 8, 7]), mix64(2))
            .unwrap();
        assert_eq!(at, -(b + 1));
        assert_eq!(
            t.find(KeyKind::Bytes, &KeyVal::Bytes(vec![9, 8]), mix64(2))
                .unwrap(),
            -1
        );
    }

    // ---- phase 2: the typed lanes + the fused grow sentinel ----------

    /// The checksum law, pinned as LITERALS: hash_payload must answer
    /// mapset's exact bits (nmapset.rut's `mix64` / `fnv1a64` constants,
    /// ported bit-for-bit) — a one-ulp difference breaks the phase-4
    /// bench gate, which compares value-derived checksums against
    /// mapset's rows.
    #[test]
    fn hash_payload_is_mapsets_bits_bit_for_bit() {
        // mix64 — the integer/bool lane: (offset ^ bits) * prime
        assert_eq!(hash_payload(&KeyVal::Bits(0)), 12638153115695167455);
        assert_eq!(hash_payload(&KeyVal::Bits(1)), 12638152016183539244);
        assert_eq!(hash_payload(&KeyVal::Bits(11)), 12638163011299821354);
        assert_eq!(hash_payload(&KeyVal::Bits(255)), 12638352127299873646);
        // u64's sign-bit set lives through raw: bits 0xFFFF..FF (an
        // i64's -1, sign-extended) hash 0x509c41b379fe466e
        assert_eq!(hash_payload(&KeyVal::Bits(u64::MAX)), 5808589858502755950);
        // FNV-1a 64 over the octets — the str/bytes lanes
        assert_eq!(
            hash_payload(&KeyVal::Str("alpha".into())),
            9999721509958787115
        );
        assert_eq!(
            hash_payload(&KeyVal::Str("beta".into())),
            8513880941419438247
        );
        assert_eq!(
            hash_payload(&KeyVal::Bytes(vec![1, 2, 3])),
            15035938162879559083
        );
    }

    /// The typed ops and the Opaque ops land the SAME slots on the SAME
    /// table operations: the key's recorded hash comes from
    /// `hash_payload` with mapset's constants, so probing behaves
    /// identically to the wrapper-hash Opaque lane.
    #[test]
    fn typed_and_opaque_lanes_answer_the_same_slots() {
        // one shared op sequence, two kinds of put: opaque-lane records
        // (wrapper bits in `h`), typed lanes (host bits from
        // hash_payload) — the answers must agree pairwise
        let keys: Vec<u64> = vec![11, 22, 33, 44, 55, 66];
        let mut t = NativeTable::new(16);
        let mut opaque_slots = Vec::new();
        for (i, k) in keys.iter().enumerate() {
            let h = mix64(*k);
            let at = if i % 2 == 0 {
                // the Opaque lane: wrapper-computed hash crosses in
                t.entry(KeyKind::Bits, bits(*k), h).unwrap()
            } else {
                // the typed lane: the host hashes the direct key
                typed_entry(&mut t, KeyVal::Bits(*k)).unwrap()
            };
            opaque_slots.push(at);
        }
        assert_eq!(t.len(), 6);
        // now the mirrored lookups: every key finds its recorded slot by
        // BOTH lanes — normalize the insert encoding (-(slot + 1)) to
        // the slot
        for (i, k) in keys.iter().enumerate() {
            let typed = typed_find(&t, &KeyVal::Bits(*k)).unwrap();
            assert!(typed >= 0, "key {k} lost: {typed}");
            let ans = opaque_slots[i];
            let expect = if ans >= 0 { ans } else { -(ans + 1) };
            assert_eq!(
                typed, expect,
                "typed find matches the recorded slot for key {k}"
            );
            assert_eq!(t.find(KeyKind::Bits, &bits(*k), mix64(*k)).unwrap(), typed);
        }
        // removes cross lanes too: remove by the typed lane what the
        // opaque lane stored, and the Opaque find answers the absence
        let freed = typed_remove(&mut t, &KeyVal::Bits(11)).unwrap();
        let ans = opaque_slots[0];
        assert_eq!(freed, if ans >= 0 { ans } else { -(ans + 1) });
        assert_eq!(t.remove(KeyKind::Bits, &bits(11), mix64(11)).unwrap(), -1);
        assert_eq!(typed_remove(&mut t, &KeyVal::Bits(77)).unwrap(), -1);
        assert_eq!(t.len(), 5);
    }

    /// Widening distinctness: `-1i8`, `255u8`, and `255i64` are three
    /// keys the closed set can carry, and the typed lanes must see what
    /// the Opaque lane's `KeyPayload::Bits` sees — signed widths cross
    /// SIGN-extended (`-1i8` hashes as bits `0xFFFF..FF`), unsigned
    /// widths ZERO-extend, and equal bits collapse to one key
    /// (`255u8` == `255i64`, the homogeneous-K contract).
    #[test]
    fn widening_distinctness_matches_the_opaque_bits_law() {
        let mut t = NativeTable::new(8);
        // -1i8 crosses as i64 -1: bits 0xFFFF..FFFF
        let neg = typed_entry(&mut t, KeyVal::Bits((-1i64) as u64)).unwrap();
        assert!(neg < 0, "fresh");
        // 255u8 and 255i64 are the SAME key (bits 255) — the i/u lanes
        // share the Bits kind, exactly as the Opaque lane does
        let u8_255 = typed_entry(&mut t, KeyVal::Bits(255)).unwrap();
        assert!(u8_255 < 0, "255 is a fresh key");
        assert_eq!(
            typed_entry(&mut t, KeyVal::Bits(255i64 as u64)).unwrap(),
            -(u8_255 + 1)
        );
        assert_eq!(t.len(), 2);
        // and neither matches -1: the three plan keys are distinct
        // (255u8/255i64 to each other, both from -1i8)
        let found_neg = typed_find(&t, &KeyVal::Bits((-1i64) as u64)).unwrap();
        assert_ne!(found_neg, -(u8_255 + 1));
        assert_ne!(found_neg, -1, "-1i8 is stored, not missing");
        assert_eq!(typed_find(&t, &KeyVal::Bits(255)).unwrap(), -(u8_255 + 1));
        assert_eq!(typed_find(&t, &KeyVal::Bits(256)).unwrap(), -1);
    }

    /// The bool lane: `true`/`false` hash mix64(1)/mix64(0) — mapset's
    /// bool impl's exact bits — and round-trip through the typed lane
    /// while staying consistent with the Opaque lane's recorded hash.
    #[test]
    fn the_bool_lane_hashes_mapsets_bools() {
        let mut t = NativeTable::new(8);
        let tru = typed_entry(&mut t, KeyVal::Bits(1)).unwrap();
        assert!(tru < 0);
        // the wrapper's bool hash: mix64(1) — what an Opaque-lane put
        // of `true` recorded
        assert_eq!(
            t.find(KeyKind::Bits, &bits(1), mix64(1)).unwrap(),
            -(tru + 1)
        );
        assert_eq!(
            typed_find(&t, &KeyVal::Bits(0)).unwrap(),
            -1,
            "false misses"
        );
        let fal = typed_entry(&mut t, KeyVal::Bits(0)).unwrap();
        assert!(fal < 0);
        assert_eq!(typed_find(&t, &KeyVal::Bits(0)).unwrap(), -(fal + 1));
        assert_eq!(
            t.remove(KeyKind::Bits, &bits(0), mix64(0)).unwrap(),
            -(fal + 1)
        );
        // hash_payload IS mix64(0)/mix64(1) for the bool bits
        assert_eq!(hash_payload(&KeyVal::Bits(0)), mix64(0));
        assert_eq!(hash_payload(&KeyVal::Bits(1)), mix64(1));
    }

    /// The fused grow sentinel fires EXACTLY at the load boundary:
    /// cap-8 table, the law (count+1)*10 >= 56 flips at count 5 — so
    /// the 5th insert succeeds (count 4 → 5, (4+1)*10 = 50 < 56) and
    /// the 6th answers `i32::MIN` without storing; after the grow +
    /// drain, the retry lands the key.
    #[test]
    fn the_fused_sentinel_fires_exactly_at_the_load_boundary() {
        let mut t = NativeTable::new(8);
        for i in 0u64..4 {
            let at = typed_entry(&mut t, KeyVal::Bits(i)).unwrap();
            assert!(at < 0, "key {i} inserts below the boundary");
        }
        assert_eq!(t.len(), 4);
        assert_eq!(t.cap(), 8);
        // (4+0+1)*10 = 50 < 56 — the 5th insert goes through
        let at = typed_entry(&mut t, KeyVal::Bits(4)).unwrap();
        assert!(at < 0, "the 5th insert is still legal: {at}");
        assert_eq!(t.len(), 5);
        assert_eq!(t.cap(), 8, "no grow happened on this path");
        // (5+0+1)*10 = 60 >= 56 — the 6th call answers the sentinel and
        // stores NOTHING
        assert_eq!(typed_entry(&mut t, KeyVal::Bits(5)).unwrap(), GROW_FIRST);
        assert_eq!(t.len(), 5, "the sentinel call inserted nothing");
        assert_eq!(t.cap(), 8);
        // the sentinel is OFF the entry encodings: no found slot (>= 0)
        // and no insert answer (-(slot+1) >= -2^30) can equal i32::MIN
        for i in 0u64..5 {
            let f = typed_find(&t, &KeyVal::Bits(i)).unwrap();
            assert!(f >= 0 && f != GROW_FIRST);
        }
        // grow + drain + retry — the wrapper's shape, exercised here
        t.grow();
        while t.take_reloc() >= 0 {}
        let at = typed_entry(&mut t, KeyVal::Bits(5)).unwrap();
        assert!(at < 0, "after the grow the retry inserts: {at}");
        assert_eq!(t.len(), 6);
    }

    /// The str/bytes typed lanes hash the CONTENT and probe on octet
    /// equality — a fresh copy of the same text finds the slot the
    /// Opaque lane stored (hash_payload == the wrapper's fnv1a64).
    #[test]
    fn typed_str_bytes_agree_with_the_opaque_lane() {
        let mut t = NativeTable::new(8);
        // Opaque-lane put: the wrapper's hash crosses in `h` (fnv1a64
        // over the octets — the SAME constants host's hash_payload
        // carries)
        let a = t
            .entry(
                KeyKind::Str,
                KeyVal::Str("alpha".into()),
                hash_payload(&KeyVal::Str("alpha".into())),
            )
            .unwrap();
        assert!(a < 0, "alpha inserts");
        let slot = -(a + 1);
        // typed find on a fresh copy: the host hashes the borrowed
        // octets and lands the same slot
        assert_eq!(typed_find(&t, &KeyVal::Str("alpha".into())).unwrap(), slot);
        assert_eq!(typed_find(&t, &KeyVal::Str("beta".into())).unwrap(), -1);
        // typed entry replaces at that slot; typed remove frees it
        let r = typed_entry(&mut t, KeyVal::Str("alpha".into())).unwrap();
        assert_eq!(r, slot, "same slot, no second entry");
        assert_eq!(
            typed_remove(&mut t, &KeyVal::Str("alpha".into())).unwrap(),
            slot
        );
        assert_eq!(typed_find(&t, &KeyVal::Str("alpha".into())).unwrap(), -1);

        let mut t = NativeTable::new(8);
        let b = typed_entry(&mut t, KeyVal::Bytes(vec![9, 8, 7])).unwrap();
        assert!(b < 0);
        assert_eq!(
            typed_find(&t, &KeyVal::Bytes(vec![9, 8, 7])).unwrap(),
            -(b + 1)
        );
        assert_eq!(typed_find(&t, &KeyVal::Bytes(vec![9, 8])).unwrap(), -1);
        assert_eq!(
            typed_remove(&mut t, &KeyVal::Bytes(vec![9, 8, 7])).unwrap(),
            -(b + 1)
        );
        assert_eq!(t.len(), 0);
    }

    /// The sentinel never collides with the entry encodings by
    /// construction: a found slot is `>= 0`, a fresh insert is
    /// `-(slot + 1) >= -2^30` — the plan's collision-free argument,
    /// pinned at values no real table reaches.
    #[test]
    fn the_sentinel_encoding_is_collision_free() {
        // the largest insert answer possible (cap 2^30): -(2^30) — one
        // ulp above i32::MIN, a 2^31 gap of headroom
        assert_eq!((-(0x4000_0000i32 + 1)) as i64, -0x4000_0001i64);
        assert!(GROW_FIRST == i32::MIN && GROW_FIRST < -0x4000_0000);
    }

    // ---- the val column (nmapset-round3, phase 1) ----------------------

    /// Raw-bit round-trips through the column: the full u64 sign range
    /// (2^63..2^64-1 cross with their bits — the u64 lane is raw in both
    /// directions) and f64 bit patterns (`to_bits`), replace at the same
    /// slot, and the presence-by-key laws — no write on remove (the
    /// stale bits stay, unreachable), and a re-insert at the reused DEAD
    /// slot stores a fresh val.
    #[test]
    fn vals_round_trip_raw_bits_through_set_and_get() {
        let mut t = NativeTable::new(8);
        // a fresh column reads zero everywhere (the zero-fill initial
        // state — there are no nil tags, the zeros are just init)
        for s in 0..8 {
            assert_eq!(t.val_get(s).unwrap(), 0);
        }
        assert_eq!(t.vals.len(), 8, "the column is cap-aligned at creation");
        // five keys at known slots
        let mut slots = Vec::new();
        for k in 0u64..5 {
            let a = t.entry(KeyKind::Bits, bits(k), mix64(k)).unwrap();
            slots.push(-(a + 1));
        }
        // the u64 sign range + f64 bit patterns, one per key
        let payloads: [u64; 5] = [
            u64::MAX,
            1u64 << 63,
            (1u64 << 63) + 12345,
            (-1.5f64).to_bits(),
            f64::INFINITY.to_bits(),
        ];
        for (i, s) in slots.iter().enumerate() {
            t.val_set(*s, payloads[i]).unwrap();
        }
        for (i, s) in slots.iter().enumerate() {
            assert_eq!(t.val_get(*s).unwrap(), payloads[i], "slot {s}");
        }
        // replace at the same slot: the found-path put overwrites
        t.val_set(slots[2], 42).unwrap();
        assert_eq!(t.val_get(slots[2]).unwrap(), 42);
        assert_eq!(t.val_get(slots[3]).unwrap(), payloads[3], "neighbors untouched");
        // remove writes NOTHING (presence-by-key): the stale bits stay
        let freed = t.remove(KeyKind::Bits, &bits(2), mix64(2)).unwrap();
        assert_eq!(freed, slots[2]);
        assert_eq!(t.val_get(slots[2]).unwrap(), 42, "no val write on remove");
        // re-insert: the DEAD slot wins the probe (the same slot), and
        // the next put stores a fresh val over the stale one
        let a = t.entry(KeyKind::Bits, bits(2), mix64(2)).unwrap();
        assert_eq!(a, -(freed + 1), "the reused DEAD slot is the fresh insert");
        t.val_set(slots[2], 7).unwrap();
        assert_eq!(t.val_get(slots[2]).unwrap(), 7);
        assert_eq!(t.vals.len(), t.cap() as usize, "cap alignment holds");
    }

    /// Grow relocates vals WITH the keys — pinned literally: the exact
    /// old→new slot mapping for keys 1..4 across cap 4→8, each val read
    /// back at its key's NEW slot, the column cap-aligned after the
    /// grow, and the drained queue matching the pairs the column moved.
    #[test]
    fn grow_moves_vals_with_the_keys_and_pins_the_mapping() {
        let mut t = NativeTable::new(4);
        let mut old_slots = Vec::new();
        for i in 1u64..4 {
            let a = t.entry(KeyKind::Bits, bits(i), mix64(i)).unwrap();
            let slot = -(a + 1);
            old_slots.push(slot);
            t.val_set(slot, i * 100).unwrap();
        }
        assert_eq!(t.grow(), 8);
        assert_eq!(t.vals.len(), 8, "the column doubles with the table");
        // pin the mapping (deterministic: recorded hashes, mapset's
        // constants) and the vals riding it
        let mut moved = std::collections::BTreeMap::new();
        loop {
            let packed = t.take_reloc();
            if packed < 0 {
                break;
            }
            let old = (packed >> 32) as i32;
            let new = (packed & 0xFFFF_FFFF) as i32;
            moved.insert(old, new);
        }
        assert_eq!(moved.len(), 3, "one pair per FULL slot");
        for (i, old) in old_slots.iter().enumerate() {
            let k = i as u64 + 1;
            let at = t.find(KeyKind::Bits, &bits(k), mix64(k)).unwrap();
            assert_eq!(moved.get(old), Some(&at), "key {k}: {old} -> {at}");
            assert_eq!(
                t.val_get(at).unwrap(),
                k * 100,
                "key {k}'s val followed its slot {old} -> {at}"
            );
        }
        // the drained queue is a second read of nothing
        assert_eq!(t.take_reloc(), -1);
    }

    /// Multi-grow sweep, the wrapper's fused shape: sentinel-driven
    /// grows with NO drain (the column path needs none), removes and
    /// DEAD-slot re-inserts interleaved, f64 bit patterns in the mix —
    /// every surviving key's val is exact after 4 grows (cap 4 → 64).
    #[test]
    fn vals_survive_multi_grow_sweeps_without_a_drain() {
        let mut t = NativeTable::new(4);
        // val bits: ints for most keys, f64 bit patterns every 5th
        let val_of = |k: u64| -> u64 {
            if k % 5 == 0 {
                (k as f64 + 0.25).to_bits()
            } else {
                k.wrapping_mul(0x9E37_79B9_7F4A_7C15)
            }
        };
        let put = |t: &mut NativeTable, k: u64| -> i32 {
            let mut at = typed_entry(t, KeyVal::Bits(k)).unwrap();
            while at == GROW_FIRST {
                t.grow(); // no drain: the column moved with the keys
                at = typed_entry(t, KeyVal::Bits(k)).unwrap();
            }
            let slot = if at >= 0 { at } else { -(at + 1) };
            t.val_set(slot, val_of(k)).unwrap();
            slot
        };
        for k in 0u64..150 {
            put(&mut t, k);
        }
        // the load law's exact stops: (count + 1)·10 >= cap·7 flips at
        // count 2 on cap 4, 5 on cap 8, 11 on 16, 22 on 32, 44 on 64,
        // 89 on 128 — so 150 puts land on cap 256
        assert_eq!(t.cap(), 256, "cap 4 grew to 256 across 150 puts");
        // removes + re-inserts churn the tombstone/reuse paths
        for k in (0u64..150).step_by(3) {
            assert!(typed_remove(&mut t, &KeyVal::Bits(k)).unwrap() >= 0);
        }
        for k in (0u64..150).step_by(3) {
            put(&mut t, k);
        }
        // every val exact, at the slot find answers now
        for k in 0u64..150 {
            let at = typed_find(&t, &KeyVal::Bits(k)).unwrap();
            assert!(at >= 0, "key {k} lost");
            assert_eq!(t.val_get(at).unwrap(), val_of(k), "key {k}");
        }
        assert_eq!(t.vals.len(), t.cap() as usize);
    }

    /// Caller-supplied slots are validated like any host lane: negative
    /// and `>= cap` trap (`Invalid` + "out of range"), `cap - 1` is the
    /// legal edge.
    #[test]
    fn val_slot_addressing_traps_out_of_range() {
        let mut t = NativeTable::new(4);
        t.val_set(3, 9).unwrap();
        assert_eq!(t.val_get(3).unwrap(), 9, "cap - 1 is the legal edge");
        for slot in [-1, 4, 5, i32::MAX] {
            let err = t.val_set(slot, 1).unwrap_err();
            assert_eq!(err.kind, TrapKind::Invalid);
            assert!(err.msg.contains("out of range"), "{}", err.msg);
            assert!(err.msg.contains("cap 4"), "{}", err.msg);
            let err = t.val_get(slot).unwrap_err();
            assert_eq!(err.kind, TrapKind::Invalid);
            assert!(err.msg.contains("out of range"), "{}", err.msg);
        }
        // an empty table traps identically (the check is the cap, not
        // occupancy — presence-by-key has no tags to consult)
        let e = NativeTable::new(4);
        assert_eq!(e.val_get(0).unwrap(), 0, "in range reads the zero init");
        let err = e.val_get(4).unwrap_err();
        assert!(err.msg.contains("out of range"));
    }

    /// The f64 val lane (phase 2): the crossing pair stores
    /// `v.to_bits()` and loads `f64::from_bits`, so float payloads ride
    /// the same u64 column byte-for-byte — bit patterns, not numeric
    /// conversions (the wrapper-facing law the `as`-cast integer lanes
    /// can't spell for floats: rut has no float bitcast, RFC 0007 §1).
    #[test]
    fn val_f64_lane_round_trips_bit_patterns() {
        let mut t = NativeTable::new(8);
        let vals = [
            1.5f64,
            -2.25,
            0.0,
            -0.0, // the sign bit is the payload — not "equal" to 0.0's bits
            f64::MIN_POSITIVE,
            f64::MAX,
            f64::MIN,
        ];
        for (i, v) in vals.iter().enumerate() {
            t.val_set(i as i32, v.to_bits()).unwrap();
        }
        for (i, v) in vals.iter().enumerate() {
            assert_eq!(t.val_get(i as i32).unwrap(), v.to_bits(), "val {i}");
            assert_eq!(f64::from_bits(t.val_get(i as i32).unwrap()), *v);
        }
        // -0.0 vs 0.0: distinct bit patterns in ONE column
        assert_ne!(t.val_get(2).unwrap(), t.val_get(3).unwrap());
    }

    /// The column sits on the PINNED hash slots: keys whose mix64 bits

    /// the hash-pin test above freezes land where those bits say
    /// (`h & mask`), including a probe collision (two pinned keys share
    /// the cap-8 home slot; linear probing separates them and the vals
    /// follow the keys, not the home slot).
    #[test]
    fn the_val_column_sits_on_the_pinned_hash_slots() {
        // hash_payload's pinned bits (the test above): key 11 ->
        // 12638163011299821354, u64::MAX -> 5808589858502755950,
        // key 255 -> 12638352127299873646
        let h11 = hash_payload(&KeyVal::Bits(11));
        let hmax = hash_payload(&KeyVal::Bits(u64::MAX));
        assert_eq!((h11 & 7) as i32, 2, "key 11's cap-8 home slot");
        assert_eq!((hmax & 7) as i32, 6, "u64::MAX's cap-8 home slot");
        let mut t = NativeTable::new(8);
        let a = t.entry(KeyKind::Bits, bits(11), h11).unwrap();
        assert_eq!(2, -(a + 1), "key 11 landed on its home slot");
        t.val_set(2, 0xDEAD_BEEF).unwrap();
        // key 255's pinned bits are 12638352127299873646 -> home 6 (the
        // same home as u64::MAX, unoccupied here): insert, then the
        // collision key u64::MAX probes onward
        let h255 = hash_payload(&KeyVal::Bits(255));
        let b = t.entry(KeyKind::Bits, bits(255), h255).unwrap();
        assert_eq!(6, -(b + 1), "key 255 landed on its home slot");
        t.val_set(6, 111).unwrap();
        let c = t.entry(KeyKind::Bits, bits(u64::MAX), hmax).unwrap();
        assert!(c < 0);
        let max_slot = -(c + 1);
        assert_ne!(max_slot, 6, "the shared home slot is taken: probe moves on");
        t.val_set(max_slot, u64::MAX).unwrap();
        // every val is at ITS key's slot — the vals followed the keys
        assert_eq!(t.val_get(2).unwrap(), 0xDEAD_BEEF);
        assert_eq!(t.val_get(6).unwrap(), 111);
        assert_eq!(t.val_get(max_slot).unwrap(), u64::MAX);
        // and the pins hold through a grow
        t.grow();
        let f11 = t.find(KeyKind::Bits, &bits(11), h11).unwrap();
        let f255 = t.find(KeyKind::Bits, &bits(255), h255).unwrap();
        let fmax = t.find(KeyKind::Bits, &bits(u64::MAX), hmax).unwrap();
        assert_eq!(t.val_get(f11).unwrap(), 0xDEAD_BEEF);
        assert_eq!(t.val_get(f255).unwrap(), 111);
        assert_eq!(t.val_get(fmax).unwrap(), u64::MAX);
    }

    /// THE CHECKSUM LAW (plan phase 1): one wrapper-shaped op sequence —
    /// fused-sentinel puts with replaces, grows, removes, hits — driven
    /// through BOTH val storages: today's `[?V]` sidecar (a `Vec`
    /// relocated by draining `take_reloc`, exactly what nmapset.rut's
    /// wrapper does) and the native column (no drain). Identical slots
    /// per key, identical checksum, pinned as a literal.
    #[test]
    fn the_column_matches_the_sidecar_path_wrapper_shaped() {
        // the shared op sequence, parameterized by the val storage
        fn run(column: bool) -> (u64, Vec<(u64, i32)>) {
            let mut t = NativeTable::new(4);
            // the sidecar's nil is 0; the column's init is 0 too
            let mut vals: Vec<u64> = Vec::new();
            let val_of = |k: u64| -> u64 {
                if k % 7 == 0 {
                    (-(k as f64)).to_bits()
                } else {
                    k * 7 + 1
                }
            };
            // the wrapper's put: fused sentinel -> grow (+ the sidecar's
            // drain) -> retry -> store the val at the answered slot
            let mut put = |t: &mut NativeTable, vals: &mut Vec<u64>, k: u64, v: u64| {
                let mut at = typed_entry(t, KeyVal::Bits(k)).unwrap();
                while at == GROW_FIRST {
                    let new_cap = t.grow();
                    if !column {
                        let mut next: Vec<u64> = vec![0; new_cap as usize];
                        loop {
                            let packed = t.take_reloc();
                            if packed < 0 {
                                break;
                            }
                            let old = (packed >> 32) as usize;
                            let new = (packed & 0xFFFF_FFFF) as usize;
                            next[new] = vals[old];
                        }
                        *vals = next;
                    }
                    at = typed_entry(t, KeyVal::Bits(k)).unwrap();
                }
                let slot = if at >= 0 { at } else { -(at + 1) };
                if column {
                    t.val_set(slot, v).unwrap();
                } else {
                    vals.resize(t.cap() as usize, 0);
                    vals[slot as usize] = v;
                }
            };
            // churn: 300 puts (cap 4 -> 128), replaces on the % 4 keys,
            // removes on the % 3 keys
            for k in 0u64..300 {
                put(&mut t, &mut vals, k, val_of(k));
            }
            for k in (0u64..300).step_by(4) {
                put(&mut t, &mut vals, k, val_of(k).wrapping_add(1));
            }
            for k in (0u64..300).step_by(3) {
                let at = typed_remove(&mut t, &KeyVal::Bits(k)).unwrap();
                if at >= 0 && !column {
                    vals[at as usize] = 0; // the wrapper nils the cell
                }
            }
            // the hit phase: every key, checksum over the found vals
            let mut acc: u64 = 0;
            let mut hits: u64 = 0;
            let mut slots = Vec::new();
            for k in 0u64..300 {
                let at = typed_find(&t, &KeyVal::Bits(k)).unwrap();
                if at < 0 {
                    continue;
                }
                let v = if column { t.val_get(at).unwrap() } else { vals[at as usize] };
                acc = acc.wrapping_add(v).rotate_left(17) ^ (k as u64);
                hits += 1;
                slots.push((k, at));
            }
            (acc ^ hits.wrapping_mul(41), slots)
        }
        let (side_sum, side_slots) = run(false);
        let (col_sum, col_slots) = run(true);
        // identical slot assignment (iteration order cannot move) and
        // an identical checksum
        assert_eq!(side_slots, col_slots, "same keys, same slots");
        assert_eq!(side_sum, col_sum, "the two storages agree bit for bit");
        // the pinned literal: the checksum this exact sequence produces
        assert_eq!(col_sum, 5344915641404318251);
    }
}
