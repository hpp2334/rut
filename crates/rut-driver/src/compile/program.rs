//! The one-module compiler: [`compile_program`] /
//! [`compile_program_resolved`]. One ~1,400-line function by decision —
//! the phases share too much live state to split without a borrow
//! fight; the module owns the surface snapshot, the seed attach, and
//! the debug dumps.

use rut_lir::check::{Ctx, FnKey, Inst};
use rut_lexer::diag::Diag;
use rut_ast::dump;
use rut_parser::{parse, Mode};
use rut_core::binary::Program;
use rut_core::ops::Op;
use rut_core::types::{Repr, TyKind, TypeId, TY_I32, TY_NIL};
use rut_core::{IdentId, sym};

use super::dump::ir_dump_of;
use super::seeds::Seeds;

/// The marker's wire code (v20): the closed set is validated at
/// collect, so an unknown word never reaches the surface build.
fn marker_code(m: Option<rut_core::IdentId>) -> u8 {
    match m {
        None => rut_core::binary::MARKER_NONE,
        Some(w) if w == rut_core::sym::DISPOSAL_MARKER => rut_core::binary::MARKER_DISPOSAL,
        Some(w) if w == rut_core::sym::ITERABLE_MARKER => rut_core::binary::MARKER_ITERABLE,
        Some(w) if w == rut_core::sym::CONSTRUCTOR_MARKER => rut_core::binary::MARKER_CONSTRUCTOR,
        Some(_) => rut_core::binary::MARKER_NONE,
    }
}

fn respell_surface_ty(
    types: &rut_core::types::TypeTable,
    t: TypeId,
    own_scope: rut_core::ScopeId,
    dep_scope: rut_core::ScopeId,
    cx_name: rut_core::IdentId,
    cx_ty: TypeId,
) -> TypeId {
    let s = rut_core::id::scope_of(t);
    if s == own_scope {
        let dense = types
            .scope_base
            .get(s as usize)
            .copied()
            .unwrap_or(0)
            + rut_core::id::local_of(t);
        if let Some(row) = types.types.get(dense as usize) {
            // the engine-minted cx record is THIS unit's row: a carried
            // surface's RunContext re-spells to it by name, so the
            // crossing check sees one type
            if row.name == cx_name {
                return cx_ty;
            }
        }
        if own_scope != dep_scope {
            return rut_core::pack(dep_scope, rut_core::id::local_of(t));
        }
    }
    t
}

/// A compiled module. `program` still carries scope-qualified ids — the
/// driver links a graph and flattens once.
pub struct ProgramOutput {
    pub diags: Vec<Diag>,
    pub ast_dump: String,
    pub ast_json: String,
    pub ir_dump: String,
    pub program: Option<Program>,
    /// the instantiation requests this unit routed to declaring
    /// packages (owner-anchored generics): `(owner, decl, args)` — the
    /// graph resolves them after the walk
    pub requests: Vec<rut_lir::check::InstRequest>,
    /// the exported generics whose templates could not lower as
    /// instantiation-independent bodies — `(decl text, construct)`.
    /// FATAL at pack (the pinned stability law); an ordinary compile
    /// runs untouched (the window drops, the unit's own diags are not
    /// the unit's error).
    pub template_failures: Vec<(String, String)>,
}

/// Compile one module under `scope`, binding used function surfaces.
/// Does not flatten or encode. Each
/// bound use carries its exporter's spec; no origin map —
/// the single-file law, so the orphan check is inert here beyond the
/// bound names' origins.
pub fn compile_program(
    src: &str,
    mode: Mode,
    module_name: &str,
    scope: rut_core::ScopeId,
    uses: &[(rut_core::ScopeId, rut_core::binary::Surface, String)],
) -> ProgramOutput {
    compile_program_resolved(
        src,
        mode,
        module_name,
        scope,
        uses,
        !uses.is_empty(),
        &Seeds::none(),
        &rut_lir::check::ModInputs::flat(),
        &[],
    )
}

