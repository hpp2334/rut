//! `nmap_host` — the native key-table experiment's HOST half: a real
//! `HashMap` whose entries live Rust-side behind an `opaque` payload
//! (RFC 0023), so rut meets it only through the `pub host fn` surface
//! bound by [`install_std_nmap`]. (The pure-rut `mapset` package — the
//! original reference and general-key implementation — was REMOVED from
//! the tree in Sep 2026, stdlib slim-down: general keys now mean a
//! `bytes` encoding or the `nmapset` wrapper; the old source lives in
//! git history.)
//!
//! Design (nmap-hostvals P4 — the real-HashMap core):
//! - the table IS `HashMap<KeyVal, Entry, MapBuild>`: the open-
//!   addressing machinery (the recorded-hash vecs, the state bytes and
//!   their tombstones, the load-factor law, `grow` and its relocation
//!   queue, the parallel handles vec) is DELETED — std owns probing,
//!   growth, and rehash now. `KeyVal`'s `Hash` writes ONE u64 —
//!   [`hash_payload`], the pinned mapset bits (mix64 for the integer/
//!   bool lanes, FNV-1a 64 over the octets for `str`/`bytes`) — through
//!   [`IdHasher`], the identity hasher: the map never re-hashes, it
//!   only buckets on the same words the wrapper's `k.hash()` produces.
//!   `Eq` is content equality (bits / octets), so an sv key and the
//!   equal-content `str` key are ONE key with ONE entry;
//! - the key set is CLOSED (i8..i64, u8..u64, bool, str, bytes), fixed
//!   by the table's first insert and admitted thereafter (the `kind`
//!   field stays as the admission trap — a mismatch is a host/wrapper
//!   bug and traps, never a silent absent). Anything else — floats,
//!   records — cannot reach the table: the h-family's key crossings
//!   are typed, and the wrapper's union bound refuses user key types
//!   at compile time;
//! - the fused h-family is the ONE op family: `hput` is the entry API
//!   (Occupied → the same handle, `newly = false`; Vacant → a fresh
//!   monotonic birth, `newly = true`), `hfind`/`hremove` answer the
//!   key's handle or `-1`. Handles are BIRTH indices — monotonic,
//!   never recycled, immune to growth (std's rehash moves nothing the
//!   handle can see). The packed `(handle << 1) | newly` i64 answer is
//!   unchanged, bit for bit;
//! - values: `Entry { handle, val: ValSlot }` — every birth holds the
//!   [`ValSlot::Empty`] placeholder until a value lane fills it (P5
//!   moves the wrapper's values INTO these entries; today the raw
//!   `map_val_{set,get}_{u,f}` lanes address them by handle, exact-arm
//!   matched: bits answer bits, `Empty`/`Ref` trap, never reinterpret).
//!
//! Borrowed keys: the old probing core compared `str` keys OVER the
//! crossed octets (zero-copy until a fresh insert). std's `HashMap`
//! has no borrowed probe, so the s/sv lanes materialize one owned
//! `KeyVal::Str` per crossing — the copy moved from insert-only to
//! per-crossing, the one measurable price of the real-HashMap
//! directive, disclosed in the movers (hashing still runs over the
//! octets via [`hash_payload`]; nothing else changed).

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hash, Hasher};

use rut_vm::interp::{HostRegistry, Vm};
use rut_vm::{HostPayload, Opaque, OpaqueRef, Slot, Trap, TrapKind, ValSlot};

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

/// A stored key: the closed native set as owned Rust data. `Eq` is
/// content equality (the derive) — a key's identity in the map is WHAT
/// IT SAYS, never where it lives. `Hash` is custom (below): ONE pre-
/// hashed word, the pinned mapset bits.
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

impl Hash for KeyVal {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(hash_payload(self)); // ONE u64 — the pinned mapset bits
    }
}

/// The identity hasher (the plan's P4 code block): the map buckets on
/// the word [`KeyVal`]'s `Hash` writes — no second hashing pass, ever.
/// The bits ARE the pinned mapset constants, so a host-side bucket
/// assignment and the wrapper's own `k.hash()` agree bit for bit.
#[derive(Default)]
pub struct IdHasher(u64);

impl Hasher for IdHasher {
    fn write_u64(&mut self, v: u64) {
        self.0 = v;
    }
    fn write(&mut self, _: &[u8]) {}
    fn finish(&self) -> u64 {
        self.0
    }
}

/// The map's build hasher.
pub type MapBuild = BuildHasherDefault<IdHasher>;

/// One entry's value storage: a birth holds [`ValSlot::Empty`] until a
/// value lane fills it; the raw lanes store [`ValSlot::Bits`] (the
/// untagged 8-byte slot moves as-is); P5's value migration adds
/// `Ref` (a held rut cell). The tag is the exact-arm-match law: a
/// valued read TRAPS on `Empty`/`Ref`, never reinterprets.
pub(crate) struct Entry {
    /// the key's stable birth index — monotonic, never recycled
    pub(crate) handle: i32,
    pub(crate) val: ValSlot,
}

/// The native table: a real `HashMap` plus the admission kind and the
/// birth counter. Pure Rust data — no rut cells inside — so the
/// payload's `Drop` (which frees every key) runs deterministically when
/// the box's rc hits 0 (P5 fills the finalize hook when values move in).
pub struct NativeTable {
    kind: KeyKind,
    map: HashMap<KeyVal, Entry, MapBuild>,
    /// the next birth index — the table's lifetime insertion count,
    /// bounded by BIRTHS not capacity (the packed i64 answer's headroom
    /// law: the handle rides i32 with room far past any real table)
    next_handle: i32,
}

impl NativeTable {
    /// `map_new`'s `cap` is HashMap's reserve: a lower bound on the
    /// bucket capacity, honored by `with_capacity` (the power-of-two
    /// ≥ 4 rounding law died with the probing core). `cap` crosses as
    /// the `i64` the wrapper passes.
    pub fn new(cap: i64) -> NativeTable {
        let cap = cap.clamp(0, i32::MAX as i64) as usize;
        NativeTable {
            kind: KeyKind::Unset,
            map: HashMap::with_capacity_and_hasher(cap, MapBuild::default()),
            next_handle: 0,
        }
    }

