//! `MoveVal` — last-use move elision (P2 of the mapset perf plan).
//!
//! Backward liveness over one function's op list (successors from
//! `Jmp`/`Br`/`BrTable`; the reverse sweep reaches loop back-edges by
//! iterating to a fixpoint). A `CloneVal` whose source register has no
//! later read becomes a `MoveVal` — dst takes over src's reference and
//! src is killed, so the frame-exit release of the copied-from binding
//! no-ops. This is the O(1) form of the rehash shape
//! (`GetF; MovRef; CloneVal old, t1` becomes `MoveVal old, t1`).
//!
//! **The release side alone is not the soundness story.** Liveness on
//! the source proves no double-release (every ref register owns one
//! independent reference, so the killed source needs no alias
//! analysis) — but a `CloneVal` also creates a FRESH cell under the
//! v1.1 copy law, and a move instead makes dst ALIAS the copied cell.
//! The rewrite therefore fires only where the aliasing is provably
//! unobservable:
//!
//! * **Sole ownership** — the source traces through single-use move
//!   links (`mov`/`movref`) to a register whose cell is private to this
//!   frame: a fresh-cell mint (`makerecord`, `cloneval`, `arrnew`,
//!   `arrlit`, a call result) or a value-typed parameter (the caller
//!   deep-copied it). Every chain register is dead after the move, so
//!   dst is the only register that can ever observe (or mutate) the
//!   cell — mutations through dst are exactly the clone's own.
//! * **Field binding with a rebind** — the source traces to
//!   `getf obj.field` (the `let old = self.keys` shape). The field slot
//!   still holds the cell, so the move is fired only when every later
//!   mutation through that same field path — and every mutation through
//!   dst — sits BEHIND a `setf obj.field` that rebinds the field to a
//!   fresh cell (the rehash `self.keys = [nil; new_cap]`). Before the
//!   rebind nothing mutates the shared cell; after it dst is the sole
//!   owner.
//!
//! Everything else stays a `cloneval` — read-after, mutate-after,
//! live-across-loop sources included.

use super::peephole::def_use;
use rut_core::ops::*;
use rut_core::binary::Program;
use std::collections::HashMap;

/// Rewrite `CloneVal` ops whose source is a provably-unobservable move
/// into `MoveVal`. Returns true when anything changed. The rewrite is
/// in-place — the op count never changes, so labels and spans stay
/// valid. Runs after the peephole's fixed point. `n_params` is the
/// function's parameter-register count (params are the first registers,
/// bound contiguously; their cells are frame-private for value types).
pub(crate) fn elide(code: &mut [Op], argv: &[Reg], labels: &[Label], n_params: usize) -> bool {
    let n = code.len();
    if n == 0 {
        return false;
    }
    let analysis = Analysis::build(code, argv, labels);
    let mut sites: Vec<(usize, u16, u16)> = Vec::new(); // (pc, dst, src)
    for (pc, op) in code.iter().enumerate() {
        if let Op::CloneVal { dst, src, .. } = *op {
            // dst == src never occurs (the emitter allocates a fresh
            // dst); guard anyway — the move would release the moved cell
            if dst != src && analysis.eligible(pc, dst, src, n_params) {
                sites.push((pc, dst, src));
            }
        }
    }
    let mut changed = false;
    for &(pc, dst, src) in &sites {
        code[pc] = Op::MoveVal { dst, src };
        changed = true;
    }
    changed
}

/// The test-side debug assert (P2.5): every `MoveVal`'s source register
/// has no later read. Public for the driver dump tests — they run it
/// over whole compiled programs.
pub fn move_srcs_are_dead(prog: &Program) -> bool {
    for f in &prog.funcs {
        if f.host_id.is_some() || f.code.is_empty() {
            continue;
        }
        let a = Analysis::build(&f.code, &f.argv, &f.labels);
        for (pc, op) in f.code.iter().enumerate() {
            if let Op::MoveVal { src, .. } = *op {
                if a.live_out[pc].has(src as usize) {
                    return false;
                }
            }
        }
    }
    true
}

// ---- the backward dataflow ----

/// A register set over one function's file (max reg index in use + 1),
/// as a flat bitset.
struct RegSet {
    bits: Vec<u64>,
}

impl RegSet {
    fn has(&self, r: usize) -> bool {
        self.bits[r / 64] & (1 << (r % 64)) != 0
    }
    fn set(&mut self, r: usize) {
        self.bits[r / 64] |= 1 << (r % 64);
    }
}

/// Liveness + def/use maps for one function.
struct Analysis<'a> {
    code: &'a [Op],
    argv: &'a [Reg],
    /// registers read after each pc (union of successors' live_in)
    live_out: Vec<RegSet>,
    /// registers holding a live value at each pc (before the op runs)
    live_in: Vec<RegSet>,
    defs: HashMap<u16, Vec<usize>>,
    uses: HashMap<u16, Vec<usize>>,
}