/// As [`compile_program`] but with an explicit `allow_uses` flag — the
/// graph compiler resolves every specifier itself — and with `seeds`
/// carrying the owner side of consumer instantiation requests: the
/// requesters' type descriptors (one sparse block per requester scope)
/// and the instantiations to materialize before the compile roots.
/// `mods` carries the file-module inputs (phase 3): the concatenated
/// text's file layout and the scope tree — flat for single-file
/// packages. `own_uses` carries the own-package use bindings the
/// graph validation recorded: `(importing file mod path, bound name,
/// target module)` — the bare-name ladder resolves these through the
/// target module's decls, per file.
#[allow(clippy::too_many_arguments)]
pub fn compile_program_resolved(
    src: &str,
    mode: Mode,
    module_name: &str,
    scope: rut_core::ScopeId,
    uses: &[(rut_core::ScopeId, rut_core::binary::Surface, String)],
    allow_uses: bool,
    seeds: &Seeds<'_>,
    mods: &rut_lir::check::ModInputs,
    own_uses: &[(String, String, String)],
) -> ProgramOutput {
    let fail = |diags: Vec<Diag>, ast_dump: String, ast_json: String| ProgramOutput {
        diags,
        ast_dump,
        ast_json,
        ir_dump: String::new(),
        program: None,
        requests: Vec::new(),
        template_failures: Vec::new(),
    };
    let (mut ast, mut diags) = parse(src, mode);
    let tree = dump::to_dump_tree(&ast);
    let ast_dump = dump::render_text(&tree, src);
    let ast_json = dump::render_json(&tree);
    if !diags.is_empty() {
        return fail(diags, ast_dump, ast_json);
    }
    // Intern every used surface name, so a namespace use (`Math`)
    // resolves its members by name even though the member name is never
    // written in `use { .. }`. Surface names are
    // ids in the exporter's interner (`surface.names`) — re-interned by
    // text into this module's.
    for (_, surface, _) in uses {
        for f in &surface.funcs {
            ast.interner.intern(surface.names.name(f.name));
        }
        for c in &surface.consts {
            ast.interner.intern(surface.names.name(c.name));
        }
        for t in &surface.ifaces {
            ast.interner.intern(surface.names.name(t.name));
        }
    }
    let mut ctx = Ctx::new_scoped(&ast, scope);
    ctx.allow_uses = allow_uses;
    // the file-module inputs (phase 3): the file layout + scope tree
    // this unit resolves against (flat for single-file packages)
    ctx.mods = mods.clone();
    // the own-package use bindings (the intra-package import): the
    // graph validation's rows — a bare name bound by `use imp::{ X }`
    // resolves through the TARGET module's decls, in the file whose
    // `use` wrote it
    for (site, name, target) in own_uses {
        if let Some(id) = ast.interner.lookup(name) {
            ctx.own_uses.insert((site.clone(), id), target.clone());
        }
    }
    // the pkg names this unit's use statements name — a qualified head
    // spelling one of these is the cross-package door the position
    // grammar refuses; the diagnostic names the use fix
    for it in ast.module_items(ast.root).to_vec() {
        if let rut_ast::ast::ItemKind::Use { path, .. } = ast.item(it) {
            if let Some(&head) = path.first() {
                ctx.used_pkgs.insert(ast.name(head).to_string());
            }
        }
    }
    // the QUALIFIED-use leaves (phase 2): `use pkg::a::b::{ leaf }`
    // composes `a::b::leaf` — the spelling a host mod tree's surface
    // rows carry (`net::connect`) — keyed by the use head (the dep
    // pkg). A host mod row binds under its LEAF name through this
    // map; flat rows keep the bare-name lane below.
    let mut qualified_leaves: std::collections::HashMap<(String, String), rut_core::IdentId> =
        std::collections::HashMap::new();
    for it in ast.module_items(ast.root).to_vec() {
        if let rut_ast::ast::ItemKind::Use { path, names } = ast.item(it) {
            if path.len() < 2 {
                continue;
            }
            let head = ast.name(path[0]).to_string();
            let prefix = path[1..]
                .iter()
                .map(|&s| ast.name(s).to_string())
                .collect::<Vec<_>>()
                .join("::");
            for &n in names {
                let composed = format!("{prefix}::{}", ast.name(n));
                qualified_leaves.insert((head.clone(), composed), n);
            }
        }
    }
    // the orphan rule's locality input: this unit's own
    // pkg spec. Every decl's origin IS its module — no source crosses
    // a boundary, the origin map is gone.
    ctx.own_spec = module_name.to_string();
    // the binding gate: the names this module's `use` statements wrote
    // — read off the AST before any binding runs
    for it in ast.module_items(ast.root).to_vec() {
        if let rut_ast::ast::ItemKind::Use { names, .. } = ast.item(it) {
            ctx.used.extend(names.iter().copied());
        }
    }
    // trait names resolve across ALL uses (an impl may live in a
    // different module than the trait it implements):
    // trait text -> this module's registered trait id
    let mut ext_trait: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut trait_maps: Vec<(
        rut_core::ScopeId,
        &rut_core::binary::Surface,
        std::collections::HashMap<u32, u32>,
        String,
    )> = Vec::new();
    for (dep_scope, surface, origin) in uses {
        // ---- pass 1: interface declarations from every surface. A
        // descriptor registers when its name was used (the use-both
        // gate's binding); inst-descriptor rows ride nameless.
        let impl_trait_texts: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut tmap: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for t in &surface.ifaces {
            let text = surface.names.name(t.name);
            // an inst descriptor's spelling (`Readable<#T>`) is not a
            // declaration name: it rides for the trait map's remap and
            // registers nameless (no use-gate, no dispatch registry)
            let is_inst_desc = text.contains('<');
            let used_name = ast
                .interner
                .lookup(text)
                .filter(|id| ctx.used.contains(id));
            if !is_inst_desc && used_name.is_none() && !impl_trait_texts.contains(text) {
                continue;
            }
            let reg_name = if is_inst_desc { None } else { used_name };
            let cid = ctx.add_extern_iface_decl(reg_name, t, &surface.names);
            // the trait's origin pkg rides the binding
            if let Some(n) = reg_name {
                ctx.extern_origins.insert(n, origin.clone());
            }
            tmap.insert(t.local, cid);
            ext_trait.insert(text.to_string(), cid);
        }
        trait_maps.push((*dep_scope, surface, tmap, origin.clone()));
    }
    // ---- pass 2: type descriptors (types first: descriptors must be in
    // the table before any own type is interned — TypeTable::use_block;
    // names re-intern from the exporter's interner)
    for (_, surface, tmap, _) in &trait_maps {
        ctx.use_types(surface.types.clone(), &surface.names, &surface.scope_blocks, tmap);
    }
    // the seeds' requester rows join AFTER the roots (see the seed
    // half below the drain): the owner's own block must intern at the
    // exact locals the un-seeded compile gave it, or every surface id
    // an earlier-compiled consumer already bound goes stale.
    // ---- pass 3: fns, consts, types, impls, natives
    for (dep_scope, surface, tmap, origin) in trait_maps.iter() {
        let dep_scope = *dep_scope;
        for f in &surface.funcs {
            let text = surface.names.name(f.name);
            // the binding gate: a flat row binds by its bare name when
            // the unit wrote it (`use` leaf or code); a host mod row's
            // surface spelling is `path::name` (`net::connect`) — it
            // binds under the LEAF of the qualified use that named it
            // (`use myhost::net::{ connect }`), never under the bare
            // text and never through a flat use
            let id = if text.contains("::") {
                qualified_leaves
                    .get(&(origin.clone(), text.to_string()))
                    .copied()
            } else {
                ctx.ast.interner.lookup(text)
            };
            if let Some(id) = id {
                let exports: Vec<(rut_core::IdentId, TypeId)> = surface
                    .type_exports
                    .iter()
                    .map(|t| (t.name, rut_core::pack(dep_scope, t.local)))
                    .collect();
                let ret2 = ctx.respell_future_ret(f.ret, &exports);
                ctx.add_extern_fn(
                    id,
                    rut_core::pack(dep_scope, f.local),
                    f.params.clone(),
                    ret2,
                    f.is_async,
                    f.host
                        .map(|h| surface.names.name(h).to_string()),
                );
            }
        }
        // the async Future trait objects re-spell at the binding: a
        // linked pkg's `Future<Response>` ret is ITS unit's
        // instantiation (per-unit minted); the importing unit's await
        // keys the Future iface_inst per-unit, so the ret re-spells to
        // THIS unit's `Future<Response>` — the element decodes from the
        // instantiation's name (its only carrier) and resolves through
        // the same surface's own type exports (carried unconditionally
        // by `use_types`)
        // exported generic fns (the linkable-classes phase): the
        // placeholder signature + the declaring pkg — the call site
        // infers against it and requests the body
        for g in &surface.fn_generics {
            if let Some(id) = ctx.ast.interner.lookup(surface.names.name(g.name)) {
                let params = g
                    .params
                    .iter()
                    .map(|&p| ctx.intern(surface.names.name(p)))
                    .collect();
                ctx.add_extern_generic_fn(id, origin.clone(), params, g.args.clone(), g.ret);
            }
        }
        // the integer prims' numeric methods (core's `builtin impl`
        // blocks): bound AMBIENT — the method call
        // `x.wrapping_add(y)` needs no `use`, the primitives themselves
        // have none
        for (prim, n, i) in &surface.native_impls {
            let name = ctx.intern(surface.names.name(*n));
            ctx.add_builtin_impl(name, *prim, *i);
        }
        // the prelude's const stays USE-GATED: the builtin
        // names ride ambient (builtin-surface phase 2 — `core` is bound
        // into every unit), its const does not — `NAN` is
        // `use core::{NAN}` explicit. Other pkgs' consts bind with their
        // surface (they arrive only via that pkg's use anyway).
        let is_prelude = !surface.native_types.is_empty();
        for c in &surface.consts {
            if let Some(id) = ctx.ast.interner.lookup(surface.names.name(c.name)) {
                if !is_prelude || ctx.used.contains(&id) {
                    ctx.add_extern_const(id, c.ty, c.bits);
                }
            }
        }
        for t in &surface.type_exports {
            if let Some(id) = ctx.ast.interner.lookup(surface.names.name(t.name)) {
                // `scope: None` — the exporter's own scope (ordinary rows);
                // `Some(s)` — an explicit one (a boot-targeted alias;
                // the shared boot table needs no rebase)
                let scope = t.scope.unwrap_or(dep_scope);
                let ty_id = rut_core::pack(scope, t.local);
                // a used enum: the member paths resolve through the
                // registry; the descriptor row rode use_types above
                if let TyKind::Enum { members } = ctx.types.kind(ty_id).clone() {
                    ctx.add_extern_enum(id, ty_id, members);
                }
                ctx.add_extern_type(id, ty_id, t.is_class, t.newtype);
                // the type's origin pkg rides the binding
                ctx.extern_origins.insert(id, origin.clone());
                // a linked generic: the template row (its placeholder
                // fields came across in the carried block), the parameter
                // names in order, and the declaring pkg — the owner every
                // instantiation is requested from
                if t.is_generic {
                    let params = t
                        .params
                        .iter()
                        .map(|&p| ctx.intern(surface.names.name(p)))
                        .collect();
                    let template_row = rut_core::pack(scope, t.local);
                    ctx.add_extern_generic(
                        id,
                        origin.clone(),
                        params,
                        template_row,
                        t.is_class,
                        t.newtype,
                    );
                }
            }
        }
        // inherent method surfaces (the linkable-classes phase): the
        // used classes' method signatures + fn ids, each fn
        // scope-qualified with the exporter's scope. A generic class's
        // row carries the TEMPLATE (placeholder signatures, fn local
        // zero) — the call site substitutes and requests. The rows'
        // ids (target, params, rets) spell the EXPORTER'S own scope
        // verbatim — re-spelled here to pack(dep_scope, local), the
        // same law the type exports speak, or two linked pkgs (each
        // compiling as its own scope 1) collide on their ids.
        //
        // Gated on the surface actually carrying rows: the cx re-spell
        // mints this unit's RunContext row (interning its field name),
        // and a unit that binds no class surface must compile to the
        // same bytes whether or not the prelude is mounted.
        if !surface.inherents.is_empty() {
            let own_scope = surface
                .scope_blocks
                .last()
                .map(|(s, _)| *s)
                .unwrap_or(dep_scope);
            let cx_name = ctx.intern(rut_core::async_frame::RUN_CONTEXT_TYPE);
            let cx_ty = ctx.run_context_ty();
            for ih in &surface.inherents {
                let methods: Vec<rut_core::binary::SurfaceMethod> = ih
                    .methods
                    .iter()
                    .map(|m| rut_core::binary::SurfaceMethod {
                        name: ctx.intern(surface.names.name(m.name)),
                        params: m
                            .params
                            .iter()
                            .map(|p| {
                                respell_surface_ty(
                                    &ctx.types, *p, own_scope, dep_scope, cx_name, cx_ty,
                                )
                            })
                            .collect(),
                        ret: respell_surface_ty(
                            &ctx.types, m.ret, own_scope, dep_scope, cx_name, cx_ty,
                        ),
                        local: rut_core::pack(dep_scope, m.local),
                        has_self: m.has_self,
                        marker: m.marker,
                        generics: m
                            .generics
                            .iter()
                            .map(|&g| ctx.intern(surface.names.name(g)))
                            .collect(),
                    })
                    .collect();
                let target2 = respell_surface_ty(
                    &ctx.types, ih.target, own_scope, dep_scope, cx_name, cx_ty,
                );
                ctx.add_extern_inherent(target2, methods);
                // the class's declaring pkg anchors the mirror requests
                // (`MutCtx::set<A, R>` — a PLAIN class's generic method —
                // names its owner through this row; `owner_of_data`
                // reads it)
                ctx.decl_owner.insert(
                    ctx.types.type_at(target2).name,
                    origin.clone(),
                );
            }
        }
        // core's native surface: builtin containers, ifaces, and
        // compiler-lowered fns. Each row's ambient bit rules the
        // binding — an AMBIENT row (the `prelude builtin` spellings)
        // binds in every unit, no `use` needed and the use statement
        // itself stays redundant-but-legal; a non-ambient row (the
        // `pub builtin` spellings) resolves only through
        // `use core::{ .. }`, the exact pattern the namespace head
        // below already uses. Pkg mounting (ink/pouch/calc fns, consts,
        // namespace heads) stays use-gated.
        for (n, kind, ambient) in &surface.native_types {
            if let Some(id) = ctx.ast.interner.lookup(surface.names.name(*n)) {
                if *ambient || ctx.used.contains(&id) {
                    ctx.add_extern_native_type(id, *kind);
                }
            }
        }
        for (n, ambient) in &surface.native_fns {
            if let Some(id) = ctx.ast.interner.lookup(surface.names.name(*n)) {
                if *ambient || ctx.used.contains(&id) {
                    ctx.add_extern_native_fn(id);
                }
            }
        }
        // the namespace head (`Math`): bound like the natives — resolving
        // exactly when the module wrote it in `use { .. }`
        if let Some(ns) = &surface.namespace {
            if let Some(id) = ctx.ast.interner.lookup(surface.names.name(*ns)) {
                if ctx.used.contains(&id) {
                    ctx.add_extern_namespace(id);
                }
            }
        }
    }
    // the carried mirror rows join the instantiation maps (see
    // `register_carried_insts`) — after the surfaces bind (the extern
    // generics must be known) and before the unit's own collect
    ctx.register_carried_insts();
    ctx.collect();
    // the seed half: the requesters' rows join AFTER the roots — the
    // owner's own block must intern at the exact locals the un-seeded
    // compile gave it, or every surface id an earlier-compiled consumer
    // already bound goes stale. NEW structural rows the seeded bodies
    // intern attribute to THIS unit's own scope (the seed block sits
    // above the own block; the default greatest-base attribution would
    // steal the ids into the requester's space).
    // the entry surface's crossing contract is compile-time: bad signatures are source diagnostics, never call-time
    // surprises for the embedder
    ctx.check_entries();
    if !ctx.diags.is_empty() {
        diags.append(&mut ctx.diags.clone());
        return fail(diags, ast_dump, ast_json);
    }
    // module lets (load-time expression check)
    ctx.compile_module_lets();
    // compilation roots: every `entry fn` — the host-callable
    // surface. A module may have none (pure library shape);
    // `main` is an ordinary name, never a root by convention.
    let mut roots: Vec<Inst> = Vec::new();
    for name in ctx.entries.clone() {
        if ctx.find_free_fn(name) {
            // an async fn is not host-callable (its woven body takes the
            // (frame, cx) pair): it compiles at its call sites
            if ctx.ast.fn_decl(ctx.fn_nodes.iter().find(|(n, _)| *n == name).map(|(_, n)| *n).unwrap())
                .is_async
            {
                continue;
            }
            roots.push(Inst { key: FnKey::Free(name), subst: vec![], iface_origins: vec![] });
        }
    }
    // library surface: every non-generic `pub fn` is usable, so its body
    // must be compiled even when nothing local calls it. PRIVATE free
    // fns join the roots at the TAIL (see the dead-code-checking block
    // before the final drain) — the pre-snapshot root set stays exactly
    // what the published envelopes were packed with, so every id a
    // consumer may already have bound keeps its local.
    for (name, node) in ctx.fn_nodes.clone() {
        let is_pub = ctx.exports.iter().any(|(n, _)| *n == name);
        if !is_pub {
            continue;
        }
        if !ctx.ast.fn_decl(node).generics.is_empty() {
            continue;
        }
        // async fns ride the weave at their call sites
        if ctx.ast.fn_decl(node).is_async {
            continue;
        }
        roots.push(Inst { key: FnKey::Free(name), subst: vec![], iface_origins: vec![] });
    }
    // library surface, class half (the linkable-classes phase): every
    // non-generic data's pub methods are callable from a
    // consumer, so their bodies compile even when nothing local calls
    // them — the inherent surface rows carry the fn ids. Generic
    // classes monomorphize per instantiation (the owner-side request
    // machinery seeds their bodies); async and generic METHODS stay
    // call-site shapes and cross no surface. Private methods join at
    // the tail (the dead-code-checking block).
    for (dname, d) in ctx.datas.clone() {
        if !d.generics.is_empty() {
            continue;
        }
        for (mname, mnode) in &d.methods {
            let md = ctx.ast.method_decl(*mnode);
            let exported = md.vis == Some(rut_ast::ast::Vis::Pub);
            if !exported || md.is_async || !md.generics.is_empty() {
                continue;
            }
            roots.push(Inst {
                key: FnKey::Method { data: dname, name: *mname },
                subst: vec![],
                iface_origins: vec![],
            });
        }
    }
    // library surface, enum half: the same eager roots for an enum's
    // pub methods — the surface rows carry their fn ids (the decl's
    // methods slot; enums are concrete, no template lane)
    for (ename, e) in ctx.enums.clone() {
        for (mname, mnode) in &e.methods {
            let md = ctx.ast.method_decl(*mnode);
            if md.vis != Some(rut_ast::ast::Vis::Pub) || md.is_async || !md.generics.is_empty() {
                continue;
            }
            roots.push(Inst {
                key: FnKey::Method { data: ename, name: *mname },
                subst: vec![],
                iface_origins: vec![],
            });
        }
    }
    for root in roots {
        if ctx.compile_queue(root).is_err() {
            diags.append(&mut ctx.diags);
            return fail(diags, ast_dump, ast_json);
        }
    }    if !ctx.diags.is_empty() {
        diags.append(&mut ctx.diags.clone());
        return fail(diags, ast_dump, ast_json);
    }
    diags.append(&mut ctx.diags);
    if ctx.drain_queue().is_err() || !ctx.diags.is_empty() {
        diags.append(&mut ctx.diags);
        return fail(diags, ast_dump, ast_json);
    }
    // vtable pass 1: collects the impl fills and queues their bodies
    // (they drain with the seeds below); the SHIPPED rows assemble at
    // the tail, once the settle makes every fill resolve. A fill body
    // that cannot compile is a unit error — the diag joins ctx.diags.
    if ctx.build_vtables().is_err() || !ctx.diags.is_empty() {
        diags.append(&mut ctx.diags);
        return fail(diags, ast_dump, ast_json);
    }
    // per-type disposal rows: `(target type, dispose func id)` per
    // `impl ..: Disposal` — the release path's table, flowing module→link
    // beside the vtables
    let disposal_impls = ctx.disposal_impls();
    if !ctx.diags.is_empty() {
        diags.append(&mut ctx.diags);
        return fail(diags, ast_dump, ast_json);
    }
    // finalize the entry table: `entry fn`s — the whole
    // host-callable surface (`main` is an ordinary name: a
    // `pub fn main` crosses no entry row, only the crossing-checked
    // `entry fn`s do).
    let mut exports: Vec<(IdentId, u32)> = Vec::new();
    for n in ctx.entries.clone() {
        if let Some(&f) = ctx.inst_map.get(&Inst { key: FnKey::Free(n), subst: vec![], iface_origins: vec![] }) {
            exports.push((n, f));
        }
    }
    // exported surface: every `pub` fn + its signature, for using modules
    let mut surface = rut_core::binary::Surface::default();
    for (name, _) in &ctx.exports {
        if let Some(&fid) = ctx.inst_map.get(&Inst { key: FnKey::Free(*name), subst: vec![], iface_origins: vec![] }) {
            if let Some(f) = ctx.funcs.get(fid as usize) {
                surface.funcs.push(rut_core::binary::SurfaceFn {
                    name: *name,
                    params: f.params.clone(),
                    ret: f.ret,
                    local: fid,
                    is_async: false,
                    host: None,
                });
            }
        }
    }
    // inherent method surface (the linkable-classes phase): one row
    // per class with exported methods — a plain class's row binds
    // the compiled fn ids; a generic class's row spells the
    // TEMPLATE (placeholder `#<param>` signatures, fn local zero),
    // and the consumer substitutes per instantiation and requests
    // the bodies. Method visibility is the pub law — `pub fn` crosses,
    // plain `fn` is module-private; async and generic METHODS cross no
    // surface (call-site shapes).
    for (dname, d) in ctx.datas.clone() {
        if d.methods.is_empty() {
            continue;
        }
        // the surface signatures resolve in the TYPE's module (phase 3)
        let saved_mod = ctx.cur_mod.clone();
        ctx.cur_mod = ctx.mod_of(d.node.id()).to_string();
        // the class's template substitution: each generic parameter
        // spells its `#<param>` placeholder row, `Self` — for a
        // GENERIC class — its own `#Self` placeholder (the
        // consumer's receiver instantiation substitutes it; a
        // plain class's `Self` is the class's own row, which
        // resolves through the carried block). `resolve_sig_ty`
        // binds `Self` at any structural depth.
        let generic = !d.generics.is_empty();
        let self_ph = if generic {
            Some(ctx.param_placeholder(sym::SELF_TY))
        } else {
            None
        };
        let env: Vec<(IdentId, TypeId)> = d
            .generics
            .iter()
            .map(|&p| (p, ctx.param_placeholder(p)))
            .collect();
        let mut methods = Vec::new();
        for (mname, mnode) in &d.methods {
            let md = ctx.ast.method_decl(*mnode);
            let exported = md.vis == Some(rut_ast::ast::Vis::Pub);
            // GENERIC methods cross too: the row carries the method's
            // own generic parameters, the call site substitutes, and
            // the bodies ride the request machinery
            if !exported || md.is_async {
                continue;
            }
            let method_generic = !md.generics.is_empty();
            let has_self = matches!(
                md.params.first().map(|p| ctx.ast.param(*p)),
                Some(rut_ast::ast::MemberKind::SelfParam(_))
            );
            // a generic method's signature resolves under ITS OWN
            // placeholder env layered on the class's (the class env for
            // the receiver's `Self`, the method env for its parameters)
            let mut menv = env.clone();
            menv.extend(
                md.generics
                    .iter()
                    .map(|&p| (p, ctx.param_placeholder(p))),
            );
            let self_ty = self_ph.unwrap_or(d.ty);
            let mut params = Vec::new();
            for p in md.params.iter().skip(if has_self { 1 } else { 0 }) {
                match ctx.ast.param(*p) {
                    rut_ast::ast::MemberKind::Param(rut_ast::ast::ParamData { ty: Some(t), .. }) => {
                        params.push(ctx.resolve_sig_ty(*t, &menv, Some(self_ty)));
                    }
                    _ => params.push(TY_I32),
                }
            }
            let ret = md
                .ret
                .map(|r| ctx.resolve_sig_ty(r, &menv, Some(self_ty)))
                .unwrap_or(TY_NIL);
            // a plain non-generic method's fn id: the eager class-method
            // roots compiled it; a generic class's or generic method's
            // body is per instantiation — zero, the request machinery
            // carries it
            let local = if generic || method_generic {
                0
            } else {
                ctx.inst_map
                    .get(&Inst {
                        key: FnKey::Method { data: dname, name: *mname },
                        subst: vec![],
                        iface_origins: vec![],
                    })
                    .copied()
                    .unwrap_or(0)
            };
            methods.push(rut_core::binary::SurfaceMethod {
                name: *mname,
                params,
                ret,
                local,
                has_self,
                // the designated-slot marker crosses the surface (the
                // consumer's for-of reads it): the checker validated the
                // closed set at collect
                marker: marker_code(md.marker),
                generics: if method_generic { md.generics.clone() } else { vec![] },
            });
        }
        if methods.is_empty() {
            continue;
        }
        surface.inherents.push(rut_core::binary::SurfaceInherent {
            target: d.ty,
            methods,
        });
        ctx.cur_mod = saved_mod;
    }
    // enum inherent rows: the pub law again — the row's target is the
    // enum's type id (its descriptor crosses in the carried type
    // block), methods are concrete (no template, no placeholder env),
    // and generic methods stay call-site shapes like everywhere else
    for (ename, e) in ctx.enums.clone() {
        if e.methods.is_empty() {
            continue;
        }
        // the surface signatures resolve in the ENUM's module (phase 3)
        let saved_mod = ctx.cur_mod.clone();
        ctx.cur_mod = ctx.mod_of(e.node).to_string();
        let mut methods = Vec::new();
        for (mname, mnode) in &e.methods {
            let md = ctx.ast.method_decl(*mnode);
            if md.vis != Some(rut_ast::ast::Vis::Pub) || md.is_async || !md.generics.is_empty() {
                continue;
            }
            let has_self = matches!(
                md.params.first().map(|p| ctx.ast.param(*p)),
                Some(rut_ast::ast::MemberKind::SelfParam(_))
            );
            let self_ty = e.ty;
            let mut params = Vec::new();
            for p in md.params.iter().skip(if has_self { 1 } else { 0 }) {
                match ctx.ast.param(*p) {
                    rut_ast::ast::MemberKind::Param(rut_ast::ast::ParamData { ty: Some(t), .. }) => {
                        params.push(ctx.resolve_sig_ty(*t, &[], Some(self_ty)));
                    }
                    _ => params.push(TY_I32),
                }
            }
            let ret = md
                .ret
                .map(|r| ctx.resolve_sig_ty(r, &[], Some(self_ty)))
                .unwrap_or(TY_NIL);
            let local = ctx
                .inst_map
                .get(&Inst {
                    key: FnKey::Method { data: ename, name: *mname },
                    subst: vec![],
                    iface_origins: vec![],
                })
                .copied()
                .unwrap_or(0);
            methods.push(rut_core::binary::SurfaceMethod {
                name: *mname,
                params,
                ret,
                local,
                has_self,
                marker: marker_code(md.marker),
                generics: vec![],
            });
        }
        if methods.is_empty() {
            continue;
        }
        surface.inherents.push(rut_core::binary::SurfaceInherent {
            target: e.ty,
            methods,
        });
        ctx.cur_mod = saved_mod;
    }

    // exported GENERIC fns (the linkable-classes phase): names +
    // placeholder signatures — the bodies exist per argument list, so
    // nothing compiles here; the consumer mints the mirror
    // instantiation and requests the body from this pkg
    for (name, node) in ctx.fn_nodes.clone() {
        let is_pub = ctx.exports.iter().any(|(n, _)| *n == name);
        if !is_pub {
            continue;
        }
        let fd = ctx.ast.fn_decl(node);
        if fd.generics.is_empty() || fd.is_async {
            continue;
        }
        // the placeholder signature resolves in the fn's module (phase 3)
        let saved_mod = ctx.cur_mod.clone();
        ctx.cur_mod = ctx.mod_of(node.id()).to_string();
        // the placeholder substitution: each generic parameter spells
        // its `#<param>` row (interned BEFORE the type snapshot below
        // carries it)
        let env: Vec<(IdentId, TypeId)> = fd
            .generics
            .iter()
            .map(|&p| (p, ctx.param_placeholder(p)))
            .collect();
        let args: Vec<TypeId> = fd
            .params
            .iter()
            .map(|p| match ctx.ast.param(*p) {
                rut_ast::ast::MemberKind::Param(rut_ast::ast::ParamData { ty: Some(t), .. }) => {
                    ctx.resolve_sig_ty(*t, &env, None)
                }
                _ => TY_I32,
            })
            .collect();
        let ret = fd.ret.map(|r| ctx.resolve_sig_ty(r, &env, None)).unwrap_or(TY_NIL);
        surface.fn_generics.push(rut_core::binary::SurfaceGenericFn {
            name,
            params: fd.generics.clone(),
            args,
            ret,
        });
        ctx.cur_mod = saved_mod;
    }

    // the GENERIC ifaces' declaration descriptors: the placeholder
    // instantiation of each (`IntoFlow<#E>`-shaped) — minted BEFORE the
    // type snapshot below so the descriptor's structural rows (`Flow<#E>`,
    // the fn-typed field spellings) ride the carried block. The trait
    // surface reads these ids.
    let mut generic_trait_rows: Vec<(IdentId, u32, usize)> = Vec::new();
    {
        let decls: Vec<(IdentId, u32, Vec<IdentId>, rut_ast::ast::NodeId)> = ctx
            .iface_decls
            .iter()
            .map(|(n, i)| (*n, i.id, i.generics.clone(), i.node))
            .collect();
        for (name, id, generics, inode) in decls {
            if id != u32::MAX {
                continue;
            }
            // the trait's signatures resolve in ITS module (phase 3)
            let saved_mod = ctx.cur_mod.clone();
            ctx.cur_mod = ctx.mod_of(inode).to_string();
            let ph_args: Vec<TypeId> =
                generics.iter().map(|&g| ctx.param_placeholder(g)).collect();
            let ph_id = ctx.mk_iface_inst(name, ph_args);
            ctx.cur_mod = saved_mod;
            generic_trait_rows.push((name, ph_id, generics.len()));
        }
    }

    // type surface: the whole non-boot block (so `(scope, local)` ids and
    // field layouts resolve in a using module) + the exported names.
    // The inherent surface above ran FIRST: its template signature rows
    // (`#<param>`, `#Self`, their structural spellings) are interned
    // before this snapshot carries them.
    {
        let boot_len = ctx.types.boot_len as usize;
        surface.types = ctx.types.types[boot_len..].to_vec();
        // the type block's interface-object references: each carried
        // `[interface] ..` row names an interface-table index the consumer's
        // `use_types` remaps through the interface map — generic ifaces'
        // per-argument-list INST DESCRIPTORS ride here (their base
        // declarations export above; neither row carries signatures —
        // dispatch reads the impl rows)
        let mut inst_traits: Vec<u32> = Vec::new();
        for d in &surface.types {
            if let rut_core::types::TyKind::IfaceObj { iface_id } = d.kind {
                if iface_id == u32::MAX || inst_traits.contains(&iface_id) {
                    continue;
                }
                // only GENERIC ifaces' per-argument-list descriptors — a
                // row whose spelled name has no `<` is a base
                // declaration, exported (with its methods) by its
                // declaring unit alone; re-exporting it bare would
                // collide with that row at link
                let Some(row) = ctx.ifaces.get(iface_id as usize) else { continue };
                if !ctx.interner.name(row.name).contains('<') {
                    continue;
                }
                inst_traits.push(iface_id);
            }
        }
        for tid in inst_traits {
            let Some(row) = ctx.ifaces.get(tid as usize) else { continue };
            if surface.ifaces.iter().any(|t| t.local == tid) {
                continue;
            }
            surface.ifaces.push(rut_core::binary::SurfaceIface {
                local: tid,
                name: row.name,
                generics: 0,
                methods: row.methods.clone(),
            });
        }
        for s in 0..ctx.types.scope_base.len() as u32 {
            if s == rut_core::BOOT_SCOPE as u32 || s == scope as u32 {
                continue;
            }
            let base = ctx.types.scope_base[s as usize];
            if base >= ctx.types.boot_len {
                surface.scope_blocks.push((s as rut_core::ScopeId, base - ctx.types.boot_len));
            }
        }
        // the unit's OWN block closes the list — `own_scope` reads it
        // back as the last row, which stays unambiguous even when blocks
        // are empty (a pkg with no own types ties every offset at the
        // boot line)
        let base = ctx.types.scope_base[scope as usize];
        if base >= ctx.types.boot_len {
            surface
                .scope_blocks
                .push((scope as rut_core::ScopeId, base - ctx.types.boot_len));
        }
        for (name, d) in &ctx.datas {
            surface.type_exports.push(rut_core::binary::SurfaceType {
                name: *name,
                local: rut_core::local_of(d.ty),
                is_class: d.kind == rut_lir::check::DataKind::Class,
                is_generic: !d.generics.is_empty(),
                newtype: d.newtype,
                params: if d.generics.is_empty() {
                    Vec::new()
                } else {
                    d.generics.clone()
                },
                scope: None,
            });
        }
        for (name, e) in &ctx.enums {
            surface.type_exports.push(rut_core::binary::SurfaceType {
                name: *name,
                local: rut_core::local_of(e.ty),
                is_class: false,
                is_generic: false,
                newtype: false,
                params: Vec::new(),
                scope: None,
            });
        }
        // type aliases: a single-target alias adds a row keyed
        // by the TARGET's id — importers need zero changes, cross-module
        // transparency by construction; `is_class`/`is_generic` false
        // (aliases are non-generic). Union aliases stay module-local.
        // A boot target (`pub type Meters = i64;`) keeps the shared boot
        // scope — its id needs no rebase.
        for (name, ty) in ctx.alias_exports() {
            let scope = rut_core::scope_of(ty);
            surface.type_exports.push(rut_core::binary::SurfaceType {
                name,
                local: rut_core::local_of(ty),
                is_class: false,
                is_generic: false,
                newtype: false,
                params: Vec::new(),
                scope: (scope == rut_core::BOOT_SCOPE).then_some(scope),
            });
        }
        // trait surface: declared ifaces with their
        // resolved signatures, keyed by the exporter's interface-table
        // index (`local`) — the key the carried `[interface] ..` descriptors
        // reference. Generic ifaces instantiate per argument list where
        // they are declared (their methods are per-inst, so the base
        // row carries none) but the DECLARATION itself crosses: a
        // consumer binds the trait's parameterized impls (`impl
        // Readable<T> for Source<T>`) through it.
        let trait_decls_snapshot: Vec<(IdentId, u32, Vec<IdentId>)> = ctx
            .iface_decls
            .iter()
            .map(|(n, i)| (*n, i.id, i.generics.clone()))
            .collect();
        for (name, id, generics) in &trait_decls_snapshot {
            if *id == u32::MAX {
                // a GENERIC trait's declaration row: the placeholder
                // instantiation's descriptor is the row that crosses —
                // `local` names a real trait-table entry (the reader
                // validates it) whose method signatures spell the
                // trait's `#<param>` placeholder leaves, so the consumer
                // binds the shape its dispatch and mirror requests need.
                // The name stays the clean trait name — the consumer's
                // use-gate binding, not a nameless inst-descriptor row.
                // (The mint itself ran BEFORE the type snapshot — the
                // `generic_trait_rows` pass above.)
                let Some(&(_, ph_id, generics_len)) =
                    generic_trait_rows.iter().find(|(n, _, _)| n == name)
                else {
                    continue;
                };
                surface.ifaces.push(rut_core::binary::SurfaceIface {
                    local: ph_id,
                    name: *name,
                    generics: generics_len,
                    methods: ctx.ifaces[ph_id as usize].methods.clone(),
                });
                continue;
            }
            if *id as usize >= ctx.ifaces.len() {
                continue;
            }
            surface.ifaces.push(rut_core::binary::SurfaceIface {
                local: *id,
                name: *name,
                generics: generics.len(),
                methods: ctx.ifaces[*id as usize].methods.clone(),
            });
        }
    }
    // the seed half attaches only AFTER the surface snapshot: the
    // surface's template rows must intern at the exact locals the
    // un-seeded compile gave them, or every surface id an
    // earlier-compiled consumer already bound goes stale (the seeded
    // bodies' fresh structural rows would otherwise land between).
    ctx.types.own_attribution = true;
    // the seed region's start: everything the seed blocks append from
    // here is the requesters' carried rows — the nested-instantiation
    // scan below walks exactly this range
    let seed_start = ctx.types.types.len();
    // ONE run per scope across ALL groups: two requesters' rows share
    // a scope (the classes of a common dependency), and a second base
    // registration would strand the first group's ids. Each group's
    // rows re-intern through ITS OWN interner first; identical texts
    // converge onto one ident here, so equal keys dedup safely.
    {
        let mut merged: std::collections::BTreeMap<(rut_core::ScopeId, u32), rut_core::types::RutType> =
            std::collections::BTreeMap::new();
        for group in seeds.groups {
            for ((s, l), row) in ctx.stage_seed_rows(group.rows.clone(), group.names) {
                merged.entry((s, l)).or_insert(row);
            }
        }
        let pad = rut_core::types::RutType {
            name: rut_core::sym::NIL,
            kind: rut_core::types::TyKind::Nil,
        };
        // one PADDED run per scope, from local 0: the row for local
        // L lands at run-start + L, and the block entry carries the
        // run's OFFSET WITHIN THE BUILT VEC — the runs concatenate
        // (two requesters' rows share no scope, but one attach serves
        // several), and an off-0 for every scope would point the
        // second-and-later scopes' ids at the FIRST run's rows.
        // `use_seed_rows` then sets scope_base[s] = attach-start + off
        // for every scope THIS TABLE does not already carry a block
        // under (a scope both units use is already registered with the
        // exporter's FULL surface copy — the sparse run never moves it;
        // the seed-scope-collision law lives on `use_seed_rows`), so
        // this table's dense of a requester's packed id `(s, L)` hits
        // the row exactly (the two sides spell every argument the
        // same way — the owner anchor's whole point). Absent locals ride
        // Nil shells.
        let mut types: Vec<rut_core::types::RutType> = Vec::new();
        let mut blocks: Vec<(rut_core::ScopeId, u32)> = Vec::new();
        let mut iter = merged.iter().peekable();
        while let Some((&(s, lo), _)) = iter.peek() {
            let s: rut_core::ScopeId = s;
            let run_off = types.len() as u32;
            types.extend((0..lo).map(|_| pad.clone()));
            blocks.push((s, run_off));
            let mut off = lo;
            loop {
                match iter.peek() {
                    Some(&(&(s2, l2), row)) if s2 == s => {
                        // pad any gap inside the scope's local space,
                        // then take the row at its local
                        while off < l2 {
                            types.push(pad.clone());
                            off += 1;
                        }
                        types.push(row.clone());
                        off += 1;
                        iter.next();
                    }
                    _ => break,
                }
            }
        }
        ctx.use_staged_seed_rows(types, &blocks);
        // the carried-type law's markers: the seed region's dense range
        // (types the owner sees as bare rows, never impls) and each
        // carried row's HOME spec (the mirror ledger's owner anchor)
        let seed_end = ctx.types.types.len() as u32;
        ctx.carried_dense.push((seed_start as u32, seed_end - seed_start as u32));
        for group in seeds.groups {
            for ((_, _), row) in &group.rows {
                let text = group.names.name(row.name).to_string();
                ctx.carried_names.entry(text).or_insert_with(|| group.requester.clone());
            }
        }
        for group in seeds.groups {
            ctx.seeded_insts.extend(group.insts.iter().cloned());
            ctx.seeded_fns.extend(group.fns.iter().cloned());
        }
    }
    let mut seeded_bodies: Vec<Inst> = Vec::new();
    // the graph-seeded instantiations (the owner side of consumer
    // requests): materialize each AFTER the roots (see the reseed note
    // in the fn-seed half) — the owner's unit carries the row, and the
    // requested method bodies queue after the drain

    for (decl, args, methods) in std::mem::take(&mut ctx.seeded_insts) {
        let Some(dname) = ctx.lookup_name(&decl) else {
            ctx.err(
                rut_lexer::span::Span::new(0, 0),
                format!("seeded instantiation `{decl}<..>` — no such type here"),
            );
            continue;
        };
        let d = ctx.find_data(dname).cloned();
        // a generic class's instantiation carries ONLY the class's own
        // arguments — a generic method's ride the request tail (below)
        let class_arity = d.as_ref().map_or(0, |d| d.generics.len());
        ctx.mk_data_inst(dname, args[..class_arity.min(args.len())].to_vec(), rut_lexer::span::Span::new(0, 0));
        if let Some(d) = d {
            // a generic CLASS's bodies substitute the class parameters
            // (a generic METHOD's own parameters ride the request's
            // tail — the caller's full substitution flattened: class
            // parameters first, the method's appended in declaration
            // order — `Flow<i32>.map<R>` crosses `[i32, i32]`);
            // a PLAIN class's seeded method (`MutCtx::set<A, R>`) reads
            // the args as the METHOD's own parameters, in declaration
            // order
            let plain_method_env = |ctx: &Ctx, mname: rut_core::IdentId| -> Vec<(rut_core::IdentId, rut_core::types::TypeId)> {
                d.methods
                    .iter()
                    .find(|(n, _)| *n == mname)
                    .map(|(_, node)| ctx.ast.method_decl(*node).generics.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .zip(args.iter().cloned())
                    .collect()
            };
            if !d.generics.is_empty() {
                let env: Vec<(rut_core::IdentId, rut_core::types::TypeId)> =
                    d.generics.iter().cloned().zip(args.iter().cloned()).collect();
                for m in &methods {
                    let Some(mname) = ctx.lookup_name(m) else { continue };
                    let mgen = d
                        .methods
                        .iter()
                        .find(|(n, _)| *n == mname)
                        .map(|(_, node)| ctx.ast.method_decl(*node).generics.clone())
                        .unwrap_or_default();
                    let mut full = env.clone();
                    for (g, t) in mgen.iter().zip(args[class_arity.min(args.len())..].iter()) {
                        if !full.iter().any(|(n, _)| n == g) {
                            full.push((*g, *t));
                        }
                    }
                    seeded_bodies.push(Inst {
                        key: FnKey::Method { data: dname, name: mname },
                        subst: full,
                        iface_origins: vec![],
                    });
                }
            } else {
                for m in &methods {
                    let Some(mname) = ctx.lookup_name(m) else { continue };
                    seeded_bodies.push(Inst {
                        key: FnKey::Method { data: dname, name: mname },
                        subst: plain_method_env(&ctx, mname),
                        iface_origins: vec![],
                    });
                }
            }
        }
    }
    // the seed half's NESTED class instantiations: the consumer's rows
    // (`Vec<Vec<i64>>` and its element `Vec<i64>`) ride the seed block,
    // and the group impl bodies recurse into the element's own impls —
    // the dispatch reads `inst_data`, which only `mk_data_inst` fills.
    // Register every seed row that parses as `Base<..>` with a bound
    // used generic; the args resolve to the closure's own rows (or the
    // boot rows for primitives).
    {
        // the scan registers BOTH used-generic and LOCAL-generic bases: an
        // owner's own class arrives as a carried row (the requester's
        // spelling), and the reseed drain's mk_data_inst must dedup onto
        // it (the type_inst cache) — or the admit re-derives a second row
        // and checks the members against the wrong id
        fn split_top_commas(s: &str) -> Vec<String> {
            let mut out = Vec::new();
            let mut depth = 0usize;
            let mut cur = String::new();
            for ch in s.chars() {
                match ch {
                    '<' => {
                        depth += 1;
                        cur.push(ch);
                    }
                    '>' => {
                        depth = depth.saturating_sub(1);
                        cur.push(ch);
                    }
                    ',' if depth == 0 => {
                        out.push(cur.trim().to_string());
                        cur = String::new();
                    }
                    _ => cur.push(ch),
                }
            }
            if !cur.trim().is_empty() {
                out.push(cur.trim().to_string());
            }
            out
        }
        // pass 1: the name -> id map over the whole table (the closure
        // copies and the boot rows answer)
        // FIRST row wins: the boot optionals' shell rows reuse their
        // element's name (`?bytes` is a row named "bytes"), and a seed
        // row's argument text must resolve to the primitive, never the
        // shell answer lane spelled over it. Only the two ranges whose
        // ids are well-defined here — boot and the seed region — feed
        // the map; the scan's candidates and their arguments live there.
        let mut by_name: std::collections::HashMap<String, TypeId> =
            std::collections::HashMap::new();
        for (i, t) in ctx.types.types.iter().enumerate() {
            if i >= ctx.types.boot_len as usize && i < seed_start as usize {
                continue;
            }
            by_name
                .entry(ctx.interner.name(t.name).to_string())
                .or_insert(ctx.types.id_for_pub(i as u32));
        }
        // pass 2: register the seed rows' instantiations
        let rows2: Vec<(String, rut_core::types::RutType)> = ctx
            .types
            .types
            .iter()
            .enumerate()
            .skip(seed_start)
            .map(|(_, t)| (ctx.interner.name(t.name).to_string(), t.clone()))
            .collect();
        for (i, row) in rows2.iter().enumerate() {
            let text = &row.0;
            let Some((base_text, rest)) = text.split_once('<') else {
                continue;
            };
            let Some(args_text) = rest.strip_suffix('>') else {
                continue;
            };
            let base_id = ctx.intern(&base_text);
            let param_len = match ctx.extern_generics.get(&base_id) {
                Some(g) => g.params.len(),
                // a local generic class: the arity reads off its decl
                None => match ctx.find_data(base_id) {
                    Some(d) if !d.generics.is_empty() => d.generics.len(),
                    _ => continue,
                },
            };
            let arg_texts = split_top_commas(args_text);
            if arg_texts.len() != param_len {
                continue;
            }
            let mut args: Vec<TypeId> = Vec::new();
            let mut ok = true;
            for a in &arg_texts {
                match by_name.get(a) {
                    Some(t) => args.push(*t),
                    None => {
                        let aid = ctx.intern(a);
                        match ctx.types.dense_id_of_name(aid) {
                            Some(t) => args.push(t),
                            None => {
                                ok = false;
                                break;
                            }
                        }
                    }
                }
            }
            if !ok {
                continue;
            }
            let row_id = ctx.types.id_for_pub((seed_start + i) as u32);
            ctx.inst_data.entry(row_id).or_insert((base_id, args.clone()));
            // the drain's mk_data_inst must dedup onto THIS row: seed the
            // type_inst cache with the carried id (a local base's carried
            // instantiation otherwise mints a second row and the admit
            // checks the members against the wrong id)
            ctx.type_inst.entry((base_id, args)).or_insert(row_id);
        }
    }

    for (name, args) in std::mem::take(&mut ctx.seeded_fns) {
        let Some(fname) = ctx.lookup_name(&name) else {
            ctx.err(
                rut_lexer::span::Span::new(0, 0),
                format!("seeded instantiation of fn `{name}` — no such fn here"),
            );
            continue;
        };
        let Some(node) = ctx.fn_nodes.iter().find(|(n, _)| *n == fname).map(|(_, n)| *n) else {
            continue;
        };
        let generics = ctx.ast.fn_decl(node).generics.clone();
        if generics.len() != args.len() {
            continue;
        }
        let subst: Vec<(rut_core::IdentId, rut_core::types::TypeId)> =
            generics.iter().cloned().zip(args.iter().cloned()).collect();
        seeded_bodies.push(Inst { key: FnKey::Free(fname), subst, iface_origins: vec![] });
    }
    // the dead-code-checking roots: every PRIVATE non-generic non-async
    // free fn and every private concrete-class/enum method compiles too
    // — the fused typecheck+codegen walk must see real source whether or
    // not anything calls it (a bogus body in dead code is a real
    // diagnostic, and rut-lsp rides this lane). They run HERE, after the
    // surface snapshot + seed attach + the seed scans: their bodies mint
    // type rows, and a row minted before the snapshot would shift every
    // later local and stale the ids a consumer already bound — the
    // committed-envelope law (a bundle's surface must keep byte-matching
    // this compile). Appended tail ids cross no surface; `ensure_inst`
    // dedups, so a private fn an earlier root's call site already
    // reached compiles exactly once.
    //
    // A private FREE fn with trait-object parameters stays call-site-
    // shaped: the call mints its Inst with one concrete origin per box
    // param (the specialization law — the origins ride the KEY), so an
    // origin-less eager root would be a SECOND instantiation whose boxed
    // lanes refuse (the origin law). Its body checks at its call sites,
    // where the origins are known. Methods need no such gate — their
    // Inst keys never carry origins — and generic/async bodies stay
    // call-site shapes everywhere (no substitution / no weave).
    {
        let mut late_roots: Vec<Inst> = Vec::new();
        for (name, node) in ctx.fn_nodes.clone() {
            let is_pub = ctx.exports.iter().any(|(n, _)| *n == name);
            if is_pub {
                continue;
            }
            let fd = ctx.ast.fn_decl(node);
            if !fd.generics.is_empty() || fd.is_async {
                continue;
            }
            // the signature resolves in the fn's module (phase 3)
            let saved_mod = ctx.cur_mod.clone();
            ctx.cur_mod = ctx.mod_of(node.id()).to_string();
            let boxed = fd.params.iter().any(|p| match ctx.ast.param(*p) {
                rut_ast::ast::MemberKind::Param(rut_ast::ast::ParamData { ty: Some(t), .. }) => {
                    let ty = ctx.resolve_sig_ty(*t, &[], None);
                    matches!(ctx.types.kind(ty), TyKind::IfaceObj { .. })
                }
                _ => false,
            });
            ctx.cur_mod = saved_mod;
            if boxed {
                continue;
            }
            late_roots.push(Inst { key: FnKey::Free(name), subst: vec![], iface_origins: vec![] });
        }
        for (dname, d) in ctx.datas.clone() {
            if !d.generics.is_empty() {
                continue;
            }
            for (mname, mnode) in &d.methods {
                let md = ctx.ast.method_decl(*mnode);
                let exported = md.vis == Some(rut_ast::ast::Vis::Pub);
                if exported || md.is_async || !md.generics.is_empty() {
                    continue;
                }
                late_roots.push(Inst {
                    key: FnKey::Method { data: dname, name: *mname },
                    subst: vec![],
                    iface_origins: vec![],
                });
            }
        }
        for (ename, e) in ctx.enums.clone() {
            for (mname, mnode) in &e.methods {
                let md = ctx.ast.method_decl(*mnode);
                if md.vis == Some(rut_ast::ast::Vis::Pub) || md.is_async || !md.generics.is_empty()
                {
                    continue;
                }
                late_roots.push(Inst {
                    key: FnKey::Method { data: ename, name: *mname },
                    subst: vec![],
                    iface_origins: vec![],
                });
            }
        }
        for root in late_roots {
            if ctx.compile_queue(root).is_err() {
                diags.append(&mut ctx.diags);
                return fail(diags, ast_dump, ast_json);
            }
        }
    }
    for inst in seeded_bodies {
        ctx.ensure_inst(inst);
    }
    diags.append(&mut ctx.diags);
    if ctx.drain_queue().is_err() || !ctx.diags.is_empty() {
        diags.append(&mut ctx.diags);
        return fail(diags, ast_dump, ast_json);
    }
    // the trait-method slot table enumerates the FINAL trait table —
    // after the second drain: the seeded bodies mint trait
    // instantiations (a `Readable<T>` dispatch inside a seeded `get`),
    // and a table snapshotted before them truncates their slots.
    // vtable pass 2: the settled rows — every fill body compiled by the
    // drain above now answers in `inst_map`, so the shipped rows carry
    // the seeded instantiations' dispatch entries too. The link re-lays
    // both tables through the merged trait table.
    let vtables = match ctx.build_vtables() {
        Ok(v) if ctx.diags.is_empty() => v,
        Err(()) | Ok(_) => {
            diags.append(&mut ctx.diags);
            return fail(diags, ast_dump, ast_json);
        }
    };
    let mut iface_slots = Vec::new();
    for (i, t) in ctx.ifaces.iter().enumerate() {
        for m in 0..t.methods.len() {
            iface_slots.push((i as u32, m as u32));
        }
    }
    // ---- the template pass (v22): generic bodies ride the binary ----
    //
    // Every exported generic decl compiles ONCE here, against its
    // `#<param>` placeholder rows, and the body appends after the
    // module's functions. A consumer-spelled shape the pack-time closure
    // never saw (`decodeJson<Todo>`) monomorphizes at LINK from these
    // rows — the delivery law: a generic surface serves requests from
    // the pack-time ledger OR its templates, never from source. Nested
    // generic calls inside a template cascade to the callee's own
    // template fid (lazily emitted by the same drain, depth-first over
    // the template graph; a cycle is the compile error it always was),
    // and a bound-member call the placeholder row cannot resolve mints
    // a STUB the wire records. The window's requests are consumed here —
    // the materialization re-derives them from the template bodies.
    let mut templates: Vec<rut_core::binary::TemplateRow> = Vec::new();
    let mut template_stubs: Vec<rut_core::binary::TemplateStub> = Vec::new();
    let mut template_failures: Vec<(String, String)> = Vec::new();
    let mut template_first_fid = ctx.funcs.len() as u32;
    let mut template_end_fid = ctx.funcs.len() as u32;
    if !surface.fn_generics.is_empty()
        || surface
            .inherents
            .iter()
            .any(|ih| ih.methods.iter().any(|m| m.local == 0))
    {
        let pre_requests = ctx.inst_requests.len();
        let pre_fns = ctx.ledger_fns.len();
        let pre_tys = ctx.ledger_types.len();
        ctx.template_first_fid = template_first_fid;
        // the decl list, in surface order: the exported generic fns
        // (the surface's fn_generics order), then the exported generic
        // methods (a generic class's members, or a generic method on a
        // plain class — the inherents order)
        enum Decl {
            Free { name: IdentId, generics: Vec<IdentId>, bounds: Vec<IdentId> },
            Method { data: IdentId, name: IdentId, generics: Vec<IdentId>, bounds: Vec<IdentId> },
        }
        let mut decls: Vec<Decl> = Vec::new();
        for g in &surface.fn_generics {
            let node = ctx
                .fn_nodes
                .iter()
                .find(|(n, _)| *n == g.name)
                .map(|(_, n)| *n);
            let bounds = node
                .map(|n| ctx.ast.fn_decl(n).bounds.iter().map(|(g, _)| *g).collect())
                .unwrap_or_default();
            decls.push(Decl::Free { name: g.name, generics: g.params.clone(), bounds });
        }
        for (dname, d) in ctx.datas.clone() {
            for (mname, mnode) in &d.methods {
                let md = ctx.ast.method_decl(*mnode);
                if md.vis != Some(rut_ast::ast::Vis::Pub) || md.is_async {
                    continue;
                }
                if d.generics.is_empty() && md.generics.is_empty() {
                    continue;
                }
                let mut generics = d.generics.clone();
                generics.extend(md.generics.iter().copied());
                let mut bounds: Vec<IdentId> =
                    d.requires.iter().map(|(g, _)| *g).collect();
                bounds.extend(md.bounds.iter().map(|(g, _)| *g));
                decls.push(Decl::Method { data: dname, name: *mname, generics, bounds });
            }
        }
        // mint the placeholder substitutions and queue every template
        // root — ensure_inst appends in decl order, so the exported
        // rows' fids are in surface order
        let mut roots = Vec::new();
        for decl in &decls {
            let (name, data, generics) = match decl {
                Decl::Free { name, generics, .. } => (*name, None, generics),
                Decl::Method { data, name, generics, .. } => (*name, Some(*data), generics),
            };
            let subst: Vec<(IdentId, TypeId)> = generics
                .iter()
                .map(|&p| (p, ctx.param_placeholder(p)))
                .collect();
            let key = match data {
                Some(d) => FnKey::Method { data: d, name },
                None => FnKey::Free(name),
            };
            let inst = Inst { key, subst, iface_origins: vec![] };
            let fid = ctx.ensure_inst(inst.clone());
            roots.push((inst, fid));
        }
        let failed = ctx.drain_queue().is_err();
        // the STABILITY law (v22): a template body's repr-carrying ops
        // must be grounded — any op whose type operand mentions a
        // `#<param>` placeholder row bakes the placeholder's own Ref
        // repr; a different byte means emission depended on the
        // concrete T beyond the TypeId operands. The offending window
        // drops (the pack's templates-must-emit gate refuses when
        // bodies were required — the pinned recipe).
        let stable = check_template_stability(&ctx, template_first_fid).is_none();
        if !failed && stable && ctx.diags.is_empty() {
            // the rows: the roots' fids, in decl order; the window's
            // stubs ride the section table
            for (inst, fid) in roots {
                let (name, data) = match inst.key {
                    FnKey::Free(n) => (n, None),
                    FnKey::Method { data, name } => (name, Some(data)),
                    _ => continue,
                };
                let kind = match data {
                    Some(d) => rut_core::binary::TemplateKind::Method { data: d, name },
                    None => rut_core::binary::TemplateKind::Free { name },
                };
                let param_rows: Vec<TypeId> = inst.subst.iter().map(|(_, t)| *t).collect();
                let bounds = match &inst.key {
                    FnKey::Free(n) => decls
                        .iter()
                        .find_map(|d| match d {
                            Decl::Free { name: dn, bounds, .. } if *dn == *n => Some(bounds.clone()),
                            _ => None,
                        })
                        .unwrap_or_default(),
                    FnKey::Method { data: d, name: n } => decls
                        .iter()
                        .find_map(|decl| match decl {
                            Decl::Method { data: dd, name: dn, bounds, .. } if *dd == *d && *dn == *n => Some(bounds.clone()),
                            _ => None,
                        })
                        .unwrap_or_default(),
                    _ => Vec::new(),
                };
                templates.push(rut_core::binary::TemplateRow { kind, param_rows, bounds, fid });
            }
            // the window's end: every body + stub the drain appended —
            // link drops the whole range from the merged table (the
            // templates serve the PRE-link owner mount only)
            template_end_fid = ctx.funcs.len() as u32;
            template_stubs = ctx
                .member_stubs
                .iter()
                .map(|(fid, param, member)| rut_core::binary::TemplateStub {
                    fid: *fid,
                    param: *param,
                    member: *member,
                })
                .collect();
        } else {
            // the window could not lower as placeholder-stable bodies:
            // drop it WHOLE (funcs back to the window start, the
            // window's ledger rows with it) — an ordinary directory
            // compile is untouched, and the pack's gate refuses a
            // generic-owning binary with no template bodies
            // the FIRST failure's decl + construct — the pack's pinned
            // refusal spells it; an ordinary compile hears nothing
            let decl_text = roots
                .first()
                .map(|(inst, _)| match inst.key {
                    FnKey::Free(n) => ctx.name(n).to_string(),
                    FnKey::Method { data, name } => {
                        format!("{}::{}", ctx.name(data), ctx.name(name))
                    }
                    _ => String::new(),
                })
                .unwrap_or_default();
            let construct = check_template_stability(&ctx, template_first_fid)
                .map(|(_, c)| c)
                .or_else(|| ctx.diags.first().map(|d| d.msg.clone()))
                .unwrap_or_else(|| "the body failed to compile against its placeholder rows".into());
            template_failures.push((decl_text, construct));
            ctx.funcs.truncate(template_first_fid as usize);
            template_end_fid = ctx.funcs.len() as u32;
            ctx.ledger_fns.truncate(pre_fns);
            ctx.ledger_types.truncate(pre_tys);
            templates.clear();
            template_stubs.clear();
            template_first_fid = ctx.funcs.len() as u32;
        }
        ctx.inst_requests.truncate(pre_requests);
        ctx.template_first_fid = u32::MAX;
        ctx.template_params.clear();
        ctx.member_stubs.clear();
    }
    let mut funcs = std::mem::take(&mut ctx.funcs);
    // resolve pc → (line, col) beside the byte-offset span
    // table, HERE while this module's source is in hand (the compiler's
    // lowering carries byte offsets only). 1-based both; parallel to
    // `spans` entry-for-entry, consumed lazily by the StackTrace members.
    let line_starts: Vec<u32> = {
        let mut ls = vec![0u32];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                ls.push(i as u32 + 1);
            }
        }
        ls
    };
    let line_col = |off: u32| -> (u32, u32) {
        let line = line_starts.partition_point(|&s| s <= off);
        if line == 0 {
            return (0, 0);
        }
        let lo = line_starts[line - 1];
        (line as u32, off - lo + 1)
    };
    for f in funcs.iter_mut() {
        f.pos = f.spans.iter().map(|(_, lo)| line_col(*lo)).collect();
    }
    let ir_dump = ir_dump_of(&funcs, &ctx.interner);
    // the program's interner moves out of the Ctx; the surface carries a
    // clone so it stays self-contained when it crosses to a using module
    // (the ids it carries stay valid: interning is append-only, so the
    // program's later tail never renumbers a surface id)
    let interner = std::mem::take(&mut ctx.interner);
    surface.names = interner.clone();
    // the instantiation ledger, canonical (sorted) order — the wire and
    // the pack must be byte-deterministic for the same source
    let name_of = |i: rut_core::IdentId| interner.name(i).to_string();
    let mut ledger_types = std::mem::take(&mut ctx.ledger_types);
    ledger_types.sort_by(|a, b| {
        let ka = (name_of(a.owner), name_of(a.decl), a.args.clone());
        let kb = (name_of(b.owner), name_of(b.decl), b.args.clone());
        ka.cmp(&kb)
    });
    let mut ledger_fns = std::mem::take(&mut ctx.ledger_fns);
    let kind_key = |f: &rut_core::binary::InstFn| -> (String, String, String, bool) {
        match &f.kind {
            rut_core::binary::InstFnKind::Free { name, .. } => {
                ("f".into(), String::new(), name_of(*name), false)
            }
            rut_core::binary::InstFnKind::Method { data, name, .. } => {
                ("m".into(), name_of(*data), name_of(*name), false)
            }
            rut_core::binary::InstFnKind::HostThunk { name } => {
                ("h".into(), String::new(), name_of(*name), false)
            }
        }
    };
    ledger_fns.sort_by(|a, b| {
        let ka = (name_of(a.owner), kind_key(a), a.fid);
        let kb = (name_of(b.owner), kind_key(b), b.fid);
        ka.cmp(&kb)
    });
    let requests = std::mem::take(&mut ctx.inst_requests);
    let program = Program {
        name: module_name.to_string(),
        scope,
        interner,
        surface,
        types: ctx.types,
        ifaces: ctx.ifaces,
        iface_slots,
        vtables,
        disposal_impls,
        consts: ctx.consts,
        funcs,
        exports,
        inst_types: ledger_types,
        inst_fns: ledger_fns,
        templates,
        template_stubs,
        template_first_fid,
        template_end_fid,
    };
    ProgramOutput { diags, ast_dump, ast_json, ir_dump, program: Some(program), requests, template_failures }
}

