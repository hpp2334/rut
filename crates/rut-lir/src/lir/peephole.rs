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

use rut_core::ops::*;
use std::collections::HashMap;

/// Run the peephole to a fixed point on one function.
pub(crate) fn run(mut code: Vec<Op>, mut spans: Vec<(u32, u32)>) -> (Vec<Op>, Vec<(u32, u32)>) {
    // a handful of rounds catches chains (a copy feeding a copy)
    for _ in 0..4 {
        let (c, s, changed) = one_round(code, spans);
        code = c;
        spans = s;
        if !changed {
            break;
        }
    }
    (code, spans)
}

fn one_round(code: Vec<Op>, spans: Vec<(u32, u32)>) -> (Vec<Op>, Vec<(u32, u32)>, bool) {
    let n = code.len();
    if n == 0 {
        return (code, spans, false);
    }

    // per-register reads and writes (pc lists, ascending)
    let mut reads: HashMap<u16, Vec<usize>> = HashMap::new();
    let mut writes: HashMap<u16, Vec<usize>> = HashMap::new();
    for (pc, op) in code.iter().enumerate() {
        let (d, u) = def_use(op);
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
            Op::BrTable { table, default, .. } => {
                for t in table {
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
        if dst == src || is_target[pc] {
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
        if matches!(&code[use_pc], Op::Mov { src, .. } | Op::MovRef { src, .. } if *src == dst) {
            continue;
        }
        // straight-line only: no branch/ret/loophead and no jump target
        // anywhere in [pc, use_pc]
        let mut safe = true;
        for k in pc..=use_pc {
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
                replace_reads(&mut op, from, to);
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
            Op::BrTable { table, default, .. } => {
                for t in table.iter_mut() {
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

pub(crate) fn def_use(op: &Op) -> (Vec<u16>, Vec<u16>) {
    let mut d = Vec::new();
    let mut u = Vec::new();
    match op {
        Op::Mov { dst, src } | Op::MovRef { dst, src } => {
            d.push(*dst);
            u.push(*src);
        }
        Op::Const { dst, .. } | Op::ConstRaw { dst, .. } | Op::NewCell { dst, .. }
        | Op::ArrNew { dst, .. } | Op::EnumNew { dst, .. }
        | Op::OptNone { dst, .. } => d.push(*dst),
        Op::ArrLit { dst, elems, .. } => {
            d.push(*dst);
            u.extend(elems.iter().copied());
        }
        Op::MakeRecord { dst, vals, .. } => {
            d.push(*dst);
            u.extend(vals.iter().copied());
        }
        Op::Arith { dst, a, b, .. }
        | Op::Wrap { dst, a, b, .. }
        | Op::Bit { dst, a, b, .. }
        | Op::Cmp { dst, a, b, .. }
        | Op::StrCmp { dst, a, b, .. }
        | Op::RefEq { dst, a, b, .. } => {
            d.push(*dst);
            u.push(*a);
            u.push(*b);
        }
        Op::Not { dst, a } | Op::Neg { dst, a, .. } => {
            d.push(*dst);
            u.push(*a);
        }
        Op::Br { cond, .. } => u.push(*cond),
        Op::BrTable { idx, .. } => u.push(*idx),
        Op::Call { args, dst, .. } => {
            u.extend(args.iter().copied());
            if let Some(x) = dst {
                d.push(*x);
            }
        }
        Op::CallM { recv, args, dst, .. } | Op::CallI { recv, args, dst, .. } => {
            u.push(*recv);
            u.extend(args.iter().copied());
            if let Some(x) = dst {
                d.push(*x);
            }
        }
        Op::CallNat { recv, args, dst, .. } => {
            if let Some(x) = recv {
                u.push(*x);
            }
            u.extend(args.iter().copied());
            if let Some(x) = dst {
                d.push(*x);
            }
        }
        Op::CallFn { fval, args, dst } => {
            u.push(*fval);
            u.extend(args.iter().copied());
            if let Some(x) = dst {
                d.push(*x);
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
        Op::OptSome { dst, val, .. } | Op::ResOk { dst, val, .. } | Op::ResErr { dst, val, .. } => {
            d.push(*dst);
            u.push(*val);
        }
        Op::SumIs { dst, v, .. } | Op::Unwrap { dst, v, .. } => {
            d.push(*dst);
            u.push(*v);
        }
        Op::UnwrapOr { dst, v, default } => {
            d.push(*dst);
            u.push(*v);
            u.push(*default);
        }
        Op::Expect { dst, v, msg } => {
            d.push(*dst);
            u.push(*v);
            u.push(*msg);
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
        Op::MakeClosure { dst, captures, .. } => {
            d.push(*dst);
            u.extend(captures.iter().copied());
        }
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
        Op::StrCharAt { dst, s, idx } => {
            d.push(*dst);
            u.push(*s);
            u.push(*idx);
        }
        Op::Jmp { .. } | Op::LoopHead => {}
    }
    (d, u)
}

/// Replace exactly the read operands equal to `from` with `to` (defs are
/// never touched).
fn replace_reads(op: &mut Op, from: u16, to: u16) {
    let f = |r: &mut u16| {
        if *r == from {
            *r = to;
        }
    };
    match op {
        Op::Arith { a, b, .. }
        | Op::Wrap { a, b, .. }
        | Op::Bit { a, b, .. }
        | Op::Cmp { a, b, .. }
        | Op::StrCmp { a, b, .. }
        | Op::RefEq { a, b, .. } => {
            f(a);
            f(b);
        }
        Op::Not { a, .. } | Op::Neg { a, .. } => f(a),
        Op::Mov { src, .. } | Op::MovRef { src, .. } => f(src),
        Op::Br { cond, .. } => f(cond),
        Op::BrTable { idx, .. } => f(idx),
        Op::Call { args, .. } => args.iter_mut().for_each(f),
        Op::CallM { recv, args, .. } | Op::CallI { recv, args, .. } => {
            f(recv);
            args.iter_mut().for_each(f);
        }
        Op::CallNat { recv, args, .. } => {
            if let Some(x) = recv {
                f(x);
            }
            args.iter_mut().for_each(f);
        }
        Op::CallFn { fval, args, .. } => {
            f(fval);
            args.iter_mut().for_each(f);
        }
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
        Op::ArrLit { elems, .. } => elems.iter_mut().for_each(f),
        Op::MakeRecord { vals, .. } => vals.iter_mut().for_each(f),
        Op::ArrGet { arr, idx, .. } => {
            f(arr);
            f(idx);
        }
        Op::ArrSet { arr, idx, val, .. } => {
            f(arr);
            f(idx);
            f(val);
        }
        Op::OptSome { val, .. } | Op::ResOk { val, .. } | Op::ResErr { val, .. } => f(val),
        Op::SumIs { v, .. } | Op::Unwrap { v, .. } => f(v),
        Op::UnwrapOr { v, default, .. } => {
            f(v);
            f(default);
        }
        Op::Expect { v, msg, .. } => {
            f(v);
            f(msg);
        }
        Op::TidOf { obj, .. } | Op::IsType { obj, .. } | Op::IsTrait { obj, .. } => f(obj),
        Op::Unbox { box_, .. } => f(box_),
        Op::Box { val, .. } => f(val),
        Op::MakeClosure { captures, .. } => captures.iter_mut().for_each(f),
        Op::Panic { msg } => f(msg),
        Op::Assert { cond, msg, .. } => {
            f(cond);
            if let Some(x) = msg {
                f(x);
            }
        }
        Op::Conv { src, .. } => f(src),
        Op::StrCharAt { s, idx, .. } => {
            f(s);
            f(idx);
        }
        _ => {}
    }
}