    /// The fused put-path crossing: the entry API. `Occupied` → the
    /// same handle, `newly = false` (a replace stores nothing new —
    /// the value lanes overwrite in place); `Vacant` → a fresh
    /// monotonic birth with the [`ValSlot::Empty`] placeholder,
    /// `newly = true`. The packed answer is bit-for-bit the hostops
    /// lane's `(handle << 1) | newly`.
    pub fn hput(&mut self, kind: KeyKind, key: KeyVal) -> Result<i64, Trap> {
        self.admit(kind)?;
        Ok(match self.map.entry(key) {
            std::collections::hash_map::Entry::Occupied(e) => pack_answer(e.get().handle, false),
            std::collections::hash_map::Entry::Vacant(v) => {
                if self.kind == KeyKind::Unset {
                    self.kind = kind; // the first insert fixes the flavor
                }
                let handle = self.next_handle;
                self.next_handle += 1;
                v.insert(Entry { handle, val: ValSlot::Empty });
                pack_answer(handle, true)
            }
        })
    }

    /// The fused lookup crossing: the key's handle, or `-1` — the
    /// wrapper answers `nil` / `false` from the sign.
    pub fn hfind(&self, kind: KeyKind, key: &KeyVal) -> Result<i32, Trap> {
        self.admit(kind)?;
        Ok(self.map.get(key).map(|e| e.handle).unwrap_or(-1))
    }

    /// The fused remove crossing: the entry leaves the map (the owned
    /// key drops with the store) and its DEAD handle answers — the
    /// wrapper nils exactly that sidecar slot — or `-1` when absent.
    /// The handle is never recycled: a re-insert is a NEW birth.
    pub fn hremove(&mut self, kind: KeyKind, key: &KeyVal) -> Result<i32, Trap> {
        self.admit(kind)?;
        Ok(self.map.remove(key).map(|e| e.handle).unwrap_or(-1))
    }

    /// The handle-rebased val write: store `raw` as [`ValSlot::Bits`]
    /// at the live key's `handle` (the h-family's birth — never a
    /// slot; slots died with the probing core). Bounds are the live
    /// births; a removed key's handle is in range but has no live
    /// entry, and THAT traps by name.
    pub fn val_set_u(&mut self, handle: i32, raw: u64) -> Result<(), Trap> {
        self.entry_by_handle_mut(handle)?.val = ValSlot::Bits(Slot::int(raw as i64));
        Ok(())
    }

    /// The val read, u lane: the stored bits iff the entry's tag is
    /// [`ValSlot::Bits`] — the `Empty` placeholder (an hput birth) and
    /// a `Ref` cell TRAP, never reinterpret (§0.8 c/g). The u lane is
    /// the raw word in both directions (the `u64` boundary reads/
    /// writes the slot's bit pattern — verified in the phase-2 driver
    /// tests), so payload bits cross sign-free.
    pub fn val_get_u(&self, handle: i32) -> Result<u64, Trap> {
        match &self.entry_by_handle(handle)?.val {
            ValSlot::Bits(s) => Ok(unsafe { s.i } as u64),
            _ => Err(Trap::new(
                TrapKind::Invalid,
                format!(
                    "nmap: val handle {handle} holds no bits (the h-family placeholder or a reference cell)"
                ),
            )),
        }
    }

    /// The val write, f lane: the same Bits storage through an
    /// `f64`-typed crossing — `f64` rides the slot word (§0.8 f), so
    /// this is `to_bits` at the boundary and nothing else.
    pub fn val_set_f(&mut self, handle: i32, v: f64) -> Result<(), Trap> {
        self.entry_by_handle_mut(handle)?.val = ValSlot::Bits(Slot::float(v));
        Ok(())
    }

    /// The val read, f lane: `from_bits` over the stored word — the
    /// old column's exact shape, re-based (the wrapper cannot spell
    /// the float reinterpret itself, RFC 0007 §1).
    pub fn val_get_f(&self, handle: i32) -> Result<f64, Trap> {
        match &self.entry_by_handle(handle)?.val {
            ValSlot::Bits(s) => Ok(f64::from_bits(unsafe { s.i } as u64)),
            _ => Err(Trap::new(
                TrapKind::Invalid,
                format!(
                    "nmap: val handle {handle} holds no bits (the h-family placeholder or a reference cell)"
                ),
            )),
        }
    }

    /// The live entry at a caller-supplied handle — the raw val lanes'
    /// address check. `handle` must be a live birth: negative or
    /// `>= births` is the house out-of-range trap; a birth whose key
    /// was removed (the handle stays inside the births range forever —
    /// monotonic, never recycled) traps `no live entry` by name.
    fn entry_by_handle(&self, handle: i32) -> Result<&Entry, Trap> {
        self.check_birth(handle)?;
        self.map
            .values()
            .find(|e| e.handle == handle)
            .ok_or_else(|| dead_handle_trap(handle, self.next_handle))
    }

    fn entry_by_handle_mut(&mut self, handle: i32) -> Result<&mut Entry, Trap> {
        self.check_birth(handle)?;
        // the births count is read BEFORE the mutable map walk — the
        // miss trap must not reach back through `&mut self`
        let births = self.next_handle;
        self.map
            .values_mut()
            .find(|e| e.handle == handle)
            .ok_or_else(|| dead_handle_trap(handle, births))
    }

    fn check_birth(&self, handle: i32) -> Result<(), Trap> {
        if handle < 0 || handle >= self.next_handle {
            return Err(Trap::new(
                TrapKind::Invalid,
                format!(
                    "nmap: val handle {handle} out of range (births {})",
                    self.next_handle
                ),
            ));
        }
        Ok(())
    }

    /// The capacity, at the i32 boundary (`len()` and indexing are
    /// i32) — HashMap's reserve (the wrapper's sidecar pre-size reads
    /// it; a hint, never a behavior). Retires with `with_capacity`'s
    /// pre-size read at P5.
    pub fn cap(&self) -> i32 {
        self.map.capacity().min(i32::MAX as usize) as i32
    }

