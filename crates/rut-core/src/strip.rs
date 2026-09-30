//! Compile-time symbol stripping — the mangle/restore pair.
//!
//! A publisher can ship a compiled artifact whose function/type/field/
//! trait/method names and source positions are unreadable, keeping a
//! private **symbol table** sidecar that — when supplied at load time —
//! restores real names and line/col for `StackTrace` symbolication. The
//! VM changes zero lines: stack-trace symbolication is already lazy,
//! per-index, against the loaded program's interner + `pos` table, and
//! degrades to pc-only text on `(0, 0)`/empty.
//!
//! The mechanics are clean because every name in a `.rutc` is an
//! [`IdentId`] index into one [`Interner`], and the binary serializes
//! only the instance-local **tail** of the name table — "renaming" is
//! rewriting those tail strings ([`Interner::remap_tail`]). Ids,
//! opcodes, vtables, fn tables: untouched. No format-version bump —
//! tail strings are opaque, and a stripped binary decodes and verifies
//! identically (`0`/empty `pos` is the documented stripped encoding).
//!
//! The mangled shape is **`%N`** (`%0`, `%1`, … in sorted-union order):
//! `%` can never appear in an identifier (the lexer's ident charset is
//! alphanumeric + `_` + `$`) nor in a package spec (`valid_spec` is
//! `[a-zA-Z0-9_]+`), so a mangled string cannot collide with any
//! source-derived tail string by construction. A uniqueness assert over
//! the mangled set ∪ keep-set stays regardless — defense in depth.
//!
//! Mangling is ONE closure-wide string→string map applied to every
//! compiled group — never per-group random names: bundles link across
//! `.rutc` groups by name *text* at load (trait merge-by-name, the
//! instantiation-ledger keys), so a consistent map preserves all
//! cross-group identity for free (interning is content-addressed).
//!
//! The keep-set ([`KEEP`]-shaped walks in [`strip_programs`]) rides
//! verbatim: well-known names (never in the tail), `FuncCode.host_id`
//! (embedder registry keys), `SurfaceFn.host` (async registration
//! rows), `InstFnKind::HostThunk` names, `Program::exports` (hosts call
//! by string), and `InstTy::owner`/`InstFn::owner` (pkg specs feeding
//! load-time unification keys). `Program::name` is a plain `String`
//! field, not interned — it stays as-is.

use std::collections::{BTreeSet, HashMap};

use crate::binary::{Dec, Enc, InstFnKind, Program};

/// The sidecar magic — a private symbol table, never an entry inside
/// the bundle.
const MAGIC: &[u8; 4] = b"RUTS";
/// The sidecar format version.
const VERSION: u32 = 1;

/// One fn's symbolication data, in the fn table's order: the pc → byte
/// offset span table and its parallel pc → (line, col) positions — the
/// pair leaves the binary and rides the sidecar together.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SymFn {
    pub spans: Vec<(u32, u32)>,
    pub pos: Vec<(u32, u32)>,
}

/// One compiled module's section, keyed by module spec (the manifest
/// name — the same key `pack.rs` writes `<name>.rutc` under), fns in
/// table order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SymSection {
    pub spec: String,
    pub fns: Vec<SymFn>,
}

/// The restore half of a strip: mangled → original name rows, and per
/// module the span/pos tables taken out of the binaries. Deterministic:
/// the same programs in, the same bytes out ([`SymbolMap::to_bytes`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SymbolMap {
    /// `(mangled, original)` rows, sorted by mangled (which is `%N` —
    /// lexicographic on the row is numeric on N).
    pub names: Vec<(String, String)>,
    /// per compiled module, sorted by spec.
    pub sections: Vec<SymSection>,
}

