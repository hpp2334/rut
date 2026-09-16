//! The VM-owned block store (RFC 0039's completion): variable-size payload
//! memory. A cell's slot in the arena stays one fixed-size record; every
//! variable-size part of it — string octets, array element runs — lives in
//! a *block* carved from size-classed pages.
//!
//! Blocks are 1:1 with their owning cell (the cell is the RC unit; the
//! block's lifetime is its cell's), so blocks carry no refcount of their
//! own: alloc → use → free when the release path drops the cell. Freeing
//! is explicit — the release walk and `Arena::drop` call `free` with the
//! `&Arena` they already hold; block payloads have no `Drop` glue.
//!
//! - **Small blocks** (≤ `SMALL_MAX`): segregated free lists per size
//!   class, no coalescing. A freed block returns to its class list; the
//!   next allocation of that class reuses it. In-place growth within a
//!   class is what keeps append-accumulation loops linear (the fasta
//!   contract).
//! - **Large blocks**: dedicated boxed allocations, freed wholesale.
//! - **Blocks never move** (RFC 0016 OQ-1): a block's address is stable
//!   for its lifetime, so raw `&[u8]`/`&mut [u8]` into the store stay
//!   valid — the same non-moving guarantee the arena gives cell slots.
//!
//! Layout: every block is an 8-byte header (`cap`) followed by the
//! payload, 8-byte aligned. `cap` is the CLASS-rounded capacity — the
//! free path re-derives the class from it with no side table, which is
//! why the header must carry the class value itself, never a raw
//! request size.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::OnceLock;

/// Size classes (bytes, multiples of 8). Above `SMALL_MAX` a block gets
/// its own allocation instead of a class.
const CLASSES: [usize; 24] = [
    16, 32, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 640, 768, 896,
    1024, 1280, 1536, 1792, 2048,
];
const SMALL_MAX: usize = *CLASSES.last().unwrap();

/// Page size in u64 words (32 KiB) — the carving granularity for small
/// blocks. Zeroed at creation, so freshly carved memory is deterministic
/// (recycled memory is not re-zeroed: `len` bounds every read).
const PAGE_WORDS: usize = 4096;

/// Header bytes before each payload: `cap: u32` + 4 pad, so the payload
/// starts 8-aligned.
const HDR: usize = 8;

fn stats_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RUT_BLOCKS_STATS").is_some())
}

pub(crate) struct Blocks {
    /// carved for small blocks
    pages: RefCell<Vec<Box<[u64; PAGE_WORDS]>>>,
    /// word offset of the bump frontier in the LAST page; `PAGE_WORDS`
    /// means "no room — carve a fresh page"
    bump: Cell<usize>,
    /// per-class free lists — a freed small block stores the next free
    /// pointer in its own payload
    free: RefCell<Vec<Vec<*mut u8>>>,
    /// dedicated large blocks, keyed by payload address (dropped wholesale)
    dedicated: RefCell<HashMap<usize, Box<[u64]>>>,
    /// (allocs, grows) — printed at drop under RUT_BLOCKS_STATS
    stats: Cell<(u64, u64)>,
    /// single-slot LIFO free cache — the churn pattern (a temp dies, the
    /// same size is minted again) allocs straight from here with no
    /// `RefCell` borrow and no `Vec` push/pop
    recent: Cell<(*mut u8, usize)>, // (payload, class bytes); null = empty
}

impl Drop for Blocks {
    fn drop(&mut self) {
        if stats_on() {
            let (a, g) = self.stats.get();
            eprintln!("blocks: {a} allocs, {g} grows");
        }
    }
}

impl Blocks {
    pub(crate) fn new() -> Blocks {
        Blocks {
            pages: RefCell::new(Vec::new()),
            bump: Cell::new(PAGE_WORDS),
            free: RefCell::new(vec![Vec::new(); CLASSES.len()]),
            dedicated: RefCell::new(HashMap::new()),
            stats: Cell::new((0, 0)),
            recent: Cell::new((std::ptr::null_mut(), 0)),
        }
    }

    fn class_of(req: usize) -> usize {
        CLASSES.iter().position(|&c| c >= req).expect("caller checks SMALL_MAX")
    }

