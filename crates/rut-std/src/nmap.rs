//! `nmap_host` — the native key-table experiment's HOST half: a real
//! `hashbrown::HashTable` whose entries live Rust-side behind an
//! `opaque` payload, so rut meets it only through the
//! `pub host fn` surface built by [`pkg`]. (The pure-rut
//! `mapset` package — the original reference and general-key
//! implementation — was REMOVED from the tree in Sep 2026, stdlib
//! slim-down: general keys now mean a `bytes` encoding or the
//! `nmapset` wrapper; the old source lives in git history.)
//!
//! Design (nmap-borrow-probe):
//! - the table IS `hashbrown::HashTable<(KeyVal, Entry)>` (the
//!   workspace's first external runtime dep — already in the lock via
//!   the LSP chain, wasm-clean): the SwissTable machinery (control
//!   bytes, buckets, the load-factor law, grow) stays the code std
//!   itself runs, and THERE IS NO HASHER TRAIT — every lane hands the
//!   table the hash word it computed itself, [`hash_payload`] through
//!   [`KeyRef`]: the pinned mapset bits (mix64 for the integer/bool
//!   lanes, FNV-1a 64 over the octets for `str`/`bytes`) — bit-for-bit
//!   the words the retired identity-hasher path bucketed on, which is
//!   why the pinned-literal tests and the bench checksums do not move;
//!   the grow hasher re-derives them from the stored key, so growth's
//!   rehash changes nothing either. `eq` is content equality over the
//!   borrowed window, so an sv key and the equal-content `str` key are
//!   ONE key with ONE entry;
//! - THE COPY LAW (the price the old `std::HashMap` core charged —
//!   phase 0 counted exactly 1.000 materializations per str-lane op,
//!   283,332 per nmapset-str run): probes BORROW — [`KeyRef`] is a
//!   `Bits` word or a `&str`/`&[u8]` window straight out of the
//!   crossing — and the owned `KeyVal` is materialized ONLY at a fresh
//!   insert, where the store needs it (`KeyRef::to_keyval`). `find`/
//!   `find_entry` never allocate; `entry(hash, eq, hasher)` replaces
//!   in place;
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
//! - values: `Entry { handle, val: ValSlot }` — every entry's value
//!   lives IN the entry ([`ValSlot::Empty`] until a valued lane fills
//!   it). Since the any-lane migration (phase 2a) the valued lanes
//!   SEAL: the `hv` family's value is the erasure box — the put
//!   takes over the `v: opaque` param's one reference and the entry
//!   holds the BOX cell ([`ValSlot::Ref`]; released on replace/remove/
//!   death — the copy/alloc laws), `map_hvget` answers the stored box
//!   as `?opaque` (a fresh owning handle; the aliasing law rides the
//!   box), and a raw [`ValSlot::Bits`] cell — a `map_val_set_*` store
//!   — is NOT a seal box: a valued get over it traps by name, never a
//!   reinterpret. The raw `map_val_{set,get}_{u,f}` lanes stay as the
//!   bits-only escape hatch (exact-arm matched: bits answer bits,
//!   `Empty`/`Ref` trap, never reinterpret; a `Ref` cell is released
//!   on overwrite).
//!
//! Keys are TYPED now too (phase 2a): they are INSPECTED — hashed and
//! kind-admitted — so they cross typed, one row per key flavor (bare =
//! `str`, `_i` = the bits lane as one `i64` crossing, `_y` = `bytes`),
//! and the host-side any-key decode is gone. Borrowed keys, end to
//! end: the s/sv/y crossings hand the table a `&str`/`&[u8]` window
//! (the sv lanes stay zero-copy through `sv_range`'s house `Invalid`
//! checks), and the ONE owned copy left in the whole surface is a
//! fresh insert. Admission still runs BEFORE the value reference is
//! taken over (the leak law). The surface DOES iterate —
//! `map_iter_new`/`map_iter_next` drive the `for..of` law (the
//! birth-order snapshot, below) — but bucket internals stay
//! unobservable: the walk hands out key boxes, never a bucket
//! address.

use hashbrown::hash_table;
use hashbrown::HashTable;

use rut_vm::heap::Heap;
use rut_vm::interp::{HostPkg, Ret, Vm};
use rut_vm::{HostPayload, Opaque, OpaqueRef, Slot, Trap, TrapKind, ValSlot, Value};
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
    /// `str` — owned copy, equality is octet equality
    Str,
    /// `bytes` — owned copy, equality is octet equality
    Bytes,
}

/// A stored key: the closed native set as owned Rust data. `Eq` is
/// content equality (the derive) — a key's identity in the map is WHAT
/// IT SAYS, never where it lives. No `Hasher` trait touches it
/// anymore: the lanes hand the table [`hash_payload`]'s word directly,
/// and probes borrow through [`KeyRef`] below (the owned copy is
/// materialized only at a fresh insert).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyVal {
    /// the slot word's raw bits (a negative integer crosses
    /// sign-extended, exactly as it hashed wrapper-side)
    Bits(u64),
    /// an owned `str` copy — fresh inserts only
    Str(String),
    /// an owned `bytes` copy — fresh inserts only
    Bytes(Vec<u8>),
}

impl KeyVal {
    /// The key's flavor — what `admit` checks (always equal to the
    /// probe's own flavor over the same content).
    pub fn kind(&self) -> KeyKind {
        match self {
            KeyVal::Bits(_) => KeyKind::Bits,
            KeyVal::Str(_) => KeyKind::Str,
            KeyVal::Bytes(_) => KeyKind::Bytes,
        }
    }
}