impl SymbolMap {
    /// Serialize — the RUTS v1 wire: magic, version, the name rows
    /// (sorted by mangled), then the sections (sorted by spec), each
    /// fn's spans with the parallel positions. Little-endian
    /// everywhere, the module binary's byte law.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut e = Enc::default();
        e.bytes(MAGIC);
        e.u32(VERSION);
        e.u32(self.names.len() as u32);
        for (mangled, original) in &self.names {
            e.str(mangled);
            e.str(original);
        }
        e.u32(self.sections.len() as u32);
        for s in &self.sections {
            e.str(&s.spec);
            e.u32(s.fns.len() as u32);
            for f in &s.fns {
                e.u32(f.spans.len() as u32);
                for (pc, lo) in &f.spans {
                    e.u32(*pc);
                    e.u32(*lo);
                }
                for (line, col) in &f.pos {
                    e.u32(*line);
                    e.u32(*col);
                }
            }
        }
        e.out
    }

    /// The decode side of [`SymbolMap::to_bytes`] — a corrupted or
    /// truncated sidecar refuses loudly, never misrestores.
    pub fn from_bytes(bytes: &[u8]) -> Result<SymbolMap, String> {
        let mut d = Dec::new(bytes);
        let mut magic = [0u8; 4];
        d.bytes(&mut magic)?;
        if &magic != MAGIC {
            return Err("not a rut symbol table (bad magic)".into());
        }
        let ver = d.u32()?;
        if ver != VERSION {
            return Err(format!("unsupported symbol table version {ver}"));
        }
        let nnames = d.u32()? as usize;
        let mut names = Vec::with_capacity(nnames);
        for _ in 0..nnames {
            let mangled = d.str()?;
            let original = d.str()?;
            names.push((mangled, original));
        }
        let nsections = d.u32()? as usize;
        let mut sections = Vec::with_capacity(nsections);
        for _ in 0..nsections {
            let spec = d.str()?;
            let nfns = d.u32()? as usize;
            let mut fns = Vec::with_capacity(nfns);
            for _ in 0..nfns {
                let nspans = d.u32()? as usize;
                let mut spans = Vec::with_capacity(nspans);
                for _ in 0..nspans {
                    spans.push((d.u32()?, d.u32()?));
                }
                let mut pos = Vec::with_capacity(nspans);
                for _ in 0..nspans {
                    pos.push((d.u32()?, d.u32()?));
                }
                fns.push(SymFn { spans, pos });
            }
            sections.push(SymSection { spec, fns });
        }
        Ok(SymbolMap { names, sections })
    }

    /// The mangled → original lookup, built once per apply pass.
    fn restore_map(&self) -> HashMap<&str, &str> {
        self.names.iter().map(|(m, o)| (m.as_str(), o.as_str())).collect()
    }
}

