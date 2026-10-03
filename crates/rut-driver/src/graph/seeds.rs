//! The seed-group assembly — the owner side of consumer instantiation
//! requests, one group per requester.

use std::collections::HashMap;

use rut_core::binary::Program;

use super::compiler::{GraphCompiler, Unit};
use crate::Seeds;

/// One seed group per requester: the descriptor closure of every
/// request's arguments, sparse at the requester's locals — the attach
/// side merges the groups into one padded run per scope. The owner-side
/// input shared by both reseed arms (a directory owner and a
/// source-riding compiled owner alike).
pub(super) fn build_seed_groups<'a>(
    programs: &'a [Program],
    done: &HashMap<String, Unit>,
    owner: &str,
    seeds: &'a [(String, rut_lir::check::InstRequest)],
) -> Vec<crate::SeedGroup<'a>> {
    // one seed group per requester: the descriptor closure of every
    // request's arguments, sparse at the requester's locals
    let mut per_requester: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<(rut_core::ScopeId, u32), rut_core::types::RutType>,
    > = std::collections::BTreeMap::new();
    for (requester, r) in seeds {
        let Some((ridx, _)) = done.get(requester).map(|u| (u.idx, ())) else {
            continue;
        };
        let entry = per_requester.entry(requester.clone()).or_default();
        let owner_scope = done.get(owner).map(|u| u.scope).unwrap_or(0);
        let closure = GraphCompiler::desc_closure(&programs[ridx], &r.args, owner_scope);
        for ((s, l), row) in closure {
            entry.insert((s, l), row);
        }
    }
    let mut groups: Vec<crate::SeedGroup> = Vec::new();
    for (requester, rows) in &per_requester {
        let Some((ridx, requester_scope)) = done.get(requester).map(|u| (u.idx, u.scope)) else { continue };
        let prog = &programs[ridx];
        let rows: Vec<((rut_core::ScopeId, u32), rut_core::types::RutType)> =
            rows.iter().map(|(k, v)| (*k, v.clone())).collect();
        let mine: Vec<_> = seeds.iter().filter(|(rq, _)| rq == requester).collect();
        let insts = mine
            .iter()
            .filter(|(_, r)| !r.is_fn)
            .map(|(_, r)| {
                (
                    prog.interner.name(r.decl).to_string(),
                    r.args.clone(),
                    r.methods
                        .iter()
                        .map(|&m| prog.interner.name(m).to_string())
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let fns = mine
            .iter()
            .filter(|(_, r)| r.is_fn)
            .map(|(_, r)| (prog.interner.name(r.decl).to_string(), r.args.clone()))
            .collect();
        groups.push(crate::SeedGroup {
            rows,
            requester: requester.clone(),
            names: &prog.interner,
            insts,
            fns,
        });
    }
    groups
}
