//! SROA — scalar replacement of non-escaping record literals (RFC 0009).
//!
//! A `MakeRecord` whose register is read only by `GetF` of non-reference
//! fields is a pure local: no other value can observe its identity, no ref
//! field escapes through it, and no field is mutated. So the allocation and
//! the reads collapse to plain moves and the record itself disappears.
//!
//! `let p = Point { x: i, y: i + 1 }; s += p.x + p.y;` becomes
//! `s += i + (i + 1)` — zero heap traffic. This is the static-typing
//! dividend a JS engine can only bank speculatively (with deopt); rut knows
//! the field types and that no alias can exist, so it emits the scalar
//! version as a fact.
//!
//! Conservative by construction: anything that could observe identity
//! (a call argument, `RefEq`, `is`, `TidOf`, `opaque(..)`, a store, a
//! return), a ref-typed field read, or a redefined field value register
//! disqualifies the record and it is left as a real allocation.
//!
//! The return-position window (err-channel phase 1) extends the same
//! machinery past that disqualification list: after the checker-level
//! inliner has spliced callee into caller, the dominant surviving mint is
//! the `(T, err)` pair that crosses the return boundary — minted, handed
//! off through `Mov`/`MovRef` copies (the ret-slot handoff), and read only
//! by the caller's destructuring `GetF`s. `ret_round` walks such chains
//! (mint → copies → destructure) and consumes them: the components pass
//! through their existing registers, the mint and the handoff copies die,
//! and each `GetF` becomes the move it always was semantically — `Mov` for
//! a prim field (the pair unboxes entirely: no allocation, no rc traffic),
//! `MovRef` for a ref field (one retain at the handoff instead of the
//! record cell + its ref-field retains + the cell's death release; never a
//! borrow across the join). A use that is not part of the chain or the
//! destructure — a call argument, a store, a real `Ret`, an identity probe,
//! a second writer on the slot (the multi-writer join) — declines the whole
//! window, so stored containers and genuinely-returned pairs still mint.
//! Pure lowering: no op change, no format change, no interpreter change.

use super::peephole::def_use;
use super::Pools;
use rut_core::ops::*;
use rut_core::types::Repr;
use std::collections::{HashMap, VecDeque};

/// Run SROA to a fixed point on one function: the scalar-replacement
/// round and the return-position destructure round alternate (each can
/// expose the other — a fused handoff leaves the nested mint getf-only;
/// a scalar-replaced binding leaves a shorter handoff), so neither pass
/// starves.
pub(crate) fn run(
    mut code: Vec<Op>,
    mut spans: Vec<(u32, u32)>,
    pools: &mut Pools,
) -> (Vec<Op>, Vec<(u32, u32)>) {
    for _ in 0..4 {
        let (c1, s1, sroa_changed) = one_round(code, spans, pools);
        let (c2, s2, ret_changed) = ret_round(c1, s1, pools);
        code = c2;
        spans = s2;
        if !sroa_changed && !ret_changed {
            break;
        }
    }
    (code, spans)
}