    /// A payload block with at least `len` usable bytes. The caller
    /// initializes the contents.
    pub(crate) fn alloc(&self, len: usize) -> *mut u8 {
        let req = (len.max(1) + 7) & !7;
        if req > SMALL_MAX {
            return self.alloc_dedicated(req);
        }
        // the header stores the CLASS value, never the raw request — `free`
        // re-derives the class from the header, so a 72-byte request must
        // still carry cap 80 or the next 80-byte alloc of that class
        // overflows into the neighbouring block
        let need = CLASSES[Self::class_of(req)];
        // the recent-free cache first: same class, no borrow, no Vec ops
        let (rp, rcap) = self.recent.get();
        if !rp.is_null() && rcap == need {
            self.recent.set((std::ptr::null_mut(), 0));
            return rp;
        }
        if let Some(p) = self.free.borrow_mut()[Self::class_of(need)].pop() {
            return p;
        }
        // carve: header + payload, in whole words, bump-allocated in the
        // last page (a fresh page when it is full)
        let words = (need + HDR + 7) / 8;
        let mut off = self.bump.get();
        let mut pages = self.pages.borrow_mut();
        if off + words > PAGE_WORDS {
            pages.push(Box::new([0u64; PAGE_WORDS]));
            off = 0;
        }
        let last = pages.len() - 1;
        let base = unsafe { pages[last].as_mut_ptr().add(off) };
        self.bump.set(off + words);
        drop(pages);
        unsafe {
            // `base` strides in u64 words — switch to bytes BEFORE applying
            // the header size, or the payload lands 64 bytes in and blocks
            // overlap
            let b = base as *mut u8;
            *(b as *mut u32) = need as u32; // cap
            b.add(HDR)
        }
    }

    fn alloc_dedicated(&self, need: usize) -> *mut u8 {
        let words = (need + HDR + 7) / 8;
        let mut b = vec![0u64; words].into_boxed_slice();
        let payload = unsafe { (b.as_mut_ptr() as *mut u8).add(HDR) };
        unsafe { *(b.as_mut_ptr() as *mut u32) = need as u32 };
        self.dedicated.borrow_mut().insert(payload as usize, b);
        payload
    }

    /// The payload capacity of a block (class-rounded for small blocks).
    #[inline]
    pub(crate) fn cap_of(&self, p: *mut u8) -> usize {
        unsafe { *(p.sub(HDR) as *const u32) as usize }
    }

    /// Grow a block to hold at least `want` bytes, geometrically: the new
    /// capacity is `max(want, 2 * current cap)`, class-rounded — so
    /// appending one byte at a time touches the allocator O(log n) times,
    /// not O(n). Returns the (possibly moved) payload; the first `old_len`
    /// bytes are preserved. The caller updates its stored pointer.
    pub(crate) fn grow(&self, p: *mut u8, old_len: usize, want: usize) -> *mut u8 {
        let c = self.cap_of(p);
        if want <= c {
            return p;
        }
        if stats_on() {
            let (a, g) = self.stats.get();
            self.stats.set((a, g + 1));
        }
        let target = want.max(c.saturating_mul(2));
        let np = self.alloc(target);
        unsafe { std::ptr::copy_nonoverlapping(p as *const u8, np, old_len) };
        self.free(p);
        np
    }

    /// Return a block. The payload's first 8 bytes become the free-list
    /// link — dead memory, safe to scribble (the cell is already dead).
    pub(crate) fn free(&self, p: *mut u8) {
        let c = self.cap_of(p);
        if c > SMALL_MAX {
            self.dedicated.borrow_mut().remove(&(p as usize));
            return;
        }
        // feed the single-slot cache (evicting anything it held to the list)
        let (rp, rcap) = self.recent.get();
        if rp.is_null() {
            self.recent.set((p, c));
            return;
        }
        if let Some(cls) = CLASSES.iter().position(|&x| x == rcap) {
            self.free.borrow_mut()[cls].push(rp);
        } else {
            self.dedicated.borrow_mut().remove(&(rp as usize));
        }
        self.recent.set((p, c));
    }
}