/// The BORROWED probe key (nmap-borrow-probe): what every crossing
/// hands the table — the register word's raw bits, or a `&str`/
/// `&[u8]` window straight from the cell or the sv range. `hash()` is
/// [`hash_payload`]'s word by construction (both spellings funnel
/// through `hash_bits`/`hash_bytes`, so a probe and its stored key
/// bucket identically — the pinned bits); `eq_val` is content
/// equality against the stored key (an sv window and the
/// equal-content `str` are ONE key, spelled at the probe); and
/// `to_keyval` is THE materialization — called only at a fresh
/// insert, the whole copy law.
#[derive(Clone, Copy, Debug)]
pub enum KeyRef<'a> {
    /// the slot word's raw bits (the sign/zero-extension law rides the
    /// lane's cast, exactly as the typed h-family lanes spell it)
    Bits(u64),
    /// a borrowed str window — zero-copy probes, s and sv alike
    Str(&'a str),
    /// a borrowed bytes window — zero-copy probes
    Bytes(&'a [u8]),
}

impl<'a> KeyRef<'a> {
    /// The key's flavor — what `admit` checks (always equal to the
    /// stored `KeyVal`'s own flavor).
    pub fn kind(&self) -> KeyKind {
        match self {
            KeyRef::Bits(_) => KeyKind::Bits,
            KeyRef::Str(_) => KeyKind::Str,
            KeyRef::Bytes(_) => KeyKind::Bytes,
        }
    }

    /// The table's hash word — the pinned mapset bits over the
    /// borrowed window (no copy to hash through).
    pub fn hash(&self) -> u64 {
        match self {
            KeyRef::Bits(v) => hash_bits(*v),
            KeyRef::Str(s) => hash_bytes(s.as_bytes()),
            KeyRef::Bytes(b) => hash_bytes(b),
        }
    }

    /// Content equality against the stored owned key.
    pub fn eq_val(&self, stored: &KeyVal) -> bool {
        match (self, stored) {
            (KeyRef::Bits(a), KeyVal::Bits(b)) => a == b,
            (KeyRef::Str(a), KeyVal::Str(b)) => *a == b.as_str(),
            (KeyRef::Bytes(a), KeyVal::Bytes(b)) => *a == b.as_slice(),
            _ => false,
        }
    }

    /// THE one materialization — a FRESH insert only (the copy law).
    pub fn to_keyval(&self) -> KeyVal {
        match self {
            KeyRef::Bits(v) => KeyVal::Bits(*v),
            KeyRef::Str(s) => KeyVal::Str((*s).to_owned()),
            KeyRef::Bytes(b) => KeyVal::Bytes((*b).to_vec()),
        }
    }
}

impl<'a> From<&'a KeyVal> for KeyRef<'a> {
    fn from(k: &'a KeyVal) -> Self {
        match k {
            KeyVal::Bits(v) => KeyRef::Bits(*v),
            KeyVal::Str(s) => KeyRef::Str(s.as_str()),
            KeyVal::Bytes(b) => KeyRef::Bytes(b.as_slice()),
        }
    }
}

/// One entry's value storage: a birth holds [`ValSlot::Empty`] until a
/// value lane fills it; the valued `hv` lanes store the SEAL BOX (the
/// erasure box cell, [`ValSlot::Ref`] — taken over from the
/// `v: opaque` param's handle) and the raw `map_val_set_*` lanes store
/// [`ValSlot::Bits`] (the untagged 8-byte slot). Every release path —
/// replace, remove, the raw lanes' overwrite, entry death
/// (`finalize`) — frees the held cell through the Vm at hand, never a
/// plain Drop (the release-context law, §0.8 i); releasing a box cell
/// walks its payload with it. The tag is the
/// exact-arm-match law: a raw bits read TRAPS on `Empty`/`Ref`, never
/// reinterprets, and a valued get TRAPS on `Bits` (bits are not a
/// seal box).
pub(crate) struct Entry {
    /// the key's stable birth index — monotonic, never recycled
    pub(crate) handle: i32,
    pub(crate) val: ValSlot,
}

/// The native table: a real `HashTable` with borrowed probes
/// (nmap-borrow-probe) plus the admission kind and the birth counter.
/// The keys are pure Rust data — their Drop frees them when the box's
/// rc hits 0; the VALUES are rut cells ([`ValSlot::Ref`]) and release
/// through [`HostPayload::finalize`] at store-entry death.
pub struct NativeTable {
    kind: KeyKind,
    /// the borrowed-probe table — one probe per op over the lane's own
    /// hash word (no hasher trait, no key materialization)
    map: HashTable<(KeyVal, Entry)>,
    /// the next birth index — the table's lifetime insertion count,
    /// bounded by BIRTHS not capacity (the packed i64 answer's headroom
    /// law: the handle rides i32 with room far past any real table)
    next_handle: i32,
}

/// The stored bucket's hash word — the grow hasher's closure (rehash
/// re-derives exactly what the lane wrote, so growth moves no bucket).
fn stored_hash(it: &(KeyVal, Entry)) -> u64 {
    hash_payload(&it.0)
}

impl NativeTable {
    /// `map_new`'s `cap` is the table's reserve: a lower bound on the
    /// bucket capacity, honored by `with_capacity` (the power-of-two
    /// ≥ 4 rounding law died with the probing core). `cap` crosses as
    /// the `i64` the wrapper passes.
    pub fn new(cap: i64) -> NativeTable {
        let cap = cap.clamp(0, i32::MAX as i64) as usize;
        NativeTable {
            kind: KeyKind::Unset,
            map: HashTable::with_capacity(cap),
            next_handle: 0,
        }
    }

    /// The fused put-path crossing, the BORROWED core: ONE probe —
    /// `Occupied` → the same handle, `newly = false`, NOTHING
    /// materialized (a replace never touches a key copy); `Vacant` →
    /// a fresh monotonic birth with the [`ValSlot::Empty`] placeholder
    /// and THE one owned copy (`to_keyval`), `newly = true`. The
    /// packed answer is bit-for-bit the hostops lane's
    /// `(handle << 1) | newly`.
    pub fn hput_ref(&mut self, key: KeyRef<'_>) -> Result<i64, Trap> {
        self.admit(key.kind())?;
        let h = key.hash();
        match self.map.entry(h, |it| key.eq_val(&it.0), stored_hash) {
            hash_table::Entry::Occupied(e) => Ok(pack_answer(e.get().1.handle, false)),
            hash_table::Entry::Vacant(v) => {
                if self.kind == KeyKind::Unset {
                    self.kind = key.kind(); // the first insert fixes the flavor
                }
                let handle = self.next_handle;
                self.next_handle += 1;
                v.insert((
                    key.to_keyval(),
                    Entry {
                        handle,
                        val: ValSlot::Empty,
                    },
                ));
                Ok(pack_answer(handle, true))
            }
        }
    }