fn one_round(
    code: Vec<Op>,
    spans: Vec<(u32, u32)>,
    pools: &mut Pools,
) -> (Vec<Op>, Vec<(u32, u32)>, bool) {
    let n = code.len();
    if n == 0 {
        return (code, spans, false);
    }

    // per-register def/use sites (ascending)
    let mut defs: HashMap<u16, Vec<usize>> = HashMap::new();
    let mut uses: HashMap<u16, Vec<usize>> = HashMap::new();
    for (pc, op) in code.iter().enumerate() {
        let (d, u) = def_use(op, &pools.argv);
        for r in d {
            defs.entry(r).or_default().push(pc);
        }
        for r in u {
            uses.entry(r).or_default().push(pc);
        }
    }

    // jump targets must not be deleted (a branch into the allocation would
    // otherwise land mid-sequence) — mirrors the peephole's guard
    let mut is_target = vec![false; n + 1];
    for op in &code {
        match op {
            Op::Jmp { target } => mark(&mut is_target, *target, n),
            Op::Br { then_t, else_t, .. } => {
                mark(&mut is_target, *then_t, n);
                mark(&mut is_target, *else_t, n);
            }
            Op::BrTable { table_off, count, default, .. } => {
                let arms = &pools.labels[*table_off as usize..*table_off as usize + *count as usize];
                for t in arms {
                    mark(&mut is_target, *t, n);
                }
                mark(&mut is_target, *default, n);
            }
            _ => {}
        }
    }

    let mut removed = vec![false; n];
    // use_pc -> the `Mov { dst, src }` that replaces `GetF dst, rec, field`
    let mut replace_getf: HashMap<usize, (u16, u16)> = HashMap::new();

    for pc in 0..n {
        let Op::MakeRecord { dst: p, argv_off, argc, .. } = &code[pc] else {
            continue;
        };
        let p = *p;
        let vals: &[Reg] = &pools.argv[*argv_off as usize..*argv_off as usize + *argc as usize];
        if is_target[pc] || defs.get(&p).map_or(true, |d| d.len() != 1) {
            continue;
        }
        // no uses at all: a dead allocation — leave it for the dead-op
        // rounds rather than reasoning about unreachable code here
        let Some(use_pcs) = uses.get(&p) else { continue };
        let mut ok = true;
        let mut repl: Vec<(usize, u16, u16)> = Vec::new();
        for &j in use_pcs {
            if j <= pc {
                ok = false;
                break;
            }
            let Op::GetF { dst: g, obj, field, repr } = &code[j] else {
                ok = false;
                break;
            };
            if *obj != p || repr.is_ref() || *g == p {
                ok = false;
                break;
            }
            let i = *field as usize;
            if i >= vals.len() {
                ok = false;
                break;
            }
            let src = vals[i];
            if src == p {
                ok = false;
                break;
            }
            // the value register must still hold the field at the read
            if defs
                .get(&src)
                .is_some_and(|ds| ds.iter().any(|&w| w > pc && w < j))
            {
                ok = false;
                break;
            }
            if replace_getf.contains_key(&j) {
                ok = false;
                break;
            }
            repl.push((j, *g, src));
        }
        if !ok {
            continue;
        }
        removed[pc] = true;
        for (j, g, src) in repl {
            replace_getf.insert(j, (g, src));
        }
    }

    if !removed.iter().any(|&b| b) {
        return (code, spans, false);
    }

    // compact, replacing each claimed GetF with a move
    let mut kept = vec![false; n];
    let mut new_code = Vec::with_capacity(n);
    for (pc, op) in code.iter().enumerate() {
        if removed[pc] {
            continue;
        }
        let op = if let Some(&(g, src)) = replace_getf.get(&pc) {
            Op::Mov { dst: g, src }
        } else {
            op.clone()
        };
        kept[pc] = true;
        new_code.push(op);
    }

    // old pc -> new pc (removed ops collapse onto the next kept op)
    let mut old_to_new = vec![0u32; n + 1];
    let mut c = 0u32;
    for pc in 0..n {
        old_to_new[pc] = c;
        if kept[pc] {
            c += 1;
        }
    }
    old_to_new[n] = c;

    for op in new_code.iter_mut() {
        match op {
            Op::Jmp { target } => *target = old_to_new[(*target as usize).min(n)],
            Op::Br { then_t, else_t, .. } => {
                *then_t = old_to_new[(*then_t as usize).min(n)];
                *else_t = old_to_new[(*else_t as usize).min(n)];
            }
            Op::BrTable { table_off, count, default, .. } => {
                let arms = &mut pools.labels[*table_off as usize..*table_off as usize + *count as usize];
                for t in arms.iter_mut() {
                    *t = old_to_new[(*t as usize).min(n)];
                }
                *default = old_to_new[(*default as usize).min(n)];
            }
            _ => {}
        }
    }

    let mut new_spans = Vec::with_capacity(spans.len());
    for (pc, lo) in spans {
        let pc = pc as usize;
        if pc < n && kept[pc] {
            new_spans.push((old_to_new[pc], lo));
        }
    }

    (new_code, new_spans, true)
}

