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

use super::peephole::def_use;
use super::Pools;
use rut_core::ops::*;
use std::collections::HashMap;

/// Run SROA to a fixed point on one function.
pub(crate) fn run(
    mut code: Vec<Op>,
    mut spans: Vec<(u32, u32)>,
    pools: &mut Pools,
) -> (Vec<Op>, Vec<(u32, u32)>) {
    for _ in 0..4 {
        let (c, s, changed) = one_round(code, spans, pools);
        code = c;
        spans = s;
        if !changed {
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

fn mark(targets: &mut [bool], t: u32, n: usize) {
    targets[(t as usize).min(n)] = true;
}