    /// The fused put-path crossing: the entry API. `Occupied` → the
    /// same handle, `newly = false` (a replace stores nothing new —
    /// the value lanes overwrite in place); `Vacant` → a fresh
    /// monotonic birth with the [`ValSlot::Empty`] placeholder,
    /// `newly = true`. The packed answer is bit-for-bit the hostops
    /// lane's `(handle << 1) | newly`. (Owned-key entry — the lanes
    /// reach the borrowed core directly; this shape serves the
    /// typed-lane tests and answers exactly as before.)
    pub fn hput(&mut self, kind: KeyKind, key: KeyVal) -> Result<i64, Trap> {
        self.admit(kind)?;
        self.hput_ref(KeyRef::from(&key))
    }

    /// The fused lookup crossing, the BORROWED core: the key's handle,
    /// or `-1` — over the window, no materialization.
    pub fn hfind_ref(&self, key: KeyRef<'_>) -> Result<i32, Trap> {
        self.admit(key.kind())?;
        Ok(self
            .map
            .find(key.hash(), |it| key.eq_val(&it.0))
            .map(|it| it.1.handle)
            .unwrap_or(-1))
    }

    /// The fused lookup crossing: the key's handle, or `-1` — the
    /// wrapper answers `nil` / `false` from the sign.
    pub fn hfind(&self, kind: KeyKind, key: &KeyVal) -> Result<i32, Trap> {
        self.admit(kind)?;
        self.hfind_ref(KeyRef::from(key))
    }

    /// The fused remove crossing, the BORROWED core: `find_entry` is
    /// ONE probe — the entry leaves the map, its held value cell (a
    /// `Ref`, when a valued lane stored one) releases through the Vm
    /// FIRST (the entry-death law; dropping a `Ref` bare would leak the
    /// cell) — and the DEAD key's handle answers, or `-1` when absent.
    /// The handle is never recycled: a re-insert is a NEW birth. This
    /// is ALSO the `map_hvremove` body: removal is value-aware in
    /// exactly one way, the release.
    pub fn hremove_ref(&mut self, vm: &Vm, key: KeyRef<'_>) -> Result<i32, Trap> {
        self.admit(key.kind())?;
        match self.map.find_entry(key.hash(), |it| key.eq_val(&it.0)) {
            Ok(occupied) => {
                let ((_, entry), _slot) = occupied.remove();
                let mut e = entry;
                release_val(vm, &mut e.val);
                Ok(e.handle)
            }
            Err(_) => Ok(-1),
        }
    }

    /// The fused remove crossing: the entry leaves the map — its held
    /// value cell (a `Ref`, when a valued lane stored one) releases
    /// through the Vm FIRST (the entry-death law; dropping a `Ref`
    /// bare would leak the cell) — and the DEAD key's handle answers,
    /// or `-1` when absent. The handle is never recycled: a re-insert
    /// is a NEW birth. This is ALSO the `map_hvremove` body: removal
    /// is value-aware in exactly one way, the release.
    pub fn hremove(&mut self, vm: &Vm, kind: KeyKind, key: &KeyVal) -> Result<i32, Trap> {
        self.admit(kind)?;
        self.hremove_ref(vm, KeyRef::from(key))
    }

    /// The valued put (the `map_hvput` body), the BORROWED core: the
    /// entry API with the value INSIDE — ONE crossing carries it.
    /// `Occupied` → release the old value in-crossing and store the
    /// new (same handle, `newly = false`, no key copy); `Vacant` → a
    /// fresh monotonic birth holding `val` (`newly = true`, the one
    /// materialization). The packed answer is bit-for-bit the
    /// h-family's. `val` arrives pre-owned — the row took over the
    /// sealed box's one reference ([`seal_transfer`]) with admission
    /// already checked — `check_kind` ran BEFORE the transfer, so a
    /// trapped mixed-kind put cannot strand it (this method re-admits
    /// as defense).
    fn hvput_ref(&mut self, vm: &Vm, key: KeyRef<'_>, val: ValSlot) -> Result<i64, Trap> {
        self.admit(key.kind())?;
        let h = key.hash();
        match self.map.entry(h, |it| key.eq_val(&it.0), stored_hash) {
            hash_table::Entry::Occupied(mut e) => {
                let ent = &mut e.get_mut().1;
                release_val(vm, &mut ent.val);
                ent.val = val;
                Ok(pack_answer(ent.handle, false))
            }
            hash_table::Entry::Vacant(v) => {
                if self.kind == KeyKind::Unset {
                    self.kind = key.kind(); // the first insert fixes the flavor
                }
                let handle = self.next_handle;
                self.next_handle += 1;
                v.insert((key.to_keyval(), Entry { handle, val }));
                Ok(pack_answer(handle, true))
            }
        }
    }

    /// The valued put (the `map_hvput` body): the entry API with the
    /// value INSIDE — ONE crossing carries it. `Occupied` → release the
    /// old value in-crossing and store the new (same handle, `newly =
    /// false`); `Vacant` → a fresh monotonic birth holding `val`
    /// (`newly = true`). The packed answer is bit-for-bit the h-family's.
    /// `val` arrives pre-owned — see `NativeTable::hvput_ref`.
    pub fn hvput(
        &mut self,
        vm: &Vm,
        kind: KeyKind,
        key: KeyVal,
        val: ValSlot,
    ) -> Result<i64, Trap> {
        self.admit(kind)?;
        self.hvput_ref(vm, KeyRef::from(&key), val)
    }

    /// The valued get (the `map_hvget` body), the BORROWED core: one
    /// `find` over the window; the stored [`ValSlot`] answers AS-IS —
    /// the row converts a `Ref` (the seal box) into an owning `?opaque`
    /// handle, traps on `Bits` (a raw `map_val_set_*` store is not a
    /// seal box), and traps on `Empty`. Absent → `None` (the miss
    /// crosses as nil). An `Empty` entry — an h-family birth read
    /// through a valued lane — TRAPS loudly (§0.8 g: a caller bug,
    /// never a silent nil).
    fn hvget_ref(&self, key: KeyRef<'_>) -> Result<Option<ValSlot>, Trap> {
        self.admit(key.kind())?;
        match self.map.find(key.hash(), |it| key.eq_val(&it.0)) {
            None => Ok(None),
            Some(it) => match &it.1.val {
                ValSlot::Empty => Err(Trap::new(
                    TrapKind::Invalid,
                    format!(
                        "nmap: valued get over the `Empty` placeholder (handle {}; an hput birth holds no value — a valued read of it is a caller bug, §0.8 g)",
                        it.1.handle
                    ),
                )),
                ValSlot::Bits(s) => Ok(Some(ValSlot::Bits(*s))),
                ValSlot::Ref(s) => Ok(Some(ValSlot::Ref(*s))),
            },
        }
    }