/// The return-position destructure round (err-channel phase 1): consume
/// `MakeRecord → (Mov | MovRef)+ → GetF*` chains — the inlined shape of
/// every `(T, err)` return destructured at the call site.
///
/// Soundness, in order of weight:
/// * every register the window owns (the mint dst plus each handoff copy)
///   is statically single-def, and the ops that write it are exactly the
///   ops deleted — so dynamically those slots only ever held the fused
///   records, and post-fusion hold nothing; the deletions remove exactly
///   the record's own rc traffic (mint retains, handoff retains, death
///   releases) and nothing else;
/// * a ref-typed component is retained at each read by the `MovRef`
///   replacement — the identical discipline of the `GetF` it replaces
///   (`retain iff repr.is_ref()`) — so the handoff is a copy, never a
///   borrow across the join;
/// * the component register must still hold the field at every read (no
///   redefinition between mint and read — the scalar-replacement guard);
/// * any use outside the chain/destructure shape, a second writer on a
///   slot, a branch into the mint, or a window owning no destructure
///   declines the whole thing.
fn ret_round(
    code: Vec<Op>,
    spans: Vec<(u32, u32)>,
    pools: &mut Pools,
) -> (Vec<Op>, Vec<(u32, u32)>, bool) {
    let n = code.len();
    if n == 0 {
        return (code, spans, false);
    }

    // per-register def/use sites (ascending)
    let mut defs: HashMap<u16, Vec<usize>> = HashMap::new();
    let mut uses: HashMap<u16, Vec<usize>> = HashMap::new();
    for (pc, op) in code.iter().enumerate() {
        let (d, u) = def_use(op, &pools.argv);
        for r in d {
            defs.entry(r).or_default().push(pc);
        }
        for r in u {
            uses.entry(r).or_default().push(pc);
        }
    }

    // jump targets must not move onto or off a deleted op un-remapped —
    // the compaction below remaps them, mirroring the peephole's guard
    let mut is_target = vec![false; n + 1];
    for op in &code {
        match op {
            Op::Jmp { target } => mark(&mut is_target, *target, n),
            Op::Br { then_t, else_t, .. } => {
                mark(&mut is_target, *then_t, n);
                mark(&mut is_target, *else_t, n);
            }
            Op::BrTable { table_off, count, default, .. } => {
                let arms = &pools.labels[*table_off as usize..*table_off as usize + *count as usize];
                for t in arms {
                    mark(&mut is_target, *t, n);
                }
                mark(&mut is_target, *default, n);
            }
            _ => {}
        }
    }

    let mut removed = vec![false; n];
    // use_pc -> the move that replaces the `GetF` (mov for a prim field,
    // movref for a ref field)
    let mut replace_getf: HashMap<usize, Op> = HashMap::new();

    for pc in 0..n {
        let Op::MakeRecord { dst: p, argv_off, argc, .. } = &code[pc] else {
            continue;
        };
        let p = *p;
        let vals: &[Reg] = &pools.argv[*argv_off as usize..*argv_off as usize + *argc as usize];
        if vals.is_empty()
            || is_target[pc]
            || vals.contains(&p)
            || defs.get(&p).map_or(true, |d| d.len() != 1)
        {
            continue;
        }
        // walk the handoff chain: (register, its single def site). The
        // mint dst's copies are the first hop; each copy's dst must again
        // be single-def with every use after its def.
        let mut visited: Vec<u16> = vec![p];
        let mut chain_pcs: Vec<usize> = Vec::new();
        // terminals: (use_pc, getf_dst, field, repr)
        let mut terminals: Vec<(usize, u16, u32, Repr)> = Vec::new();
        let mut queue: VecDeque<(u16, usize)> = VecDeque::new();
        queue.push_back((p, pc));
        let mut ok = true;
        'walk: while let Some((r, rpc)) = queue.pop_front() {
            let Some(us) = uses.get(&r) else {
                // dies unread — not a destructure; the dead-op rounds own it
                ok = false;
                break 'walk;
            };
            if us.iter().any(|&j| j <= rpc) {
                ok = false;
                break 'walk;
            }
            let mut all_getf = true;
            let mut all_copy = true;
            for &j in us {
                match &code[j] {
                    Op::GetF { obj, .. } if *obj == r => all_copy = false,
                    Op::Mov { src, .. } | Op::MovRef { src, .. } if *src == r => all_getf = false,
                    _ => {
                        // anything else observes the record: escape
                        ok = false;
                        break 'walk;
                    }
                }
            }
            if all_copy && !all_getf {
                for &j in us {
                    let si = match &code[j] {
                        Op::Mov { dst, .. } | Op::MovRef { dst, .. } => *dst,
                        _ => unreachable!("classified above"),
                    };
                    let good = si != r
                        && !visited.contains(&si)
                        && defs.get(&si).map_or(false, |d| d.len() == 1)
                        && uses.get(&si).map_or(false, |uu| uu.iter().all(|&u| u > j));
                    if !good {
                        ok = false;
                        break 'walk;
                    }
                    visited.push(si);
                    chain_pcs.push(j);
                    queue.push_back((si, j));
                }
            } else if all_getf {
                for &j in us {
                    if let Op::GetF { dst: g, obj, field, repr } = &code[j] {
                        debug_assert!(*obj == r);
                        terminals.push((j, *g, *field, *repr));
                    }
                }
            } else {
                // mixed copies and reads: the record escapes one way or
                // the other — decline
                ok = false;
            }
        }
        // the pre-registered window hands the record through at least one
        // copy (the ret-slot shape); the chain-free destructure is the
        // scalar-replacement round's territory
        if !ok || chain_pcs.is_empty() || terminals.is_empty() {
            continue;
        }
        // validate every destructure read against the mint's components
        for &(j, g, field, _) in &terminals {
            let i = field as usize;
            if i >= vals.len() || g == p {
                ok = false;
                break;
            }
            let src = vals[i];
            if src == p || visited.contains(&src) {
                ok = false;
                break;
            }
            // the component register must still hold the field at the read
            if defs
                .get(&src)
                .is_some_and(|ds| ds.iter().any(|&w| w > pc && w < j))
            {
                ok = false;
                break;
            }
            if replace_getf.contains_key(&j) {
                ok = false;
                break;
            }
        }
        if !ok {
            continue;
        }
        removed[pc] = true;
        for &j in &chain_pcs {
            removed[j] = true;
        }
        for &(j, g, field, repr) in &terminals {
            let src = vals[field as usize];
            let op = if repr.is_ref() {
                Op::MovRef { dst: g, src }
            } else {
                Op::Mov { dst: g, src }
            };
            replace_getf.insert(j, op);
        }
    }

    if !removed.iter().any(|&b| b) {
        return (code, spans, false);
    }

    // compact, replacing each claimed GetF with its move
    let mut kept = vec![false; n];
    let mut new_code = Vec::with_capacity(n);
    for (pc, op) in code.iter().enumerate() {
        if removed[pc] {
            continue;
        }
        let op = replace_getf.get(&pc).cloned().unwrap_or_else(|| op.clone());
        kept[pc] = true;
        new_code.push(op);
    }

    // old pc -> new pc (removed ops collapse onto the next kept op)
    let mut old_to_new = vec![0u32; n + 1];
    let mut c = 0u32;
    for pc in 0..n {
        old_to_new[pc] = c;
        if kept[pc] {
            c += 1;
        }
    }
    old_to_new[n] = c;

    for op in new_code.iter_mut() {
        match op {
            Op::Jmp { target } => *target = old_to_new[(*target as usize).min(n)],
            Op::Br { then_t, else_t, .. } => {
                *then_t = old_to_new[(*then_t as usize).min(n)];
                *else_t = old_to_new[(*else_t as usize).min(n)];
            }
            Op::BrTable { table_off, count, default, .. } => {
                let arms = &mut pools.labels[*table_off as usize..*table_off as usize + *count as usize];
                for t in arms.iter_mut() {
                    *t = old_to_new[(*t as usize).min(n)];
                }
                *default = old_to_new[(*default as usize).min(n)];
            }
            _ => {}
        }
    }

    let mut new_spans = Vec::with_capacity(spans.len());
    for (pc, lo) in spans {
        let pc = pc as usize;
        if pc < n && kept[pc] {
            new_spans.push((old_to_new[pc], lo));
        }
    }

    (new_code, new_spans, true)
}

fn mark(targets: &mut [bool], t: u32, n: usize) {
    targets[(t as usize).min(n)] = true;
}
