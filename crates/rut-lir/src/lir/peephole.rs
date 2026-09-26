//! LIR peephole — RFC 0032 follow-through (the planned `regalloc.rs`
//! neighbourhood): remove the copy flood the tree-walking emitter leaves
//! behind. Today every variable read materialises into a fresh register
//! with `mov`, so straight-line code is full of single-use copies.
//!
//! This pass removes a non-ref `mov dst, src` when:
//!   * `dst` is read exactly once in the whole function,
//!   * that read is after the `mov`, in the same straight-line run
//!     (no control transfer or jump target in between),
//!   * neither `src` nor `dst` is written before the read,
//!   * neither the `mov` nor the read is a jump target.
//! It rewrites the read to `src` and deletes the `mov`, then remaps all
//! labels and spans so the code stays consistent. Ref moves (`movref`)
//! are left alone — their retain/release is not yet provably removable.

use super::Pools;
use rut_core::ops::*;
use rut_core::types::Repr;
use std::collections::HashMap;

/// Run the peephole to a fixed point on one function.
pub(crate) fn run(
    mut code: Vec<Op>,
    mut spans: Vec<(u32, u32)>,
    mut pools: Pools,
) -> (Vec<Op>, Vec<(u32, u32)>, Pools) {
    // a handful of rounds catches chains (a copy feeding a copy)
    for _ in 0..6 {
        let (c, s, changed) = one_round(code, spans, &mut pools);
        code = c;
        spans = s;
        let (c, s, changed2) = forward_once(code, spans, &mut pools);
        code = c;
        spans = s;
        let (c, s, changed3) = fuse_once(code, spans, &mut pools);
        code = c;
        spans = s;
        let (c, s, changed4) = opt_prim_once(code, spans, &mut pools);
        code = c;
        spans = s;
        if !changed && !changed2 && !changed3 && !changed4 {
            break;
        }
    }
    (code, spans, pools)
}