/// Mangle every renameable tail name across the closure and strip every
/// fn's `spans`/`pos`, collecting the restore data. `groups` is one
/// (module-spec, &mut Program) per compiled group — exactly the set the
/// packer encodes as `.rutc` binaries. Deterministic: same programs in,
/// same map out.
///
/// Steps: (1) walk all programs + surfaces, collect the keep-set; (2)
/// collect the renameable union — every tail string not in the keep-set;
/// (3) sort, number `%0..`, build the name map; (4) per program:
/// [`Interner::remap_tail`] via the map, `mem::take` each fn's
/// `spans`/`pos` into the section (fn table order); (5) the uniqueness
/// assert — loud on any mangled ∩ kept collision (`%` makes it
/// unreachable; the assert is the defense in depth).
pub fn strip_programs(groups: &mut [(String, &mut Program)]) -> Result<SymbolMap, String> {
    // 1. the keep-set, as interned TEXT — every string a load-time or
    // host-facing path consults by name
    let mut keep: BTreeSet<String> = BTreeSet::new();
    for (_, prog) in &*groups {
        for f in &prog.funcs {
            if let Some(h) = f.host_id {
                keep.insert(prog.interner.name(h).to_string());
            }
        }
        for sf in &prog.surface.funcs {
            if let Some(h) = sf.host {
                keep.insert(prog.interner.name(h).to_string());
            }
        }
        for r in &prog.inst_fns {
            keep.insert(prog.interner.name(r.owner).to_string());
            if let InstFnKind::HostThunk { name } = &r.kind {
                keep.insert(prog.interner.name(*name).to_string());
            }
        }
        for r in &prog.inst_types {
            keep.insert(prog.interner.name(r.owner).to_string());
        }
        for (n, _) in &prog.exports {
            keep.insert(prog.interner.name(*n).to_string());
        }
    }
    // 2. the renameable union — every tail string no law consults,
    // deduplicated across the closure (interning is content-addressed
    // per program, so the same text may ride in several tails)
    let mut union: BTreeSet<String> = BTreeSet::new();
    for (_, prog) in &*groups {
        let wk = prog.interner.well_known_len() as usize;
        for name in &prog.interner.names()[wk..] {
            if !keep.contains(name.as_ref()) {
                union.insert(name.to_string());
            }
        }
    }
    // 3. sorted union → `%0..` — the deterministic numbering; the name
    // map rows are (mangled, original), sorted by mangled
    let names: Vec<(String, String)> = union
        .iter()
        .enumerate()
        .map(|(i, original)| (format!("%{i}"), original.clone()))
        .collect();
    let forward: HashMap<&str, &str> =
        names.iter().map(|(m, o)| (o.as_str(), m.as_str())).collect();
    // 4. rewrite each tail through the one closure-wide map, take the
    // symbolication tables out
    let mut sections: Vec<SymSection> = Vec::with_capacity(groups.len());
    for (spec, prog) in groups.iter_mut() {
        let prog: &mut Program = prog;
        prog.interner
            .remap_tail(|s| forward.get(s).copied().unwrap_or(s).to_string());
        let mut fns = Vec::with_capacity(prog.funcs.len());
        for f in prog.funcs.iter_mut() {
            fns.push(SymFn {
                spans: std::mem::take(&mut f.spans),
                pos: std::mem::take(&mut f.pos),
            });
        }
        sections.push(SymSection { spec: spec.clone(), fns });
    }
    sections.sort_by(|a, b| a.spec.cmp(&b.spec));
    // 5. the uniqueness assert: mangled ∪ kept must not collide. `%`
    // cannot appear in an ident or a pkg spec, so this is unreachable
    // for source-derived strings — an embedder key with a `%` in it is
    // exactly what the loud failure is for.
    let mut seen = keep;
    for (mangled, _) in &names {
        if !seen.insert(mangled.clone()) {
            return Err(format!(
                "symbol strip: mangled name `{mangled}` collides with a kept \
                 name (host ABI, export, or pkg spec) — refusing"
            ));
        }
    }
    Ok(SymbolMap { names, sections })
}