/// The template stability law (v22): a template body's emission must
/// have followed the MECHANICAL rule — every repr-carrying op bakes the
/// repr its (template-side) type operands resolve to. A different byte
/// means emission depended on the concrete T beyond the TypeId
/// operands, and the materialization's rewrite could not repair it.
/// Returns `(fn name, construct)` on the first violation.
fn check_template_stability(ctx: &Ctx, first_fid: u32) -> Option<(String, String)> {
    for fc in ctx.funcs.iter().skip(first_fid as usize) {
        let reg_ty = |r: u16| -> TypeId { fc.regs.get(r as usize).copied().unwrap_or(rut_core::types::TY_NIL) };
        let field_ty = |t: TypeId, field: u32| -> Option<TypeId> {
            match ctx.types.kind(t) {
                TyKind::Data { fields } => fields.get(field as usize).map(|f| f.ty),
                _ => None,
            }
        };
        let arr_elem = |t: TypeId| -> Option<TypeId> {
            match ctx.types.kind(t) {
                TyKind::Array { elem } => Some(*elem),
                _ => None,
            }
        };
        let refuse = |fc: &rut_core::binary::FuncCode, what: &str| -> Option<(String, String)> {
            Some((ctx.name(fc.name).to_string(), what.to_string()))
        };
        for op in &fc.code {
            match op {
                Op::GetF { obj, field, repr, .. } | Op::SetF { obj, field, repr, .. } => {
                    let want = field_ty(reg_ty(*obj), *field).map(|f| ctx.types.repr_of(f));
                    if let Some(w) = want {
                        if w != *repr {
                            return refuse(fc, "a field access whose baked repr is not the field type's");
                        }
                    }
                }
                Op::ArrNew { ty, repr, .. } => {
                    if let Some(e) = arr_elem(*ty) {
                        if rut_core::types::arr_elem_repr(&ctx.types, e) != *repr {
                            return refuse(fc, "an array op whose baked repr is not the element type's");
                        }
                    }
                }
                Op::ArrGet { arr, repr, .. } | Op::ArrSet { arr, repr, .. } => {
                    if let Some(e) = arr_elem(reg_ty(*arr)) {
                        if rut_core::types::arr_elem_repr(&ctx.types, e) != *repr {
                            return refuse(fc, "an array element op whose baked repr is not the element type's");
                        }
                    }
                }
                Op::ArrGetF { obj, field, repr, .. } | Op::ArrSetF { obj, field, repr, .. } => {
                    let want = field_ty(reg_ty(*obj), *field)
                        .and_then(arr_elem)
                        .map(|e| rut_core::types::arr_elem_repr(&ctx.types, e));
                    if let Some(w) = want {
                        if w != *repr {
                            return refuse(fc, "a field-array op whose baked repr is not the element type's");
                        }
                    }
                }
                _ => {}
            }
        }
    }
    None
}