impl<'a> Analysis<'a> {
    fn build(code: &'a [Op], argv: &'a [Reg], labels: &'a [Label]) -> Analysis<'a> {
        let n = code.len();
        let mut nregs = 0usize;
        let mut defs: HashMap<u16, Vec<usize>> = HashMap::new();
        let mut uses: HashMap<u16, Vec<usize>> = HashMap::new();
        for (pc, op) in code.iter().enumerate() {
            let (d, u) = def_use(op, argv);
            for r in d {
                nregs = nregs.max(r as usize + 1);
                defs.entry(r).or_default().push(pc);
            }
            for r in u {
                nregs = nregs.max(r as usize + 1);
                uses.entry(r).or_default().push(pc);
            }
        }
        let words = nregs.div_ceil(64).max(1);
        let mut live_in: Vec<Vec<u64>> = vec![vec![0u64; words]; n];
        let mut succs: Vec<usize> = Vec::new();
        let mut changed = true;
        while changed {
            changed = false;
            // reverse sweep: converges in (loop-nesting depth + 1) rounds
            for pc in (0..n).rev() {
                successors(&code[pc], labels, pc, n, &mut succs);
                let mut live = vec![0u64; words];
                // live_out = ∪ live_in of successors
                for &s in &succs {
                    for w in 0..words {
                        live[w] |= live_in[s][w];
                    }
                }
                // live_in = use ∪ (live_out \ def)
                let (d, u) = def_use(&code[pc], argv);
                for r in u {
                    let r = r as usize;
                    live[r / 64] |= 1 << (r % 64);
                }
                for r in d {
                    let r = r as usize;
                    live[r / 64] &= !(1 << (r % 64));
                }
                if live != live_in[pc] {
                    live_in[pc] = live;
                    changed = true;
                }
            }
        }
        let to_bits = |bits: &[u64]| RegSet { bits: bits.to_vec() };
        let mut live_out = Vec::with_capacity(n);
        for pc in 0..n {
            successors(&code[pc], labels, pc, n, &mut succs);
            let mut rs = RegSet { bits: vec![0u64; words] };
            for &s in &succs {
                for (w, b) in live_in[s].iter().enumerate() {
                    rs.bits[w] |= b;
                }
            }
            live_out.push(rs);
        }
        Analysis { code, argv, live_out, live_in: live_in.into_iter().map(|b| to_bits(&b)).collect(), defs, uses }
    }

    /// Is `r` free of later reads at `pc` (exclusive)?
    fn dead_after(&self, pc: usize, r: u16) -> bool {
        !self.live_out[pc].has(r as usize)
    }

    /// Is `r` unread from `pc` (inclusive) on any path?
    fn unread_at(&self, pc: usize, r: u16) -> bool {
        !self.live_in[pc].has(r as usize)
    }

    fn single_def(&self, r: u16) -> Option<usize> {
        match self.defs.get(&r) {
            Some(v) if v.len() == 1 => Some(v[0]),
            _ => None,
        }
    }

    /// The op that defines `r` (single def), if any.
    fn def_op(&self, r: u16) -> Option<&'a Op> {
        self.single_def(r).map(|pc| &self.code[pc])
    }

    /// True when `r`'s only role in the function is this chain: defined
    /// once by a `mov`/`movref` and read exactly once (by the next
    /// chain link). The single-use property IS the no-other-reader
    /// guarantee — a liveness check at the link's own def would always
    /// see the chain's consumer and refuse every chain.
    fn is_move_link(&self, r: u16, def_pc: usize) -> bool {
        matches!(self.code[def_pc], Op::Mov { .. } | Op::MovRef { .. })
            && self.uses.get(&r).map_or(false, |u| u.len() == 1)
    }

    /// Follow single-use `mov`/`movref` links from `src` back to the
    /// register that holds the cell at the copy. Returns `None` when any
    /// link is multi-use or re-defined — the cell then has another live
    /// holder the move cannot account for.
    fn chain_root(&self, src: u16, dst: u16) -> Option<u16> {
        let mut cur = src;
        for _ in 0..16 {
            if cur == dst {
                return None; // self-referential chain — refuse
            }
            let Some(def_pc) = self.single_def(cur) else {
                // no def: a parameter register — frame-private cell
                return Some(cur);
            };
            match self.code[def_pc] {
                Op::Mov { src: prev, .. } | Op::MovRef { src: prev, .. }
                    if self.is_move_link(cur, def_pc) =>
                {
                    cur = prev;
                }
                _ => return Some(cur), // terminal: whatever defines cur
            }
        }
        None // chain too long — refuse
    }