    /// The valued get (the `map_hvget` body): the stored [`ValSlot`]
    /// answers AS-IS — the ROW turns a `Ref` (the seal box) into the
    /// `?opaque` answer; see `NativeTable::hvget_ref`. Absent →
    /// `None` (the miss crosses as nil). An `Empty` entry — an
    /// h-family birth read through a valued lane — TRAPS loudly (§0.8
    /// g: a caller bug, never a silent nil).
    pub fn hvget(&self, kind: KeyKind, key: &KeyVal) -> Result<Option<ValSlot>, Trap> {
        self.admit(kind)?;
        self.hvget_ref(KeyRef::from(key))
    }

    /// The admission check the valued put lanes run BEFORE taking over
    /// the sealed box's reference ([`seal_transfer`]): a trapped
    /// mixed-kind crossing must not strand a reference it has not been
    /// given yet. [`NativeTable::hvput`] re-admits internally as
    /// defense.
    pub fn check_kind(&self, kind: KeyKind) -> Result<(), Trap> {
        self.admit(kind)
    }

    /// The handle-rebased val write: store `raw` as [`ValSlot::Bits`]
    /// at the live key's `handle` (the h-family's birth — never a
    /// slot; slots died with the probing core). A displaced `Ref` cell
    /// releases through the Vm first (P5: the raw lanes are value-aware
    /// in exactly one way too — overwrite never leaks). Bounds are the
    /// live births; a removed key's handle is in range but has no live
    /// entry, and THAT traps by name.
    pub fn val_set_u(&mut self, vm: &Vm, handle: i32, raw: u64) -> Result<(), Trap> {
        let e = self.entry_by_handle_mut(handle)?;
        release_val(vm, &mut e.val);
        e.val = ValSlot::Bits(Slot::int(raw as i64));
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
    /// this is `to_bits` at the boundary and nothing else. A displaced
    /// `Ref` releases first (see [`NativeTable::val_set_u`]).
    pub fn val_set_f(&mut self, vm: &Vm, handle: i32, v: f64) -> Result<(), Trap> {
        let e = self.entry_by_handle_mut(handle)?;
        release_val(vm, &mut e.val);
        e.val = ValSlot::Bits(Slot::float(v));
        Ok(())
    }

    /// The val read, f lane: `from_bits` over the stored word — the
    /// old column's exact shape, re-based (the wrapper cannot spell
    /// the float reinterpret itself).
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
            .iter()
            .map(|it| &it.1)
            .find(|e| e.handle == handle)
            .ok_or_else(|| dead_handle_trap(handle, self.next_handle))
    }