/// The decode side: restore names (exact-key lookup — kept strings and
/// non-keys pass through) and reattach `spans`/`pos` per section, fns in
/// table order. Sections whose spec is absent from `groups` are
/// reported back as skipped specs (the caller decides to warn); a map
/// from a different build simply matches nothing — tolerated, not an
/// error. Re-applying a map is a no-op (restored names are not keys;
/// the tables reattach to the same values).
pub fn apply_symbols(groups: &mut [(String, &mut Program)], map: &SymbolMap) -> Vec<String> {
    let restore = map.restore_map();
    for (_, prog) in groups.iter_mut() {
        let prog: &mut Program = prog;
        prog.interner
            .remap_tail(|s| restore.get(s).copied().unwrap_or(s).to_string());
    }
    let mut skipped: Vec<String> = Vec::new();
    for section in &map.sections {
        let Some((_, prog)) = groups.iter_mut().find(|(spec, _)| *spec == section.spec) else {
            skipped.push(section.spec.clone());
            continue;
        };
        let prog: &mut Program = prog;
        // a fn-count mismatch means the section was taken from a
        // different build — matches nothing, tolerated silently (the
        // same law as the name rows)
        if prog.funcs.len() != section.fns.len() {
            continue;
        }
        for (f, sy) in prog.funcs.iter_mut().zip(&section.fns) {
            f.spans = sy.spans.clone();
            f.pos = sy.pos.clone();
        }
    }
    skipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary::{FuncCode, Surface, SurfaceFn};
    use crate::types::{TY_I32, TY_NIL};

    /// A synthetic compiled program with one renameable fn (`worker`),
    /// one export (`run`), one host thunk row (`rt:tick`), and one
    /// instantiation row owned by pkg spec `util` — every keep-set kind
    /// present. `extra` interns additional renameable names.
    fn synth(name: &str, extra: &[&str]) -> Program {
        let mut p = Program::default();
        p.name = name.to_string();
        let worker = p.interner.intern("worker");
        let run = p.interner.intern("run");
        let tick = p.interner.intern("rt:tick");
        let util = p.interner.intern("util");
        for e in extra {
            p.interner.intern(e);
        }
        p.funcs.push(FuncCode {
            name: worker,
            params: vec![],
            ret: TY_NIL,
            is_method: false,
            n_captures: 0,
            regs: vec![],
            argv: vec![],
            labels: vec![],
            code: vec![],
            spans: vec![(0, 4), (3, 19)],
            pos: vec![(2, 5), (4, 9)],
            host_id: Some(tick),
        });
        p.exports.push((run, 0));
        p.inst_fns.push(crate::binary::InstFn {
            owner: util,
            kind: InstFnKind::HostThunk { name: tick },
            fid: 0,
        });
        p.inst_types.push(crate::binary::InstTy {
            owner: util,
            decl: p.interner.intern("Holder"),
            args: vec![],
            ty: TY_I32,
        });
        p.surface = Surface {
            funcs: vec![SurfaceFn {
                name: run,
                params: vec![],
                ret: TY_NIL,
                local: 0,
                is_async: true,
                host: Some(tick),
            }],
            ..Default::default()
        };
        p
    }

    #[test]
    fn keep_set_survives_and_the_rest_mangles() {
        let mut a = synth("app", &["secret_a"]);
        let mut b = synth("util", &["secret_b"]);
        let keep_cases = ["run", "rt:tick", "util"];
        let before: Vec<_> = keep_cases
            .iter()
            .map(|k| (k.to_string(), a.interner.lookup(k).map(|id| (id, a.interner.name(id).to_string()))))
            .collect();
        let map = strip_programs(&mut [("app".into(), &mut a), ("util".into(), &mut b)])
            .expect("strip");
        // keep-set verbatim, same ids
        for (k, before) in &before {
            let (id, _) = before.as_ref().expect("interned keep name");
            assert_eq!(a.interner.name(*id), k, "keep-set string rewritten");
        }
        // everything else is %N-shaped and NOT the original text
        for p in [&a, &b] {
            for n in &p.interner.names()[p.interner.well_known_len() as usize..] {
                if keep_cases.contains(&n.as_ref()) {
                    continue;
                }
                assert!(
                    n.starts_with('%') && n[1..].bytes().all(|b| b.is_ascii_digit()),
                    "renameable name `{n}` did not mangle"
                );
            }
            assert!(p.interner.lookup("worker").is_none());
        }
        // closure-wide consistency: `worker` (and every extra) got the
        // SAME mangled string in both programs
        assert_eq!(a.interner.lookup("%0"), b.interner.lookup("%0"));
        // spans/pos leave the binaries together
        assert!(a.funcs[0].spans.is_empty() && a.funcs[0].pos.is_empty());
        // the map carries the restore rows and the sections, sorted
        assert!(map.names.iter().all(|(m, _)| m.starts_with('%')));
        assert!(map.names.windows(2).all(|w| w[0].0 < w[1].0), "sorted by mangled");
        assert!(map
            .names
            .iter()
            .any(|(_, o)| o == "worker"), "the original rides the map");
        assert_eq!(map.sections.len(), 2);
        assert_eq!(map.sections[0].spec, "app");
        assert_eq!(map.sections[1].spec, "util");
        assert_eq!(map.sections[0].fns.len(), 1);
        assert_eq!(map.sections[0].fns[0].spans, vec![(0, 4), (3, 19)]);
        assert_eq!(map.sections[0].fns[0].pos, vec![(2, 5), (4, 9)]);
    }

    #[test]
    fn strip_is_deterministic() {
        let one = {
            let mut a = synth("app", &["zeta", "alpha"]);
            let mut b = synth("util", &["mid"]);
            strip_programs(&mut [("app".into(), &mut a), ("util".into(), &mut b)])
                .expect("strip")
                .to_bytes()
        };
        let two = {
            let mut a = synth("app", &["zeta", "alpha"]);
            let mut b = synth("util", &["mid"]);
            strip_programs(&mut [("app".into(), &mut a), ("util".into(), &mut b)])
                .expect("strip")
                .to_bytes()
        };
        assert_eq!(one, two, "same programs in => byte-identical sidecar");
    }

    #[test]
    fn sidecar_round_trips_byte_exact() {
        let mut a = synth("app", &["s1"]);
        let mut b = synth("util", &[]);
        let map = strip_programs(&mut [("app".into(), &mut a), ("util".into(), &mut b)])
            .expect("strip");
        let bytes = map.to_bytes();
        let back = SymbolMap::from_bytes(&bytes).expect("decode");
        assert_eq!(back, map, "row-for-row round trip");
        assert_eq!(back.to_bytes(), bytes, "byte-exact round trip");
        // bad magic and truncation refuse loudly
        assert!(SymbolMap::from_bytes(b"JUNKJUNK").is_err());
        assert!(SymbolMap::from_bytes(&bytes[..bytes.len() - 3]).is_err());
        let mut stale = bytes.clone();
        stale[4..8].copy_from_slice(&999u32.to_le_bytes());
        assert!(SymbolMap::from_bytes(&stale).is_err());
    }

    #[test]
    fn stripped_program_encodes_and_decodes_with_ids_intact() {
        let mut p = synth("app", &["quiet"]);
        let before: Vec<u32> = p.funcs.iter().map(|f| f.name.0).collect();
        let exports_before: Vec<(u32, u32)> =
            p.exports.iter().map(|(n, f)| (n.0, *f)).collect();
        let mut b = synth("util", &[]);
        strip_programs(&mut [("app".into(), &mut p), ("util".into(), &mut b)])
            .expect("strip");
        let bytes = crate::binary::encode(&p);
        let q = crate::binary::decode(&bytes).expect("a stripped binary decodes");
        // ids unchanged — only the tail text behind them moved
        assert_eq!(q.funcs.iter().map(|f| f.name.0).collect::<Vec<_>>(), before);
        assert_eq!(
            q.exports.iter().map(|(n, f)| (n.0, *f)).collect::<Vec<_>>(),
            exports_before
        );
        assert_eq!(q.name_of(q.exports[0].0), "run", "exports kept verbatim");
        assert_eq!(q.name_of(q.funcs[0].host_id.unwrap()), "rt:tick");
        assert!(q.name_of(q.funcs[0].name).starts_with('%'));
        assert!(q.funcs[0].spans.is_empty() && q.funcs[0].pos.is_empty());
    }

    #[test]
    fn apply_symbols_restores_reapply_is_noop_and_foreign_maps_match_nothing() {
        let mut a = synth("app", &["s1"]);
        let mut b = synth("util", &[]);
        let worker_id = a.interner.lookup("worker").expect("pre-strip id");
        let map = strip_programs(&mut [("app".into(), &mut a), ("util".into(), &mut b)])
            .expect("strip");
        let skipped = apply_symbols(&mut [("app".into(), &mut a), ("util".into(), &mut b)], &map);
        assert!(skipped.is_empty(), "every section found its spec");
        // names restore exactly; the id keeps its slot
        assert_eq!(a.interner.name(worker_id), "worker");
        assert_eq!(a.interner.lookup("worker"), Some(worker_id));
        assert_eq!(a.funcs[0].spans, vec![(0, 4), (3, 19)]);
        assert_eq!(a.funcs[0].pos, vec![(2, 5), (4, 9)]);
        // re-apply is a no-op
        let snapshot = crate::binary::encode(&a);
        let skipped2 =
            apply_symbols(&mut [("app".into(), &mut a), ("util".into(), &mut b)], &map);
        assert!(skipped2.is_empty());
        assert_eq!(crate::binary::encode(&a), snapshot, "re-apply changed nothing");
        // a foreign map: nothing matches, nothing errors; a section
        // whose spec is absent comes back skipped
        let mut c = synth("other", &[]);
        let foreign_skipped = apply_symbols(&mut [("other".into(), &mut c)], &map);
        assert_eq!(foreign_skipped, vec!["app".to_string(), "util".to_string()]);
        assert_eq!(c.funcs[0].spans.len(), 2, "foreign map touches nothing");
        assert!(c.interner.lookup("worker").is_some());
        // an empty map is the degenerate foreign case
        let empty = SymbolMap::default();
        let skipped3 = apply_symbols(&mut [("other".into(), &mut c)], &empty);
        assert!(skipped3.is_empty(), "no sections to skip");
    }
}