    /// The P2 eligibility decision for one `CloneVal` site.
    fn eligible(&self, pc: usize, dst: u16, src: u16, n_params: usize) -> bool {
        // the source must be dead at the copy (no later read) — the
        // release-side condition every rewrite below shares
        if !self.dead_after(pc, src) {
            return false;
        }
        let Some(root) = self.chain_root(src, dst) else { return false };
        // the root must be unread from the copy onward: a live root is a
        // second observer of the cell (the `let r = p; r.x = 9` shape).
        // root == src is already covered by the dead_after check above —
        // live_in at the copy includes the clone's own use of src.
        if root != src && !self.unread_at(pc, root) {
            return false;
        }
        // parameter root: the caller deep-copied a value argument, so
        // the cell is frame-private — dead root ⇒ dst becomes its only
        // observer, and later mutations through dst are the clone's own
        if self.defs.get(&root).is_none() {
            return (root as usize) < n_params;
        }
        match self.def_op(root) {
            // fresh-cell mint: the cell exists only in the (dead) chain
            Some(
                Op::MakeRecord { .. }
                | Op::CloneVal { .. }
                | Op::ArrNew { .. }
                | Op::ArrLit { .. }
                | Op::Call { .. }
                | Op::CallM { .. }
                | Op::CallFn { .. }
                | Op::CallNat { .. },
            ) => true,
            // field load: the field slot still holds the cell — fire
            // only under the rebind discipline (the rehash shape)
            Some(Op::GetF { obj, field, repr, .. }) if repr.is_ref() => {
                self.field_rebind_covers(pc, dst, *obj, *field)
            }
            _ => false,
        }
    }

    /// Soundness of the `let old = obj.field` move: every later mutation
    /// of the field's cell — direct (`arrsetf obj.field`), through a
    /// `getf` temp (`arrset`/`setf` on the loaded handle), or through
    /// dst — must sit behind a `setf obj.field` that rebinds the field
    /// slot to a fresh cell. Until that rebind the moved binding and the
    /// field alias one cell, so nothing may write through either.
    fn field_rebind_covers(&self, pc: usize, dst: u16, obj: u16, field: u32) -> bool {
        // the field's owner must not be re-defined (a re-defined obj
        // would detach the rebind analysis from the register)
        if self.defs.get(&obj).map_or(false, |v| v.len() != 1) {
            return false;
        }
        let mut rebind_seen = false;
        for m in (pc + 1)..self.code.len() {
            match self.code[m] {
                // the rebind itself: dst becomes sole owner past this op
                Op::SetF { obj: o, field: f, .. } if o == obj && f == field => {
                    rebind_seen = true;
                }
                Op::ArrSetF { obj: o, field: f, .. } if o == obj && f == field => {
                    if !rebind_seen {
                        return false;
                    }
                }
                Op::ArrSet { arr, .. } => {
                    if arr == dst && !rebind_seen {
                        return false;
                    }
                    if self.temp_of(arr) == Some((obj, field)) && !rebind_seen {
                        return false;
                    }
                    if self.flows_from(arr, dst) && !rebind_seen {
                        return false;
                    }
                }
                Op::SetF { obj: o, field: _, .. } => {
                    if self.temp_of(o) == Some((obj, field)) && !rebind_seen {
                        return false; // mutating a sub-object of the cell
                    }
                    if self.flows_from(o, dst) && !rebind_seen {
                        return false;
                    }
                }
                Op::ArrGetRef { arr, .. } => {
                    // a for-of box aliases an element slot — writes
                    // through the box reach the cell's elements
                    if (self.temp_of(arr) == Some((obj, field)) || self.flows_from(arr, dst))
                        && !rebind_seen
                    {
                        return false;
                    }
                }
                _ => {}
            }
        }
        true
    }

    /// `(obj, field)` when `r` is (a single-def chain ending at) a field
    /// load — the temp whose handle aliases `obj.field`'s cell.
    fn temp_of(&self, r: u16) -> Option<(u16, u32)> {
        let mut cur = r;
        for _ in 0..16 {
            let def_pc = self.single_def(cur)?;
            match &self.code[def_pc] {
                Op::Mov { src, .. } | Op::MovRef { src, .. } => cur = *src,
                Op::GetF { obj, field, .. } => return Some((*obj, *field)),
                _ => return None,
            }
        }
        None
    }

    /// Whether `r` is (a single-def chain from) `from` — dst flowing
    /// into a mutation target.
    fn flows_from(&self, r: u16, from: u16) -> bool {
        let mut cur = r;
        for _ in 0..16 {
            if cur == from {
                return true;
            }
            let Some(def_pc) = self.single_def(cur) else { return false };
            match &self.code[def_pc] {
                Op::Mov { src, .. } | Op::MovRef { src, .. } | Op::GetF { obj: src, .. } => {
                    cur = *src
                }
                _ => return false,
            }
        }
        false
    }
}

fn successors(op: &Op, labels: &[Label], pc: usize, n: usize, out: &mut Vec<usize>) {
    out.clear();
    match op {
        Op::Jmp { target } => out.push(*target as usize),
        Op::Br { then_t, else_t, .. } => {
            out.push(*then_t as usize);
            out.push(*else_t as usize);
        }
        Op::BrTable { table_off, count, default, .. } => {
            for t in &labels[*table_off as usize..*table_off as usize + *count as usize] {
                out.push(*t as usize);
            }
            out.push(*default as usize);
        }
        Op::Ret { .. } => {}
        // fallthrough (the verifier rejects falling off the end)
        _ => {
            if pc + 1 < n {
                out.push(pc + 1);
            }
        }
    }
}
