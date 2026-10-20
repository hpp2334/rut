//! Instantiate — the consumer-side codegen stage: template
//! monomorphization, the instantiation-ledger lookups, and the
//! repr-dependent refolds (via [`crate::compile::material`], the
//! substitution engine whose `materialize_fn` re-runs `refold_optprim`).
//!
//! This is the first half of the post-compile chain — instantiate, then
//! the merge (`rut_core::link`, arithmetic only), then the load gate's
//! final verify pass. Everything here serves a consumer's instantiation
//! requests against the OWNING package: a source owner recompiles with
//! the requesters' descriptors as seeds (`reseed_source_owner`), a
//! packaged owner serves from its pack-time ledger first and its
//! template bodies second (`reseed_ir_owner`). It is pure code motion
//! from the walk ([`super::compiler`]) — the seam, not a new behavior.

use std::collections::HashMap;

use rut_core::binary::Program;
use rut_core::ops::Op;
use rut_lexer::diag::Diag;
use rut_lexer::span::Span;
use rut_parser::{parse, Mode};

use super::build_seed_groups;
use super::compiler::{attribute_diags, GraphCompiler};
use crate::compile::material::split_top_commas;
use crate::Seeds;

impl<'a> GraphCompiler<'a> {
    /// A request's canonical key — owner, declaration text, and the
    /// argument spellings through the requester's table. The sort order
    /// of these keys is the resolution order: deterministic for the same
    /// source, byte-stable for the same pack.
    fn request_key(&self, requester: &str, r: &rut_lir::check::InstRequest) -> String {
        let Some((idx, _)) = self.done.get(requester).map(|u| (u.idx, ())) else {
            return format!("{}#?", r.owner);
        };
        let prog = &self.programs[idx];
        let decl = prog.interner.name(r.decl).to_string();
        let args = r
            .args
            .iter()
            .map(|&a| {
                // the semantic spelling: boot rows by id, everything else
                // by the row's own name text. A reseeded owner's own-block
                // locals shift between rounds (its table grows with every
                // materialized body), and a scope-packed canon would read
                // the same request as a new one forever. The text — not
                // the interner id — crosses compiles.
                if rut_core::scope_of(a) == rut_core::BOOT_SCOPE {
                    format!("b{}", rut_core::local_of(a))
                } else {
                    prog.interner.name(prog.types.type_at(a).name).to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        // the kind + methods ride the key: one (decl, args) pair can carry
        // several method-body requests, and a fn body request is not a
        // type instantiation
        let kind = if r.is_fn { "f" } else { "t" };
        let methods = r
            .methods
            .iter()
            .map(|&m| prog.interner.name(m).to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!("{}#{}#{}<{}>#{}", r.owner, kind, decl, args, methods)
    }

    /// Resolve the consumer requests: instantiation is owned by the
    /// declaring package, and each request routes to its owner. A SOURCE
    /// owner recompiles with the seeds (the requesters' descriptors as
    /// sparse blocks — the locals match the requester's layout — plus
    /// the instantiations to materialize before its compile roots). A
    /// PACKAGED owner serves the request from its own binary: the
    /// pack-time ledger first (the fast path — the closure already
    /// compiled the shape), then its TEMPLATES (v22 — the body
    /// monomorphizes in place by id substitution); a miss of both is a
    /// torn or doctored bundle, refused loudly. Seeds cascade — a
    /// seeded owner's compile may request from further owners — until
    /// the queue drains; every round processes in canonical (sorted)
    /// order.
    pub(super) fn resolve_requests(&mut self) {
        for _round in 0..32 {
            if self.requests.is_empty() {
                return;
            }
            let mut requests = std::mem::take(&mut self.requests);
            requests.sort_by_cached_key(|(requester, r)| self.request_key(requester, r));
            for (requester, r) in requests {
                let key = self.request_key(&requester, &r);
                if self.seeded_keys.contains(&key) {
                    continue;
                }
                self.seeded_keys.insert(key);
                self.fresh_owners.insert(r.owner.clone());
                self.seed_pool
                    .entry(r.owner.clone())
                    .or_default()
                    .push((requester, r));
            }
            // only owners that received NEW seeds this round re-resolve:
            // a reseed re-emits its own mirrors' requests, and re-running
            // an unchanged owner would feed the loop forever
            let mut owners: Vec<String> = self.fresh_owners.drain().collect();
            owners.sort();
            for owner in owners {
                let Some(seeds) = self.seed_pool.get(&owner).cloned() else {
                    continue;
                };
                match self.body_kind.get(&owner).copied() {
                    Some(0) => self.reseed_source_owner(&owner, &seeds),
                    Some(1) => self.reseed_ir_owner(&owner, &seeds),
                    _ => {
                        let first = &seeds[0].1;
                        let decl = first.decl.0.to_string();
                        self.diags.push(Diag::new(
                            Span::new(0, 0),
                            format!(
                                "instantiation request for `{owner}` (decl {decl}) — the pkg is mounted as a host module and declares no generics"
                            ),
                        ));
                    }
                }
                // the pool ACCUMULATES: a later round's reseed must carry
                // every seed this owner has seen, or the replacement
                // program drops the bodies an earlier round already
                // materialized (a consumer's mirror then claims nothing).
                // The loop ends when the request queue drains; identical
                // seeds re-materialize nothing (the materialization memo
                // dedups).
                if !self.diags.is_empty() {
                    return;
                }
            }
        }
        if !self.requests.is_empty() {
            self.diags.push(Diag::new(
                Span::new(0, 0),
                "instantiation requests did not settle in 32 resolution rounds — a generic cycle the request law cannot close",
            ));
        }
    }

    /// One seed's argument list in the OWNER's id space: the seed's args
    /// name the requester's registered block rows, which the recompile
    /// registers verbatim — the ids carry over untouched.
    fn reseed_source_owner(
        &mut self,
        owner: &str,
        seeds: &[(String, rut_lir::check::InstRequest)],
    ) {
        let Some((idx, scope)) = self.done.get(owner).map(|u| (u.idx, u.scope)) else {
            return;
        };
        let Some(src) = self.unit_src.get(owner) else {
            return;
        };
        let (text, is_decl, bound) = (src.text.clone(), src.is_decl, src.bound.clone());
        let layout = src.layout.clone();
        let own_uses = src.own_uses.clone();
        let groups = build_seed_groups(&self.programs, &self.done, owner, seeds);
        let out = crate::compile::compile_program_pre(
            parse(&text, if is_decl { Mode::Decl } else { Mode::Impl }),
            &text,
            if is_decl { Mode::Decl } else { Mode::Impl },
            owner,
            scope,
            &bound,
            true,
            &Seeds { groups: &groups },
            &layout,
            &own_uses,
            // the replay of a unit that already passed the must-bind
            // gate (same text, same bindings) — no classification input
            // to carry
            &[],
        );
        for r in &out.requests {
            self.requests.push((owner.to_string(), r.clone()));
        }
        if !out.diags.is_empty() || out.program.is_none() {
            self.diags
                .extend(attribute_diags(out.diags, owner, &layout));
            return;
        }
        self.programs[idx] = out.program.expect("checked above");
    }

    /// A packaged owner serves a missed request from its TEMPLATES (v22
    /// — the delivery law's second lane): the owner program's
    /// placeholder-compiled bodies monomorphize IN PLACE, by id
    /// substitution. The owner's types grow the requester's argument
    /// rows (staged at the requester's own locals — the seed law), the
    /// template body substitutes, the repr bytes rewrite, the
    /// repr-keyed folds re-run, and the result is appended with its
    /// owner-anchored ledger rows — byte-equal to the directory-compiled
    /// instantiation (the pinned law). Bound-member stubs resolve
    /// against the concrete argument's own surface: an owner-local
    /// member binds directly, a member template cascades to its own
    /// materialization, a foreign generic routes a fresh InstRequest
    /// (the cascade), and a consumer-local member mints the mirror stub
    /// whose ledger row names the consumer (link unifies it with the
    /// real body there — the carried-member law's direction).
    fn reseed_ir_owner(&mut self, owner: &str, seeds: &[(String, rut_lir::check::InstRequest)]) {
        let Some(unit) = self.done.get(owner) else {
            return;
        };
        let idx = unit.idx;
        // the live seeds (the pack-time ledger answers the rest — link
        // unifies the mirror, zero new work)
        let live: Vec<(String, rut_lir::check::InstRequest)> = seeds
            .iter()
            .filter(|(requester, r)| !self.ir_ledger_hit(idx, requester, r))
            .map(|(requester, r)| (requester.clone(), r.clone()))
            .collect();
        // the seed law's staging, ONCE per requester: every live seed's
        // argument rows share the requester's scope, so the union of
        // their closures stages as ONE block at one base — a per-request
        // staging would re-base the scope per shape and strand the
        // earlier requests' ids (locals beyond the first block's extent
        // densify into unrelated rows)
        let mut requesters: Vec<String> = live.iter().map(|(re, _)| re.clone()).collect();
        requesters.sort();
        requesters.dedup();
        for requester in &requesters {
            let args: Vec<Vec<rut_core::types::TypeId>> = live
                .iter()
                .filter(|(re, _)| re == requester)
                .map(|(_, r)| r.args.clone())
                .collect();
            if let Err(msg) = self.stage_ir_seeds(idx, owner, requester, &args) {
                self.diags.push(Diag::new(Span::new(0, 0), msg));
                return;
            }
        }
        for (requester, r) in &live {
            if r.is_fn {
                if let Err(msg) =
                    self.materialize_request(idx, owner, requester, true, r.decl, &r.args, None)
                {
                    self.diags.push(Diag::new(Span::new(0, 0), msg));
                    return;
                }
            } else if r.methods.is_empty() {
                // a type-only request: the instantiation's identity row —
                // the staged substitution mints it (no body exists)
                if let Err(msg) =
                    self.materialize_request(idx, owner, requester, false, r.decl, &r.args, None)
                {
                    self.diags.push(Diag::new(Span::new(0, 0), msg));
                    return;
                }
            } else {
                for &m in &r.methods {
                    if let Err(msg) = self.materialize_request(
                        idx,
                        owner,
                        requester,
                        false,
                        r.decl,
                        &r.args,
                        Some(m),
                    ) {
                        self.diags.push(Diag::new(Span::new(0, 0), msg));
                        return;
                    }
                }
            }
        }
    }

    /// The seed law's staging half, per requester: the union closure of
    /// every live seed's arguments appends as ONE block under a FRESH
    /// scope allocated for this staging — never the requester's own
    /// number, which may be a ghost (a carried block from a replaced
    /// mount), may BE the owner's own scope, and may stage into several
    /// owners (one coordinate system per staged block; link's
    /// staged-scope sweep maps each block's base). The staged rows'
    /// inner ids spelling the requester scope rewrite to the fresh
    /// scope (a recursive kind walk); ids in other scopes pass (the
    /// owner carries them, or a later arg stages them). Every row's
    /// name re-interns by text (ids are per-program).
    /// `materialize_request` spells each staged request's arguments
    /// through the fresh scope.
    fn stage_ir_seeds(
        &mut self,
        idx: usize,
        owner: &str,
        requester: &str,
        args_per_request: &[Vec<rut_core::types::TypeId>],
    ) -> Result<(), String> {
        let r_idx = self.done.get(requester).map(|u| u.idx);
        let Some(r_idx) = r_idx else { return Ok(()) }; // hand-routed: nothing to stage
                                                        // the requester scopes that need staging: non-boot, not already
                                                        // carried by the owner, and not staged for this (owner,
                                                        // requester) pair yet
        let mut to_stage: Vec<rut_core::id::ScopeId> = Vec::new();
        {
            let owner_prog = &self.programs[idx];
            for args in args_per_request {
                for &a in args {
                    let s = rut_core::scope_of(a);
                    if s == rut_core::BOOT_SCOPE
                        || owner_prog
                            .types
                            .scope_base
                            .get(s as usize)
                            .copied()
                            .unwrap_or(0)
                            != 0
                        || self.staged_scopes.contains_key(&(owner.to_string(), s))
                    {
                        continue;
                    }
                    if !to_stage.contains(&s) {
                        to_stage.push(s);
                    }
                }
            }
        }
        if to_stage.is_empty() {
            return Ok(());
        }
        // read the requester's closures immutably: per staged scope, the
        // union closure of every arg spelling it, padded 0..=max_local
        let mut plan: Vec<(rut_core::types::RutType, rut_core::id::ScopeId, u32)> = Vec::new();
        {
            let r_prog = &self.programs[r_idx];
            let mut cursor = self.programs[idx].types.types.len() as u32;
            for &s in &to_stage {
                let mut closure: std::collections::HashMap<
                    (rut_core::id::ScopeId, u32),
                    rut_core::types::RutType,
                > = std::collections::HashMap::new();
                for args in args_per_request {
                    for &a in args {
                        if rut_core::scope_of(a) != s {
                            continue;
                        }
                        for ((s2, l2), row) in
                            Self::desc_closure(r_prog, &[a], rut_core::id::ScopeId::MAX)
                        {
                            if s2 == s {
                                closure.insert((s2, l2), row);
                            }
                        }
                    }
                }
                if closure.is_empty() {
                    continue;
                }
                let mut max_local = 0u32;
                for (s2, l) in closure.keys() {
                    if *s2 == s {
                        max_local = max_local.max(*l);
                    }
                }
                let mut filled = 0u32;
                while filled <= max_local {
                    match closure.get(&(s, filled)) {
                        Some(row) => plan.push((row.clone(), s, cursor)),
                        None => plan.push((
                            rut_core::types::RutType {
                                name: rut_core::sym::NIL,
                                kind: rut_core::types::TyKind::Nil,
                            },
                            s,
                            cursor,
                        )),
                    }
                    cursor += 1;
                    filled += 1;
                }
            }
        }
        if plan.is_empty() {
            return Ok(());
        }
        // the staged rows' IDENT ids spell the REQUESTER's interner —
        // ids are per-program, so every name re-interns here by text
        // (the surface's own law: the text crosses, never the id). A
        // name left as the requester's raw ident reads as an arbitrary
        // word in the owner's interner — the mint's structural dedup
        // then matches on garbage and the merged table spells nonsense.
        let rename_texts: Vec<(rut_core::sym::IdentId, String)> = {
            let req_interner = &self.programs[r_idx].interner;
            plan.iter()
                .flat_map(|(row, _, _)| {
                    let mut out = vec![row.name];
                    if let rut_core::types::TyKind::Data { fields } = &row.kind {
                        out.extend(fields.iter().map(|f| f.name));
                    }
                    out
                })
                .map(|id| (id, req_interner.name(id).to_string()))
                .collect()
        };
        // the fresh staging scope: allocated from the graph's counter so
        // it collides with no mount scope in any program of this graph
        let stage_scope = {
            let fresh = self.next_scope;
            self.next_scope += 1;
            fresh
        };
        for &s in &to_stage {
            self.staged_scopes
                .insert((owner.to_string(), s), stage_scope);
        }
        let own = &mut self.programs[idx];
        let renames: Vec<(rut_core::sym::IdentId, rut_core::sym::IdentId)> = rename_texts
            .iter()
            .map(|&(id, ref text)| (id, own.interner.intern(text)))
            .filter(|&(from, to)| from != to)
            .collect();
        for (row, _, _) in plan.iter_mut() {
            if let Some(&(_, to)) = renames.iter().find(|(from, _)| *from == row.name) {
                row.name = to;
            }
            if let rut_core::types::TyKind::Data { fields } = &mut row.kind {
                for f in fields.iter_mut() {
                    if let Some(&(_, to)) = renames.iter().find(|(from, _)| *from == f.name) {
                        f.name = to;
                    }
                }
            }
            // the requester-scope inner ids rewrite to the fresh scope
            for &s in &to_stage {
                remap_kind_scope(&mut row.kind, s, stage_scope);
            }
        }
        let tail = own.types.types.len() as u32;
        for (row, s_old, _) in plan.iter_mut() {
            *s_old = stage_scope;
            own.types.types.push(row.clone());
        }
        // the staged block's base: the block's first appended row
        let need = stage_scope as usize + 1;
        if own.types.scope_base.len() < need {
            own.types.scope_base.resize(need, 0);
        }
        own.types.scope_base[stage_scope as usize] = tail;
        Ok(())
    }

    /// The pack-time ledger's answer for one request — the same
    /// name-text canon the request side speaks (boot rows by id,
    /// everything else by the row's name text).
    fn ir_ledger_hit(&self, idx: usize, requester: &str, r: &rut_lir::check::InstRequest) -> bool {
        let Some(unit) = self.done.get(requester) else {
            return false;
        };
        let pidx = unit.idx;
        let req = &self.programs[pidx];
        let own = &self.programs[idx];
        // non-boot rows canon by their name text; an id that densifies
        // OUT of its table (the pack's template-pass ledger rows carry
        // subst ids spelling the template window's rows, which the
        // carried blocks of a consumer's mount do not include) can
        // never match an in-bounds request canon — it canons to a
        // unique per-id sentinel and the row misses
        let canon_of = |prog: &Program, t: rut_core::types::TypeId| -> String {
            if rut_core::scope_of(t) == rut_core::BOOT_SCOPE {
                format!("b{}", rut_core::local_of(t))
            } else {
                let d = prog.types.dense(t) as usize;
                match prog.types.types.get(d) {
                    Some(row) => prog.interner.name(row.name).to_string(),
                    None => format!("#oob({t})"),
                }
            }
        };
        let args = r
            .args
            .iter()
            .map(|&a| canon_of(req, a))
            .collect::<Vec<_>>()
            .join(",");
        let owner_text = r.owner.as_str();
        let decl_text = req.interner.name(r.decl).to_string();
        if r.is_fn {
            own.inst_fns.iter().any(|row| {
                own.interner.name(row.owner) == owner_text
                    && match &row.kind {
                        rut_core::binary::InstFnKind::Free { name, subst, .. } => {
                            own.interner.name(*name) == decl_text
                                && subst
                                    .iter()
                                    .map(|&t| canon_of(own, t))
                                    .collect::<Vec<_>>()
                                    .join(",")
                                    == args
                        }
                        _ => false,
                    }
            })
        } else if r.methods.is_empty() {
            own.inst_types.iter().any(|row| {
                own.interner.name(row.owner) == owner_text
                    && own.interner.name(row.decl) == decl_text
                    && row
                        .args
                        .iter()
                        .map(|&t| canon_of(own, t))
                        .collect::<Vec<_>>()
                        .join(",")
                        == args
            })
        } else {
            r.methods.iter().all(|&m| {
                let mtext = req.interner.name(m).to_string();
                own.inst_fns.iter().any(|row| {
                    own.interner.name(row.owner) == owner_text
                        && match &row.kind {
                            rut_core::binary::InstFnKind::Method {
                                data, name, subst, ..
                            } => {
                                own.interner.name(*data) == decl_text
                                    && own.interner.name(*name) == mtext
                                    && subst
                                        .iter()
                                        .map(|&t| canon_of(own, t))
                                        .collect::<Vec<_>>()
                                        .join(",")
                                        == args
                            }
                            _ => false,
                        }
                })
            })
        }
    }

    /// Materialize one request against owner program `idx`: spell the
    /// arguments through the staged block (`stage_ir_seeds` laid it —
    /// the ids carry over untouched), find the template, substitute,
    /// append. `method` is `Some` for a method-body request, `None` for
    /// a free fn or a type-only request.
    #[allow(clippy::too_many_arguments)]
    fn materialize_request(
        &mut self,
        idx: usize,
        owner: &str,
        requester: &str,
        is_fn: bool,
        decl: rut_core::sym::IdentId,
        args: &[rut_core::types::TypeId],
        method: Option<rut_core::sym::IdentId>,
    ) -> Result<(), String> {
        // the seed law: rows land in the staged block under the fresh
        // staging scope (`stage_ir_seeds` laid it and recorded the
        // mapping), so the owner's substitution spells the staged
        // world. Boot ids are global; a scope the owner already carries
        // (its own world, or a previously staged block) passes through.
        let r_idx = self.done.get(requester).map(|u| u.idx);
        let mut arg_map: Vec<(rut_core::types::TypeId, rut_core::types::TypeId)> = Vec::new();
        for &a in args {
            let s = rut_core::scope_of(a);
            let staged = if s == rut_core::BOOT_SCOPE {
                None
            } else {
                self.staged_scopes.get(&(owner.to_string(), s)).copied()
            };
            let out = staged
                .map(|fresh| rut_core::pack(fresh, rut_core::local_of(a)))
                .unwrap_or(a);
            arg_map.push((a, out));
        }
        // the consumer-local members' index (case (c) of the stub
        // resolution): each program's inherent rows, keyed by scope —
        // computed once, so the materialization never holds a second
        // program borrow while growing the owner
        let mut home = Homes::default();
        for p in self.programs.iter() {
            if p.scope == rut_core::BOOT_SCOPE {
                continue;
            }
            home.name.insert(p.scope, p.name.clone());
            let entry = home.members.entry(p.scope).or_default();
            for ih in &p.surface.inherents {
                let set = entry.entry(ih.target).or_default();
                for m in &ih.methods {
                    set.insert(p.interner.name(m.name).to_string());
                }
            }
        }
        // the request's name text spells through the REQUESTER's
        // interner (the ids are per-program; the text is what crosses)
        let (decl_text, method_text) = match r_idx {
            Some(ri) => {
                let req = &self.programs[ri];
                (
                    req.interner.name(decl).to_string(),
                    method.map(|m| req.interner.name(m).to_string()),
                )
            }
            None => (String::new(), None),
        };
        // ---- phase B: substitute + materialize, inside the owner
        // program's borrow
        let Self {
            programs, requests, ..
        } = self;
        let own = &mut programs[idx];
        // the template row, matched by (kind, name text, arity) — a
        // BODY request only; a type-only request needs no template
        // (its identity row mints structurally below)
        let arity = args.len();
        let row = if is_fn || method.is_some() {
            own.templates.iter().enumerate().find(|(_, t)| {
                let name_ok = match &t.kind {
                    rut_core::binary::TemplateKind::Free { name } => {
                        is_fn && own.interner.name(*name) == decl_text
                    }
                    rut_core::binary::TemplateKind::Method { data, name } => {
                        !is_fn
                            && own.interner.name(*data) == decl_text
                            && method_text.as_deref() == Some(own.interner.name(*name))
                    }
                };
                name_ok && t.param_rows.len() == arity
            })
        } else {
            None
        };
        // THE DELIVERY LAW's loud refusal: a BODY request with no
        // template row is a torn or doctored bundle — the pack could
        // not have produced it (a body request is never answered by the
        // type-only identity mint below, which would fabricate
        // `Method { .., fid: 0 }` and mislink at runtime)
        if (is_fn || method.is_some()) && row.is_none() {
            let what = if is_fn {
                decl_text.clone()
            } else {
                format!(
                    "{}${}",
                    decl_text,
                    method_text.as_deref().unwrap_or("<method>")
                )
            };
            return Err(format!(
                "instantiation `{owner}#{what}<..>` is served by neither `{owner}`'s \
                 pack-time ledger nor its template bodies — corrupt or doctored, re-pack \
                 the directory"
            ));
        }
        let fid = if let Some((_, trow)) = row {
            let params = trow.param_rows.clone();
            let first_fid = own.template_first_fid;
            let stubs = own.template_stubs.clone();
            // the memo: (template fid, argument values) → the
            // materialized fid; a body reserves its slot BEFORE cloning,
            // so recursion lands on the reserved fid (the ensure_inst
            // pattern)
            let mut memo: HashMap<(u32, Vec<rut_core::types::TypeId>), u32> = HashMap::new();
            materialize_body(
                own,
                &home,
                &stubs,
                first_fid,
                trow.fid,
                &params,
                &arg_map.iter().map(|(_, o)| *o).collect::<Vec<_>>(),
                owner,
                &mut memo,
                requests,
                0,
            )?
        } else {
            // a type-only request: the instantiation's identity row —
            // the class's template export substitutes structurally and
            // the owner-anchored row mints (no body exists)
            let base = own
                .surface
                .type_exports
                .iter()
                .find(|t| t.is_generic && own.interner.name(t.name) == decl_text);
            let Some(base) = base else {
                return Err(format!(
                    "instantiation `{owner}#{decl_text}<..>` is served by neither `{owner}`'s \
                     pack-time ledger nor its template bodies — corrupt or doctored, re-pack \
                     the directory"
                ));
            };
            let own_scope = rut_core::link::own_scope(own).unwrap_or(0);
            let tmpl = rut_core::pack(base.scope.unwrap_or(own_scope), base.local);
            let mut pairs = Vec::new();
            for (p, &a) in base.params.iter().zip(args.iter()) {
                let ptext = own.interner.name(*p).to_string();
                let Some(row) = own
                    .interner
                    .lookup(&format!("#{ptext}"))
                    .and_then(|id| own.types.dense_id_of_name(id))
                else {
                    return Err(format!(
                        "`{owner}`'s template row for `{decl_text}` carries no `#{ptext}` placeholder — \
                         corrupt or doctored, re-pack the directory"
                    ));
                };
                pairs.push((row, a));
            }
            let ledger_rows = own.inst_fns.clone();
            let mut sub = crate::compile::material::Substitution::new(
                &mut own.types,
                &mut own.interner,
                pairs,
                owner.to_string(),
            )
            .with_ledger_owners(&ledger_rows);
            let _ = sub.run(tmpl);
            own.inst_types
                .extend(std::mem::take(&mut sub.minted_inst_tys));
            0 // no body — the kind arm below returns early
        };
        // the request's own ledger row: the owner-anchored identity the
        // consumer's mirror unifies with at link. The NAME TEXTS intern
        // in the OWNER's interner — the request's idents spell the
        // requester's table and mean nothing here.
        let subst_owner = own.interner.intern(owner);
        let decl_ident = own.interner.intern(&decl_text);
        let kind = if is_fn {
            rut_core::binary::InstFnKind::Free {
                name: decl_ident,
                subst: arg_map.iter().map(|(_, o)| *o).collect(),
                origins: vec![],
            }
        } else if method.is_some() {
            rut_core::binary::InstFnKind::Method {
                data: decl_ident,
                name: own.interner.intern(&method_text.unwrap_or_default()),
                subst: arg_map.iter().map(|(_, o)| *o).collect(),
                origins: vec![],
            }
        } else {
            return Ok(()); // a type-only request: the identity row minted above
        };
        own.inst_fns.push(rut_core::binary::InstFn {
            owner: subst_owner,
            kind,
            fid,
        });
        Ok(())
    }
}

/// The consumer-local member index (the stub resolution's case (c)):
/// every program's inherent rows, keyed by its own scope — the
/// materialization reads it instead of holding a second program
/// borrow while growing the owner.
#[derive(Default)]
struct Homes {
    name: HashMap<rut_core::id::ScopeId, String>,
    members: HashMap<rut_core::id::ScopeId, HashMap<u32, std::collections::HashSet<String>>>,
}

/// Materialize one template body into the owner program: reserve the
/// fn slot, clone the template's `FuncCode` through the substitution
/// (`TypeId`s rebuilt structurally, repr bytes recomputed, the
/// repr-keyed folds re-run), then fix every callee operand — a
/// bound-member stub resolves against the concrete argument's own
/// surface, a template-window callee materializes recursively, a
/// foreign mirror stub re-mints with the substituted ledger row and
/// routes a fresh `InstRequest`. Returns the materialized fid.
#[allow(clippy::too_many_arguments)]
fn materialize_body(
    own: &mut Program,
    home: &Homes,
    stubs: &[rut_core::binary::TemplateStub],
    first_fid: u32,
    template_fid: u32,
    parent_params: &[rut_core::types::TypeId],
    parent_values: &[rut_core::types::TypeId],
    owner: &str,
    memo: &mut HashMap<(u32, Vec<rut_core::types::TypeId>), u32>,
    requests: &mut Vec<(String, rut_lir::check::InstRequest)>,
    depth: u32,
) -> Result<u32, String> {
    if depth > 100 {
        return Err(format!(
            "template materialization for `{owner}` recursed past 100 frames — a generic cycle the templates cannot close"
        ));
    }
    // the callee's own substitution context: a ledger row naming the
    // fid spells its template-side arguments (the row's subst) — those
    // run through the parent's substitution; a rowless callee (a
    // lambda / for-of closure minted inside the template) inherits the
    // parent's context wholesale
    let (params, values) = match own.inst_fns.iter().find(|r| r.fid == template_fid) {
        Some(row) => {
            let row_subst: Vec<rut_core::types::TypeId> = match &row.kind {
                rut_core::binary::InstFnKind::Free { subst, .. }
                | rut_core::binary::InstFnKind::Method { subst, .. } => subst.clone(),
                rut_core::binary::InstFnKind::HostThunk { .. } => vec![],
            };
            let ledger_rows = own.inst_fns.clone();
            let mut sub = crate::compile::material::Substitution::new(
                &mut own.types,
                &mut own.interner,
                parent_params
                    .iter()
                    .cloned()
                    .zip(parent_values.iter().cloned())
                    .collect(),
                owner.to_string(),
            )
            .with_ledger_owners(&ledger_rows);
            let vals = row_subst.iter().map(|&t| sub.run(t)).collect();
            (row_subst, vals)
        }
        None => (parent_params.to_vec(), parent_values.to_vec()),
    };
    let key = (template_fid, values.clone());
    if let Some(&f) = memo.get(&key) {
        return Ok(f);
    }
    let source = own
        .funcs
        .get(template_fid as usize)
        .ok_or_else(|| format!("template body #{template_fid} out of range in `{owner}`"))?
        .clone();
    let ledger_row = own.inst_fns.iter().find(|r| r.fid == template_fid).cloned();
    if source.code.is_empty() && source.host_id.is_none() {
        // a MIRROR STUB (the template window's request to a foreign
        // generic): never clone an empty body — re-mint the stub with
        // the substituted ledger row and route the request to the
        // foreign owner (the cascade)
        let row = ledger_row.ok_or_else(|| {
            format!("`{owner}`: bodyless fn #{template_fid} carries no ledger row")
        })?;
        let owner_text = own.interner.name(row.owner).to_string();
        let (is_fn, data_ident, name_ident): (bool, _, _) = match &row.kind {
            rut_core::binary::InstFnKind::Free { name, .. } => (true, *name, *name),
            rut_core::binary::InstFnKind::Method { data, name, .. } => (false, *data, *name),
            rut_core::binary::InstFnKind::HostThunk { .. } => {
                return Err(format!(
                    "`{owner}`: a host thunk (#{template_fid}) rode a template call — doctored"
                ));
            }
        };
        let args: Vec<rut_core::types::TypeId> = values.clone();
        let methods: Vec<rut_core::sym::IdentId> = if is_fn { vec![] } else { vec![name_ident] };
        // the new stub: bodyless, its ledger row spells the substituted
        // key — link binds the call to the foreign owner's served body
        let data_text = own.interner.name(data_ident).to_string();
        let name_text = own.interner.name(name_ident).to_string();
        let stub_name = own
            .interner
            .intern(&format!("mirror#{owner_text}${data_text}${name_text}"));
        let new_fid = own.funcs.len() as u32;
        own.funcs.push(rut_core::binary::FuncCode {
            name: stub_name,
            params: vec![],
            ret: rut_core::types::TY_NIL,
            is_method: !is_fn,
            n_captures: 0,
            regs: vec![],
            argv: vec![],
            labels: vec![],
            code: vec![],
            spans: vec![],
            pos: vec![],
            host_id: None,
        });
        let k = if is_fn {
            rut_core::binary::InstFnKind::Free {
                name: name_ident,
                subst: args.clone(),
                origins: vec![],
            }
        } else {
            rut_core::binary::InstFnKind::Method {
                data: data_ident,
                name: name_ident,
                subst: args.clone(),
                origins: vec![],
            }
        };
        own.inst_fns.push(rut_core::binary::InstFn {
            owner: own.interner.intern(&owner_text),
            kind: k,
            fid: new_fid,
        });
        requests.push((
            owner.to_string(),
            rut_lir::check::InstRequest {
                owner: owner_text,
                decl: data_ident,
                args,
                methods,
                is_fn,
                is_impl: false,
                impl_target: 0,
            },
        ));
        memo.insert(key, new_fid);
        return Ok(new_fid);
    }
    // reserve the slot, then substitute
    let new_fid = own.funcs.len() as u32;
    memo.insert(key.clone(), new_fid);
    own.funcs.push(source.clone());
    // the template row's `data` text, read before the substitution
    // borrows the table — the receiver test's expected spelling
    let receiver_data_text = ledger_row.as_ref().and_then(|r| match &r.kind {
        rut_core::binary::InstFnKind::Method { data, .. } => {
            Some(own.interner.name(*data).to_string())
        }
        _ => None,
    });
    let (mut body, minted, iface_fills) = {
        let ledger_rows = own.inst_fns.clone();
        let mut sub = crate::compile::material::Substitution::new(
            &mut own.types,
            &mut own.interner,
            params.iter().cloned().zip(values.iter().cloned()).collect(),
            owner.to_string(),
        )
        .with_ledger_owners(&ledger_rows);
        // the method template's RECEIVER: the body spells the class's
        // OWN bare row (`HashMap` — the record shape), whose tie to the
        // type parameters rides the template row (`data` +
        // `param_rows`), not any argument spell of its own — no name
        // rebuild can derive it. Pin bare row → instantiation row
        // (`HashMap<i64, str>` — the exact row a fresh compile mints
        // for the consumer's shape), so the served body, its
        // MakeRecord/field ops and the boundary checks all speak the
        // consumer's spelling. The receiver test is textual: the first
        // parameter names the template row's `data` (a static method
        // like `with_capacity(n: i32)` has no receiver — its first
        // parameter names something else).
        if let Some(&self_row) = source.params.first() {
            let base_text = sub.name_of(self_row).to_string();
            let is_receiver = receiver_data_text.as_deref() == Some(base_text.as_str());
            if is_receiver && !base_text.contains('<') {
                let kind = sub.tt.kind(self_row).clone();
                let row = sub.mint_instantiation(&base_text, &values, kind);
                sub.force(self_row, row);
            }
        }
        // T2a: re-target the template's interface dispatches. The body
        // bakes the PLACEHOLDER row's slots (`Readable<#T>.get`); the
        // substituted body must speak the CONCRETE row — the one its
        // consumer-side fills answer on. Identity for every row the
        // substitution cannot change (non-generic interfaces,
        // already-concrete instantiations).
        let iface_remaps = crate::compile::material::build_iface_remaps(
            &mut sub,
            &mut own.ifaces,
            &mut own.iface_slots,
            &mut own.vtables,
            &source,
        )?;
        let mut body =
            crate::compile::material::materialize_fn(&mut sub, &source, &mut |f| f, &iface_remaps);
        // the CONST half of the template-stability law: an op may pin a
        // TYPE ID as a literal (`opaque::downcast<V>`'s want — the
        // emitter bakes `Const(TypeId(#V))`). Every type OPERAND
        // already re-spelled through the substitution; the baked
        // literals take the same walk, each landing on a (possibly
        // fresh) pool entry carrying the SUBSTITUTED id — a stale
        // placeholder id makes every runtime downcast of the
        // substituted type answer nil.
        {
            let mut seen: HashMap<u32, u32> = HashMap::new();
            for op in body.code.iter_mut() {
                let rut_core::ops::Op::Const { k, .. } = op else {
                    continue;
                };
                if let Some(&to) = seen.get(k) {
                    *k = to;
                    continue;
                }
                let new_k = match own.consts.get(*k as usize) {
                    Some(rut_core::binary::ConstVal::TypeId(t)) => {
                        let nt = sub.run(*t);
                        if nt == *t {
                            *k
                        } else {
                            match own.consts.iter().position(
                                |c| matches!(c, rut_core::binary::ConstVal::TypeId(x) if *x == nt),
                            ) {
                                Some(p) => p as u32,
                                None => {
                                    own.consts.push(rut_core::binary::ConstVal::TypeId(nt));
                                    (own.consts.len() - 1) as u32
                                }
                            }
                        }
                    }
                    _ => *k,
                };
                seen.insert(*k, new_k);
                *k = new_k;
            }
        }
        // T2b (collect): no check phase runs over a materialization, so
        // the concrete (payload × iface) fills are demanded from what
        // the body SPEAKS: interface widening emits NO op (`widen_to_slot`
        // is a no-op — the cell handle flows and dispatch reads the
        // cell's own type), so the pairing's only witnesses are the
        // types themselves. Every concrete class/enum row the body
        // builds or carries × every iface row it names (regs, params,
        // ret, CallI/IsIface operands): a satisfiable pair is exactly
        // the fill a fresh compile's check phase demands at its widen;
        // an unsatisfiable co-mention is skipped at the lay step below
        let mut payloads: Vec<rut_core::types::TypeId> = body
            .params
            .iter()
            .chain(body.regs.iter())
            .copied()
            .filter(|&t| {
                matches!(
                    sub.tt.kind(t),
                    rut_core::types::TyKind::Data { .. } | rut_core::types::TyKind::Enum { .. }
                )
            })
            .collect();
        payloads.sort_unstable();
        payloads.dedup();
        let mut spoken_rows: Vec<u32> = body
            .params
            .iter()
            .chain(std::iter::once(&body.ret))
            .chain(body.regs.iter())
            .filter_map(|&t| match sub.tt.kind(t) {
                rut_core::types::TyKind::IfaceObj { iface_id } => Some(*iface_id),
                _ => None,
            })
            // the signature/reg rows still spell the PACK ids (the
            // substitution does not rewrite IfaceObj kinds) — map them
            // onto the substituted rows; op-derived rows below are
            // already re-targeted by remap_op
            .map(|r| iface_remaps.rows.get(&r).copied().unwrap_or(r))
            .collect();
        for op in body.code.iter() {
            match op {
                Op::CallI { slot, .. } => {
                    if let Some(&(row, _)) = own.iface_slots.get(*slot as usize) {
                        spoken_rows.push(row);
                    }
                }
                Op::IsIface { want, .. } => spoken_rows.push(*want),
                _ => {}
            }
        }
        spoken_rows.sort_unstable();
        spoken_rows.dedup();
        let mut iface_fills: Vec<(rut_core::types::TypeId, u32)> = Vec::new();
        for &p in &payloads {
            for &r in &spoken_rows {
                iface_fills.push((p, r));
            }
        }
        let minted = std::mem::take(&mut sub.minted_inst_tys);
        (body, minted, iface_fills)
    };
    // fix the callee operands against the substituted world
    let member_stubs: HashMap<u32, (rut_core::types::TypeId, rut_core::sym::IdentId)> =
        stubs.iter().map(|s| (s.fid, (s.param, s.member))).collect();
    for op in body.code.iter_mut() {
        let needs_fix = matches!(
            op,
            Op::Call { .. } | Op::CallM { .. } | Op::MakeClosure { .. }
        );
        if !needs_fix {
            continue;
        }
        let f = match op {
            Op::Call { func, .. } | Op::CallM { func, .. } | Op::MakeClosure { func, .. } => *func,
            _ => unreachable!("guarded above"),
        };
        let fixed = if let Some((param, member)) = member_stubs.get(&f).copied() {
            let mut sub2 = crate::compile::material::Substitution::new(
                &mut own.types,
                &mut own.interner,
                params.iter().cloned().zip(values.iter().cloned()).collect(),
                owner.to_string(),
            );
            let concrete = sub2.run(param);
            resolve_member_stub(
                own, home, stubs, first_fid, concrete, member, owner, &params, &values, memo,
                requests, depth,
            )?
        } else if rut_core::scope_of(f) == rut_core::BOOT_SCOPE && f >= first_fid {
            // a plain operand at or above the window start names THIS
            // program's template window (own fn refs compile to plain
            // boot-scope ids; a scope-qualified operand is a FOREIGN
            // concrete fn reference — link maps it through the module
            // bases, exactly as a directory-compiled instantiation's
            // would, so it passes through untouched)
            materialize_body(
                own,
                home,
                stubs,
                first_fid,
                f,
                &params,
                &values,
                owner,
                memo,
                requests,
                depth + 1,
            )?
        } else {
            f
        };
        match op {
            Op::Call { func, .. } => *func = fixed,
            Op::CallM { func, .. } => *func = fixed,
            Op::MakeClosure { func, .. } => *func = fixed,
            _ => {}
        }
    }
    // T2b (lay): each demanded (payload × iface) pair binds every member
    // slot here, in the OWNER's numbering: the payload row is the row the
    // materialized MakeRecord bakes (the row the runtime dispatch reads),
    // and link re-lays both keys at merge. Only an OWNER-SERVED member
    // fills — a consumer-local payload's pair was demanded by the
    // requester's own check at its boundary, and an unsatisfiable
    // co-mention has no member to bind (skipped, never an error)
    for (payload, iface_id) in iface_fills {
        let payload_row = payload; // the packed id the stub resolver speaks
        let payload = own.types.dense(payload) as usize;
        let method_names: Vec<rut_core::sym::IdentId> = own.ifaces[iface_id as usize]
            .methods
            .iter()
            .map(|m| m.name)
            .collect();
        for (midx, mname) in method_names.into_iter().enumerate() {
            let member_text = own.interner.name(mname).to_string();
            if !owner_serves_member(own, payload_row, &member_text) {
                continue;
            }
            let Some(slot) = own
                .iface_slots
                .iter()
                .position(|&(r, m)| r == iface_id && m == midx as u32)
            else {
                continue;
            };
            if own
                .vtables
                .get(payload)
                .and_then(|row| row.get(slot))
                .is_some_and(|f| f.is_some())
            {
                continue; // a pack-time fill or an earlier demand — idempotent
            }
            let fid = resolve_member_stub(
                own,
                home,
                stubs,
                first_fid,
                payload_row,
                mname,
                owner,
                &params,
                &values,
                memo,
                requests,
                depth,
            )?;
            if payload >= own.vtables.len() {
                own.vtables.resize_with(payload + 1, Vec::new);
            }
            let total = own.iface_slots.len();
            let row = &mut own.vtables[payload];
            if row.len() < total {
                row.resize(total, None);
            }
            row[slot] = Some(fid);
        }
    }
    own.inst_types.extend(minted);
    own.funcs[new_fid as usize] = body;
    Ok(new_fid)
}

/// The T2b fill's member probe: only an OWNER-SERVED member fills —
/// the surface's own compiled member (a local inherent on the concrete
/// row) or the generic class's member template. A consumer-local
/// payload (the mirror-stub shape) was demanded by the requester's own
/// check at its boundary; an unsatisfiable co-mention has no member to
/// bind and the pair is skipped.
fn owner_serves_member(own: &Program, concrete: rut_core::types::TypeId, member: &str) -> bool {
    // (a) the owner's own compiled member on this very row
    if own.surface.inherents.iter().any(|ih| {
        ih.target == concrete
            && ih
                .methods
                .iter()
                .any(|m| m.local != 0 && own.interner.name(m.name) == member)
    }) {
        return true;
    }
    // (b) an OWN generic class instantiation: the member template of
    // the matching arity exists
    let concrete_text = own.type_name(concrete).to_string();
    if let Some((base, args_text)) = concrete_text.split_once('<') {
        let args_text = args_text.strip_suffix('>').unwrap_or(args_text);
        let arity = split_top_commas(args_text).len();
        return own.templates.iter().any(|t| {
            t.param_rows.len() == arity
                && match &t.kind {
                    rut_core::binary::TemplateKind::Method { data, name } => {
                        own.interner.name(*data) == base && own.interner.name(*name) == member
                    }
                    rut_core::binary::TemplateKind::Free { .. } => false,
                }
        });
    }
    false
}

/// Resolve one bound-member stub: the concrete argument's own surface
/// answers. An owner-local class binds its compiled fn directly; an
/// instantiation of an OWN generic class materializes the member
/// template recursively; a consumer-local class mints the mirror stub
/// whose ledger row names the consumer (link unifies it with the real
/// body there — the carried-member law's direction).
#[allow(clippy::too_many_arguments)]
fn resolve_member_stub(
    own: &mut Program,
    home: &Homes,
    stubs: &[rut_core::binary::TemplateStub],
    first_fid: u32,
    concrete: rut_core::types::TypeId,
    member: rut_core::sym::IdentId,
    owner: &str,
    _params: &[rut_core::types::TypeId],
    _values: &[rut_core::types::TypeId],
    memo: &mut HashMap<(u32, Vec<rut_core::types::TypeId>), u32>,
    requests: &mut Vec<(String, rut_lir::check::InstRequest)>,
    depth: u32,
) -> Result<u32, String> {
    let member_text = own.interner.name(member).to_string();
    // (a) the owner's own compiled member
    let local = own.surface.inherents.iter().find_map(|ih| {
        if ih.target != concrete {
            return None;
        }
        ih.methods
            .iter()
            .find(|m| own.interner.name(m.name) == member_text && m.local != 0)
            .map(|m| m.local)
    });
    if let Some(local) = local {
        return Ok(local);
    }
    // (b) an instantiation of an OWN generic class: the member template
    // materializes at the concrete's arguments
    let concrete_text = own.type_name(concrete).to_string();
    if let Some((base, args_text)) = concrete_text.split_once('<') {
        let args_text = args_text.strip_suffix('>').unwrap_or(args_text);
        let arg_rows: Vec<rut_core::types::TypeId> = split_top_commas(args_text)
            .iter()
            .filter_map(|t| {
                own.interner
                    .lookup(t)
                    .and_then(|id| own.types.dense_id_of_name(id))
            })
            .collect();
        let trow = own.templates.iter().find(|t| {
            t.param_rows.len() == arg_rows.len()
                && match &t.kind {
                    rut_core::binary::TemplateKind::Method { data, name } => {
                        own.interner.name(*data) == base && own.interner.name(*name) == member_text
                    }
                    rut_core::binary::TemplateKind::Free { name } => {
                        own.interner.name(*name) == base
                    }
                }
        });
        if let Some(trow) = trow {
            let (f, p) = (trow.fid, trow.param_rows.clone());
            return materialize_body(
                own,
                home,
                stubs,
                first_fid,
                f,
                &p,
                &arg_rows,
                owner,
                memo,
                requests,
                depth + 1,
            );
        }
    }
    // (c) a consumer-local class: the mirror stub — a bodyless fn whose
    // ledger row names the concrete's HOME unit; link unifies it with
    // the real body there
    let home_scope = rut_core::scope_of(concrete);
    if home_scope != rut_core::BOOT_SCOPE {
        if let Some(home_name) = home.name.get(&home_scope).cloned() {
            let served = home
                .members
                .get(&home_scope)
                .and_then(|m| m.get(&concrete))
                .map(|set| set.contains(&member_text))
                .unwrap_or(false);
            if !served {
                return Err(format!(
                    "`{owner}`'s template calls `{concrete_text}.{member_text}`, but the type's \
                     own surface does not carry that member — the shape the consumer spelled is \
                     not satisfied; re-pack after fixing the caller"
                ));
            }
            let data_ident = own
                .interner
                .lookup(&concrete_text)
                .unwrap_or_else(|| own.interner.intern(&concrete_text));
            let stub_name = own.interner.intern(&format!(
                "mirror#{}${}${}",
                home_name, concrete_text, member_text
            ));
            let fid = own.funcs.len() as u32;
            own.funcs.push(rut_core::binary::FuncCode {
                name: stub_name,
                params: vec![],
                ret: rut_core::types::TY_NIL,
                is_method: true,
                n_captures: 0,
                regs: vec![],
                argv: vec![],
                labels: vec![],
                code: vec![],
                spans: vec![],
                pos: vec![],
                host_id: None,
            });
            own.inst_fns.push(rut_core::binary::InstFn {
                owner: own.interner.intern(&home_name),
                kind: rut_core::binary::InstFnKind::Method {
                    data: data_ident,
                    name: member,
                    subst: vec![],
                    origins: vec![],
                },
                fid,
            });
            return Ok(fid);
        }
    }
    Err(format!(
        "`{owner}`'s template calls `{concrete_text}.{member_text}`, but no surface carries that \
         member — corrupt or doctored, re-pack the directory"
    ))
}

/// Rewrite every `(from, local)` type id inside `kind` to `(to, local)` —
/// the staging half's scope rebase: the staged rows' inner ids spelling
/// the requester's scope move into the fresh staging scope's coordinate
/// system. Structural recursion over the type-carrying variants; the
/// leaves and non-type payloads pass.
fn remap_kind_scope(
    kind: &mut rut_core::types::TyKind,
    from: rut_core::id::ScopeId,
    to: rut_core::id::ScopeId,
) {
    if from == to {
        return;
    }
    let rb = |t: &mut rut_core::types::TypeId| {
        if rut_core::scope_of(*t) == from && *t != u32::MAX {
            *t = rut_core::pack(to, rut_core::local_of(*t));
        }
    };
    match kind {
        rut_core::types::TyKind::Nil
        | rut_core::types::TyKind::Prim(_)
        | rut_core::types::TyKind::Str
        | rut_core::types::TyKind::Bytes
        | rut_core::types::TyKind::Enum { .. }
        | rut_core::types::TyKind::IfaceObj { .. }
        | rut_core::types::TyKind::Opaque
        | rut_core::types::TyKind::Trace
        | rut_core::types::TyKind::DisposalContext => {}
        rut_core::types::TyKind::Array { elem }
        | rut_core::types::TyKind::Opt { elem }
        | rut_core::types::TyKind::Weak { elem }
        | rut_core::types::TyKind::Future { elem } => rb(elem),
        rut_core::types::TyKind::Data { fields } => {
            for f in fields.iter_mut() {
                rb(&mut f.ty);
            }
        }
        rut_core::types::TyKind::Fn { params, ret } => {
            for p in params.iter_mut() {
                rb(p);
            }
            rb(ret);
        }
    }
}