    fn entry_by_handle_mut(&mut self, handle: i32) -> Result<&mut Entry, Trap> {
        self.check_birth(handle)?;
        // the births count is read BEFORE the mutable map walk — the
        // miss trap must not reach back through `&mut self`
        let births = self.next_handle;
        self.map
            .iter_mut()
            .map(|it| &mut it.1)
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
    /// i32) — the table's reserve, kept as a test/observation surface
    /// only: the `map_cap` crossing retired at P5 with the wrapper's
    /// sidecar pre-size (nothing rut-side has anything left to size).
    pub fn cap(&self) -> i32 {
        self.map.capacity().min(i32::MAX as usize) as i32
    }

    /// The live entry count.
    pub fn len(&self) -> i32 {
        self.map.len() as i32
    }

    /// The table's admitted key flavor — `Unset` until the first insert
    /// fixes it (the closed-set admission state, read back as-is).
    pub fn kind(&self) -> KeyKind {
        self.kind
    }

    /// True when no entries are live.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// THE SNAPSHOT (the `for..of` law): the live keys cloned in BIRTH
    /// order — the never-recycled handle IS the order, so the walk reads
    /// insertion order of live entries and a remove-then-reinsert moves
    /// the key to the end. The walk IS this Vec: puts/removes after
    /// `iter_new` (inside an emit body, user code) never touch it.
    pub fn iter_snapshot(&self) -> Vec<KeyVal> {
        let mut births: Vec<(i32, KeyVal)> = self
            .map
            .iter()
            .map(|it| (it.1.handle, it.0.clone()))
            .collect();
        births.sort_by_key(|(h, _)| *h);
        births.into_iter().map(|(_, k)| k).collect()
    }

    /// Kind admission: homogeneous per table by construction; a mismatch
    /// is a host/wrapper bug and traps instead of silently missing.
    pub fn admit(&self, kind: KeyKind) -> Result<(), Trap> {
        if self.kind != KeyKind::Unset && self.kind != kind {
            return Err(Trap::new(
                TrapKind::Invalid,
                format!("nmap: mixed key kinds in one table (table holds {kind:?} keys)"),
            ));
        }
        Ok(())
    }
}

/// The iteration cursor's payload (the `for..of` law): the SNAPSHOT —
/// the live keys in birth order ([`NativeTable::iter_snapshot`]) — plus
/// the mint's exact key type (the wrapper's `type_id<K>()` crossing) and
/// the walk position. Pure Rust data, no rut references: plain
/// [`Opaque::alloc`], no finalize hook — the Vec drops with the box at
/// rc-0, and `map_iter_next` mints each key box fresh per yield.
pub struct IterCursor {
    /// the snapshot's keys, birth order ([`NativeTable::iter_snapshot`])
    pub keys: Vec<KeyVal>,
    /// the mint's exact `val_ty` (the wrapper's `type_id<K>()` crossing)
    pub ty: u32,
    /// the walk position
    pub pos: usize,
}

/// The table is a HostOpaque payload (nmap-hostvals P2) whose release
/// hook is REAL since P5: at store-entry death (rc-0 inside the release
/// walk, BEFORE the payload's own Drop) every held value
/// cell releases through the provided heap view — the record-field
/// pattern, children through the same walk that frees the parent — and
/// the map clears. Keys are pure Rust data: their Drop rides the
/// payload's own Drop, deterministically.
impl HostPayload for NativeTable {
    fn finalize(&mut self, heap: &Heap) {
        for it in self.map.iter_mut() {
            if let ValSlot::Ref(s) = std::mem::replace(&mut it.1.val, ValSlot::Empty) {
                heap.release(s);
            }
        }
        self.map.clear();
    }
}

/// The in-crossing value release (the release-context law, §0.8 i):
/// `Ref` cells release through the Vm at hand; `Bits`/`Empty` are raw
/// words (and the placeholder) with nothing to free. The displaced tag
/// becomes `Empty` first, so the caller's store/overwrite/drop can
/// never double-release. Since the seal law the `Ref` arm frees the
/// erasure box — the box's own release walks its payload, so the wrapper's V dies with its box exactly as it used to.
fn release_val(vm: &Vm, val: &mut ValSlot) {
    if let ValSlot::Ref(s) = std::mem::replace(val, ValSlot::Empty) {
        vm.release(s);
    }
}

/// The seal handoff (phase 2a): the `v: opaque` param arrives as an
/// OWNED handle — the boundary's read took one reference — and the
/// entry takes it over verbatim (`into_slot`'s TRANSFER, no extra
/// retain; the crossing-ownership law). Called only AFTER admission
/// and the sv range check: until the transfer the handle's own Drop
/// releases, so a trapped crossing strands nothing (the leak law, in
/// its new spelling).
pub fn seal_transfer(v: OpaqueRef, vm: &mut Vm) -> Result<ValSlot, Trap> {
    Ok(ValSlot::Ref(v.into_slot(vm)?))
}

/// The `?opaque` answer (phase 2a): the stored cell as an owning box
/// handle. `None` is the miss (the flat nil at the crossing); a `Ref`
/// bumps once — the register owns the new reference, the store keeps
/// its own (the aliasing law, now riding the box). A `Bits` cell is a
/// raw `map_val_set_*` store, NOT a seal box — trap by name, never a
/// reinterpret; `Empty` is the §0.8 g caller-bug trap (hvget_ref
/// already refuses it; kept as defense).
pub fn seal_answer(vm: &Vm, stored: Option<ValSlot>) -> Result<Option<OpaqueRef>, Trap> {
    match stored {
        None => Ok(None),
        // Ret::from_slot IS the read: an owning handle over the box's
        // slot word (one bump, TY_OPAQUE-checked)
        Some(ValSlot::Ref(s)) => Ok(Some(OpaqueRef::from_slot(vm, s, rut_core::types::TY_OPAQUE)?)),
        Some(ValSlot::Bits(_)) => Err(Trap::new(
            TrapKind::Invalid,
            "nmap: the stored value is raw bits (a `map_val_set_*` store), not a seal box — read it through the raw lanes (the valued lanes answer sealed boxes only)",
        )),
        Some(ValSlot::Empty) => Err(Trap::new(
            TrapKind::Invalid,
            "nmap: valued get over the `Empty` placeholder (an hput birth holds no value — a valued read of it is a caller bug, §0.8 g)",
        )),
    }
}

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

/// The iteration lane's key-type decode: the wrapper's `type_id<K>()`
/// crossing as the mint's exact `val_ty`, mapped to the table's flavor.
/// Defense: anything outside the closed key set — floats, records, user
/// types — traps by name (the wrapper's union bound is the compile-time
/// admission, this is the host's, exactly the put crossing's law).
pub fn iter_key_kind(ty: u32) -> Result<KeyKind, Trap> {
    use rut_core::types::{
        TY_BOOL, TY_BYTES, TY_I16, TY_I32, TY_I64, TY_I8, TY_STR, TY_U16, TY_U32, TY_U64, TY_U8,
    };
    match ty {
        TY_I8 | TY_I16 | TY_I32 | TY_I64 | TY_U8 | TY_U16 | TY_U32 | TY_U64 | TY_BOOL => {
            Ok(KeyKind::Bits)
        }
        TY_STR => Ok(KeyKind::Str),
        TY_BYTES => Ok(KeyKind::Bytes),
        _ => Err(Trap::new(
            TrapKind::Invalid,
            format!(
                "nmap: iter key type #{ty} outside the closed key set — the integer primitives, `bool`, `str`, `bytes`"
            ),
        )),
    }
}

/// THE MINT (the `for..of` law): one fresh key box per yield — the exact
/// inverse of the put crossing's decode (`slot_to_value`'s law,
/// mirrored). The integer and `bool` lanes ride the RAW slot word: Bits
/// stored it sign/zero-extended at the put, and the word is the identity
/// — the mint writes it back under the requested type, so an `i8` key
/// reads `-5` and a `u64` key reads its high bit through
/// `opaque::downcast<K>` bit-for-bit. The octet lanes mint a FRESH cell
/// (`String`/`Vec<u8>`'s boundary return — the charged alloc) — the
/// fresh-copy law: there is no stored rut key cell to hand out (the sv
/// borrow law minted none), so every yield is a new cell of equal
/// content. `ty` is the box's `val_ty` — the exact check
/// `opaque::downcast<K>` runs — which is why the wrapper's
/// `type_id<K>()` crosses at all; the recovery can never nil.
pub fn mint_key_box(vm: &mut Vm, ty: u32, key: &KeyVal) -> Result<OpaqueRef, Trap> {
    let payload = match (iter_key_kind(ty)?, key) {
        (KeyKind::Bits, KeyVal::Bits(word)) => {
            if ty == rut_core::types::TY_BOOL {
                Slot::bool(*word != 0)
            } else {
                Slot::int(*word as i64)
            }
        }
        (KeyKind::Str, KeyVal::Str(s)) => <String as Ret>::into_slot(s.clone(), vm)?,
        (KeyKind::Bytes, KeyVal::Bytes(b)) => <Vec<u8> as Ret>::into_slot(b.clone(), vm)?,
        _ => {
            return Err(Trap::new(
                TrapKind::Invalid,
                "nmap: iter flavor mismatch — the snapshot's keys disagree with the requested key type (a host/wrapper bug)",
            ))
        }
    };
    let box_slot = vm.alloc_opaque_box(payload, ty)?;
    OpaqueRef::from_slot(vm, box_slot, rut_core::types::TY_OPAQUE)
}

/// FNV-1a 64's offset basis and prime — the checksum law, ported
/// BIT-FOR-BIT from the removed pure-rut `mapset` pkg, so the map's
/// bucket hashing equals the bench checksums pinned in
/// `expected.json`. A one-ulp difference here breaks the gate; the
/// unit test pins the exact outputs as literals.
const FNV_OFFSET: u64 = 14695981039346656037;
const FNV_PRIME: u64 = 1099511628211;

/// FNV-1a 64 over an octet range — the ONE str/bytes hasher, shared by
/// every lane that hashes octets. The constants are the checksum law
/// (see `FNV_OFFSET`); the s/sv lanes hash OVER the borrowed range
/// and it answers the same bits [`hash_payload`] answers for the same
/// octets, by construction.
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = FNV_OFFSET;
    for b in bytes {
        h = (h ^ *b as u64).wrapping_mul(FNV_PRIME);
    }
    h
}