    /// The live entry count.
    pub fn len(&self) -> i32 {
        self.map.len() as i32
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

/// The table is a HostOpaque payload (nmap-hostvals P2): the store
/// entry's release hook is available to it, and at THIS phase the
/// no-op default is the whole law — the table is pure Rust data with no
/// rut cells inside, so its `Drop` (frees every key and entry) at
/// entry death is behaviorally identical to the old box's. P5 fills
/// `finalize` (release every held value cell) when the values move
/// into the entries.
impl HostPayload for NativeTable {}

/// The removed-key handle's trap: the handle stays inside the births
/// range forever (monotonic, never recycled) but its entry is gone.
fn dead_handle_trap(handle: i32, births: i32) -> Trap {
    Trap::new(
        TrapKind::Invalid,
        format!(
            "nmap: val handle {handle} has no live entry (the key was removed; births {births})"
        ),
    )
}

/// The two hash constants the removed pure-rut `mapset` pkg's
/// `Hashable` impls ran (the ONE key contract, now nmapset.rut's
/// `mix64`/`fnv1a64` alone): FNV-1a 64's offset basis
/// and prime. These are the checksum law — ported BIT-FOR-BIT, so the
/// map's bucket hashing equals the wrapper's `k.hash()` bit for bit
/// and the bench checksums pinned in `expected.json` hold. A one-ulp
/// difference here breaks the gate; the unit test pins
/// the exact outputs as literals.
const FNV_OFFSET: u64 = 14695981039346656037;
const FNV_PRIME: u64 = 1099511628211;

/// FNV-1a 64 over an octet range — the ONE str/bytes hasher, shared by
/// every lane that hashes octets. The constants are the checksum law
/// (see [`FNV_OFFSET`]); the s/sv lanes hash OVER the borrowed range
/// and it answers the same bits [`hash_payload`] answers for the same
/// octets, by construction.
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    for b in bytes {
        h = (h ^ *b as u64).wrapping_mul(FNV_PRIME);
    }
    h
}

/// The ONE shared payload hasher — the word [`KeyVal`]'s `Hash` writes
/// into the map: mix64 for the integer/bool bits, FNV-1a 64 over the
/// octets for `str`/`bytes`. Same inputs, nmapset.rut's constants,
/// bit-exact.
pub fn hash_payload(k: &KeyVal) -> u64 {
    match k {
        KeyVal::Bits(v) => (FNV_OFFSET ^ v).wrapping_mul(FNV_PRIME),
        KeyVal::Str(s) => hash_bytes(s.as_bytes()),
        KeyVal::Bytes(b) => hash_bytes(b),
    }
}

/// The payload's kind — the table's fixed flavor. All integer
/// primitives and `bool` share `Bits` (equality is the payload bits:
/// `255u8`, `255i64`, and `255i32` collapse to one key, exactly as the
/// wrapper's homogeneous `K` guarantees), `str`/`bytes` are their own
/// kinds.
fn kind_of(key: &KeyVal) -> KeyKind {
    match key {
        KeyVal::Bits(_) => KeyKind::Bits,
        KeyVal::Str(_) => KeyKind::Str,
        KeyVal::Bytes(_) => KeyKind::Bytes,
    }
}

/// The sv lanes' range check (strings-round1 phase 1): `off`/`len` are
/// BYTE offsets into the parent's octets — `0 <= off`, `0 <= len`,
/// `off + len <= parent.len()` (usize math, so the i32 corners cannot
/// overflow the check), and BOTH ends must sit on UTF-8 codepoint
/// boundaries — a mid-codepoint window is a caller bug, trapped with
/// the house `Invalid` shape, never a silent mis-read.
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

// ---- the fused handle lanes (nmapset-hostops, re-mounted at P4) -----
// ONE crossing per op with ALL control flow host-side. `hput` inserts
// or replaces via the entry API and answers the packed
// `(handle << 1) | newly` i64; `hfind`/`hremove` answer the key's
// STABLE birth handle or `-1`. The wrapper indexes its `[?V]` sidecar
// by the handle — std's growth rehashes nothing the handle can see, so
// the wrapper never drains and never reallocates on grow. The answers:
// - `map_hput_*`: `(handle << 1) | newly` as i64 — bit 0 is the newly
//   bit (`(ans & 1) == 1` — HashSet shares this lane and reads exactly
//   that bit), the rest the handle. Handles are bounded by BIRTHS, not
//   capacity, so the i64 carrier has headroom far past any real table.
// - `map_hfind_*` / `map_hremove_*`: the handle, or `-1` (absent).
//   `hremove` answers the dead key's own handle so the wrapper can nil
//   exactly that sidecar slot and release the cell.
//
// The borrowed s/sv lanes materialize one owned `KeyVal::Str` per
// crossing (std has no borrowed probe — see the module header); the
// hashing still runs over the octets, and an sv key and the
// equal-content `str` key are ONE key: same entry, same HANDLE.

/// The packed `hput` answer: bit 0 = the newly bit, the rest the handle.
#[inline]
fn pack_answer(hd: i32, newly: bool) -> i64 {
    ((hd as i64) << 1) | (newly as i64)
}

/// The i/u/b/y lanes' fused put: hash and insert/replace via the entry
/// API — growth is std's, internal, and invisible to the answer.
fn fused_put(t: &mut NativeTable, key: KeyVal) -> Result<i64, Trap> {
    t.hput(kind_of(&key), key)
}

/// The i/u/b/y lanes' fused find: the handle or `-1`.
fn fused_find(t: &NativeTable, key: &KeyVal) -> Result<i32, Trap> {
    t.hfind(kind_of(key), key)
}

/// The i/u/b/y lanes' fused remove: the dead key's handle or `-1`.
fn fused_remove(t: &mut NativeTable, key: &KeyVal) -> Result<i32, Trap> {
    t.hremove(kind_of(key), key)
}

/// The s lane's fused put: one owned `KeyVal::Str` per crossing (std
/// has no borrowed probe), the entry API on top of it.
fn fused_put_s(t: &mut NativeTable, k: &str) -> Result<i64, Trap> {
    t.hput(KeyKind::Str, KeyVal::Str(k.to_owned()))
}

/// The s lane's fused find.
fn fused_find_s(t: &NativeTable, k: &str) -> Result<i32, Trap> {
    t.hfind(KeyKind::Str, &KeyVal::Str(k.to_owned()))
}

/// The s lane's fused remove.
fn fused_remove_s(t: &mut NativeTable, k: &str) -> Result<i32, Trap> {
    t.hremove(KeyKind::Str, &KeyVal::Str(k.to_owned()))
}

/// The sv lane's fused put: validate the byte window (the house
/// `Invalid` trap on a bad range — the table is left intact), then the
/// s lane. Same entry and same HANDLE as the s lane on the same
/// content (the parity law) — an sv key and the equal-content `str`
/// key are one key with one identity.
fn fused_put_sv(t: &mut NativeTable, parent: &str, off: i32, len: i32) -> Result<i64, Trap> {
    let range = sv_range(parent, off, len)?;
    fused_put_s(t, range)
}

/// The sv lane's fused find.
fn fused_find_sv(t: &NativeTable, parent: &str, off: i32, len: i32) -> Result<i32, Trap> {
    let range = sv_range(parent, off, len)?;
    fused_find_s(t, range)
}

/// The sv lane's fused remove.
fn fused_remove_sv(t: &mut NativeTable, parent: &str, off: i32, len: i32) -> Result<i32, Trap> {
    let range = sv_range(parent, off, len)?;
    fused_remove_s(t, range)
}

/// Install the `nmap_host` bodies under the `nmap_host` scope (the `calc`
/// pattern, RFC 0023/0025): the callable's Rust shape IS the `.d.rut`
/// row, so the surface declares exactly these signatures. `map_new`'s
/// box carries the table; every other fn's first param borrows it typed
/// (`Opaque<NativeTable>` — a wrong payload is a checked trap).
pub fn install_std_nmap(hosts: &mut HostRegistry) {
    rut_vm::register!(hosts, "nmap_host::map_new", (i64,) -> OpaqueRef, |vm: &mut Vm, cap: i64| {
        let b = Opaque::alloc_hosted(vm, NativeTable::new(cap))?;
        Ok(b.handle().clone())
    });
    rut_vm::register!(
        hosts,
        "nmap_host::map_len",
        (Opaque<NativeTable>,) -> i32,
        |_vm: &mut Vm, b: Opaque<NativeTable>| b.with(|t| t.len()),
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_cap",
        (Opaque<NativeTable>,) -> i32,
        |_vm: &mut Vm, b: Opaque<NativeTable>| b.with(|t| t.cap()),
    );

    // ---- the handle-rebased raw val lanes (nmap-hostvals P4) --------
    // Four crossings, raw bits in/out, ONE storage serving every
    // primitive val kind (i64/u64/f64/bool — the monomorphized wrapper
    // reinterprets client-side; no lane explosion). The address is a
    // live birth HANDLE from the h-family — the slot addressing died
    // with the probing core. `map_val_set_u` stores the word;
    // `map_val_get_u` answers the stored bits iff the entry's tag is
    // Bits — the Empty placeholder traps (an hput birth holds no
    // value; a valued read is a caller bug, §0.8 g). The f pair is the
    // same storage through `to_bits`/`from_bits` at the boundary
    // (rut has no float bitcast, RFC 0007 §1). Bounds are the live
    // births; a removed key's handle traps `no live entry`.
    rut_vm::register!(
        hosts,
        "nmap_host::map_val_set_u",
        (Opaque<NativeTable>, i32, u64) -> (),
        |vm: &mut Vm, b: Opaque<NativeTable>, handle: i32, raw: u64| -> Result<(), Trap> {
            b.with_mut(vm, |_vm, t| t.val_set_u(handle, raw))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_val_get_u",
        (Opaque<NativeTable>, i32) -> u64,
        |_vm: &mut Vm, b: Opaque<NativeTable>, handle: i32| -> Result<u64, Trap> {
            b.with(|t| t.val_get_u(handle))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_val_set_f",
        (Opaque<NativeTable>, i32, f64) -> (),
        |vm: &mut Vm, b: Opaque<NativeTable>, handle: i32, v: f64| -> Result<(), Trap> {
            b.with_mut(vm, |_vm, t| t.val_set_f(handle, v))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_val_get_f",
        (Opaque<NativeTable>, i32) -> f64,
        |_vm: &mut Vm, b: Opaque<NativeTable>, handle: i32| -> Result<f64, Trap> {
            b.with(|t| t.val_get_f(handle))?
        },
    );

    // ---- the fused handle lanes (nmapset-hostops) ----------------------
    // 18 fns `map_h{put,find,remove}_{i,u,b,s,y,sv}`: the takeover
    // surface. The key crosses DIRECTLY on the typed lanes; the answer
    // is the packed `(handle << 1) | newly` i64 (`hput`) or the key's
    // STABLE birth handle (or `-1`), which the wrapper's `[?V]` sidecar
    // is indexed by. `HashSet` shares `hput` and reads bit 0. Growth is
    // std's — internal, and invisible to every answer (the handle is a
    // birth, not an address).
    rut_vm::register!(
        hosts,
        "nmap_host::map_hput_i",
        (Opaque<NativeTable>, i64) -> i64,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: i64| -> Result<i64, Trap> {
            b.with_mut(vm, |_vm, t| fused_put(t, KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hput_u",
        (Opaque<NativeTable>, u64) -> i64,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: u64| -> Result<i64, Trap> {
            b.with_mut(vm, |_vm, t| fused_put(t, KeyVal::Bits(k)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hput_b",
        (Opaque<NativeTable>, bool) -> i64,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: bool| -> Result<i64, Trap> {
            b.with_mut(vm, |_vm, t| fused_put(t, KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hput_s",
        (Opaque<NativeTable>, &str) -> i64,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: &str| -> Result<i64, Trap> {
            b.with_mut(vm, |_vm, t| fused_put_s(t, k))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hput_y",
        (Opaque<NativeTable>, &[u8]) -> i64,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: &[u8]| -> Result<i64, Trap> {
            b.with_mut(vm, |_vm, t| fused_put(t, KeyVal::Bytes(k.to_vec())))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hput_sv",
        (Opaque<NativeTable>, &str, i32, i32) -> i64,
        |vm: &mut Vm, b: Opaque<NativeTable>, parent: &str, off: i32, len: i32| -> Result<i64, Trap> {
            b.with_mut(vm, |_vm, t| fused_put_sv(t, parent, off, len))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hfind_i",
        (Opaque<NativeTable>, i64) -> i32,
        |_vm: &mut Vm, b: Opaque<NativeTable>, k: i64| -> Result<i32, Trap> {
            b.with(|t| fused_find(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hfind_u",
        (Opaque<NativeTable>, u64) -> i32,
        |_vm: &mut Vm, b: Opaque<NativeTable>, k: u64| -> Result<i32, Trap> {
            b.with(|t| fused_find(t, &KeyVal::Bits(k)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hfind_b",
        (Opaque<NativeTable>, bool) -> i32,
        |_vm: &mut Vm, b: Opaque<NativeTable>, k: bool| -> Result<i32, Trap> {
            b.with(|t| fused_find(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hfind_s",
        (Opaque<NativeTable>, &str) -> i32,
        |_vm: &mut Vm, b: Opaque<NativeTable>, k: &str| -> Result<i32, Trap> {
            b.with(|t| fused_find_s(t, k))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hfind_y",
        (Opaque<NativeTable>, &[u8]) -> i32,
        |_vm: &mut Vm, b: Opaque<NativeTable>, k: &[u8]| -> Result<i32, Trap> {
            b.with(|t| fused_find(t, &KeyVal::Bytes(k.to_vec())))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hfind_sv",
        (Opaque<NativeTable>, &str, i32, i32) -> i32,
        |_vm: &mut Vm, b: Opaque<NativeTable>, parent: &str, off: i32, len: i32| -> Result<i32, Trap> {
            b.with(|t| fused_find_sv(t, parent, off, len))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hremove_i",
        (Opaque<NativeTable>, i64) -> i32,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: i64| -> Result<i32, Trap> {
            b.with_mut(vm, |_vm, t| fused_remove(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hremove_u",
        (Opaque<NativeTable>, u64) -> i32,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: u64| -> Result<i32, Trap> {
            b.with_mut(vm, |_vm, t| fused_remove(t, &KeyVal::Bits(k)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hremove_b",
        (Opaque<NativeTable>, bool) -> i32,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: bool| -> Result<i32, Trap> {
            b.with_mut(vm, |_vm, t| fused_remove(t, &KeyVal::Bits(k as u64)))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hremove_s",
        (Opaque<NativeTable>, &str) -> i32,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: &str| -> Result<i32, Trap> {
            b.with_mut(vm, |_vm, t| fused_remove_s(t, k))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hremove_y",
        (Opaque<NativeTable>, &[u8]) -> i32,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: &[u8]| -> Result<i32, Trap> {
            b.with_mut(vm, |_vm, t| fused_remove(t, &KeyVal::Bytes(k.to_vec())))?
        },
    );
    rut_vm::register!(
        hosts,
        "nmap_host::map_hremove_sv",
        (Opaque<NativeTable>, &str, i32, i32) -> i32,
        |vm: &mut Vm, b: Opaque<NativeTable>, parent: &str, off: i32, len: i32| -> Result<i32, Trap> {
            b.with_mut(vm, |_vm, t| fused_remove_sv(t, parent, off, len))?
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// mapset.rut's mix64 — the pinned integer/bool hash, mirrored here
    /// so the hash-law tests can name the exact bits.
    fn mix64(bits: u64) -> u64 {
        (14695981039346656037u64 ^ bits).wrapping_mul(1099511628211u64)
    }

    fn bits(v: u64) -> KeyVal {
        KeyVal::Bits(v)
    }

    /// The shallow-size tripwire (the refcolumn batch's precedent):
    /// `Opaque::alloc` charges this at every `map_new`, so the heap
    /// movers' arithmetic pins on it. The HashMap core made the table
    /// SMALLER — the parallel key/hash/state/val/handle vecs are gone.
    #[test]
    fn native_table_shallow_size_is_pinned() {
        assert_eq!(std::mem::size_of::<NativeTable>(), 40);
    }

    #[test]
    fn the_capacity_law_is_hashmaps_reserve_semantics() {
        // the open-addressing power-of-two law is gone with the probing
        // core; `map_new`'s `cap` is HashMap's reserve — a lower bound
        // on capacity, which `map_cap` reads back for the wrapper's
        // sidecar pre-size (a hint, never a behavior)
        assert_eq!(NativeTable::new(0).cap(), 0);
        assert_eq!(NativeTable::new(-5).cap(), 0);
        assert!(NativeTable::new(4).cap() >= 4);
        assert!(NativeTable::new(1000).cap() >= 1000);
    }

    #[test]
    fn key_kinds_admit_but_never_mix() {
        let mut t = NativeTable::new(8);
        assert_eq!(fused_put(&mut t, bits(1)).unwrap(), pack_answer(0, true));
        // the same kind admits; a different one is a trap, not a miss
        assert_eq!(fused_put(&mut t, bits(2)).unwrap(), pack_answer(1, true));
        let err = fused_put(&mut t, KeyVal::Str("x".into())).unwrap_err();
        assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
        let err = fused_find(&t, &KeyVal::Str("x".into())).unwrap_err();
        assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
        let err = fused_remove(&mut t, &KeyVal::Str("x".into())).unwrap_err();
        assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
        assert_eq!(t.len(), 2, "the rejected keys stored nothing");
        assert_eq!(t.kind, KeyKind::Bits);
    }

    #[test]
    fn str_key_equality_is_content_equality() {
        // one table, one key kind — homogeneity is the contract
        let mut t = NativeTable::new(8);
        let a = fused_put_s(&mut t, "alpha").unwrap();
        assert_eq!(a & 1, 1, "fresh");
        let alpha = (a >> 1) as i32;
        // a fresh copy of the same octets finds the same entry
        assert_eq!(fused_find_s(&t, "alpha").unwrap(), alpha);
        assert_eq!(fused_find_s(&t, "alph").unwrap(), -1);
        // a replace answers the same handle, not newly
        assert_eq!(fused_put_s(&mut t, "alpha").unwrap(), pack_answer(alpha, false));
        assert_eq!(t.len(), 1);
        assert_eq!(fused_remove_s(&mut t, "alpha").unwrap(), alpha);
        assert_eq!(fused_find_s(&t, "alpha").unwrap(), -1);

        let mut t = NativeTable::new(8);
        let b = fused_put(&mut t, KeyVal::Bytes(vec![9, 8, 7])).unwrap();
        assert_eq!(b & 1, 1, "fresh");
        let hd = (b >> 1) as i32;
        assert_eq!(fused_find(&t, &KeyVal::Bytes(vec![9, 8, 7])).unwrap(), hd);
        assert_eq!(fused_find(&t, &KeyVal::Bytes(vec![9, 8])).unwrap(), -1);
    }

    // ---- the hash law (the checksum constants — unchanged) ------------

    /// The checksum law, pinned as LITERALS: hash_payload must answer
    /// nmapset.rut's exact bits (the `mix64` / `fnv1a64` constants,
    /// originally ported bit-for-bit from the removed mapset pkg) — a
    /// one-ulp difference breaks the bench gate, which pins the
    /// value-derived checksums in `expected.json`. These are also the
    /// words `KeyVal`'s `Hash` writes into the map.
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

    /// Widening distinctness: `-1i8`, `255u8`, and `255i64` are three
    /// keys the closed set can carry, and the map must see what the
    /// crossing bits law sees — signed widths cross SIGN-extended
    /// (`-1i8` hashes as bits `0xFFFF..FF`), unsigned widths
    /// ZERO-extend, and equal bits collapse to one key (`255u8` ==
    /// `255i64`, the homogeneous-K contract).
    #[test]
    fn widening_distinctness_matches_the_bits_law() {
        let mut t = NativeTable::new(8);
        // -1i8 crosses as i64 -1: bits 0xFFFF..FFFF
        let neg = fused_put(&mut t, bits((-1i64) as u64)).unwrap();
        assert_eq!(neg & 1, 1, "fresh");
        // 255u8 and 255i64 are the SAME key (bits 255) — the i/u lanes
        // share the Bits kind, exactly as the crossing law does
        let u8_255 = fused_put(&mut t, bits(255)).unwrap();
        assert_eq!(u8_255 & 1, 1, "255 is a fresh key");
        assert_eq!(
            fused_put(&mut t, bits(255i64 as u64)).unwrap(),
            u8_255 & !1,
            "same handle, newly = false — equal bits collapse"
        );
        assert_eq!(t.len(), 2);
        // and neither matches -1: the three plan keys are distinct
        // (255u8/255i64 to each other, both from -1i8)
        let found_neg = fused_find(&t, &bits((-1i64) as u64)).unwrap();
        assert_ne!(found_neg, (u8_255 >> 1) as i32);
        assert!(found_neg >= 0, "-1i8 is stored, not missing");
        assert_eq!(fused_find(&t, &bits(255)).unwrap(), (u8_255 >> 1) as i32);
        assert_eq!(fused_find(&t, &bits(256)).unwrap(), -1);
    }

    /// The bool lane: `true`/`false` hash mix64(1)/mix64(0) — the
    /// wrapper's bool hash's exact bits, and the words the map's
    /// identity hasher buckets on.
    #[test]
    fn the_bool_lane_hashes_the_wrappers_bools() {
        let mut t = NativeTable::new(8);
        let tru = fused_put(&mut t, bits(1)).unwrap();
        assert_eq!(tru & 1, 1, "fresh");
        assert_eq!(fused_find(&t, &bits(1)).unwrap(), (tru >> 1) as i32);
        assert_eq!(fused_find(&t, &bits(0)).unwrap(), -1, "false misses");
        let fal = fused_put(&mut t, bits(0)).unwrap();
        assert_eq!(fal & 1, 1);
        assert_eq!(fused_find(&t, &bits(0)).unwrap(), (fal >> 1) as i32);
        assert_eq!(fused_remove(&mut t, &bits(0)).unwrap(), (fal >> 1) as i32);
        // hash_payload IS mix64(0)/mix64(1) for the bool bits
        assert_eq!(hash_payload(&KeyVal::Bits(0)), mix64(0));
        assert_eq!(hash_payload(&KeyVal::Bits(1)), mix64(1));
    }

    // ---- the handle-rebased val lanes (the retired column suite's
    // ---- laws, migrated) -------------------------------------------

    /// Raw-bit round-trips through the handle-rebased lanes: the u64
    /// sign range, f64 bit patterns (`to_bits`), replace at the same
    /// handle, the EMPTY-trap law (§0.8 g — an hput birth holds no
    /// bits; reading them is a caller bug, never a silent zero), and
    /// the bounds law (live births; a removed key's handle traps by
    /// name, never reinterpreted).
    #[test]
    fn val_lanes_round_trip_raw_bits_and_trap_on_empty_and_dead_handles() {
        let mut t = NativeTable::new(8);
        for k in 0u64..5 {
            let ans = fused_put(&mut t, bits(k)).unwrap();
            let hd = (ans >> 1) as i32;
            // the h-family birth holds Empty — a valued read TRAPS
            let err = t.val_get_u(hd).unwrap_err();
            assert!(err.msg.contains("no bits"), "{}", err.msg);
            t.val_set_u(hd, u64::MAX - k).unwrap();
        }
        for k in 0u64..5 {
            let hd = fused_find(&t, &bits(k)).unwrap();
            assert_eq!(t.val_get_u(hd).unwrap(), u64::MAX - k, "key {k}");
        }
        // f64 patterns ride the same Bits storage through the f lane
        let vals = [
            1.5f64,
            -2.25,
            0.0,
            -0.0, // the sign bit is the payload — not "equal" to 0.0's bits
            f64::MIN_POSITIVE,
            f64::MAX,
            f64::MIN,
        ];
        let hd = fused_find(&t, &bits(0)).unwrap();
        for (i, v) in vals.iter().enumerate() {
            t.val_set_f(hd, *v).unwrap();
            assert_eq!(t.val_get_f(hd).unwrap(), *v, "val {i}");
            assert_eq!(t.val_get_u(hd).unwrap(), v.to_bits(), "the raw word {i}");
        }
        // -0.0 vs 0.0: distinct bit patterns in ONE storage
        t.val_set_f(hd, 0.0).unwrap();
        let zero = t.val_get_u(hd).unwrap();
        t.val_set_f(hd, -0.0).unwrap();
        assert_ne!(t.val_get_u(hd).unwrap(), zero);
        // bounds = live births: negative and unborn handles trap
        for h in [-1, 5, i32::MAX] {
            let err = t.val_get_u(h).unwrap_err();
            assert_eq!(err.kind, TrapKind::Invalid);
            assert!(err.msg.contains("out of range"), "{}", err.msg);
            let err = t.val_set_u(h, 1).unwrap_err();
            assert!(err.msg.contains("out of range"), "{}", err.msg);
        }
        // remove kills the handle: a removed key's handle stays inside
        // the births range (monotonic, never recycled) but has no live
        // entry — the trap names exactly that
        let hd = fused_find(&t, &bits(1)).unwrap();
        assert_eq!(fused_remove(&mut t, &bits(1)).unwrap(), hd);
        let err = t.val_get_u(hd).unwrap_err();
        assert!(err.msg.contains("no live entry"), "{}", err.msg);
        let err = t.val_set_u(hd, 9).unwrap_err();
        assert!(err.msg.contains("no live entry"), "{}", err.msg);
    }

    /// THE PARITY LAW (the retired `nmap_valcolumn.rs`'s checksum leg,
    /// migrated per the survey §7): one churn sequence — puts with
    /// internal growth, replaces, removes, re-adds — driven through the
    /// h-family + the raw val lanes must agree with a std `HashMap`
    /// reference on every observable: the live set, each key's value,
    /// len, and the handle identity (`hfind` answers what `hput` packed).
    #[test]
    fn the_handle_rebased_val_lanes_match_a_reference_map() {
        let mut t = NativeTable::new(4);
        let mut reference: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
        let mut handles: std::collections::HashMap<u64, i32> = std::collections::HashMap::new();
        let val_of = |k: u64| -> u64 {
            if k % 7 == 0 {
                (-(k as f64)).to_bits()
            } else {
                k * 7 + 1
            }
        };
        // 150 puts (growth is std's, internal), val stored per birth
        for k in 0u64..150 {
            let ans = fused_put(&mut t, bits(k)).unwrap();
            assert_eq!(ans & 1, 1, "key {k} fresh");
            let hd = (ans >> 1) as i32;
            handles.insert(k, hd);
            t.val_set_u(hd, val_of(k)).unwrap();
            reference.insert(k, val_of(k));
        }
        assert_eq!(t.len(), 150);
        assert!(t.cap() >= 150, "growth happened internally: {}", t.cap());
        // replaces on the % 4 keys: same handle, new bits
        for k in (0u64..150).step_by(4) {
            let ans = fused_put(&mut t, bits(k)).unwrap();
            assert_eq!(ans, pack_answer(handles[&k], false), "replace key {k}");
            t.val_set_u(handles[&k], val_of(k).wrapping_add(1)).unwrap();
            reference.insert(k, val_of(k).wrapping_add(1));
        }
        // removes on the % 3 keys: the dead handle traps afterwards
        for k in (0u64..150).step_by(3) {
            let hd = handles.remove(&k).unwrap();
            assert_eq!(fused_remove(&mut t, &bits(k)).unwrap(), hd);
            reference.remove(&k);
            let err = t.val_get_u(hd).unwrap_err();
            assert!(err.msg.contains("no live entry"), "key {k}: {}", err.msg);
        }
        // re-adds on half the removed keys: a NEW birth, fresh val
        for k in (0u64..150).step_by(6) {
            let ans = fused_put(&mut t, bits(k)).unwrap();
            assert_eq!(ans & 1, 1, "re-add key {k} is a new birth");
            let hd = (ans >> 1) as i32;
            assert_ne!(hd, handles.get(&k).copied().unwrap_or(-1), "never recycled");
            handles.insert(k, hd);
            t.val_set_u(hd, val_of(k).wrapping_add(2)).unwrap();
            reference.insert(k, val_of(k).wrapping_add(2));
        }
        // every survivor reads back exact, and identity holds
        assert_eq!(t.len(), reference.len() as i32);
        for (k, v) in &reference {
            let hd = handles[k];
            assert_eq!(fused_find(&t, &bits(*k)).unwrap(), hd, "key {k}");
            assert_eq!(t.val_get_u(hd).unwrap(), *v, "key {k}");
        }
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

    /// The parity law (strings-round1 phase 1), re-based: the sv lanes
    /// and the s lanes on the SAME content build the SAME map — same
    /// kind, same handles, same find answers — with replaces and
    /// removes crossing lanes, both directions. Identity: an sv key and
    /// the equal-content str key are ONE entry.
    #[test]
    fn sv_and_s_lanes_assign_identical_handles_both_directions() {
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
        // growth is std's, internal
        let build = |mode: u8| -> NativeTable {
            let mut t = NativeTable::new(4);
            for i in 0..8 {
                let ans = match mode {
                    0 => fused_put_sv(&mut t, parent, windows[i].0, windows[i].1),
                    1 => fused_put_s(&mut t, &texts[i]),
                    _ => {
                        if i % 2 == 0 {
                            fused_put_sv(&mut t, parent, windows[i].0, windows[i].1)
                        } else {
                            fused_put_s(&mut t, &texts[i])
                        }
                    }
                }
                .unwrap();
                assert_eq!(ans & 1, 1, "key {i} inserts fresh");
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
        // the three builds are THE SAME MAP: same handles per content,
        // by construction (identity is the content, never an address)
        for i in 0..8 {
            let (off, len) = windows[i];
            let ha = fused_find_sv(&a, parent, off, len).unwrap();
            assert!(ha >= 0, "key {i} lost on the sv build");
            assert_eq!(ha, fused_find_s(&a, &texts[i]).unwrap());
            assert_eq!(ha, fused_find_sv(&b, parent, off, len).unwrap());
            assert_eq!(ha, fused_find_s(&b, &texts[i]).unwrap());
            assert_eq!(ha, fused_find_s(&c, &texts[i]).unwrap(), "the mixed build too");
        }

        // replaces cross lanes: a put through the OPPOSITE lane answers
        // the same handle, newly = false, and stores no second entry
        let mut a = a;
        let alpha = fused_find_sv(&a, parent, 0, 5).unwrap();
        assert_eq!(fused_put_s(&mut a, "alpha").unwrap(), pack_answer(alpha, false));
        assert_eq!(a.len(), 8);
        let beta = fused_find_s(&a, "beta").unwrap();
        assert_eq!(fused_put_sv(&mut a, parent, 6, 4).unwrap(), pack_answer(beta, false));
        assert_eq!(a.len(), 8);

        // removes cross lanes: sv remove of the s-replaced key answers
        // its handle; the owned lane then misses too; a re-add is a NEW
        // birth (handles are never recycled)
        assert_eq!(fused_remove_sv(&mut a, parent, 0, 5).unwrap(), alpha);
        assert_eq!(fused_remove_s(&mut a, "alpha").unwrap(), -1);
        assert_eq!(fused_find_sv(&a, parent, 0, 5).unwrap(), -1);
        assert_eq!(fused_find_s(&a, "alpha").unwrap(), -1);
        assert_eq!(a.len(), 7);
        let ans = fused_put_s(&mut a, "alpha").unwrap();
        assert_eq!(ans & 1, 1, "the re-add is a new birth");
        assert!(((ans >> 1) as i32) > alpha, "births are monotonic");
    }

    /// The sv lanes admit like the s lanes: a Str table takes them, a
    /// Bits table traps mixed kinds BEFORE touching the map (never a
    /// silent absent), and a fresh table's first sv put fixes its kind.
    #[test]
    fn sv_lanes_admit_and_trap_like_the_s_lanes() {
        let mut t = NativeTable::new(8);
        assert_eq!(fused_put(&mut t, bits(1)).unwrap() & 1, 1);
        let err = fused_find_sv(&t, "ab", 0, 2).unwrap_err();
        assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
        let err = fused_put_sv(&mut t, "ab", 0, 2).unwrap_err();
        assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
        let err = fused_remove_sv(&mut t, "ab", 0, 2).unwrap_err();
        assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
        let err = fused_put_s(&mut t, "ab").unwrap_err();
        assert!(err.msg.contains("mixed key kinds"), "{}", err.msg);
        assert_eq!(t.len(), 1, "the rejected keys stored nothing");
        // a fresh table admits the sv lanes (Unset -> Str)
        let mut t = NativeTable::new(8);
        let ans = fused_put_sv(&mut t, "xy", 0, 2).unwrap();
        assert_eq!(ans & 1, 1);
        assert_eq!(t.kind, KeyKind::Str);
    }

    /// Multi-grow sweep through the sv path, the k-mer shape: 150
    /// distinct 3-byte windows carved from ONE generated parent, std's
    /// internal growth, remove/re-add churn across lanes — every key
    /// findable at the end, both lanes agreeing on every handle.
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
            let ans = fused_put_sv(&mut t, &parent, off_of(i), 3).unwrap();
            assert_eq!(ans & 1, 1, "window {i} inserts fresh");
            distinct.insert(text_of(i));
        }
        assert_eq!(distinct.len(), 150, "the sweep's keys are distinct");
        assert_eq!(t.len(), 150);
        assert!(t.cap() >= 150, "growth is std's and internal: {}", t.cap());

        // churn: remove every 3rd key through the sv lane, re-add
        // through the s lane (mixed-lane churn over the same map)
        for i in (0..150).step_by(3) {
            assert!(fused_remove_sv(&mut t, &parent, off_of(i), 3).unwrap() >= 0);
        }
        assert_eq!(t.len(), 100);
        for i in (0..150).step_by(3) {
            let ans = fused_put_s(&mut t, &text_of(i)).unwrap();
            assert_eq!(ans & 1, 1, "re-add {i} is a fresh birth");
        }
        assert_eq!(t.len(), 150);
        // every key findable at the end, BOTH lanes agreeing per handle
        for i in 0..150usize {
            let via_sv = fused_find_sv(&t, &parent, off_of(i), 3).unwrap();
            assert!(via_sv >= 0, "key {i} lost");
            assert_eq!(via_sv, fused_find_s(&t, &text_of(i)).unwrap(), "key {i}");
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
        // growth (std's, internal) rehashes nothing the handle can see,
        // and the wrapper's sidecar (indexed by it) never drains
        let mut t = NativeTable::new(4);
        let mut handles = Vec::new();
        for i in 0..500u64 {
            let ans = fused_put(&mut t, bits(i.wrapping_mul(0x9E3779B1))).unwrap();
            assert_eq!(ans & 1, 1, "every key fresh");
            handles.push((ans >> 1) as i32);
        }
        assert!(t.cap() >= 500, "the sweep grew the map internally: {}", t.cap());
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
        // the survivor is untouched
        assert_eq!(fused_find(&t, &bits(8)).unwrap(), 1);
    }

    #[test]
    fn the_sv_and_s_lanes_answer_one_handle() {
        // the parity law extended to identity: an sv key and the
        // equal-content str key are ONE key — same entry, same HANDLE —
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
