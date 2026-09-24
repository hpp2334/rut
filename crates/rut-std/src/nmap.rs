//! `nmap_host` — the native key-table experiment (the mapset-host plan, H2):
//! an open-addressing hash table whose state lives Rust-side behind an
//! `opaque` payload box (RFC 0023), so rut meets it only through the
//! `pub host fn` surface bound by [`install_std_nmap`]. This is the HOST
//! half of the experiment. (The pure-rut `mapset` package — the original
//! reference and general-key implementation — was REMOVED from the tree
//! in Sep 2026, stdlib slim-down: general keys now mean a `bytes`
//! encoding or the `nmapset` wrapper; the old source lives in git
//! history.)
//!
//! Design (plan §0/§1):
//! - the native side owns KEYS only — owned Rust data (integer bits /
//!   `String` / `Vec<u8>` copies). Values stay rut-side in the wrapper's
//!   parallel array, so `get -> *V` aliasing semantics match mapset
//!   exactly and the payload box stays free of rut cells (no release
//!   hazards — the rc-0 `Drop` is plain Rust, RFC 0016 §3). PROBING
//!   (strings-round1 phase 1) is borrowed: the `s` lanes hash and
//!   compare OVER the crossed octets — a whole owned `str` block or a
//!   slice view's window, zero-copy — and only a fresh insert
//!   materializes the owned stored form;
//! - the key set is CLOSED (i8..i64, u8..u64, bool, str, bytes). Keys
//!   hash wrapper-side and cross in `h` — the table only RECORDS hashes;
//!   the payload box crosses once per call and is read through the H1
//!   accessor (`Vm::opaque_key_payload`). Anything else — floats, chars,
//!   user records, host boxes — traps loudly, pointing at the escape
//!   hatches (encode the key canonically to `bytes`, or use
//!   `nmapset`). No
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
//!
//! The handle column + the fused lanes (nmapset-hostops): the takeover
//! — every impl control flow moves host-side. The table also carries
//! `handles: Vec<i32>` (one STABLE birth index per key, relocated with
//! the keys by `grow`), and the `map_h{put,find,remove}_*` family
//! rides it: `hput` grows INTERNALLY at the load boundary and answers
//! the packed `(handle << 1) | newly` i64; `hfind`/`hremove` answer
//! the key's handle (or `-1`). A wrapper indexed by the handle has no
//! sentinel loop, no relocation drain, and never reallocates its
//! `[?V]` sidecar on grow. Handles are MONOTONIC birth indices, never
//! recycled (v1; the churn cost is one nil sidecar slot per distinct
//! key ever inserted). The legacy crossings stay bound — same table,
//! same slots, same hashes — so every existing row's answers are
//! untouched by construction.

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
    /// the handle column (nmapset-hostops): one STABLE birth index per
    /// slot, assigned at first insert, relocated WITH the keys by
    /// `grow` (the same re-slot walk — the handle is the key's
    /// identity, not its address). The fused `hput`/`hfind`/`hremove`
    /// lanes answer it; the wrapper's `[?V]` sidecar is indexed by it,
    /// so the wrapper never drains and never reallocates on grow. An
    /// EMPTY slot holds `-1`; a DEAD slot holds its dead key's handle
    /// until the slot's next insert overwrites it with a fresh birth
    /// (unreachable through the protocol — `find` never answers a DEAD
    /// slot). MONOTONIC v1, no free-list: a removed key's sidecar slot
    /// nils and dies with it; the cost under churn is one nil slot per
    /// distinct key ever inserted (the survey §3a tradeoff).
    ///
    /// Accounting note (the vals precedent): another Vec header grows
    /// `size_of::<NativeTable>()` 144 → 168 and `next_handle` packs
    /// into the trailing padding — +24 B per table box at `map_new`.
    /// Plain Rust backing, invisible to cell accounting; the legacy
    /// crossings' paths never read this column, so every existing
    /// row's checksum and fuel are untouched.
    handles: Vec<i32>,
    /// the next birth index — the table's lifetime insertion count,
    /// bounded by BIRTHS not cap (the packed i64 answer's headroom
    /// law: the handle rides i32 with room far past any real table)
    next_handle: i32,
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
            handles: vec![-1; c as usize],
            next_handle: 0,
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
        Ok(self.insert_at(kind, at, key, h))
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
        Ok(self.tombstone_at(at))
    }

    /// The s lane's borrowed put (strings-round1 phase 1): probe OVER
    /// the borrowed range; a hit answers its slot with nothing stored
    /// and NO copy — the per-crossing `to_owned()` the phase-0 stub
    /// deleted lives only on the fresh insert now. The stored form is
    /// still an owned copy of the range (one copy, at insert), so the
    /// stored-key law is unchanged: same content, same slot, same
    /// recorded hash as the owned `entry` on the same op stream.
    pub fn entry_str_range(&mut self, range: &str, h: u64) -> Result<i32, Trap> {
        self.admit(KeyKind::Str)?;
        let at = self.probe_str(range, h);
        if at >= 0 {
            return Ok(at); // found — replace in place, nothing stored
        }
        Ok(self.insert_at(KeyKind::Str, at, KeyVal::Str(range.to_owned()), h))
    }

    /// The s lane's borrowed lookup: probe only, never a copy. Same
    /// answers as the owned `find` for the same content.
    pub fn find_str_range(&self, range: &str, h: u64) -> Result<i32, Trap> {
        self.admit(KeyKind::Str)?;
        Ok(match self.probe_str(range, h) {
            at if at >= 0 => at,
            _ => -1,
        })
    }

    /// The s lane's borrowed remove: probe, tombstone the found slot,
    /// never a copy. Same answers as the owned `remove`.
    pub fn remove_str_range(&mut self, range: &str, h: u64) -> Result<i32, Trap> {
        self.admit(KeyKind::Str)?;
        let at = self.probe_str(range, h);
        if at < 0 {
            return Ok(-1);
        }
        Ok(self.tombstone_at(at))
    }

    /// The shared post-miss insert (every entry lane's tail): the probe
    /// already said MISS at `at` (the insertion slot, encoded
    /// `-(at + 1)`); take the tomb count, mark FULL, store the owned
    /// key and its recorded hash. On the borrowed path the owned key
    /// materializes BEFORE this runs — after the probe said miss — so
    /// the copy is the insert's, once, never per-probe. (`#[inline]`:
    /// shared by the entry lanes, but it must melt into each caller —
    /// a real call here cost the probe-dense rows ~3%.)
    #[inline]
    fn insert_at(&mut self, kind: KeyKind, at: i32, owned: KeyVal, h: u64) -> i32 {
        if self.kind == KeyKind::Unset {
            self.kind = kind;
        }
        let slot = -(at + 1);
        if self.states[slot as usize] == 2 {
            self.tomb -= 1; // the DEAD slot leaves the tomb count
        }
        self.states[slot as usize] = 1;
        self.keys[slot as usize] = owned;
        self.hashes[slot as usize] = h;
        // the fresh birth's stable handle (nmapset-hostops): monotonic,
        // never recycled — the fused lanes answer it, and a reused DEAD
        // slot overwrites its dead tenant's handle here
        self.handles[slot as usize] = self.next_handle;
        self.next_handle += 1;
        self.count += 1;
        -(slot + 1)
    }

    /// The shared remove tail: tombstone the found slot (the stored key
    /// drops with the store) and answer it.
    #[inline]
    fn tombstone_at(&mut self, at: i32) -> i32 {
        self.keys[at as usize] = KeyVal::Bits(0);
        self.states[at as usize] = 2;
        self.count -= 1;
        self.tomb += 1;
        at
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
        let old_handles = std::mem::take(&mut self.handles);
        let new_cap = self.cap * 2;
        self.keys = vec![KeyVal::Bits(0); new_cap as usize];
        self.hashes = vec![0; new_cap as usize];
        self.states = vec![0; new_cap as usize];
        self.vals = vec![0; new_cap as usize];
        self.handles = vec![-1; new_cap as usize];
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
            self.handles[at as usize] = old_handles[i];
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

    /// The slot's stable handle — the fused lanes' answer. Callers pass
    /// probe-validated slots (a found/fresh/removed slot from this
    /// call), but the crossing honesty holds the same bounds check as
    /// the val column (defense, never expected to fire).
    fn handle_at(&self, slot: i32) -> Result<i32, Trap> {
        if slot < 0 || slot >= self.cap as i32 {
            return Err(Trap::new(
                TrapKind::Invalid,
                format!("nmap: handle slot {slot} out of range (cap {})", self.cap),
            ));
        }
        Ok(self.handles[slot as usize])
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
    /// This is the OWNED-key probe — the Opaque and i/u/b/y lanes'
    /// machine path, byte-for-byte what it has always been.
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

    /// The BORROWED-probe pass (strings-round1 phase 1) — the s lanes'
    /// stages: state byte, recorded hash, then the octet compare OVER
    /// THE RANGE. `range` covers both key shapes with one borrowed
    /// `&str`: a whole owned `str` cell's block (the `s` lane's key), or
    /// a slice view's window into its parent block — parent + off + len
    /// flattened by the RFC 0042 read and carved to bytes by `sv_range`
    /// (the `sv` lanes' key). No copy on any step; only a fresh insert
    /// materializes. Same answers as [`Self::probe`] on the same
    /// content: the stored keys are the same `KeyVal::Str`s and the
    /// recorded hashes are [`hash_bytes`]' bits either way.
    fn probe_str(&self, range: &str, h: u64) -> i32 {
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
            if st == 1 && self.hashes[at as usize] == h {
                if let KeyVal::Str(s) = &self.keys[at as usize] {
                    if s.as_bytes() == range.as_bytes() {
                        return at;
                    }
                }
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
/// message names the type and points at the escape hatches (encode the
/// key canonically to `bytes`, or use the `nmapset` wrapper).
fn key_val(vm: &Vm, k: &OpaqueRef) -> Result<(KeyKind, KeyVal), Trap> {
    match vm.opaque_key_payload(k)? {
        KeyPayload::Bits { val, .. } => Ok((KeyKind::Bits, KeyVal::Bits(val))),
        KeyPayload::Str(s) => Ok((KeyKind::Str, KeyVal::Str(s))),
        KeyPayload::Bytes(b) => Ok((KeyKind::Bytes, KeyVal::Bytes(b))),
        KeyPayload::Unsupported(ty) => Err(Trap::new(
            TrapKind::Invalid,
            format!(
                "nmap: key type `{}` is not natively supported — encode the key canonically to `bytes`, or use `nmapset` (the native key set is the integer primitives, `bool`, `str`, and `bytes`)",
                vm.prog.type_name(ty)
            ),
        )),
    }
}

/// The two hash constants the removed pure-rut `mapset` pkg's
/// `Hashable` impls ran (the ONE key contract, now nmapset.rut's
/// `mix64`/`fnv1a64` alone): FNV-1a 64's offset basis
/// and prime. These are the checksum law — ported BIT-FOR-BIT, so the
/// host's recorded hashes equal the wrapper's `k.hash()` bit for bit
/// and the bench checksums pinned in `expected.json` hold. A one-ulp
/// difference here breaks the gate; the unit test pins
/// the exact outputs as literals.
const FNV_OFFSET: u64 = 14695981039346656037;
const FNV_PRIME: u64 = 1099511628211;

/// FNV-1a 64 over an octet range — the ONE str/bytes hasher, shared by
/// every lane that hashes octets. The constants are the checksum law
/// (see [`FNV_OFFSET`]); the borrowed-probe core (strings-round1 phase
/// 1) runs this OVER a borrowed range — no copy — and it answers the
/// same bits [`hash_payload`] answers for the same octets, by
/// construction (the s lanes route through here).
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    for b in bytes {
        h = (h ^ *b as u64).wrapping_mul(FNV_PRIME);
    }
    h
}

/// The typed lanes' host-side hash — the ONE shared payload hasher the
/// crossings run instead of minting an Opaque box and reading the
/// wrapper's `h`: mix64 for the integer/bool bits, FNV-1a 64 over the
/// octets for `str`/`bytes`. Same inputs, nmapset.rut's constants,
/// bit-exact.
pub fn hash_payload(k: &KeyVal) -> u64 {
    match k {
        KeyVal::Bits(v) => (FNV_OFFSET ^ v).wrapping_mul(FNV_PRIME),
        KeyVal::Str(s) => hash_bytes(s.as_bytes()),
        KeyVal::Bytes(b) => hash_bytes(b),
    }
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

/// The sv lanes' range check (strings-round1 phase 1): `off`/`len` are
/// BYTE offsets into the parent's octets — `0 <= off`, `0 <= len`,
/// `off + len <= parent.len()` (usize math, so the i32 corners cannot
/// overflow the check), and BOTH ends must sit on UTF-8 codepoint
/// boundaries — a mid-codepoint window is a caller bug, trapped with
/// the house `Invalid` shape, never a silent mis-read. The answers are
/// the table's `str`-lane lanes: [`NativeTable::entry_str_range`] /
/// `find_str_range` / `remove_str_range` hash and compare over the
/// range zero-copy.
fn sv_range<'a>(parent: &'a str, off: i32, len: i32) -> Result<&'a str, Trap> {
    // the end offset in i64 — the trap messages name it, so the i32
    // corners must not overflow the naming arithmetic
    let end = off as i64 + len as i64;
    if off < 0 || len < 0 {
        return Err(Trap::new(
            TrapKind::Invalid,
            format!(
                "nmap: sv range off {off} len {len} out of range (parent {} bytes)",
                parent.len()
            ),
        ));
    }
    let (o, l) = (off as usize, len as usize);
    if o + l > parent.len() {
        return Err(Trap::new(
            TrapKind::Invalid,
            format!(
                "nmap: sv range [{off}..{end}] out of range (parent {} bytes)",
                parent.len()
            ),
        ));
    }
    if !parent.is_char_boundary(o) {
        return Err(Trap::new(
            TrapKind::Invalid,
            format!("nmap: sv offset {off} is not a UTF-8 boundary (parent {} bytes)", parent.len()),
        ));
    }
    if !parent.is_char_boundary(o + l) {
        return Err(Trap::new(
            TrapKind::Invalid,
            format!(
                "nmap: sv end offset {end} is not a UTF-8 boundary (parent {} bytes)",
                parent.len()
            ),
        ));
    }
    Ok(&parent[o..o + l])
}

/// The s lane's borrowed put — the hash is [`hash_bytes`] over the
/// crossed octets (the crossed `&str` borrows its home block: an owned
/// `str` cell's, or a slice view's parent window), then the fused
/// grow-first sentinel, then the borrowed entry. The per-crossing
/// `to_owned()` is GONE: the only copy left is the fresh insert's.
fn typed_entry_s(t: &mut NativeTable, k: &str) -> Result<i32, Trap> {
    let h = hash_bytes(k.as_bytes());
    if t.needs_grow() {
        return Ok(GROW_FIRST); // grow-first: nothing stored on this call
    }
    t.entry_str_range(k, h)
}

/// The s lane's borrowed lookup — probe over the octets, never a copy.
fn typed_find_s(t: &NativeTable, k: &str) -> Result<i32, Trap> {
    let h = hash_bytes(k.as_bytes());
    t.find_str_range(k, h)
}

/// The s lane's borrowed remove — probe over the octets, never a copy.
fn typed_remove_s(t: &mut NativeTable, k: &str) -> Result<i32, Trap> {
    let h = hash_bytes(k.as_bytes());
    t.remove_str_range(k, h)
}

/// The sv entry lane (strings-round1 phase 1): validate the byte range,
/// hash over it, then the fused grow-first sentinel + the borrowed
/// entry — a fresh insert stores ONE owned copy of the range. Same
/// recorded hash, same slot, same answers as the `s` lane on the same
/// content (the parity law, by construction).
fn typed_entry_sv(t: &mut NativeTable, parent: &str, off: i32, len: i32) -> Result<i32, Trap> {
    let range = sv_range(parent, off, len)?;
    let h = hash_bytes(range.as_bytes());
    if t.needs_grow() {
        return Ok(GROW_FIRST); // grow-first: nothing stored on this call
    }
    t.entry_str_range(range, h)
}

/// The sv lookup lane: validate, hash, probe — never a copy.
fn typed_find_sv(t: &NativeTable, parent: &str, off: i32, len: i32) -> Result<i32, Trap> {
    let range = sv_range(parent, off, len)?;
    let h = hash_bytes(range.as_bytes());
    t.find_str_range(range, h)
}

/// The sv remove lane: validate, hash, probe, tombstone — never a copy.
fn typed_remove_sv(t: &mut NativeTable, parent: &str, off: i32, len: i32) -> Result<i32, Trap> {
    let range = sv_range(parent, off, len)?;
    let h = hash_bytes(range.as_bytes());
    t.remove_str_range(range, h)
}

// ---- the fused handle lanes (nmapset-hostops) ------------------------
// ONE crossing per op with ALL control flow host-side. `hput` fuses
// entry + INTERNAL grow + the packed answer; `hfind`/`hremove` answer
// the key's STABLE handle or `-1`. The wrapper indexes its `[?V]`
// sidecar by the handle — no sentinel loop, no relocation drain, no
// slot ever crosses the boundary. The answers:
// - `map_hput_*`: `(handle << 1) | newly` as i64 — bit 0 is the newly
//   bit (`(ans & 1) == 1` — HashSet shares this lane and reads exactly
//   that bit), the rest the handle. Handles are bounded by BIRTHS, not
//   cap, so the i64 carrier has headroom far past any real table.
// - `map_hfind_*` / `map_hremove_*`: the handle, or `-1` (absent).
//   `hremove` answers the DEAD key's own handle (the tombstoned slot
//   keeps it until reuse) so the wrapper can nil exactly that sidecar
//   slot and release the cell.
// The legacy lanes stay (the user's keep call): same table, same slot
// assignment, same recorded hashes — the handle column is additive
// state relocated with the keys, so the old crossings' answers and
// every existing checksum are untouched by construction.

/// The packed `hput` answer: bit 0 = the newly bit, the rest the handle.
#[inline]
fn pack_answer(hd: i32, newly: bool) -> i64 {
    ((hd as i64) << 1) | (newly as i64)
}

/// The answer tail shared by every `hput` lane: the entry's slot answer
/// (found `>= 0` / fresh `-(slot + 1)`) into the packed form.
#[inline]
fn packed_entry_answer(t: &NativeTable, at: i32) -> Result<i64, Trap> {
    let (slot, newly) = if at >= 0 { (at, false) } else { (-(at + 1), true) };
    Ok(pack_answer(t.handle_at(slot)?, newly))
}

/// The i/u/b/y lanes' fused put: hash, grow internally when the load
/// factor would breach (ONE grow always suffices — count halves its
/// ratio when cap doubles and tomb resets), then the owned entry.
fn fused_put(t: &mut NativeTable, key: KeyVal) -> Result<i64, Trap> {
    let h = hash_payload(&key);
    let kind = kind_of(&key);
    if t.needs_grow() {
        t.grow();
    }
    let at = t.entry(kind, key, h)?;
    packed_entry_answer(t, at)
}

/// The i/u/b/y lanes' fused find: probe, answer the handle or `-1`.
fn fused_find(t: &NativeTable, key: &KeyVal) -> Result<i32, Trap> {
    match typed_find(t, key)? {
        at if at >= 0 => t.handle_at(at),
        _ => Ok(-1),
    }
}

/// The i/u/b/y lanes' fused remove: tombstone, answer the dead key's
/// handle or `-1`.
fn fused_remove(t: &mut NativeTable, key: &KeyVal) -> Result<i32, Trap> {
    match typed_remove(t, key)? {
        at if at >= 0 => t.handle_at(at),
        _ => Ok(-1),
    }
}

/// The s lane's fused put — the borrowed probe, zero-copy until a
/// fresh insert materializes the owned stored form.
fn fused_put_s(t: &mut NativeTable, k: &str) -> Result<i64, Trap> {
    let h = hash_bytes(k.as_bytes());
    if t.needs_grow() {
        t.grow();
    }
    let at = t.entry_str_range(k, h)?;
    packed_entry_answer(t, at)
}

/// The s lane's fused find.
fn fused_find_s(t: &NativeTable, k: &str) -> Result<i32, Trap> {
    match typed_find_s(t, k)? {
        at if at >= 0 => t.handle_at(at),
        _ => Ok(-1),
    }
}

/// The s lane's fused remove.
fn fused_remove_s(t: &mut NativeTable, k: &str) -> Result<i32, Trap> {
    match typed_remove_s(t, k)? {
        at if at >= 0 => t.handle_at(at),
        _ => Ok(-1),
    }
}

/// The sv lane's fused put: validate the byte window (the house
/// `Invalid` trap on a bad range — the table is left intact), hash over
/// the range, grow internally, insert. Same recorded hash and same slot
/// as the s lane on the same content (the parity law) — and therefore
/// the same HANDLE: an sv key and the equal-content `str` key are one
/// key with one identity.
fn fused_put_sv(t: &mut NativeTable, parent: &str, off: i32, len: i32) -> Result<i64, Trap> {
    let range = sv_range(parent, off, len)?;
    let h = hash_bytes(range.as_bytes());
    if t.needs_grow() {
        t.grow();
    }
    let at = t.entry_str_range(range, h)?;
    packed_entry_answer(t, at)
}

/// The sv lane's fused find.
fn fused_find_sv(t: &NativeTable, parent: &str, off: i32, len: i32) -> Result<i32, Trap> {
    match typed_find_sv(t, parent, off, len)? {
        at if at >= 0 => t.handle_at(at),
        _ => Ok(-1),
    }
}

/// The sv lane's fused remove.
fn fused_remove_sv(t: &mut NativeTable, parent: &str, off: i32, len: i32) -> Result<i32, Trap> {
    match typed_remove_sv(t, parent, off, len)? {
        at if at >= 0 => t.handle_at(at),
        _ => Ok(-1),
    }
}

/// Install the nine `nmap_host` bodies under the `nmap_host` scope (the `calc`
/// pattern, RFC 0023/0025): the callable's Rust shape IS the `.d.rut`
/// row, so the surface declares exactly these signatures. `map_new`'s
/// box carries the table; every other fn's first param borrows it typed
/// (`OpaqueBox<NativeTable>` — a wrong payload is a checked trap).
pub fn install_std_nmap(hosts: &mut HostRegistry) {
    rut_vm::register!(hosts, "nmap_host::map_new", (i64,) -> OpaqueRef, |vm: &mut Vm, cap: i64| {
        let b = OpaqueBox::alloc(vm, NativeTable::new(cap))?;
        Ok(b.handle().clone())
    });
    rut_vm::register!(
        hosts,
        "nmap_host::map_entry",
        (OpaqueBox<NativeTable>, OpaqueRef, i64) -> i32,
        |vm: &mut Vm, b: OpaqueBox<NativeTable>, k: OpaqueRef, h: i64| -> Result<i32, Trap> {
            let (kind, key) = key_val(vm, &k)?;
            Ok(b.with_mut(|t| t.entry(kind, key, h as u64))??)
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_find",
        (OpaqueBox<NativeTable>, OpaqueRef, i64) -> i32,
        |vm: &mut Vm, b: OpaqueBox<NativeTable>, k: OpaqueRef, h: i64| -> Result<i32, Trap> {
            let (kind, key) = key_val(vm, &k)?;
            Ok(b.with(|t| t.find(kind, &key, h as u64))??)
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_remove",
        (OpaqueBox<NativeTable>, OpaqueRef, i64) -> i32,
        |vm: &mut Vm, b: OpaqueBox<NativeTable>, k: OpaqueRef, h: i64| -> Result<i32, Trap> {
            let (kind, key) = key_val(vm, &k)?;
            Ok(b.with_mut(|t| t.remove(kind, &key, h as u64))??)
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_needs_grow",
        (OpaqueBox<NativeTable>,) -> bool,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>| b.with(|t| t.needs_grow()),
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_grow",
        (OpaqueBox<NativeTable>,) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>| b.with_mut(|t| t.grow()),
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_take_reloc",
        (OpaqueBox<NativeTable>,) -> i64,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>| b.with_mut(|t| t.take_reloc()),
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_cap",
        (OpaqueBox<NativeTable>,) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>| b.with(|t| t.cap()),
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_len",
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
    // `str`/`bytes` ZERO-COPY (strings-round1 phase 1: the str key
    // probes OVER the crossed octets — the per-crossing `to_owned()` is
    // gone, only a fresh entry stores an owned copy; the bytes lane
    // keeps its owned probe key). `map_entry_*` answers the fused grow
    // sentinel ([`GROW_FIRST`] = `i32::MIN`) before any insert.
    rut_vm::register!(
        hosts,
        "nmap_host::map_entry_i",
        (OpaqueBox<NativeTable>, i64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: i64| -> Result<i32, Trap> {
            b.with_mut(|t| typed_entry(t, KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_entry_u",
        (OpaqueBox<NativeTable>, u64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: u64| -> Result<i32, Trap> {
            b.with_mut(|t| typed_entry(t, KeyVal::Bits(k)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_entry_b",
        (OpaqueBox<NativeTable>, bool) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: bool| -> Result<i32, Trap> {
            b.with_mut(|t| typed_entry(t, KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_entry_s",
        (OpaqueBox<NativeTable>, &str) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &str| -> Result<i32, Trap> {
            // strings-round1 phase 1: the key probes OVER the crossed
            // octets — no per-crossing `to_owned()`; only a fresh
            // insert stores an owned copy (the stored-key law unchanged)
            b.with_mut(|t| typed_entry_s(t, k))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_entry_y",
        (OpaqueBox<NativeTable>, &[u8]) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &[u8]| -> Result<i32, Trap> {
            b.with_mut(|t| typed_entry(t, KeyVal::Bytes(k.to_vec())))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_find_i",
        (OpaqueBox<NativeTable>, i64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: i64| -> Result<i32, Trap> {
            b.with(|t| typed_find(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_find_u",
        (OpaqueBox<NativeTable>, u64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: u64| -> Result<i32, Trap> {
            b.with(|t| typed_find(t, &KeyVal::Bits(k)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_find_b",
        (OpaqueBox<NativeTable>, bool) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: bool| -> Result<i32, Trap> {
            b.with(|t| typed_find(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_find_s",
        (OpaqueBox<NativeTable>, &str) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &str| -> Result<i32, Trap> {
            b.with(|t| typed_find_s(t, k))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_find_y",
        (OpaqueBox<NativeTable>, &[u8]) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &[u8]| -> Result<i32, Trap> {
            b.with(|t| typed_find(t, &KeyVal::Bytes(k.to_vec())))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_remove_i",
        (OpaqueBox<NativeTable>, i64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: i64| -> Result<i32, Trap> {
            b.with_mut(|t| typed_remove(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_remove_u",
        (OpaqueBox<NativeTable>, u64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: u64| -> Result<i32, Trap> {
            b.with_mut(|t| typed_remove(t, &KeyVal::Bits(k)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_remove_b",
        (OpaqueBox<NativeTable>, bool) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: bool| -> Result<i32, Trap> {
            b.with_mut(|t| typed_remove(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_remove_s",
        (OpaqueBox<NativeTable>, &str) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &str| -> Result<i32, Trap> {
            b.with_mut(|t| typed_remove_s(t, k))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_remove_y",
        (OpaqueBox<NativeTable>, &[u8]) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &[u8]| -> Result<i32, Trap> {
            b.with_mut(|t| typed_remove(t, &KeyVal::Bytes(k.to_vec())))?
        },
    );

    // ---- the sv lanes (strings-round1, phase 1) ------------------------
    // str keys cross as a BORROWED RANGE: `parent` is any str value (an
    // owned `str` cell or a slice view over one — both read as their
    // range through the RFC 0042 zero-copy boundary) and `off`/`len`
    // carve the probe range out of it in BYTES. The host hashes and
    // compares OVER THE RANGE — the parent's octets never copy on a
    // probe; only a fresh `map_entry_sv` insert stores an owned copy of
    // the range, so stored keys stay owned (the stored-key/iteration
    // law unchanged). Both offsets must sit on UTF-8 codepoint
    // boundaries with `0 <= off`, `0 <= len`, `off + len <= parent
    // byte length` — anything else is the house `Invalid` trap. The
    // hash constants are the s lanes' exact FNV-1a 64, so an sv key and
    // an s key of the same content are the SAME key: same recorded
    // hash, same slot, same iteration position — either lane finds what
    // the other stored. Answers = the typed lanes' (`map_find_sv` /
    // `map_remove_sv`: `>= 0` the slot, `-1` absent; `map_entry_sv`:
    // the fused grow-first `i32::MIN`, else the found/fresh slot).
    rut_vm::register!(
        hosts,
        "nmap_host::map_entry_sv",
        (OpaqueBox<NativeTable>, &str, i32, i32) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, parent: &str, off: i32, len: i32| -> Result<i32, Trap> {
            b.with_mut(|t| typed_entry_sv(t, parent, off, len))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_find_sv",
        (OpaqueBox<NativeTable>, &str, i32, i32) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, parent: &str, off: i32, len: i32| -> Result<i32, Trap> {
            b.with(|t| typed_find_sv(t, parent, off, len))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_remove_sv",
        (OpaqueBox<NativeTable>, &str, i32, i32) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, parent: &str, off: i32, len: i32| -> Result<i32, Trap> {
            b.with_mut(|t| typed_remove_sv(t, parent, off, len))?
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
        "nmap_host::map_val_set_u",
        (OpaqueBox<NativeTable>, i32, u64) -> (),
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, slot: i32, raw: u64| -> Result<(), Trap> {
            b.with_mut(|t| t.val_set(slot, raw))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_val_get_u",
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
        "nmap_host::map_val_set_f",
        (OpaqueBox<NativeTable>, i32, f64) -> (),
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, slot: i32, v: f64| -> Result<(), Trap> {
            b.with_mut(|t| t.val_set(slot, v.to_bits()))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_val_get_f",
        (OpaqueBox<NativeTable>, i32) -> f64,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, slot: i32| -> Result<f64, Trap> {
            b.with(|t| t.val_get(slot).map(f64::from_bits))?
        },
    );

    // ---- the fused handle lanes (nmapset-hostops) ----------------------
    // 18 fns `map_h{put,find,remove}_{i,u,b,s,y,sv}`: the takeover
    // surface. The key crosses DIRECTLY on the same typed lanes as the
    // legacy family; the difference is the ANSWER and the control flow
    // — `hput` grows INTERNALLY at the load boundary (the legacy
    // `map_entry_*` sentinel loop and the wrapper's relocation drain
    // both die here, host-side) and answers the packed
    // `(handle << 1) | newly` i64; `hfind`/`hremove` answer the key's
    // STABLE birth handle (or `-1`), which the wrapper's `[?V]` sidecar
    // is indexed by. `HashSet` shares `hput` and reads bit 0. The
    // legacy crossings above stay bound and documented — same table,
    // same slots, same hashes.
    rut_vm::register!(
        hosts,
        "nmap_host::map_hput_i",
        (OpaqueBox<NativeTable>, i64) -> i64,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: i64| -> Result<i64, Trap> {
            b.with_mut(|t| fused_put(t, KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hput_u",
        (OpaqueBox<NativeTable>, u64) -> i64,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: u64| -> Result<i64, Trap> {
            b.with_mut(|t| fused_put(t, KeyVal::Bits(k)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hput_b",
        (OpaqueBox<NativeTable>, bool) -> i64,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: bool| -> Result<i64, Trap> {
            b.with_mut(|t| fused_put(t, KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hput_s",
        (OpaqueBox<NativeTable>, &str) -> i64,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &str| -> Result<i64, Trap> {
            b.with_mut(|t| fused_put_s(t, k))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hput_y",
        (OpaqueBox<NativeTable>, &[u8]) -> i64,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &[u8]| -> Result<i64, Trap> {
            b.with_mut(|t| fused_put(t, KeyVal::Bytes(k.to_vec())))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hput_sv",
        (OpaqueBox<NativeTable>, &str, i32, i32) -> i64,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, parent: &str, off: i32, len: i32| -> Result<i64, Trap> {
            b.with_mut(|t| fused_put_sv(t, parent, off, len))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hfind_i",
        (OpaqueBox<NativeTable>, i64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: i64| -> Result<i32, Trap> {
            b.with(|t| fused_find(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hfind_u",
        (OpaqueBox<NativeTable>, u64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: u64| -> Result<i32, Trap> {
            b.with(|t| fused_find(t, &KeyVal::Bits(k)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hfind_b",
        (OpaqueBox<NativeTable>, bool) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: bool| -> Result<i32, Trap> {
            b.with(|t| fused_find(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hfind_s",
        (OpaqueBox<NativeTable>, &str) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &str| -> Result<i32, Trap> {
            b.with(|t| fused_find_s(t, k))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hfind_y",
        (OpaqueBox<NativeTable>, &[u8]) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &[u8]| -> Result<i32, Trap> {
            b.with(|t| fused_find(t, &KeyVal::Bytes(k.to_vec())))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hfind_sv",
        (OpaqueBox<NativeTable>, &str, i32, i32) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, parent: &str, off: i32, len: i32| -> Result<i32, Trap> {
            b.with(|t| fused_find_sv(t, parent, off, len))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hremove_i",
        (OpaqueBox<NativeTable>, i64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: i64| -> Result<i32, Trap> {
            b.with_mut(|t| fused_remove(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hremove_u",
        (OpaqueBox<NativeTable>, u64) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: u64| -> Result<i32, Trap> {
            b.with_mut(|t| fused_remove(t, &KeyVal::Bits(k)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hremove_b",
        (OpaqueBox<NativeTable>, bool) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: bool| -> Result<i32, Trap> {
            b.with_mut(|t| fused_remove(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hremove_s",
        (OpaqueBox<NativeTable>, &str) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &str| -> Result<i32, Trap> {
            b.with_mut(|t| fused_remove_s(t, k))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hremove_y",
        (OpaqueBox<NativeTable>, &[u8]) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, k: &[u8]| -> Result<i32, Trap> {
            b.with_mut(|t| fused_remove(t, &KeyVal::Bytes(k.to_vec())))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hremove_sv",
        (OpaqueBox<NativeTable>, &str, i32, i32) -> i32,
        |_vm: &mut Vm, b: OpaqueBox<NativeTable>, parent: &str, off: i32, len: i32| -> Result<i32, Trap> {
            b.with_mut(|t| fused_remove_sv(t, parent, off, len))?
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
    fn the_load_factor_law_is_the_pinned_constants() {
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
    /// nmapset.rut's exact bits (the `mix64` / `fnv1a64` constants,
    /// originally ported bit-for-bit from the removed mapset pkg) — a
    /// one-ulp difference breaks the bench gate, which pins the
    /// value-derived checksums in `expected.json`.
    #[test]
    fn hash_payload_bits_are_pinned_bit_for_bit() {
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
    /// `hash_payload` with the wrapper's constants, so probing behaves
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

    /// The bool lane: `true`/`false` hash mix64(1)/mix64(0) — the
    /// wrapper's bool hash's exact bits — and round-trip through the
    /// typed lane while staying consistent with the Opaque lane's
    /// recorded hash.
    #[test]
    fn the_bool_lane_hashes_the_wrappers_bools() {
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

    // ---- the sv lanes (strings-round1, phase 1) ------------------------

    /// Hash-pin for the range hasher: `hash_bytes` is FNV-1a 64 with
    /// the ONE constants — bit-for-bit what `hash_payload` answers for
    /// the same octets (the Str/Bytes arms route through it), the empty
    /// range is the bare offset basis, and a carved window of a longer
    /// parent hashes exactly like the whole text.
    #[test]
    fn hash_bytes_is_bit_for_bit_the_payload_hasher() {
        // the s-lane pins from the hash test above, over raw ranges
        assert_eq!(hash_bytes(b"alpha"), 9999721509958787115);
        assert_eq!(hash_bytes(b"beta"), 8513880941419438247);
        assert_eq!(hash_bytes(b""), 14695981039346656037, "the bare offset basis");
        assert_eq!(hash_bytes(b"alpha"), hash_payload(&KeyVal::Str("alpha".into())));
        assert_eq!(hash_bytes(b"beta"), hash_payload(&KeyVal::Str("beta".into())));
        // multi-byte content: the range hash runs over the OCTETS
        let s = "héllo→";
        assert_eq!(hash_bytes(s.as_bytes()), hash_payload(&KeyVal::Str(s.into())));
        // the bytes lane rides the same hasher
        assert_eq!(hash_bytes(&[1, 2, 3]), hash_payload(&KeyVal::Bytes(vec![1, 2, 3])));
        // a carved window of a longer parent = the whole text's bits
        let parent = "alpha-beta";
        assert_eq!(hash_bytes(&parent.as_bytes()[0..5]), hash_bytes(b"alpha"));
        assert_eq!(
            hash_bytes(&parent.as_bytes()[6..10]),
            hash_payload(&KeyVal::Str("beta".into()))
        );
    }

    /// The sv range check: byte offsets over the parent's octets, both
    /// ends on UTF-8 codepoint boundaries; negative, out-of-range, and
    /// mid-codepoint windows are the house `Invalid` trap (named, never
    /// a silent mis-read). The empty range is legal at every boundary.
    #[test]
    fn sv_range_traps_mid_codepoint_and_out_of_range() {
        let ascii = "abcdef";
        assert_eq!(sv_range(ascii, 0, 6).unwrap(), "abcdef", "the full range");
        assert_eq!(sv_range(ascii, 2, 3).unwrap(), "cde", "a middle window");
        assert_eq!(sv_range(ascii, 6, 0).unwrap(), "", "the empty tail window");
        assert_eq!(sv_range(ascii, 0, 0).unwrap(), "", "the empty head window");

        // h(1) é(2) l(1) l(1) o(1) = 6 bytes; é spans bytes 1..3
        let utf8 = "héllo";
        assert_eq!(sv_range(utf8, 0, 3).unwrap(), "hé");
        assert_eq!(sv_range(utf8, 3, 3).unwrap(), "llo");
        assert_eq!(sv_range(utf8, 0, 6).unwrap(), utf8);
        // a window STARTING mid-codepoint traps
        let err = sv_range(utf8, 2, 2).unwrap_err();
        assert_eq!(err.kind, TrapKind::Invalid);
        assert!(err.msg.contains("not a UTF-8 boundary"), "{}", err.msg);
        assert!(err.msg.contains("offset 2"), "{}", err.msg);
        // a window ENDING mid-codepoint traps the same way
        let err = sv_range(utf8, 0, 2).unwrap_err();
        assert!(
            err.msg.contains("end offset 2") && err.msg.contains("not a UTF-8 boundary"),
            "{}",
            err.msg
        );

        // negative offsets and lengths
        for (off, len) in [(-1, 2), (2, -1), (i32::MIN, 0), (0, i32::MIN)] {
            let err = sv_range(utf8, off, len).unwrap_err();
            assert_eq!(err.kind, TrapKind::Invalid);
            assert!(err.msg.contains("out of range"), "{off}/{len}: {}", err.msg);
        }
        // past the end — usize math, so the i32 corners cannot overflow
        // the check
        let err = sv_range(utf8, 0, 7).unwrap_err();
        assert!(err.msg.contains("out of range"), "{}", err.msg);
        let err = sv_range(utf8, i32::MAX, 1).unwrap_err();
        assert!(err.msg.contains("out of range"), "{}", err.msg);
        let err = sv_range("", 0, 1).unwrap_err();
        assert!(err.msg.contains("out of range"), "{}", err.msg);
        // but the empty range on an empty parent is fine
        assert_eq!(sv_range("", 0, 0).unwrap(), "");
    }

    /// The parity law (strings-round1 phase 1): the sv lanes and the s
    /// lanes on the SAME content build the SAME table — same kind,
    /// same slot assignment (the recorded hashes are the pinned
    /// constants either way), same stored owned keys, same iteration
    /// order (slot order) — and every lane finds what any other lane
    /// stored, both directions, with replaces and removes crossing
    /// lanes too.
    #[test]
    fn sv_and_s_lanes_assign_identical_tables_both_directions() {
        // the sv side's parent; the windows are BYTE ranges carved out
        // of it, and the texts are what they carve (the test's ground
        // truth — checked below)
        let parent = "alpha-beta-gamma-delta";
        let windows: [(i32, i32); 8] = [
            (0, 5),  // alpha
            (6, 4),  // beta
            (11, 5), // gamma
            (17, 5), // delta
            (0, 4),  // alph — a prefix of alpha, distinct content
            (5, 5),  // -beta
            (10, 6), // -gamma
            (16, 6), // -delta
        ];
        let texts: [String; 8] = [
            "alpha".into(),
            "beta".into(),
            "gamma".into(),
            "delta".into(),
            "alph".into(),
            "-beta".into(),
            "-gamma".into(),
            "-delta".into(),
        ];
        for ((off, len), text) in windows.iter().zip(texts.iter()) {
            assert_eq!(&parent[*off as usize..(off + len) as usize], text);
        }

        // one op stream, three builds: all-sv, all-s, mixed lanes —
        // sentinel-driven grows, exactly the wrapper's put shape
        let build = |mode: u8| -> NativeTable {
            let mut t = NativeTable::new(4);
            for i in 0..8 {
                let put = |t: &mut NativeTable| {
                    match mode {
                        0 => typed_entry_sv(t, parent, windows[i].0, windows[i].1),
                        1 => typed_entry_s(t, &texts[i]),
                        _ => {
                            if i % 2 == 0 {
                                typed_entry_sv(t, parent, windows[i].0, windows[i].1)
                            } else {
                                typed_entry_s(t, &texts[i])
                            }
                        }
                    }
                    .unwrap()
                };
                let mut at = put(&mut t);
                while at == GROW_FIRST {
                    t.grow(); // no drain needed: no sidecar on this path
                    at = put(&mut t);
                }
                assert!(at < 0, "key {i} inserts fresh");
            }
            t
        };
        let a = build(0);
        let b = build(1);
        let c = build(2);
        assert_eq!(a.len(), 8);
        assert_eq!(b.len(), 8);
        assert_eq!(c.len(), 8);
        assert_eq!(a.kind, KeyKind::Str);
        // the three builds are THE SAME TABLE: same capacity after the
        // same grows, same state bytes, same recorded hashes, same
        // owned stored keys
        assert_eq!(a.cap, b.cap);
        assert_eq!(a.states, b.states);
        assert_eq!(a.hashes, b.hashes);
        assert_eq!(a.keys, b.keys, "stored keys are owned copies either way");
        assert_eq!(a.states, c.states);
        assert_eq!(a.hashes, c.hashes);
        assert_eq!(a.keys, c.keys, "the mixed-lane build is indistinguishable");
        // iteration order is slot order, and the slots agree
        let walk = |t: &NativeTable| -> Vec<(u64, String)> {
            (0..t.cap as usize)
                .filter(|&i| t.states[i] == 1)
                .map(|i| match &t.keys[i] {
                    KeyVal::Str(s) => (t.hashes[i], s.clone()),
                    other => panic!("str table holds {other:?}"),
                })
                .collect()
        };
        assert_eq!(walk(&a), walk(&b), "same slots, same order");
        assert_eq!(walk(&a), walk(&c));

        // every key findable by BOTH lanes on BOTH builds — sv-stored
        // probed by the owned lane, s-stored probed by the range lane
        for i in 0..8 {
            let (off, len) = windows[i];
            let on_a = typed_find_sv(&a, parent, off, len).unwrap();
            assert!(on_a >= 0, "key {i} lost on the sv build");
            assert_eq!(on_a, typed_find_s(&a, &texts[i]).unwrap());
            assert_eq!(on_a, typed_find_sv(&b, parent, off, len).unwrap());
            assert_eq!(on_a, typed_find_s(&b, &texts[i]).unwrap());
        }

        // replaces cross lanes: a put through the OPPOSITE lane answers
        // the found slot and stores no second entry
        let mut a = a;
        let alpha = typed_find_sv(&a, parent, 0, 5).unwrap();
        assert_eq!(typed_entry_s(&mut a, "alpha").unwrap(), alpha);
        assert_eq!(a.len(), 8);
        let beta = typed_find_s(&a, "beta").unwrap();
        assert_eq!(typed_entry_sv(&mut a, parent, 6, 4).unwrap(), beta);
        assert_eq!(a.len(), 8);

        // removes cross lanes: sv remove of the s-replaced key answers
        // its slot; the owned lane then misses too; the DEAD slot
        // re-lands the same key through the s lane
        assert_eq!(typed_remove_sv(&mut a, parent, 0, 5).unwrap(), alpha);
        assert_eq!(typed_remove_s(&mut a, "alpha").unwrap(), -1);
        assert_eq!(typed_find_sv(&a, parent, 0, 5).unwrap(), -1);
        assert_eq!(typed_find_s(&a, "alpha").unwrap(), -1);
        assert_eq!(a.len(), 7);
        let at = typed_entry_s(&mut a, "alpha").unwrap();
        assert_eq!(at, -(alpha + 1), "the DEAD slot wins the probe");
        assert_eq!(a.tomb, 0, "the DEAD slot left the tomb count");
    }

    /// The sv lanes admit like the s lanes: a Str table takes them, a
    /// Bits table traps mixed kinds BEFORE probing (never a silent
    /// absent), and a fresh table's first sv put fixes its kind.
    #[test]
    fn sv_lanes_admit_and_trap_like_the_s_lanes() {
        let mut t = NativeTable::new(8);
        t.entry(KeyKind::Bits, KeyVal::Bits(1), mix64(1)).unwrap();
        let err = typed_find_sv(&t, "ab", 0, 2).unwrap_err();
        assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
        let err = typed_entry_sv(&mut t, "ab", 0, 2).unwrap_err();
        assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
        let err = typed_remove_sv(&mut t, "ab", 0, 2).unwrap_err();
        assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
        let err = typed_entry_s(&mut t, "ab").unwrap_err();
        assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
        assert_eq!(t.len(), 1, "the rejected keys stored nothing");
        // a fresh table admits the sv lanes (Unset -> Str)
        let mut t = NativeTable::new(8);
        let at = typed_entry_sv(&mut t, "xy", 0, 2).unwrap();
        assert!(at < 0);
        assert_eq!(t.kind, KeyKind::Str);
        assert_eq!(t.keys[-(at + 1) as usize], KeyVal::Str("xy".into()));
    }

    /// Multi-grow sweep through the sv path, the k-mer shape: 150
    /// distinct 3-byte windows carved from ONE generated parent,
    /// sentinel-driven grows (cap 4 -> 256), remove/re-add churn across
    /// lanes — every key findable at the end, both lanes agreeing on
    /// every slot.
    #[test]
    fn sv_lanes_survive_multi_grow_sweeps() {
        // a marker parent: window i (bytes 3i..3i+3) spells
        // ('A'+i%26)('a'+i/26)('0'+i%10) — unique per i BY CONSTRUCTION
        // (the first two bytes determine i for i < 156), so no periodic
        // collision can sneak into the sweep
        let mut parent = String::new();
        for i in 0..150usize {
            parent.push((b'A' + (i % 26) as u8) as char);
            parent.push((b'a' + (i / 26) as u8) as char);
            parent.push((b'0' + (i % 10) as u8) as char);
        }
        let off_of = |i: usize| (3 * i) as i32;
        let text_of = |i: usize| parent[3 * i..3 * i + 3].to_string();

        let mut t = NativeTable::new(4);
        let mut distinct = std::collections::HashSet::new();
        for i in 0..150usize {
            let mut at = typed_entry_sv(&mut t, &parent, off_of(i), 3).unwrap();
            while at == GROW_FIRST {
                t.grow(); // no drain: no sidecar on this path
                at = typed_entry_sv(&mut t, &parent, off_of(i), 3).unwrap();
            }
            assert!(at < 0, "window {i} inserts fresh");
            distinct.insert(text_of(i));
        }
        assert_eq!(distinct.len(), 150, "the sweep's keys are distinct");
        assert_eq!(t.len(), 150);
        // the load law's exact stops: (count + 1)·10 >= cap·7 flips at
        // count 89 on cap 128 — so 150 puts land on cap 256
        assert_eq!(t.cap(), 256, "cap 4 grew to 256 across 150 sv puts");

        // churn: remove every 3rd key through the sv lane, re-add
        // through the s lane (mixed-lane churn over the same table)
        for i in (0..150).step_by(3) {
            assert!(typed_remove_sv(&mut t, &parent, off_of(i), 3).unwrap() >= 0);
        }
        assert_eq!(t.len(), 100);
        assert_eq!(t.tomb, 50);
        for i in (0..150).step_by(3) {
            let text = text_of(i);
            let mut at = typed_entry_s(&mut t, &text).unwrap();
            while at == GROW_FIRST {
                t.grow();
                at = typed_entry_s(&mut t, &text).unwrap();
            }
        }
        assert_eq!(t.len(), 150);
        assert_eq!(t.tomb, 0, "the re-adds reused every DEAD slot");
        // every key findable at the end, BOTH lanes agreeing per slot
        for i in 0..150usize {
            let via_sv = typed_find_sv(&t, &parent, off_of(i), 3).unwrap();
            assert!(via_sv >= 0, "key {i} lost");
            assert_eq!(via_sv, typed_find_s(&t, &text_of(i)).unwrap(), "key {i}");
        }
    }

    // ---- the fused handle lanes (nmapset-hostops) ---------------------

    #[test]
    fn the_packed_answer_law_is_bit0_newly_rest_handle() {
        // bit 0 = the newly bit, the rest the handle — distinct pairs
        // answer distinctly, and handle 0 rides both flags (the probe's
        // own PACK row, pinned)
        assert_eq!(pack_answer(0, false), 0);
        assert_eq!(pack_answer(0, true), 1);
        assert_eq!(pack_answer(1, false), 2);
        assert_eq!(pack_answer(1, true), 3);
        assert_eq!(pack_answer(1_000_000_000, true), 2_000_000_001);
        for hd in [0i32, 1, 7, 1 << 20] {
            for newly in [false, true] {
                let ans = pack_answer(hd, newly);
                assert_eq!((ans & 1) == 1, newly);
                assert_eq!((ans >> 1) as i32, hd);
            }
        }
        // collision-freedom over the whole realistic range: the map is
        // injective by construction (one bit vs the shifted rest)
        let mut seen = std::collections::HashSet::new();
        for hd in 0..2000i32 {
            assert!(seen.insert(pack_answer(hd, true)));
            assert!(seen.insert(pack_answer(hd, false)));
        }
    }

    #[test]
    fn hput_answers_handles_and_hfind_agrees() {
        let mut t = NativeTable::new(8);
        // three fresh births: handles 0, 1, 2 — each newly
        for i in 0..3u64 {
            let ans = fused_put(&mut t, bits(i)).unwrap();
            assert_eq!(ans, pack_answer(i as i32, true), "birth {i}");
        }
        // replace: the SAME handle, newly = false
        assert_eq!(
            fused_put(&mut t, bits(1)).unwrap(),
            pack_answer(1, false)
        );
        // hfind answers the handle; a miss is -1
        assert_eq!(fused_find(&t, &bits(0)).unwrap(), 0);
        assert_eq!(fused_find(&t, &bits(1)).unwrap(), 1);
        assert_eq!(fused_find(&t, &bits(99)).unwrap(), -1);
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn handles_survive_multi_grow_sweeps_untouched() {
        // the takeover's core law: the handle is the key's identity —
        // grow re-slots, the handle NEVER moves, and the wrapper's
        // sidecar (indexed by it) never drains
        let mut t = NativeTable::new(4);
        let mut handles = Vec::new();
        for i in 0..500u64 {
            let ans = fused_put(&mut t, bits(i.wrapping_mul(0x9E3779B1))).unwrap();
            assert_eq!(ans & 1, 1, "every key fresh");
            handles.push((ans >> 1) as i32);
        }
        assert!(t.cap() >= 1024, "the sweep grew the table internally");
        assert_eq!(t.len(), 500);
        for (i, hd) in handles.iter().enumerate() {
            let key = (i as u64).wrapping_mul(0x9E3779B1);
            assert_eq!(fused_find(&t, &bits(key)).unwrap(), *hd, "key {i}");
        }
        // handles are the dense births 0..500
        let mut sorted = handles.clone();
        sorted.sort();
        for (i, hd) in sorted.iter().enumerate() {
            assert_eq!(*hd, i as i32);
        }
    }

    #[test]
    fn hput_grows_internally_exactly_at_the_load_boundary() {
        // the legacy sentinel's boundary law, held by the fused lane
        // without ever exposing a sentinel: the (count+1)th entry that
        // would breach the load factor grows FIRST, stores, and answers
        // a normal packed pair
        let mut t = NativeTable::new(16);
        for i in 0..11u64 {
            // cap 16: (10+0+1)*10 = 110 < 112 — no grow yet, and the
            // fused put must NOT have grown early either
            fused_put(&mut t, bits(i)).unwrap();
            assert_eq!(t.cap(), 16, "no premature grow at {i}");
        }
        // the 11th live entry would breach: hput grows internally
        let ans = fused_put(&mut t, bits(11)).unwrap();
        assert_eq!(t.cap(), 32, "the boundary put grew the table");
        assert_eq!(ans & 1, 1);
        assert_eq!(t.len(), 12);
        assert_eq!(t.tomb, 0);
        // nothing was lost, handles intact
        for i in 0..12u64 {
            assert!(fused_find(&t, &bits(i)).unwrap() >= 0, "key {i}");
        }
    }

    #[test]
    fn hremove_answers_the_dead_handle_and_reinsert_is_a_new_birth() {
        let mut t = NativeTable::new(8);
        fused_put(&mut t, bits(7)).unwrap(); // handle 0
        fused_put(&mut t, bits(8)).unwrap(); // handle 1
        // remove answers the DEAD key's OWN handle — the wrapper nils
        // exactly that sidecar slot
        assert_eq!(fused_remove(&mut t, &bits(7)).unwrap(), 0);
        assert_eq!(fused_remove(&mut t, &bits(7)).unwrap(), -1);
        assert_eq!(fused_find(&t, &bits(7)).unwrap(), -1);
        // monotonic v1: the re-inserted key is a NEW birth (handle 2),
        // never the recycled 0 — the removed slot's sidecar nil stays
        // correct (slot 0 was nilled; the new birth writes slot 2)
        let ans = fused_put(&mut t, bits(7)).unwrap();
        assert_eq!(ans, pack_answer(2, true));
        // the DEAD slot reuse moved the key to a new slot with a fresh
        // handle, and the survivor is untouched
        assert_eq!(fused_find(&t, &bits(8)).unwrap(), 1);
    }

    #[test]
    fn the_sv_and_s_lanes_answer_one_handle() {
        // the parity law extended to identity: an sv key and the
        // equal-content str key are ONE key — same slot, same HANDLE —
        // through every fused op, both directions. The full answers
        // differ in the newly bit by design (the first put is a fresh
        // birth, the second finds it); the HANDLE must not.
        let mut t = NativeTable::new(8);
        let parent = "xx-alpha-xx-beta-xx";
        let via_sv = fused_put_sv(&mut t, parent, 3, 5).unwrap(); // "alpha"
        let via_s = fused_put_s(&mut t, "alpha").unwrap();
        assert_eq!(via_sv & 1, 1, "the sv put was the fresh birth");
        assert_eq!(via_s & 1, 0, "the s put found the sv key: a replace");
        assert_eq!(via_sv >> 1, via_s >> 1, "ONE key, ONE handle");
        assert_eq!(
            fused_find_s(&t, "alpha").unwrap(),
            fused_find_sv(&t, parent, 3, 5).unwrap()
        );
        assert_eq!(
            fused_remove_sv(&mut t, parent, 3, 5).unwrap(),
            (via_s >> 1) as i32
        );
        assert_eq!(fused_find_s(&t, "alpha").unwrap(), -1);
    }

    #[test]
    fn fused_lanes_agree_with_the_legacy_opaque_lane() {
        // same table, same slots: the legacy `entry` answer's slot
        // carries exactly the handle the fused lane packed (the
        // additive-state law — the old crossings never read the
        // column, the new ones never move a slot)
        let mut t = NativeTable::new(8);
        for i in 0..40u64 {
            let key = i.wrapping_mul(0x9E3779B1);
            let ans = fused_put(&mut t, bits(key)).unwrap();
            let slot = t.find(KeyKind::Bits, &bits(key), mix64(key)).unwrap();
            assert!(slot >= 0);
            assert_eq!(t.handles[slot as usize], (ans >> 1) as i32, "key {i}");
        }
    }

    #[test]
    fn bytes_and_bool_lanes_ride_the_fused_family() {
        // bytes on its own table (kinds are homogeneous per table)
        let mut t = NativeTable::new(8);
        assert_eq!(
            fused_put(&mut t, KeyVal::Bytes(vec![1, 2, 3])).unwrap(),
            pack_answer(0, true)
        );
        assert_eq!(fused_find(&t, &KeyVal::Bytes(vec![1, 2, 3])).unwrap(), 0);
        assert_eq!(
            fused_put(&mut t, KeyVal::Bytes(vec![1, 2, 3])).unwrap(),
            pack_answer(0, false)
        );
        assert_eq!(fused_remove(&mut t, &KeyVal::Bytes(vec![1, 2, 3])).unwrap(), 0);
        // bool on its own table — `true` is bits 1 on the b lane
        let mut b = NativeTable::new(8);
        assert_eq!(
            fused_put(&mut b, bits(1)).unwrap(),
            pack_answer(0, true),
            "bool true = bits 1"
        );
        assert_eq!(fused_find(&b, &bits(1)).unwrap(), 0);
        assert_eq!(fused_remove(&mut b, &bits(1)).unwrap(), 0);
        // kind mixing still traps through the fused path
        assert!(fused_put(&mut t, KeyVal::Str("s".into())).is_err());
    }
}