/// Producer forwarding: `OP d, ...; mov y, d` where `d` is defined only by
/// `OP` and read only by the `mov` becomes `OP y, ...` with the `mov`
/// deleted. This is the assignment shape the tree-walking emitter produces
/// for `x = <op>(...)` (the op writes a temp, then a copy lands it in `x`).
fn forward_once(code: Vec<Op>, spans: Vec<(u32, u32)>, pools: &mut Pools) -> (Vec<Op>, Vec<(u32, u32)>, bool) {
    let n = code.len();
    if n == 0 {
        return (code, spans, false);
    }
    let mut reads: HashMap<u16, Vec<usize>> = HashMap::new();
    let mut writes: HashMap<u16, Vec<usize>> = HashMap::new();
    for (pc, op) in code.iter().enumerate() {
        let (d, u) = def_use(op, &pools.argv);
        for r in d {
            writes.entry(r).or_default().push(pc);
        }
        for r in u {
            reads.entry(r).or_default().push(pc);
        }
    }
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
    let mut retarget: HashMap<usize, u16> = HashMap::new();
    for use_pc in 0..n {
        let Op::Mov { dst: y, src: d } = &code[use_pc] else {
            continue;
        };
        let (y, d) = (*y, *d);
        if y == d || is_target[use_pc] {
            continue;
        }
        // the producer is the unique writer of `d`, and the copy is its only
        // reader (otherwise the other readers would lose the definition)
        let Some(ws) = writes.get(&d) else { continue };
        if ws.len() != 1 {
            continue;
        }
        let Some(rs) = reads.get(&d) else { continue };
        if rs.len() != 1 || rs[0] != use_pc {
            continue;
        }
        let pc = ws[0];
        if pc >= use_pc {
            continue;
        }
        // skip copy chains (handled by `one_round`); ref moves must not
        // have their dst retargeted
        if matches!(code[pc], Op::Mov { .. } | Op::MovRef { .. }) {
            continue;
        }
        // straight-line (pc, use_pc]: no jump target or control transfer
        let mut safe = true;
        for k in (pc + 1)..=use_pc {
            if is_target[k]
                || matches!(
                    code[k],
                    Op::Jmp { .. } | Op::Br { .. } | Op::BrTable { .. } | Op::Ret { .. } | Op::LoopHead
                )
            {
                safe = false;
                break;
            }
        }
        if !safe {
            continue;
        }
        // `y` must not be read or written between the producer and the copy
        if reads.get(&y).is_some_and(|rs| rs.iter().any(|&r| r > pc && r < use_pc)) {
            continue;
        }
        if writes.get(&y).is_some_and(|ws| ws.iter().any(|&w| w > pc && w < use_pc)) {
            continue;
        }
        // only one copy per producer
        if retarget.contains_key(&pc) {
            continue;
        }
        removed[use_pc] = true;
        retarget.insert(pc, y);
    }
    if !removed.iter().any(|&b| b) {
        return (code, spans, false);
    }

    let mut kept = vec![false; n];
    let mut new_code = Vec::with_capacity(n);
    for (pc, op) in code.iter().enumerate() {
        if removed[pc] {
            continue;
        }
        let mut op = op.clone();
        if let Some(&y) = retarget.get(&pc) {
            if let Some(slot) = dst_slot(&mut op) {
                *slot = y;
            }
        }
        kept[pc] = true;
        new_code.push(op);
    }
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

/// Field-array fusion: `getf f, obj, k; arrget d, f, i` (or `arrset`), where
/// `f` is a fresh temp read exactly once by that consumer, becomes
/// `arrgetf d, obj, k, i` / `arrsetf obj, k, i, …`. The field handle is then
/// borrowed, not retained/released per element — this is the `Vec<T>` class's
/// index path (RFC 0005 `Slice<T>`), so a pouch sequence costs one
/// op per element, not a field read plus an RC pair.
fn fuse_once(code: Vec<Op>, spans: Vec<(u32, u32)>, pools: &mut Pools) -> (Vec<Op>, Vec<(u32, u32)>, bool) {
    let n = code.len();
    if n < 2 {
        return (code, spans, false);
    }
    let mut reads: HashMap<u16, Vec<usize>> = HashMap::new();
    let mut writes: HashMap<u16, Vec<usize>> = HashMap::new();
    for (pc, op) in code.iter().enumerate() {
        let (d, u) = def_use(op, &pools.argv);
        for r in d {
            writes.entry(r).or_default().push(pc);
        }
        for r in u {
            reads.entry(r).or_default().push(pc);
        }
    }
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
    let mut replace: HashMap<usize, Op> = HashMap::new();
    for pc in 0..n - 1 {
        if is_target[pc] || is_target[pc + 1] {
            continue;
        }
        let Op::GetF { dst: f, obj, field, repr } = code[pc] else { continue };
        if !repr.is_ref() {
            continue;
        }
        if writes.get(&f).map_or(true, |w| w.len() != 1 || w[0] != pc) {
            continue;
        }
        if reads.get(&f).map_or(true, |r| r.len() != 1 || r[0] != pc + 1) {
            continue;
        }
        let fused = match &code[pc + 1] {
            Op::ArrGet { dst, arr, idx, repr: er } if *arr == f => {
                Some(Op::ArrGetF { dst: *dst, obj, field, idx: *idx, repr: *er })
            }
            Op::ArrSet { arr, idx, val, repr: er } if *arr == f => {
                Some(Op::ArrSetF { obj, field, idx: *idx, val: *val, repr: *er })
            }
            _ => None,
        };
        if let Some(op) = fused {
            removed[pc] = true;
            replace.insert(pc + 1, op);
        }
    }
    if !removed.iter().any(|&b| b) {
        return (code, spans, false);
    }
    let mut kept = vec![false; n];
    let mut new_code = Vec::with_capacity(n);
    for (pc, op) in code.iter().enumerate() {
        if removed[pc] {
            continue;
        }
        kept[pc] = true;
        new_code.push(replace.remove(&pc).unwrap_or_else(|| op.clone()));
    }
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

/// The primitive-optional element folds (RFC 0044 §5, the primitive-store
/// tier): local rewrites on `[?prim]` backing stores that remove the box the
/// `T → ?T` coercion and the auto-deref would otherwise mint per element
/// access. Both folds are two-op local windows with the same safety shape as
/// `fuse_once`/`forward_once` (single-use temp, same straight-line run, no
/// jump targets), so control flow never crosses a rewritten pair.
///
/// 1. store elision — `makeopt d, v; arrset[a/f] .., d ..optprim` where `d`
///    feeds only the store becomes `arrset[a/f] .., v ..optprimraw`: the
///    store writes the raw payload, the box never exists. Sound because a
///    primitive-optional box's identity is unobservable (RFC 0044 §3: `?T ==
///    ?T` compares payloads, and the raw store is value semantics by law).
/// 2. deref fold — `arrget[a/f] d, .. ..optprim; getf u, d, 0 ..prim` (the
///    exact pair the auto-deref emits) becomes one
///    `arrget[a/f] u, .. ..optprimload`: the payload is read straight out of
///    the raw store, a nil tag still traps `NilDeref`, and the fresh opt
///    value is never minted.
fn opt_prim_once(mut code: Vec<Op>, spans: Vec<(u32, u32)>, pools: &mut Pools) -> (Vec<Op>, Vec<(u32, u32)>, bool) {
    let n = code.len();
    if n < 2 {
        return (code, spans, false);
    }
    let mut reads: HashMap<u16, Vec<usize>> = HashMap::new();
    let mut writes: HashMap<u16, Vec<usize>> = HashMap::new();
    for (pc, op) in code.iter().enumerate() {
        let (d, u) = def_use(op, &pools.argv);
        for r in d {
            writes.entry(r).or_default().push(pc);
        }
        for r in u {
            reads.entry(r).or_default().push(pc);
        }
    }
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
    let mut replace: HashMap<usize, Op> = HashMap::new();
    for pc in 0..n - 1 {
        if is_target[pc] || is_target[pc + 1] {
            continue;
        }
        // 1. store elision: makeopt + adjacent optprim store
        if let Op::MakeOpt { dst, src, .. } = &code[pc] {
            let (d, s) = (*dst, *src);
            if writes.get(&d).map_or(true, |w| w.len() != 1 || w[0] != pc) {
                continue;
            }
            if reads.get(&d).map_or(true, |r| r.len() != 1 || r[0] != pc + 1) {
                continue;
            }
            let folded = match &code[pc + 1] {
                Op::ArrSet { arr, idx, val, repr: Repr::OptPrim(p) } if *val == d => {
                    Some(Op::ArrSet { arr: *arr, idx: *idx, val: s, repr: Repr::OptPrimRaw(*p) })
                }
                Op::ArrSetF { obj, field, idx, val, repr: Repr::OptPrim(p) } if *val == d => {
                    Some(Op::ArrSetF { obj: *obj, field: *field, idx: *idx, val: s, repr: Repr::OptPrimRaw(*p) })
                }
                _ => None,
            };
            if let Some(op) = folded {
                removed[pc] = true;
                replace.insert(pc + 1, op);
            }
            continue;
        }
        // 2. deref fold: optprim element read + adjacent field-0 payload read
        let (t, p) = match &code[pc] {
            Op::ArrGet { dst: t, repr: Repr::OptPrim(p), .. } => (*t, *p),
            Op::ArrGetF { dst: t, repr: Repr::OptPrim(p), .. } => (*t, *p),
            _ => continue,
        };
        if writes.get(&t).map_or(true, |w| w.len() != 1 || w[0] != pc) {
            continue;
        }
        if reads.get(&t).map_or(true, |r| r.len() != 1 || r[0] != pc + 1) {
            continue;
        }
        // the consumer must be the payload read (field 0, at the payload's
        // own prim repr) — the RFC 0044 auto-deref shape
        let ok = match &code[pc + 1] {
            Op::GetF { dst: _, obj, field: 0, repr: Repr::Prim(q) } => *obj == t && *q == p,
            _ => false,
        };
        if !ok {
            continue;
        }
        let Op::GetF { dst: u, .. } = code[pc + 1] else { unreachable!() };
        // the producer becomes the load form, pointing at the deref's dst:
        // the raw payload comes straight out of the store, a nil tag still
        // traps NilDeref, and the fresh opt value is never minted
        match &mut code[pc] {
            Op::ArrGet { dst, repr, .. } => {
                *dst = u;
                *repr = Repr::OptPrimLoad(p);
            }
            Op::ArrGetF { dst, repr, .. } => {
                *dst = u;
                *repr = Repr::OptPrimLoad(p);
            }
            _ => unreachable!("shape checked above"),
        }
        removed[pc + 1] = true;
    }
    if !removed.iter().any(|&b| b) {
        return (code, spans, false);
    }
    let mut kept = vec![false; n];
    let mut new_code = Vec::with_capacity(n);
    for (pc, op) in code.iter().enumerate() {
        if removed[pc] {
            continue;
        }
        kept[pc] = true;
        new_code.push(replace.remove(&pc).unwrap_or_else(|| op.clone()));
    }
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

fn one_round(code: Vec<Op>, spans: Vec<(u32, u32)>, pools: &mut Pools) -> (Vec<Op>, Vec<(u32, u32)>, bool) {
    let n = code.len();
    if n == 0 {
        return (code, spans, false);
    }

    // per-register reads and writes (pc lists, ascending)
    let mut reads: HashMap<u16, Vec<usize>> = HashMap::new();
    let mut writes: HashMap<u16, Vec<usize>> = HashMap::new();
    for (pc, op) in code.iter().enumerate() {
        let (d, u) = def_use(op, &pools.argv);
        for r in d {
            writes.entry(r).or_default().push(pc);
        }
        for r in u {
            reads.entry(r).or_default().push(pc);
        }
    }

    // jump targets must not be moved onto or off
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
    let mut replace_at: HashMap<usize, Vec<(u16, u16)>> = HashMap::new();

    for pc in 0..n {
        let Op::Mov { dst, src } = &code[pc] else {
            continue;
        };
        let (dst, src) = (*dst, *src);
        if dst == src {
            continue;
        }
        let Some(rd) = reads.get(&dst) else { continue };
        if rd.len() != 1 {
            continue;
        }
        let use_pc = rd[0];
        if use_pc <= pc || is_target.get(use_pc).copied().unwrap_or(false) {
            continue;
        }
        // don't chain copies: rewriting another `mov`'s source would change
        // that op's own removal analysis below
        if matches!(
            &code[use_pc],
            Op::Mov { src, .. } | Op::MovRef { src, .. } if *src == dst
        ) {
            continue;
        }
        // straight-line only: no branch/ret/loophead and no jump target
        // anywhere after the `mov` up to the read. The `mov` itself may be a
        // jump target: deleting it collapses every branch to it onto the next
        // kept op, and the read has already been rewritten to `src`.
        let mut safe = true;
        for k in (pc + 1)..=use_pc {
            if is_target[k]
                || matches!(
                    code[k],
                    Op::Jmp { .. } | Op::Br { .. } | Op::BrTable { .. } | Op::Ret { .. } | Op::LoopHead
                )
            {
                safe = false;
                break;
            }
        }
        if !safe {
            continue;
        }
        // src must not be clobbered before the read; dst must not be
        // written before the read (so the read really sees the copy)
        if writes
            .get(&src)
            .is_some_and(|ws| ws.iter().any(|&w| w > pc && w < use_pc))
        {
            continue;
        }
        if writes
            .get(&dst)
            .is_some_and(|ws| ws.iter().any(|&w| w > pc && w <= use_pc))
        {
            continue;
        }
        removed[pc] = true;
        replace_at.entry(use_pc).or_default().push((dst, src));
    }

    if !removed.iter().any(|&b| b) {
        return (code, spans, false);
    }

    // compact + rewrite the single read at each use site
    let mut kept = vec![false; n];
    let mut new_code = Vec::with_capacity(n);
    for (pc, op) in code.iter().enumerate() {
        if removed[pc] {
            continue;
        }
        let mut op = op.clone();
        if let Some(pairs) = replace_at.get(&pc) {
            for &(from, to) in pairs {
                replace_reads(&mut op, pools, from, to);
            }
        }
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
    let t = (t as usize).min(n);
    targets[t] = true;
}

/// The destination register slot of an op, if it defines one.
fn dst_slot(op: &mut Op) -> Option<&mut u16> {
    match op {
        Op::Mov { dst, .. }
        | Op::MovRef { dst, .. }
        | Op::Const { dst, .. }
        | Op::ConstRaw { dst, .. }
        | Op::StrCmp { dst, .. }
        | Op::ArrayCmp { dst, .. }
        | Op::RefEq { dst, .. }
        | Op::Not { dst, .. }
        | Op::AddF { dst, .. }
        | Op::SubF { dst, .. }
        | Op::MulF { dst, .. }
        | Op::DivF { dst, .. }
        | Op::ModF { dst, .. }
        | Op::NegF { dst, .. }
        | Op::EqF { dst, .. }
        | Op::NeF { dst, .. }
        | Op::LtF { dst, .. }
        | Op::GtF { dst, .. }
        | Op::LeF { dst, .. }
        | Op::GeF { dst, .. }
        | Op::AddI { dst, .. }
        | Op::SubI { dst, .. }
        | Op::MulI { dst, .. }
        | Op::DivI { dst, .. }
        | Op::ModI { dst, .. }
        | Op::WAddI { dst, .. }
        | Op::WSubI { dst, .. }
        | Op::WMulI { dst, .. }
        | Op::AndI { dst, .. }
        | Op::OrI { dst, .. }
        | Op::XorI { dst, .. }
        | Op::ShlI { dst, .. }
        | Op::ShrI { dst, .. }
        | Op::WrapShlI { dst, .. }
        | Op::EqI { dst, .. }
        | Op::NeI { dst, .. }
        | Op::LtI { dst, .. }
        | Op::GtI { dst, .. }
        | Op::LeI { dst, .. }
        | Op::GeI { dst, .. }
        | Op::NegI { dst, .. }
        | Op::NewCell { dst, .. }
        | Op::MakeRecord { dst, .. }
        | Op::GetF { dst, .. }
        | Op::Own { dst, .. }
        | Op::MakeOpt { dst, .. }
        | Op::ArrNew { dst, .. }
        | Op::ArrLit { dst, .. }
        | Op::ArrGet { dst, .. }
        | Op::ArrGetF { dst, .. }
        | Op::EnumNew { dst, .. }
        | Op::TidOf { dst, .. }
        | Op::IsType { dst, .. }
        | Op::IsTrait { dst, .. }
        | Op::Unbox { dst, .. }
        | Op::Box { dst, .. }
        | Op::MakeClosure { dst, .. }
        | Op::Conv { dst, .. }
        | Op::StrCodeAt { dst, .. } => Some(dst),
        Op::Call { dst, .. }
        | Op::CallM { dst, .. }
        | Op::CallI { dst, .. }
        | Op::CallNat { dst, .. }
        | Op::CallFn { dst, .. } => {
            if *dst != NOREG {
                Some(dst)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub(crate) fn def_use(op: &Op, argv: &[Reg]) -> (Vec<u16>, Vec<u16>) {
    let mut d = Vec::new();
    let mut u = Vec::new();
    // pooled operand lists first (the span covers every list-carrying op)
    if let Some((off, argc)) = op.argv_span() {
        u.extend(argv[off as usize..off as usize + argc as usize].iter().copied());
    }
    match op {
        Op::Mov { dst, src } | Op::MovRef { dst, src } => {
            d.push(*dst);
            u.push(*src);
        }
        // MakeOpt defs a fresh box; OnDrop reads both and defs nothing
        Op::MakeOpt { dst, src, .. } => {
            d.push(*dst);
            u.push(*src);
        }
        // WeakNew defs a fresh box and reads the referent; WeakUpgrade
        // defs the answer and reads the box (RFC 0017)
        Op::WeakNew { dst, src, .. } => {
            d.push(*dst);
            u.push(*src);
        }
        Op::WeakUpgrade { recv, dst, .. } => {
            d.push(*dst);
            u.push(*recv);
        }
        Op::OnDrop { obj, cleanup } => {
            u.push(*obj);
            u.push(*cleanup);
        }
        Op::Const { dst, .. } | Op::ConstRaw { dst, .. } | Op::NewCell { dst, .. }
        | Op::ArrNew { dst, .. } | Op::EnumNew { dst, .. } => d.push(*dst),
        Op::ArrLit { dst, .. } | Op::MakeRecord { dst, .. } => d.push(*dst),
        Op::StrCmp { dst, a, b, .. }
        | Op::ArrayCmp { dst, a, b, .. }
        | Op::RefEq { dst, a, b, .. }
        | Op::AddF { dst, a, b, .. }
        | Op::SubF { dst, a, b, .. }
        | Op::MulF { dst, a, b, .. }
        | Op::DivF { dst, a, b, .. }
        | Op::ModF { dst, a, b, .. }
        | Op::EqF { dst, a, b, .. }
        | Op::NeF { dst, a, b, .. }
        | Op::LtF { dst, a, b, .. }
        | Op::GtF { dst, a, b, .. }
        | Op::LeF { dst, a, b, .. }
        | Op::GeF { dst, a, b, .. }
        | Op::AddI { dst, a, b, .. }
        | Op::SubI { dst, a, b, .. }
        | Op::MulI { dst, a, b, .. }
        | Op::DivI { dst, a, b, .. }
        | Op::ModI { dst, a, b, .. }
        | Op::WAddI { dst, a, b, .. }
        | Op::WSubI { dst, a, b, .. }
        | Op::WMulI { dst, a, b, .. }
        | Op::AndI { dst, a, b, .. }
        | Op::OrI { dst, a, b, .. }
        | Op::XorI { dst, a, b, .. }
        | Op::ShlI { dst, a, b, .. }
        | Op::ShrI { dst, a, b, .. }
        | Op::WrapShlI { dst, a, b, .. }
        | Op::EqI { dst, a, b, .. }
        | Op::NeI { dst, a, b, .. }
        | Op::LtI { dst, a, b, .. }
        | Op::GtI { dst, a, b, .. }
        | Op::LeI { dst, a, b, .. }
        | Op::GeI { dst, a, b, .. } => {
            d.push(*dst);
            u.push(*a);
            u.push(*b);
        }
        Op::Not { dst, a } | Op::NegF { dst, a, .. }
        | Op::NegI { dst, a, .. } => {
            d.push(*dst);
            u.push(*a);
        }
        Op::Br { cond, .. } => u.push(*cond),
        Op::BrTable { idx, .. } => u.push(*idx),
        Op::Call { dst, .. } | Op::CallM { dst, .. } | Op::CallI { dst, .. } | Op::CallFn { dst, .. } => {
            if *dst != NOREG {
                d.push(*dst);
            }
        }
        Op::CallNat { recv, dst, .. } => {
            if *recv != NOREG {
                u.push(*recv);
            }
            if *dst != NOREG {
                d.push(*dst);
            }
        }
        Op::Ret { val } => {
            if let Some(x) = val {
                u.push(*x);
            }
        }
        Op::GetF { dst, obj, .. } => {
            d.push(*dst);
            u.push(*obj);
        }
        Op::SetF { obj, val, .. } => {
            u.push(*obj);
            u.push(*val);
        }
        Op::Own { dst, src, .. } => {
            d.push(*dst);
            u.push(*src);
        }
        Op::ArrNew { dst, len, .. } => {
            d.push(*dst);
            u.push(*len);
        }
        Op::ArrGet { dst, arr, idx, .. } => {
            d.push(*dst);
            u.push(*arr);
            u.push(*idx);
        }
        Op::ArrSet { arr, idx, val, .. } => {
            u.push(*arr);
            u.push(*idx);
            u.push(*val);
        }
        Op::ArrGetF { dst, obj, idx, .. } => {
            d.push(*dst);
            u.push(*obj);
            u.push(*idx);
        }
        Op::ArrSetF { obj, idx, val, .. } => {
            u.push(*obj);
            u.push(*idx);
            u.push(*val);
        }
        Op::TidOf { dst, obj } | Op::IsType { dst, obj, .. } | Op::IsTrait { dst, obj, .. } => {
            d.push(*dst);
            u.push(*obj);
        }
        Op::Unbox { dst, box_, .. } => {
            d.push(*dst);
            u.push(*box_);
        }
        Op::Box { dst, val, .. } => {
            d.push(*dst);
            u.push(*val);
        }
        Op::MakeClosure { dst, .. } => d.push(*dst),
        Op::Panic { msg } => u.push(*msg),
        Op::Assert { cond, msg } => {
            u.push(*cond);
            if let Some(x) = msg {
                u.push(*x);
            }
        }
        Op::Conv { dst, src, .. } => {
            d.push(*dst);
            u.push(*src);
        }
        Op::StrCodeAt { dst, s, idx } => {
            d.push(*dst);
            u.push(*s);
            u.push(*idx);
        }
        Op::Jmp { .. } | Op::LoopHead => {}
        #[allow(unreachable_patterns)]
        Op::Pad { .. } => unreachable!("layout pin, never constructed"),
    }
    (d, u)
}

/// Replace exactly the read operands equal to `from` with `to` (defs are
/// never touched).
fn replace_reads(op: &mut Op, pools: &mut Pools, from: u16, to: u16) {
    let f = |r: &mut u16| {
        if *r == from {
            *r = to;
        }
    };
    // pooled lists are cloned, remapped, and re-interned — a shared entry
    // remapped to itself re-interns to the same span, so sharing survives
    if let Some((off, argc)) = op.argv_span() {
        let mut v = pools.argv[off as usize..off as usize + argc as usize].to_vec();
        v.iter_mut().for_each(f);
        let (o2, c2) = pools.args(&v);
        if let Some((dst_off, dst_argc)) = op.argv_span_mut() {
            *dst_off = o2;
            *dst_argc = c2;
        }
    }
    match op {
        Op::StrCmp { a, b, .. }
        | Op::ArrayCmp { a, b, .. }
        | Op::RefEq { a, b, .. }
        | Op::AddF { a, b, .. }
        | Op::SubF { a, b, .. }
        | Op::MulF { a, b, .. }
        | Op::DivF { a, b, .. }
        | Op::ModF { a, b, .. }
        | Op::EqF { a, b, .. }
        | Op::NeF { a, b, .. }
        | Op::LtF { a, b, .. }
        | Op::GtF { a, b, .. }
        | Op::LeF { a, b, .. }
        | Op::GeF { a, b, .. }
        | Op::AddI { a, b, .. }
        | Op::SubI { a, b, .. }
        | Op::MulI { a, b, .. }
        | Op::DivI { a, b, .. }
        | Op::ModI { a, b, .. }
        | Op::WAddI { a, b, .. }
        | Op::WSubI { a, b, .. }
        | Op::WMulI { a, b, .. }
        | Op::AndI { a, b, .. }
        | Op::OrI { a, b, .. }
        | Op::XorI { a, b, .. }
        | Op::ShlI { a, b, .. }
        | Op::ShrI { a, b, .. }
        | Op::WrapShlI { a, b, .. }
        | Op::EqI { a, b, .. }
        | Op::NeI { a, b, .. }
        | Op::LtI { a, b, .. }
        | Op::GtI { a, b, .. }
        | Op::LeI { a, b, .. }
        | Op::GeI { a, b, .. } => {
            f(a);
            f(b);
        }
        Op::Not { a, .. } | Op::NegF { a, .. } | Op::NegI { a, .. } => f(a),
        Op::Mov { src, .. } | Op::MovRef { src, .. } => f(src),
        Op::MakeOpt { src, .. } => f(src),
        Op::OnDrop { obj, cleanup } => {
            f(obj);
            f(cleanup);
        }
        Op::Br { cond, .. } => f(cond),
        Op::BrTable { idx, .. } => f(idx),
        Op::CallNat { recv, .. } => f(recv),
        Op::CallFn { fval, .. } => f(fval),
        Op::Ret { val } => {
            if let Some(x) = val {
                f(x);
            }
        }
        Op::GetF { obj, .. } => f(obj),
        Op::SetF { obj, val, .. } => {
            f(obj);
            f(val);
        }
        Op::Own { src, .. } => f(src),
        Op::ArrNew { len, .. } => f(len),

        Op::ArrGet { arr, idx, .. } => {
            f(arr);
            f(idx);
        }
        Op::ArrSet { arr, idx, val, .. } => {
            f(arr);
            f(idx);
            f(val);
        }
        Op::ArrGetF { obj, idx, .. } => {
            f(obj);
            f(idx);
        }
        Op::ArrSetF { obj, idx, val, .. } => {
            f(obj);
            f(idx);
            f(val);
        }
        Op::TidOf { obj, .. } | Op::IsType { obj, .. } | Op::IsTrait { obj, .. } => f(obj),
        Op::Unbox { box_, .. } => f(box_),
        Op::Box { val, .. } => f(val),

        Op::Panic { msg } => f(msg),
        Op::Assert { cond, msg, .. } => {
            f(cond);
            if let Some(x) = msg {
                f(x);
            }
        }
        Op::Conv { src, .. } => f(src),
        Op::StrCodeAt { s, idx, .. } => {
            f(s);
            f(idx);
        }
        _ => {}
    }
}