/// The integer/bool lane's word — mix64 over the two pinned constants
/// (both hash spellings funnel through this and [`hash_bytes`], so a
/// borrowed probe and its stored key are bit-identical by
/// construction).
fn hash_bits(v: u64) -> u64 {
    (FNV_OFFSET ^ v).wrapping_mul(FNV_PRIME)
}

/// The ONE shared payload hasher — the exact word the table buckets a
/// stored [`KeyVal`] on: mix64 for the integer/bool bits, FNV-1a 64
/// over the octets for `str`/`bytes`. Same inputs, the pinned
/// constants, bit-exact (the pinned-literal tests). No `Hasher` trait
/// involved — lanes hand this word to `HashTable::{find, entry}`
/// directly, and [`KeyRef::hash`] answers the same bits over the
/// borrowed window.
pub fn hash_payload(k: &KeyVal) -> u64 {
    KeyRef::from(k).hash()
}

/// The sv lanes' range check (strings-round1 phase 1): `off`/`len` are
/// BYTE offsets into the parent's octets — `0 <= off`, `0 <= len`,
/// `off + len <= parent.len()` (usize math, so the i32 corners cannot
/// overflow the check), and BOTH ends must sit on UTF-8 codepoint
/// boundaries — a mid-codepoint window is a caller bug, trapped with
/// the house `Invalid` shape, never a silent mis-read.
pub fn sv_range(parent: &str, off: i32, len: i32) -> Result<&str, Trap> {
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
            format!(
                "nmap: sv offset {off} is not a UTF-8 boundary (parent {} bytes)",
                parent.len()
            ),
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
// The s/sv/y lanes probe BORROWED (nmap-borrow-probe): the window
// itself crosses as `&str`/`&[u8]` and the owned key exists only at a
// fresh insert — the hash still runs over the octets, and an sv key
// and the equal-content `str` key are ONE key: same entry, same
// HANDLE.

/// The packed `hput` answer: bit 0 = the newly bit, the rest the handle.
#[inline]
pub fn pack_answer(hd: i32, newly: bool) -> i64 {
    ((hd as i64) << 1) | (newly as i64)
}

/// The i/u/b lanes' fused put: the borrowed core over the register
/// word (`KeyRef::Bits` copies, nothing materializes) — growth is the
/// table's own, internal, and invisible to the answer.
fn fused_put(t: &mut NativeTable, key: KeyVal) -> Result<i64, Trap> {
    t.hput_ref(KeyRef::from(&key))
}

/// The i/u/b lanes' fused find: the handle or `-1`.
fn fused_find(t: &NativeTable, key: &KeyVal) -> Result<i32, Trap> {
    t.hfind_ref(KeyRef::from(key))
}

/// The i/u/b lanes' fused remove: the dead key's handle or `-1`,
/// the held value releasing in-crossing.
fn fused_remove(vm: &Vm, t: &mut NativeTable, key: &KeyVal) -> Result<i32, Trap> {
    t.hremove_ref(vm, KeyRef::from(key))
}

/// Install the `nmap_host` bodies under the `nmap_host` scope (the `calc`
/// pattern): the callable's Rust shape IS the `.d.rut`
/// row, so the surface declares exactly these signatures. `map_new`'s
/// box carries the table; every other fn's first param borrows it typed
/// (`Opaque<NativeTable>` — a wrong payload is a checked trap).
pub fn pkg() -> HostPkg {
    let mut pkg = HostPkg::new("nmap_host");
    rut_vm::pkg_fn!(pkg, "map_new", (i64,) -> OpaqueRef, |vm: &mut Vm, cap: i64| {
        let b = Opaque::alloc_hosted(vm, NativeTable::new(cap))?;
        Ok(b.handle().clone())
    });
    rut_vm::pkg_fn!(
        pkg,
        "map_len",
        (Opaque<NativeTable>,) -> i32,
        |_vm: &mut Vm, b: Opaque<NativeTable>| b.with(|t| t.len()),
    );
    // `map_cap` retired at nmap-hostvals P5: with the values living in
    // the entries there is no rut-side sidecar left to pre-size, and
    // the table's own with_capacity hint rides `map_new`'s `cap`.

    // ---- the handle-rebased raw val lanes (nmap-hostvals P4) --------
    // Four crossings, raw bits in/out, ONE storage serving every
    // primitive val kind (i64/u64/f64/bool — the monomorphized wrapper
    // reinterprets client-side; no lane explosion). The address is a
    // live birth HANDLE from the h-family — the slot addressing died
    // with the probing core. `map_val_set_u` stores the word, releasing
    // any displaced `Ref` cell first (P5); `map_val_get_u` answers the
    // stored bits iff the entry's tag is Bits — the Empty placeholder
    // traps (an hput birth holds no value; a valued read is a caller
    // bug, §0.8 g), and so does a `Ref` cell (never a reinterpret).
    // The f pair is the same storage through `to_bits`/`from_bits` at
    // the boundary (rut has no float bitcast). Bounds are
    // the live births; a removed key's handle traps `no live entry`.
    rut_vm::pkg_fn!(
        pkg,
        "map_val_set_u",
        (Opaque<NativeTable>, i32, u64) -> (),
        |vm: &mut Vm, b: Opaque<NativeTable>, handle: i32, raw: u64| -> Result<(), Trap> {
            b.with_mut(vm, |vm, t| t.val_set_u(vm, handle, raw))?
        },
    );
    rut_vm::pkg_fn!(
        pkg,
        "map_val_get_u",
        (Opaque<NativeTable>, i32) -> u64,
        |_vm: &mut Vm, b: Opaque<NativeTable>, handle: i32| -> Result<u64, Trap> {
            b.with(|t| t.val_get_u(handle))?
        },
    );
    rut_vm::pkg_fn!(
        pkg,
        "map_val_set_f",
        (Opaque<NativeTable>, i32, f64) -> (),
        |vm: &mut Vm, b: Opaque<NativeTable>, handle: i32, v: f64| -> Result<(), Trap> {
            b.with_mut(vm, |vm, t| t.val_set_f(vm, handle, v))?
        },
    );
    rut_vm::pkg_fn!(
        pkg,
        "map_val_get_f",
        (Opaque<NativeTable>, i32) -> f64,
        |_vm: &mut Vm, b: Opaque<NativeTable>, handle: i32| -> Result<f64, Trap> {
            b.with(|t| t.val_get_f(handle))?
        },
    );

    // ---- the fused handle lanes (the boxed-key law) -------------------
    // Six crossings. The key SEALS at the wrapper's edge — `opaque(k)`,
    // exactly the law the values always followed — and the host opens
    // the box ([`rut_vm::rut_box_payload`]): the register word's raw
    // bits for the integer and `bool` lanes (bits ARE the key identity,
    // bit-for-bit the words the typed lanes bucketed), the octets for
    // `str`/`bytes` (the same FNV-1a 64). Anything else outside the
    // closed key set traps `Invalid` by name — the wrapper's union
    // bound is the compile-time admission, this is the host's defense.
    // The answers are unchanged: the packed `(handle << 1) | newly` i64
    // from `map_hput` (HashSet reads bit 0), the birth handle or `-1`
    // from `map_hfind`, the was-present i32 from `map_hremove`, and the
    // valued trio keeps its seal/answer laws (the box IS the `?opaque`).
    fn key_val_of(vm: &Vm, k: &OpaqueRef) -> Result<KeyVal, Trap> {
        let (_ty, v) = rut_vm::rut_box_payload(vm, k)?;
        match v {
            Value::I64(bits) => Ok(KeyVal::Bits(bits as u64)),
            Value::Bool(b) => Ok(KeyVal::Bits(b as u64)),
            Value::Str(s) => Ok(KeyVal::Str(s)),
            Value::Bytes(b) => Ok(KeyVal::Bytes(b)),
            Value::F64(_) => Err(Trap::new(
                TrapKind::Invalid,
                "nmap: a float key — the closed key set admits the integer primitives, `bool`, `str`, `bytes` (floats have no stable equality contract)",
            )),
            _ => Err(Trap::new(
                TrapKind::Invalid,
                "nmap: key box outside the closed key set — the integer primitives, `bool`, `str`, `bytes`",
            )),
        }
    }

    rut_vm::pkg_fn!(
        pkg,
        "map_hput",
        (Opaque<NativeTable>, OpaqueRef) -> i64,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: OpaqueRef| -> Result<i64, Trap> {
            let key = key_val_of(vm, &k)?;
            b.with_mut(vm, |_vm, t| {
                t.check_kind(key.kind())?;
                fused_put(t, key)
            })?
        },
    );
    rut_vm::pkg_fn!(
        pkg,
        "map_hfind",
        (Opaque<NativeTable>, OpaqueRef) -> i32,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: OpaqueRef| -> Result<i32, Trap> {
            let key = key_val_of(vm, &k)?;
            b.with(|t| {
                t.check_kind(key.kind())?;
                fused_find(t, &key)
            })?
        },
    );
    rut_vm::pkg_fn!(
        pkg,
        "map_hremove",
        (Opaque<NativeTable>, OpaqueRef) -> i32,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: OpaqueRef| -> Result<i32, Trap> {
            let key = key_val_of(vm, &k)?;
            b.with_mut(vm, |vm, t| {
                t.check_kind(key.kind())?;
                fused_remove(vm, t, &key)
            })?
        },
    );

    // ---- the valued lanes (the boxed-key law; values seal as ever) ----
    // Three crossings + the `_sv` range twins. The KEY is the seal box
    // (opened for hash/eq, never stored — a fresh insert materializes
    // the owned copy, the one alloc the probe law always had); the
    // VALUE is never inspected, so it seals: the put's `v: opaque` is
    // the erasure box, the entry takes over the param handle's one
    // reference ([`seal_transfer`], after admission — the leak law),
    // and the get answers `?opaque` — the STORED box ([`seal_answer`];
    // nil = absent; a `Bits` cell traps by name, never a reinterpret;
    // an `Empty` birth traps loudly, a caller bug).
    rut_vm::pkg_fn!(
        pkg,
        "map_hvput",
        (Opaque<NativeTable>, OpaqueRef, OpaqueRef) -> i64,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: OpaqueRef, v: OpaqueRef| -> Result<i64, Trap> {
            let key = key_val_of(vm, &k)?;
            b.with_mut(vm, |vm, t| {
                // admission BEFORE the transfer: a trapped put strands
                // nothing — until `seal_transfer` the handle's own Drop
                // releases (the leak law)
                t.check_kind(key.kind())?;
                let sealed = seal_transfer(v, vm)?;
                t.hvput_ref(vm, KeyRef::from(&key), sealed)
            })?
        },
    );
    rut_vm::pkg_fn!(
        pkg,
        "map_hvget",
        (Opaque<NativeTable>, OpaqueRef) -> Option<OpaqueRef>,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: OpaqueRef| -> Result<Option<OpaqueRef>, Trap> {
            let key = key_val_of(vm, &k)?;
            let stored = b.with(|t| {
                t.check_kind(key.kind())?;
                t.hvget_ref(KeyRef::from(&key))
            })??;
            seal_answer(vm, stored)
        },
    );
    rut_vm::pkg_fn!(
        pkg,
        "map_hvremove",
        (Opaque<NativeTable>, OpaqueRef) -> i32,
        |vm: &mut Vm, b: Opaque<NativeTable>, k: OpaqueRef| -> Result<i32, Trap> {
            let key = key_val_of(vm, &k)?;
            b.with_mut(vm, |vm, t| {
                t.check_kind(key.kind())?;
                t.hremove_ref(vm, KeyRef::from(&key))
            })?
        },
    );

    // the `_sv` range twins — the key is a borrowed `(parent, off,
    // len)` BYTE range (the house UTF-8 boundary traps; the range runs
    // BEFORE the transfer, as admission does)
    // the range twins' key-only faces: the same borrowed
    // `(parent, off, len)` BYTE range, no value machinery (HashSet's
    // put/has/remove over a range key)
    rut_vm::pkg_fn!(
        pkg,
        "map_hput_sv",
        (Opaque<NativeTable>, &str, i32, i32) -> i64,
        |vm: &mut Vm, b: Opaque<NativeTable>, parent: &str, off: i32, len: i32| -> Result<i64, Trap> {
            let range = sv_range(parent, off, len)?;
            let key = KeyVal::Str(range.to_owned());
            b.with_mut(vm, |_vm, t| {
                t.check_kind(key.kind())?;
                fused_put(t, key)
            })?
        },
    );
    rut_vm::pkg_fn!(
        pkg,
        "map_hfind_sv",
        (Opaque<NativeTable>, &str, i32, i32) -> i32,
        |_vm: &mut Vm, b: Opaque<NativeTable>, parent: &str, off: i32, len: i32| -> Result<i32, Trap> {
            let key = KeyVal::Str(sv_range(parent, off, len)?.to_owned());
            b.with(|t| {
                t.check_kind(key.kind())?;
                fused_find(t, &key)
            })?
        },
    );
    rut_vm::pkg_fn!(
        pkg,
        "map_hremove_sv",
        (Opaque<NativeTable>, &str, i32, i32) -> i32,
        |vm: &mut Vm, b: Opaque<NativeTable>, parent: &str, off: i32, len: i32| -> Result<i32, Trap> {
            let key = KeyVal::Str(sv_range(parent, off, len)?.to_owned());
            b.with_mut(vm, |vm, t| {
                t.check_kind(key.kind())?;
                fused_remove(vm, t, &key)
            })?
        },
    );

    rut_vm::pkg_fn!(
        pkg,
        "map_hvput_sv",
        (Opaque<NativeTable>, &str, i32, i32, OpaqueRef) -> i64,
        |vm: &mut Vm, b: Opaque<NativeTable>, parent: &str, off: i32, len: i32, v: OpaqueRef| -> Result<i64, Trap> {
            let range = sv_range(parent, off, len)?;
            let key = KeyRef::Str(range);
            b.with_mut(vm, |vm, t| {
                // admission before the transfer, as everywhere (leak law)
                t.check_kind(key.kind())?;
                let sealed = seal_transfer(v, vm)?;
                t.hvput_ref(vm, key, sealed)
            })?
        },
    );
    rut_vm::pkg_fn!(
        pkg,
        "map_hvget_sv",
        (Opaque<NativeTable>, &str, i32, i32) -> Option<OpaqueRef>,
        |vm: &mut Vm, b: Opaque<NativeTable>, parent: &str, off: i32, len: i32| -> Result<Option<OpaqueRef>, Trap> {
            let range = sv_range(parent, off, len)?;
            let stored = b.with(|t| t.hvget_ref(KeyRef::Str(range)))??;
            seal_answer(vm, stored)
        },
    );
    rut_vm::pkg_fn!(
        pkg,
        "map_hvremove_sv",
        (Opaque<NativeTable>, &str, i32, i32) -> i32,
        |vm: &mut Vm, b: Opaque<NativeTable>, parent: &str, off: i32, len: i32| -> Result<i32, Trap> {
            let range = sv_range(parent, off, len)?;
            b.with_mut(vm, |vm, t| t.hremove_ref(vm, KeyRef::Str(range)))?
        },
    );

    // ---- the iteration lanes (the `for..of` law) -----------------------
    // Two crossings. `map_iter_new` snapshots the live keys (BIRTH order —
    // insertion order of live entries) into a cursor box under the
    // wrapper's own `type_id<K>()` (the mint's exact `val_ty`; the flavor
    // mismatch traps by name before any cursor exists). Every
    // `map_iter_next` mints ONE fresh key box ([`mint_key_box`]) and
    // answers it as `?opaque` — nil at the end. The snapshot is pure Rust
    // data: puts/removes inside the emit body never touch the walk, and
    // the cursor drops with the wrapper's loop.
    rut_vm::pkg_fn!(
        pkg,
        "map_iter_new",
        (Opaque<NativeTable>, u32) -> OpaqueRef,
        |vm: &mut Vm, b: Opaque<NativeTable>, ty: u32| -> Result<OpaqueRef, Trap> {
            let kind = iter_key_kind(ty)?;
            let keys = b.with(|t| -> Result<Vec<KeyVal>, Trap> {
                t.admit(kind)?; // the flavor check — a mismatched ty traps by name
                Ok(t.iter_snapshot())
            })??;
            let cur = Opaque::alloc(vm, IterCursor { keys, ty, pos: 0 })?;
            Ok(cur.handle().clone())
        },
    );
    rut_vm::pkg_fn!(
        pkg,
        "map_iter_next",
        (Opaque<IterCursor>) -> Option<OpaqueRef>,
        |vm: &mut Vm, cur: Opaque<IterCursor>| -> Result<Option<OpaqueRef>, Trap> {
            cur.with_mut(vm, |vm, c| {
                let key = match c.keys.get(c.pos) {
                    Some(k) => k,
                    None => return Ok(None), // the walk's end — the flat nil
                };
                c.pos += 1;
                mint_key_box(vm, c.ty, key).map(Some)
            })?
        },
    );
    pkg.build()
}
