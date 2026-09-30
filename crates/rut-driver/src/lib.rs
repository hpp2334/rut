//! Pipeline driver: Ast ─► resolve ─► typecheck (fused with
//! body compilation, M1) ─► LIR ─► binary emit. One entry point
//! for the CLI and the wasm demo: `compile_module`.

use rut_lir::check::{Ctx, FnKey, Inst};
use rut_lexer::diag::Diag;
use rut_ast::dump;
use rut_parser::{parse, Mode};
use rut_core::binary::{encode, Program};
use rut_core::ops::{NOREG, Op};
use rut_core::types::{TyKind, TypeId, TY_F32, TY_F64, TY_I32, TY_OPAQUE, TY_STR, TY_NIL};
use rut_core::{IdentId, sym};

pub mod session;
pub use session::{Module, ModuleBody, PeerDecl, ResolveError, Session};

pub mod decl;
pub use decl::lower_decl_module;

pub mod graph;
pub use graph::{compile_graph, compile_units, GraphOutput, Units};

pub mod pack;
pub use pack::pack_dir;

pub mod loader;
pub use loader::{
    assemble_peers, compile_dir, load_bundle_bytes, load_bundle_session, load_dir_session,
    load_module_source, load_path_session, mount_dir,
};

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

pub struct CompileOutput {
    pub diags: Vec<Diag>,
    pub ast_dump: String,
    /// the demo tree UI's structured AST (flattened tagged JSON)
    pub ast_json: String,
    pub ir_dump: String,
    pub binary: Option<Vec<u8>>,
}

/// The owner side of consumer requests, seeded into a recompiled unit.
/// One group per requester: the requester's reachable type descriptors
/// (registered as a sparse block under the requester's scope — the
/// locals match the requester's own layout, which is what makes the
/// instantiation keys agree), and the instantiations to materialize.
#[derive(Clone, Default)]
pub struct Seeds<'a> {
    pub groups: &'a [SeedGroup<'a>],
}

/// One requester's seed block: the descriptor closure as
/// `(scope, local) → row` pairs + the requester program's interner
/// (the descriptors' name ids resolve against it, then re-intern into
/// the owner's). The attach side merges EVERY group's rows into one
/// run per scope — two requesters' rows share a scope (the classes of
/// a common dependency), and a second base registration would strand
/// the first group's ids.
#[derive(Clone)]
pub struct SeedGroup<'a> {
    pub rows: Vec<((rut_core::ScopeId, u32), rut_core::types::RutType)>,
    pub names: &'a rut_core::Interner,
    /// `(decl name text, args in the registered blocks' id space, the
    /// method names whose bodies this request needs — empty for a
    /// plain type instantiation)`
    pub insts: Vec<(String, Vec<rut_core::types::TypeId>, Vec<String>)>,
    /// `(fn name text, type arguments)` — the generic fn bodies whose
    /// mirrors this requester calls
    pub fns: Vec<(String, Vec<rut_core::types::TypeId>)>,
    /// `(trait text, target row in the requester's space, method text)`
    /// — the generic-target impl methods whose mirrors this requester
    /// calls; the owner mints the template impl at the target and
    /// compiles the body
    pub impl_methods: Vec<(String, rut_core::types::TypeId, String)>,
    /// the requester's own impl registrations, scope-qualified into
    /// the requester's scope: a generic body the owner compiles for
    /// this requester dispatches through THESE impls (a consumer's
    /// `impl JsonSerialize for Json` lives in the consumer — the
    /// owner's monomorphized `encodeJson<Json>` binds it as a foreign
    /// registration; link rebases the fn ids onto the requester's
    /// block)
    pub impls: Vec<SeedImpl>,
}

/// One seed impl row (see [`SeedGroup::impls`]).
#[derive(Clone)]
pub struct SeedImpl {
    pub trait_name: String,
    /// the target type id, packed with the requester's scope
    pub target: rut_core::types::TypeId,
    /// (method name text, requester-scope fn id) — slot ABI
    pub methods: Vec<(String, u32)>,
    /// (method name text, requester-scope fn id) — concrete ABI
    pub methods_concrete: Vec<(String, u32)>,
}

impl<'a> Seeds<'a> {
    pub fn none() -> Seeds<'a> {
        Seeds { groups: &[] }
    }
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
    compile_program_resolved(src, mode, module_name, scope, uses, !uses.is_empty(), &Seeds::none())
}

/// As [`compile_program`] but with an explicit `allow_uses` flag — the
/// graph compiler resolves every specifier itself — and with `seeds`
/// carrying the owner side of consumer instantiation requests: the
/// requesters' type descriptors (one sparse block per requester scope)
/// and the instantiations to materialize before the compile roots.
pub fn compile_program_resolved(
    src: &str,
    mode: Mode,
    module_name: &str,
    scope: rut_core::ScopeId,
    uses: &[(rut_core::ScopeId, rut_core::binary::Surface, String)],
    allow_uses: bool,
    seeds: &Seeds<'_>,
) -> ProgramOutput {
    let fail = |diags: Vec<Diag>, ast_dump: String, ast_json: String| ProgramOutput {
        diags,
        ast_dump,
        ast_json,
        ir_dump: String::new(),
        program: None,
        requests: Vec::new(),
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
        for t in &surface.traits {
            ast.interner.intern(surface.names.name(t.name));
        }
    }
    let mut ctx = Ctx::new_scoped(&ast, scope);
    ctx.allow_uses = allow_uses;
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
        // ---- pass 1: trait declarations from every surface. A descriptor registers when its name was used, or when
        // any bound impl names it (an unused trait's impl must stay
        // visible to the use-gate diagnostic).
        let impl_trait_texts: std::collections::HashSet<&str> = surface
            .impls
            .iter()
            .map(|im| surface.names.name(im.trait_name))
            .collect();
        let mut tmap: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for t in &surface.traits {
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
            let cid = ctx.add_extern_trait_decl(reg_name, t, &surface.names);
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
    for (dep_scope, surface, _, origin) in trait_maps.iter() {
        let dep_scope = *dep_scope;
        for f in &surface.funcs {
            if let Some(id) = ctx.ast.interner.lookup(surface.names.name(f.name)) {
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
        // keys the Future trait_inst per-unit, so the ret re-spells to
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
                ctx.add_extern_type(id, ty_id, t.is_class);
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
                    );
                }
            }
        }
        // impl registrations: `(trait, target, method → fn)`,
        // the fns scope-qualified with the exporter's scope. A trait the
        // module never used still binds its impls — dispatch stays gated
        // on the trait's name (`extern_trait_decls`), the diagnostic
        // ("use `I` ..") reads the registration. Both ABI lists bind;
        // a missing concrete list falls back at the call site.
        for im in &surface.impls {
            let text = surface.names.name(im.trait_name);
            // an impl of a native trait (`Iterator`) stays in its
            // declaring module — the consumer instantiates its own
            // trait per element type (v1)
            let Some(&tid) = ext_trait.get(text) else {
                continue;
            };
            let tname = ctx.intern(text);
            let methods = im
                .methods
                .iter()
                .map(|(n, f)| (ctx.intern(surface.names.name(*n)), rut_core::pack(dep_scope, *f)))
                .collect();
            let methods_concrete = im
                .methods_concrete
                .iter()
                .map(|(n, f)| (ctx.intern(surface.names.name(*n)), rut_core::pack(dep_scope, *f)))
                .collect();
            ctx.add_extern_impl(tname, tid, im.target, methods, methods_concrete, im.trait_args.clone());
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
                let methods: Vec<(
                    rut_core::IdentId,
                    Vec<TypeId>,
                    TypeId,
                    u32,
                    bool,
                    Vec<rut_core::IdentId>,
                )> = ih
                    .methods
                    .iter()
                    .map(|m| {
                        (
                            ctx.intern(surface.names.name(m.name)),
                            m.params
                                .iter()
                                .map(|p| {
                                    respell_surface_ty(
                                        &ctx.types, *p, own_scope, dep_scope, cx_name, cx_ty,
                                    )
                                })
                                .collect(),
                            respell_surface_ty(
                                &ctx.types, m.ret, own_scope, dep_scope, cx_name, cx_ty,
                            ),
                            rut_core::pack(dep_scope, m.local),
                            m.has_self,
                            m.generics
                                .iter()
                                .map(|&g| ctx.intern(surface.names.name(g)))
                                .collect(),
                        )
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
        // core's native surface: builtin containers, traits, and
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
        for (n, native, ambient) in &surface.native_traits {
            if let Some(id) = ctx.ast.interner.lookup(surface.names.name(*n)) {
                if *ambient || ctx.used.contains(&id) {
                    ctx.add_extern_trait(id, *native);
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
    // compilation roots: the conventional `main` and every
    // `entry fn` — the host-callable surface. A module may
    // have either, both, or neither (pure library shape).
    let mut roots: Vec<Inst> = Vec::new();
    if ctx.find_free_fn(sym::MAIN) {
        roots.push(Inst { key: FnKey::Free(sym::MAIN), subst: vec![], trait_origins: vec![] });
    }
    for name in ctx.entries.clone() {
        if ctx.find_free_fn(name) {
            // an async fn is not host-callable (its woven body takes the
            // (frame, cx) pair): it compiles at its call sites
            if ctx.ast.fn_decl(ctx.fn_nodes.iter().find(|(n, _)| *n == name).map(|(_, n)| *n).unwrap())
                .is_async
            {
                continue;
            }
            roots.push(Inst { key: FnKey::Free(name), subst: vec![], trait_origins: vec![] });
        }
    }
    // library surface: every non-generic `pub fn` is usable, so its body
    // must be compiled even when nothing local calls it
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
        roots.push(Inst { key: FnKey::Free(name), subst: vec![], trait_origins: vec![] });
    }
    // library surface, class half (the linkable-classes phase): every
    // non-generic class's exported methods are callable from a
    // consumer, so their bodies compile even when nothing local calls
    // them — the inherent surface rows carry the fn ids. Generic
    // classes monomorphize per instantiation (the owner-side request
    // machinery seeds their bodies); async and generic METHODS stay
    // call-site shapes and cross no surface.
    for (dname, d) in ctx.datas.clone() {
        if !d.generics.is_empty() {
            continue;
        }
        for (mname, mnode) in &d.methods {
            let md = ctx.ast.method_decl(*mnode);
            let exported =
                d.kind == rut_lir::check::DataKind::Dataclass || md.vis == Some(rut_ast::ast::Vis::Pub);
            if !exported || md.is_async || !md.generics.is_empty() {
                continue;
            }
            roots.push(Inst {
                key: FnKey::Method { data: dname, name: *mname },
                subst: vec![],
                trait_origins: vec![],
            });
        }
    }
    for root in roots {
        if ctx.compile_queue(root).is_err() {
            diags.append(&mut ctx.diags);
            return fail(diags, ast_dump, ast_json);
        }
    }
    if !ctx.diags.is_empty() {
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
    // the tail, once the settle makes every fill resolve
    let _ = ctx.build_vtables();
    // per-type disposal rows: `(target type, dispose func id)` per
    // `impl ..: Disposal` — the release path's table, flowing module→link
    // beside the vtables
    let disposal_impls = ctx.disposal_impls();
    if !ctx.diags.is_empty() {
        diags.append(&mut ctx.diags);
        return fail(diags, ast_dump, ast_json);
    }
    // finalize the entry table: `entry fn`s — plus the
    // conventional `main` when it is exported.
    let mut exports: Vec<(IdentId, u32)> = Vec::new();
    let mut names: Vec<IdentId> = ctx.entries.clone();
    if ctx.exports.iter().any(|(n, _)| *n == sym::MAIN) && !names.contains(&sym::MAIN) {
        names.push(sym::MAIN);
    }
    for n in names {
        if let Some(&f) = ctx.inst_map.get(&Inst { key: FnKey::Free(n), subst: vec![], trait_origins: vec![] }) {
            exports.push((n, f));
        }
    }
    // exported surface: every `pub` fn + its signature, for using modules
    let mut surface = rut_core::binary::Surface::default();
    for (name, _) in &ctx.exports {
        if let Some(&fid) = ctx.inst_map.get(&Inst { key: FnKey::Free(*name), subst: vec![], trait_origins: vec![] }) {
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
    // the bodies. Member visibility is the class law (dataclasses
    // are all-public); async and generic METHODS cross no surface
    // (call-site shapes).
    for (dname, d) in ctx.datas.clone() {
        if d.methods.is_empty() {
            continue;
        }
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
            let exported =
                d.kind == rut_lir::check::DataKind::Dataclass || md.vis == Some(rut_ast::ast::Vis::Pub);
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
                        trait_origins: vec![],
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
    }

    // type surface: the whole non-boot block (so `(scope, local)` ids and
    // field layouts resolve in a using module) + the exported names.
    // The inherent surface above ran FIRST: its template signature rows
    // (`#<param>`, `#Self`, their structural spellings) are interned
    // before this snapshot carries them.
    {
        let boot_len = ctx.types.boot_len as usize;
        surface.types = ctx.types.types[boot_len..].to_vec();
        // the type block's trait-object references: each carried
        // `[trait] ..` row names a trait-table index the consumer's
        // `use_types` remaps through the trait map — generic traits'
        // per-argument-list INST DESCRIPTORS ride here (their base
        // declarations export above; neither row carries signatures —
        // dispatch reads the impl rows)
        let mut inst_traits: Vec<u32> = Vec::new();
        for d in &surface.types {
            if let rut_core::types::TyKind::TraitObj { trait_id } = d.kind {
                if trait_id == u32::MAX || inst_traits.contains(&trait_id) {
                    continue;
                }
                // only GENERIC traits' per-argument-list descriptors — a
                // row whose spelled name has no `<` is a base
                // declaration, exported (with its methods) by its
                // declaring unit alone; re-exporting it bare would
                // collide with that row at link
                let Some(row) = ctx.traits.get(trait_id as usize) else { continue };
                if !ctx.interner.name(row.name).contains('<') {
                    continue;
                }
                inst_traits.push(trait_id);
            }
        }
        for tid in inst_traits {
            let Some(row) = ctx.traits.get(tid as usize) else { continue };
            if surface.traits.iter().any(|t| t.local == tid) {
                continue;
            }
            surface.traits.push(rut_core::binary::SurfaceTrait {
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
                params: Vec::new(),
                scope: (scope == rut_core::BOOT_SCOPE).then_some(scope),
            });
        }
        // trait surface: declared traits with their
        // resolved signatures, keyed by the exporter's trait-table
        // index (`local`) — the key the carried `[trait] ..` descriptors
        // reference. Generic traits instantiate per argument list where
        // they are declared (their methods are per-inst, so the base
        // row carries none) but the DECLARATION itself crosses: a
        // consumer binds the trait's parameterized impls (`impl
        // Readable<T> for Source<T>`) through it.
        for (name, info) in &ctx.trait_decls {
            if info.id == u32::MAX {
                surface.traits.push(rut_core::binary::SurfaceTrait {
                    local: u32::MAX,
                    name: *name,
                    generics: info.generics.len(),
                    methods: Vec::new(),
                });
                continue;
            }
            if info.id as usize >= ctx.traits.len() {
                continue;
            }
            surface.traits.push(rut_core::binary::SurfaceTrait {
                local: info.id,
                name: *name,
                generics: info.generics.len(),
                methods: ctx.traits[info.id as usize].methods.clone(),
            });
        }
        // impl registrations: `(trait, target, method → fn
        // ref)` — link merges them and errors on a duplicate pair.
        // Concrete-target impls bind their compiled fns (both ABI
        // variants — identical ids for single-ABI impls, see
        // `SurfaceImpl`). GENERIC-target impls (`impl I for Vec<T>`)
        // ride the same row shape with the TEMPLATE target and zero fn
        // ids: one body exists per instantiation, none at surface build
        // time — the consumer mirrors the request (see `ExternImpl`).
        let impls_snapshot = ctx.impls.clone();
        for (idx, im) in impls_snapshot.iter().enumerate() {
            if im.inherent {
                continue;
            }
            let template = im.target_data.is_some();
            let mut methods = Vec::new();
            let mut methods_concrete = Vec::new();
            for (mname, _) in &im.methods {
                if template {
                    // request-bound: the names dispatch, the ids don't
                    methods.push((*mname, 0));
                    methods_concrete.push((*mname, 0));
                    continue;
                }
                let slot_key = Inst {
                    key: ctx.impl_method_key(idx, *mname, true),
                    subst: vec![],
                    trait_origins: vec![],
                };
                let concrete_key = Inst {
                    key: ctx.impl_method_key(idx, *mname, false),
                    subst: vec![],
                    trait_origins: vec![],
                };
                if let Some(&fid) = ctx.inst_map.get(&slot_key) {
                    methods.push((*mname, fid));
                }
                if let Some(&fid) = ctx.inst_map.get(&concrete_key) {
                    methods_concrete.push((*mname, fid));
                }
            }
            // a parameterized impl head's trait arguments (`impl
            // Readable<T> for Source<T>` → the `#T` placeholder rows),
            // resolved under the target's own placeholder env — the
            // consumer unifies its call's trait inst against these
            let trait_args: Vec<TypeId> = im
                .trait_arg_nodes
                .iter()
                .map(|g| {
                    let env: Vec<(IdentId, TypeId)> = im
                        .target_data
                        .as_ref()
                        .map(|(_, ps)| {
                            ps.iter()
                                .map(|&p| (p, ctx.param_placeholder(p)))
                                .collect()
                        })
                        .unwrap_or_default();
                    ctx.resolve_type(*g, &env)
                })
                .collect();
            surface.impls.push(rut_core::binary::SurfaceImpl {
                trait_name: im.trait_name,
                target: im.target,
                methods,
                methods_concrete,
                trait_args: if template { trait_args } else { vec![] },
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
        // `use_seed_rows` then sets scope_base[s] = attach-start + off,
        // so this table's dense of a requester's packed id `(s, L)`
        // hits the row exactly (the two sides spell every argument the
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
        for group in seeds.groups {
            ctx.seeded_insts.extend(group.insts.iter().cloned());
            ctx.seeded_fns.extend(group.fns.iter().cloned());
            ctx.seeded_impl_methods.extend(group.impl_methods.iter().cloned());
            // the requesters' own impls (registered below, once the trait
            // table exists — it does, post-collect)
            ctx.seed_impls.extend(group.impls.iter().map(|im| {
                (
                    im.trait_name.clone(),
                    im.target,
                    im.methods.clone(),
                    im.methods_concrete.clone(),
                )
            }));
        }
    }
    // the requesters' own impls: a generic body the owner compiles for
    // a requester dispatches through THESE — a consumer's
    // `impl JsonSerialize for Json` lives in the consumer, and the
    // owner's monomorphized `encodeJson<Json>` binds it as a foreign
    // registration whose fn ids are scope-qualified into the
    // requester's block (link rebases them). The trait resolves here
    // (local declaration or bound extern); a target that cannot
    // resolve in this table (a foreign generic's TEMPLATE row this pkg
    // never bound) skips — those impls instantiate in their own pkg.
    for (trait_name, target, methods, methods_concrete) in std::mem::take(&mut ctx.seed_impls) {
        let tname = ctx.intern(&trait_name);
        let Some(tid) = ctx.trait_id_of(tname) else { continue };
        let tsc = rut_core::scope_of(target);
        let resolves = tsc == rut_core::BOOT_SCOPE
            || ctx
                .types
                .scope_base
                .get(tsc as usize)
                .map_or(false, |&b| b >= ctx.types.boot_len);
        if !resolves {
            continue;
        }
        let methods = methods.into_iter().map(|(n, f)| (ctx.intern(&n), f)).collect();
        let methods_concrete = methods_concrete
            .into_iter()
            .map(|(n, f)| (ctx.intern(&n), f))
            .collect();
        ctx.add_extern_impl(tname, tid, target, methods, methods_concrete, vec![]);
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
        ctx.mk_data_inst(dname, args.clone(), rut_lexer::span::Span::new(0, 0));
        if let Some(d) = ctx.find_data(dname).cloned() {
            // a generic CLASS's bodies substitute the class parameters;
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
                    seeded_bodies.push(Inst {
                        key: FnKey::Method { data: dname, name: mname },
                        subst: env.clone(),
                        trait_origins: vec![],
                    });
                }
            } else {
                for m in &methods {
                    let Some(mname) = ctx.lookup_name(m) else { continue };
                    seeded_bodies.push(Inst {
                        key: FnKey::Method { data: dname, name: mname },
                        subst: plain_method_env(&ctx, mname),
                        trait_origins: vec![],
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
    if !ctx.extern_generics.is_empty() {
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
            .map(|(i, t)| (ctx.interner.name(t.name).to_string(), t.clone()))
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
            let Some(g) = ctx.extern_generics.get(&base_id) else {
                continue;
            };
            let arg_texts = split_top_commas(args_text);
            if arg_texts.len() != g.params.len() {
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
            ctx.inst_data.entry(row_id).or_insert((base_id, args));
        }
    }
    // the mirrored GENERIC-TARGET impl methods: the owner mints its
    // template impl at the concrete target (the requesters' rows are
    // registered above, so the target's shape answers here) and queues
    // the method body at the target's substitution
    let impl_seed_count = ctx.seeded_impl_methods.len();
    
    for (trait_text, target, method_text) in std::mem::take(&mut ctx.seeded_impl_methods) {
        let tname = ctx.intern(&trait_text);
        let Some(info) = ctx.find_trait(tname) else { continue };
        let tid = info.id;
        if tid == u32::MAX {
            continue;
        }
        // the concrete target's shape: a structural row (?T / [T]) or a
        // class instantiation already in inst_data (the seeded rows above)
        let shape = match ctx.types.kind(target).clone() {
            TyKind::Opt { elem } => Some((rut_core::sym::OPT, vec![elem])),
            TyKind::Array { elem } => Some((rut_core::sym::ARRAY, vec![elem])),
            _ => ctx.inst_data.get(&target).cloned(),
        };
        let Some((dname, cargs)) = shape else { continue };
        // the mint's target materializes as THIS unit's instantiation
        // row: the fn ledger's target must spell the same row the
        // inst_types ledger carries (the seed-block copy has no ledger
        // row of its own, so link's canonical key would miss it).
        // mk_data_inst routes the owning package's request — already
        // seeded, deduped by the graph's request keys.
        let target = ctx.mk_data_inst(dname, cargs.clone(), rut_lexer::span::Span::new(0, 0));
        
        // the local TEMPLATE impl answering (trait, shape)
        let templates: Vec<(usize, rut_lir::check::ImplDecl)> = ctx
            .impls
            .iter()
            .enumerate()
            .filter(|(_, im)| {
                !im.inherent
                    && !im.is_template
                    && im.trait_id == tid
                    && matches!(&im.target_data, Some((d, ps)) if *d == dname && ps.len() == cargs.len())
            })
            .map(|(i, im)| (i, im.clone()))
            .collect();
        let Some((_tidx, template)) = templates.first().cloned() else { continue };
        let env: Vec<(rut_core::IdentId, rut_core::types::TypeId)> = template
            .target_data
            .as_ref()
            .map(|(_, ps)| ps.iter().cloned().zip(cargs.iter().cloned()).collect())
            .unwrap_or_default();
        // mint the concrete registration (the template's clone at the
        // concrete target — the phase-2 dispatch door's mint)
        ctx.impls.push(rut_lir::check::ImplDecl {
            trait_id: tid,
            trait_name: template.trait_name,
            target,
            target_data: template.target_data.clone(),
            trait_arg_nodes: template.trait_arg_nodes.clone(),
            is_template: true,
            inherent: false,
            methods: template.methods.clone(),
            origin: template.origin.clone(),
        });
        let minted = ctx.impls.len() - 1;
        let Some(mname) = ctx.lookup_name(&method_text) else { continue };
        ctx.ensure_inst(Inst {
            key: FnKey::ImplMethod { idx: minted, name: mname, slot_abi: false },
            subst: env,
            trait_origins: vec![],
        });
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
        seeded_bodies.push(Inst { key: FnKey::Free(fname), subst, trait_origins: vec![] });
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
    let vtables = ctx.build_vtables();
    let mut trait_slots = Vec::new();
    for (i, t) in ctx.traits.iter().enumerate() {
        for m in 0..t.methods.len() {
            trait_slots.push((i as u32, m as u32));
        }
    }
    if std::env::var("RUT_DEBUG_LINK").is_ok() {
        let mut parts: Vec<String> = Vec::new();
        for (s, &b) in ctx.types.scope_base.iter().enumerate() {
            let n = ctx
                .types
                .types
                .get(b as usize)
                .map(|t| ctx.interner.name(t.name).to_string())
                .unwrap_or("-".into());
            parts.push(format!("s{}@{}:{}", s, b, n));
        }
        let own: Vec<String> = ctx.types.types
            [ctx.types.scope_base.get(ctx.types.scope as usize).copied().unwrap_or(ctx.types.boot_len) as usize..]
            .iter()
            .take(8)
            .map(|t| ctx.interner.name(t.name).to_string())
            .collect();
        eprintln!(
            "DBG unit-scopes unit={} scope={} boot={} len={} {} own={:?}",
            module_name,
            ctx.types.scope,
            ctx.types.boot_len,
            ctx.types.types.len(),
            parts.join(" "),
            own
        );
    }
    if std::env::var("RUT_DEBUG_LINK").is_ok() && module_name == "store_probe" {
        for (ti, t) in ctx.types.types.iter().enumerate() {
            let n = ctx.interner.name(t.name).to_string();
            if n == "Vec<str>" || n == "Array<str>" {
                if let rut_core::types::TyKind::Data { fields } = &t.kind {
                    let fs: Vec<String> = fields.iter().map(|f| {
                        let fty = f.ty;
                        format!("{}:{}(s{},l{})", ctx.interner.name(f.name), ctx.interner.name(ctx.types.type_at(fty).name), rut_core::id::scope_of(fty), rut_core::id::local_of(fty))
                    }).collect();
                    eprintln!("DBG probe-desc dense={} {} ty-id=(s{},l{}) fields=[{}]", ti, n, rut_core::id::scope_of(0), 0, fs.join(", "));
                }
            }
        }
        for f in ctx.funcs.iter() {
            if ctx.interner.name(f.name) != "probe_pull" { continue; }
            for (ri, r) in f.regs.iter().enumerate().skip(10).take(14) {
                let n = ctx
                    .types
                    .type_at(*r)
                    .name;
                eprintln!("DBG probe-reg reg{} raw={} name={} len={}", ri, r, ctx.interner.name(n), ctx.types.types.len());
            }
        }
    }
    if std::env::var("RUT_DEBUG_LINK").is_ok() {
        let mut s4rows: Vec<String> = Vec::new();
        for (i, t) in ctx.types.types.iter().enumerate() {
            for f in match &t.kind {
                rut_core::types::TyKind::Data { fields } => fields.iter().map(|f| f.ty).collect::<Vec<_>>(),
                rut_core::types::TyKind::Opt { elem } => vec![*elem],
                _ => vec![],
            } {
                if rut_core::id::scope_of(f) == 4 {
                    s4rows.push(format!("dense{} {} -> (4,{})", i, ctx.interner.name(t.name), rut_core::id::local_of(f)));
                    break;
                }
            }
        }
        eprintln!(
            "DBG surface unit={} types={} table_len={} s4rows={:?}",
            module_name,
            surface.types.len(),
            ctx.types.types.len(),
            s4rows.first()
        );
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
            rut_core::binary::InstFnKind::ImplMethod { trait_name, name, slot_abi, .. } => {
                ("i".into(), name_of(*trait_name), name_of(*name), *slot_abi)
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
        traits: ctx.traits,
        trait_slots,
        vtables,
        disposal_impls,
        consts: ctx.consts,
        funcs,
        exports,
        inst_types: ledger_types,
        inst_fns: ledger_fns,
    };
    ProgramOutput { diags, ast_dump, ast_json, ir_dump, program: Some(program), requests }
}

/// The `calc` body mount note: `calc` is engine-mounted for now (the
/// float `Math` surface — host fns in `rut-std`, consts); the
/// host-pkgs plan moves it to a declared package and its rut-able
/// helpers to source (deferred with the numeric-methods phase).

/// Mount `core` — the prelude surface: the erasure
/// primitive (`opaque`), the builtin trait (`Iterator`), and
/// the compiler-lowered functions (`panic`, the
/// `str`/`bytes` natives). v1.1 removed `Option`/`Result`/`own` — use
/// sites diagnose with the removal. A
/// native module with no body: its surface is
/// [`rut_core::binary::Surface::core`], the single source of truth
/// (`rut/core/core.d.rut` mirrors it for the LSP). Builtin names are
/// ambient except the `pub builtin` spellings (the disposal pair) —
/// those resolve only through `use core::{ .. }`; each mounted row
/// keeps its ambient bit so the binding loops see the split.
/// `core` needs no `[deps]`
/// declaration: the driver mounts it unconditionally (§0.14), while
/// every other package resolves through `[deps]` or host registration.
pub fn mount_std_core(session: &mut Session) {
    // the surface is symbol-id based; the host-facing mount table is
    // string-based — `sym::text` bridges at this boundary only
    let core = rut_core::binary::Surface::core();
    let txt = |id: rut_core::IdentId| -> String {
        rut_core::sym::text(id).unwrap_or_default().to_string()
    };
    let _ = session.register_module(
        "core",
        Module {
            // each row keeps its ambient bit — the binding loops read
            // it off the synthesized surface, so the `pub builtin`
            // spellings stay import-gated end to end
            body: ModuleBody::Host {
                native_types: core.native_types.iter().map(|(n, k, a)| (txt(*n), *k, *a)).collect(),
                native_traits: core.native_traits.iter().map(|(n, k, a)| (txt(*n), *k, *a)).collect(),
                native_fns: core.native_fns.iter().map(|(n, a)| (txt(*n), *a)).collect(),
                consts: core.consts.iter().map(|c| (txt(c.name), c.ty, c.bits)).collect(),
                native_impls: core
                    .native_impls
                    .iter()
                    .map(|(t, n, i)| (*t, txt(*n), *i))
                    .collect(),
                host_funcs: vec![],
            },
            ..Default::default()
        },
    );
}

/// Mount the engine's own packages: `core` (the prelude — always;
/// nothing else is ambient) and `calc` (the `Math` float surface).
/// `pouch` and `ink` are third-party libraries in the toolchain tree
/// (`rut/pouch/`, `rut/ink/`) — a consumer declares them in its
/// manifest `[deps]`, or a native host mounts them with
/// [`mount_dir`]; nothing in the engine knows their names (the
/// host-pkgs plan §2).
pub fn mount_std(session: &mut Session) {
    mount_std_core(session);
    mount_calc(session);
}

/// Mount the standard async set: the engine rows
/// (`async_engine` — a decl module) and the typed launcher surface
/// (`async_host` — an inline rut package). Pair with
/// `rut_std::async_host::pkg()` installed through
/// `HostRegistry::install_host_pkg` before `Vm::new`; a session
/// that mounts neither simply has no launcher, and `await` stays
/// cold-poll inline.
pub fn mount_std_async(session: &mut Session) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../rut")
        .canonicalize()
        .expect("the toolchain tree's rut/ dir");
    mount_dir(session, &root.join("async_engine")).expect("mount async_engine");
    mount_dir(session, &root.join("async_host")).expect("mount async_host");
}

/// Mount `calc` — a native module: `f64` host functions
/// (bodies in `rut-std`, including the float `abs`/`min`/`max`/`signum`)
/// plus their `f32` twins (`Math.sqrt_f(x: f32) -> f32` — the `_f`
/// suffix carries the width; rut has no overloading) and the `f64`
/// constants. The integer intrinsics live in `core` now — `builtin
/// impl` methods on the primitives.
pub fn mount_calc(session: &mut Session) {
    let u = |name: &str| (name.to_string(), vec![TY_F64], TY_F64, false);
    let b = |name: &str| (name.to_string(), vec![TY_F64, TY_F64], TY_F64, false);
    let uf = |name: &str| (name.to_string(), vec![TY_F32], TY_F32, false);
    let bf = |name: &str| (name.to_string(), vec![TY_F32, TY_F32], TY_F32, false);
    let host_funcs = vec![
        u("sqrt"), u("floor"), u("ceil"), u("round"), u("trunc"),
        u("exp"), u("ln"), u("log2"), u("log10"),
        u("sin"), u("cos"), u("tan"), u("asin"), u("acos"), u("atan"),
        u("sinh"), u("cosh"), u("tanh"),
        b("pow"), b("atan2"), b("hypot"), b("copysign"),
        ("fma".to_string(), vec![TY_F64, TY_F64, TY_F64], TY_F64, false),
        // the former float intrinsics — ordinary host fns now (the
        // integer intrinsics moved to core's `builtin impl` methods)
        u("abs"), b("min"), b("max"), u("signum"),
        // the f32 twins — same libm bodies in rut-std, `_f` names
        uf("sqrt_f"), uf("floor_f"), uf("ceil_f"), uf("round_f"), uf("trunc_f"),
        uf("exp_f"), uf("ln_f"), uf("log2_f"), uf("log10_f"),
        uf("sin_f"), uf("cos_f"), uf("tan_f"), uf("asin_f"), uf("acos_f"), uf("atan_f"),
        uf("sinh_f"), uf("cosh_f"), uf("tanh_f"),
        bf("pow_f"), bf("atan2_f"), bf("hypot_f"), bf("copysign_f"),
        ("fma_f".to_string(), vec![TY_F32, TY_F32, TY_F32], TY_F32, false),
        uf("abs_f"), bf("min_f"), bf("max_f"), uf("signum_f"),
    ];

    let c = |name: &str, v: f64| (name.to_string(), TY_F64, v.to_bits());
    let consts = vec![
        c("PI", std::f64::consts::PI),
        c("TAU", std::f64::consts::TAU),
        c("E", std::f64::consts::E),
        c("SQRT_2", std::f64::consts::SQRT_2),
        c("LN_2", std::f64::consts::LN_2),
        c("LN_10", std::f64::consts::LN_10),
        c("LOG2_E", std::f64::consts::LOG2_E),
        c("LOG10_E", std::f64::consts::LOG10_E),
        c("INFINITY", f64::INFINITY),
        c("NEG_INFINITY", f64::NEG_INFINITY),
        // `NAN` moved to core (`use core::{NAN}`)
        c("EPSILON", f64::EPSILON),
        c("MAX", f64::MAX),
        c("MIN", f64::MIN),
        c("MIN_POSITIVE", f64::MIN_POSITIVE),
    ];

    let _ = session.register_module(
        "calc",
        Module {
            namespace: Some("Math".to_string()),
            body: ModuleBody::Host {
                host_funcs,
                consts,
                native_types: vec![],
                native_traits: vec![],
                native_fns: vec![],
                native_impls: vec![],
            },
            ..Default::default()
        },
    );
}

/// Full pipeline over one module: resolve its `use` statements against a
/// session with the engine's packages mounted (`core`, `calc`), then
/// link, flatten, encode.
pub fn compile_module(src: &str, mode: Mode, module_name: &str) -> CompileOutput {
    let mut session = Session::new();
    mount_std(&mut session);
    compile_module_in(&mut session, src, mode, module_name)
}

/// [`compile_module`] against a caller-built session — hosts and tests
/// that mount their own libraries (the third-party pkgs are NOT in
/// [`mount_std`]; mount them with [`crate::mount_dir`]).
pub fn compile_module_in(
    session: &mut Session,
    src: &str,
    mode: Mode,
    module_name: &str,
) -> CompileOutput {
    let (ast, mut diags) = parse(src, mode);
    let tree = dump::to_dump_tree(&ast);
    let ast_dump = dump::render_text(&tree, src);
    let ast_json = dump::render_json(&tree);
    if !diags.is_empty() {
        return CompileOutput { diags, ast_dump, ast_json, ir_dump: String::new(), binary: None };
    }
    let spec = module_name.to_string();
    if let Err(e) = session.register_module(
        &spec,
        Module {
            body: ModuleBody::Source { text: src.to_string(), is_decl: mode == Mode::Decl },
            ..Default::default()
        },
    ) {
        diags.push(Diag::new(rut_lexer::span::Span::new(0, 0), e.to_string()));
        return CompileOutput { diags, ast_dump, ast_json, ir_dump: String::new(), binary: None };
    }
    let g = compile_graph(session, &spec);
    let ir_dump = g.program.as_ref().map(|p| ir_dump_of(&p.funcs, &p.interner)).unwrap_or_default();
    let binary = g.program.map(|p| encode(&p));
    CompileOutput { diags: g.diags, ast_dump, ast_json, ir_dump, binary }
}

/// irDump — the demo page's IR pane: per-function typed
/// register tables and op listings. Names resolve through the program's
/// interner.
pub fn ir_dump_of(funcs: &[rut_core::binary::FuncCode], interner: &rut_core::Interner) -> String {
    let mut out = String::new();
    for (i, f) in funcs.iter().enumerate() {
        if f.code.is_empty() {
            continue; // reserved placeholder, never compiled
        }
        out.push_str(&format!(
            "fn #{} {}({}) -> {}\n",
            i,
            interner.name(f.name),
            f.params
                .iter()
                .map(|&p| f_type_name(funcs, p))
                .collect::<Vec<_>>()
                .join(", "),
            f.ret
        ));
        out.push_str(&format!(
            "  regs: {}\n",
            (0..f.regs.len())
                .map(|r| format!("r{}:{}", r, f.regs[r]))
                .collect::<Vec<_>>()
                .join(" ")
        ));
        for (pc, op) in f.code.iter().enumerate() {
            out.push_str(&format!("  {:4} {}\n", pc, op_str(op, f)));
        }
    }
    out
}

fn f_type_name(_funcs: &[rut_core::binary::FuncCode], ty: u32) -> String {
    ty.to_string()
}

fn op_str(op: &Op, f: &rut_core::binary::FuncCode) -> String {
    // resolver-aware: pooled spans print as the lists they address
    let argv = |off: u32, argc: u16| {
        &f.argv[off as usize..off as usize + argc as usize]
    };
    match op {
        Op::MakeOpt { dst, src, .. } => format!("makeopt r{dst}, r{src}"),
        Op::WeakNew { dst, src, .. } => format!("weaknew r{dst}, r{src}"),
        Op::WeakUpgrade { recv, dst, .. } => format!("weakupgrade r{dst}, r{recv}"),
        Op::Mov { dst, src } => format!("mov r{dst}, r{src}"),
        Op::MovRef { dst, src } => format!("movref r{dst}, r{src}"),
        Op::Const { dst, k } => format!("const r{dst}, k{k}"),
        Op::ConstRaw { dst, bits } => format!("const r{dst}, {bits}"),
        Op::Not { dst, a } => format!("not r{dst}, r{a}"),
        Op::AddF { prim, dst, a, b } => format!("addf.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::SubF { prim, dst, a, b } => format!("subf.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::MulF { prim, dst, a, b } => format!("mulf.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::DivF { prim, dst, a, b } => format!("divf.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::ModF { prim, dst, a, b } => format!("modf.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::NegF { prim, dst, a } => format!("negf.{} r{dst}, r{a}", prim.name()),
        Op::EqF { dst, a, b } => format!("eqf r{dst}, r{a}, r{b}"),
        Op::NeF { dst, a, b } => format!("nef r{dst}, r{a}, r{b}"),
        Op::LtF { dst, a, b } => format!("ltf r{dst}, r{a}, r{b}"),
        Op::GtF { dst, a, b } => format!("gtf r{dst}, r{a}, r{b}"),
        Op::LeF { dst, a, b } => format!("lef r{dst}, r{a}, r{b}"),
        Op::GeF { dst, a, b } => format!("gef r{dst}, r{a}, r{b}"),
        Op::AddI { prim, dst, a, b } => format!("addi.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::SubI { prim, dst, a, b } => format!("subi.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::MulI { prim, dst, a, b } => format!("muli.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::DivI { prim, dst, a, b } => format!("divi.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::ModI { prim, dst, a, b } => format!("modi.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::WAddI { prim, dst, a, b } => format!("waddi.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::WSubI { prim, dst, a, b } => format!("wsubi.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::WMulI { prim, dst, a, b } => format!("wmuli.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::AndI { prim, dst, a, b } => format!("andi.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::OrI { prim, dst, a, b } => format!("ori.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::XorI { prim, dst, a, b } => format!("xori.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::ShlI { prim, dst, a, b } => format!("shli.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::ShrI { prim, dst, a, b } => format!("shri.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::WrapShlI { prim, dst, a, b } => format!("wshli.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::EqI { prim, dst, a, b } => format!("eqi.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::NeI { prim, dst, a, b } => format!("nei.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::LtI { prim, dst, a, b } => format!("lti.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::GtI { prim, dst, a, b } => format!("gti.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::LeI { prim, dst, a, b } => format!("lei.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::GeI { prim, dst, a, b } => format!("gei.{} r{dst}, r{a}, r{b}", prim.name()),
        Op::NegI { prim, dst, a } => format!("negi.{} r{dst}, r{a}", prim.name()),
        Op::StrCmp { eq, dst, a, b } => format!("strcmp {eq} r{dst}, r{a}, r{b}"),
        Op::ArrayCmp { eq, dst, a, b } => format!("arraycmp {eq} r{dst}, r{a}, r{b}"),
        Op::RefEq { eq, dst, a, b } => format!("refeq {eq} r{dst}, r{a}, r{b}"),
        Op::Jmp { target } => format!("jmp L{target}"),
        Op::Br { cond, then_t, else_t } => format!("br r{cond}, L{then_t}, L{else_t}"),
        Op::BrTable { idx, table_off, count, default } => {
            format!("brtable r{idx}, [{}] default L{default}", f.labels[*table_off as usize..*table_off as usize + *count as usize].iter().map(|t| format!("L{t}")).collect::<Vec<_>>().join(", "))
        }
        Op::Call { func, argv_off, argc, dst } => format!(
            "call f{func}({}) -> {}",
            argv(*argv_off, *argc).iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", "),
            if *dst != NOREG { format!("r{dst}") } else { "_".into() }
        ),
        Op::CallM { func, argv_off, argc, dst } => format!(
            "callm f{func}({}) -> {}",
            argv(*argv_off, *argc).iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", "),
            if *dst != NOREG { format!("r{dst}") } else { "_".into() }
        ),
        Op::CallI { slot, argv_off, argc, dst } => format!(
            "calli slot{slot}({}) -> {}",
            argv(*argv_off, *argc).iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", "),
            if *dst != NOREG { format!("r{dst}") } else { "_".into() }
        ),
        Op::CallNat { nat, recv, argv_off, argc, dst } => format!(
            "callnat {nat:?} {}({}) -> {}",
            if *recv != NOREG { format!("r{recv}") } else { "_".into() },
            argv(*argv_off, *argc).iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", "),
            if *dst != NOREG { format!("r{dst}") } else { "_".into() }
        ),
        Op::CallFn { fval, argv_off, argc, dst } => format!(
            "callfn r{fval}({}) -> {}",
            argv(*argv_off, *argc).iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", "),
            if *dst != NOREG { format!("r{dst}") } else { "_".into() }
        ),
        Op::Ret { val } => format!("ret {}", val.map(|r| format!("r{r}")).unwrap_or("_".into())),
        Op::NewCell { dst, ty } => format!("newcell r{dst}, t{ty}"),
        Op::MakeRecord { dst, ty, argv_off, argc } => format!(
            "makerecord r{dst}, t{ty}, [{}]",
            argv(*argv_off, *argc).iter().map(|v| format!("r{v}")).collect::<Vec<_>>().join(", ")
        ),
        Op::GetF { dst, obj, field, repr } => format!("getf r{dst}, r{obj}, f{field} :{}", repr.to_u8()),
        Op::SetF { obj, field, val, repr } => format!("setf r{obj}, f{field}, r{val} :{}", repr.to_u8()),
        Op::Own { dst, src, ty } => format!("own r{dst}, r{src}, t{ty}"),
        Op::ArrNew { dst, ty, len, .. } => format!("arrnew r{dst}, t{ty}, r{len}"),
        Op::ArrLit { dst, ty, argv_off, argc } => format!(
            "arrlit r{dst}, t{ty}, [{}]",
            argv(*argv_off, *argc).iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", ")
        ),
        Op::ArrGet { dst, arr, idx, repr } => format!("arrget r{dst}, r{arr}, r{idx} :{}", repr.to_u8()),
        Op::ArrSet { arr, idx, val, repr } => format!("arrset r{arr}, r{idx}, r{val} :{}", repr.to_u8()),
        Op::ArrGetF { dst, obj, field, idx, repr } => format!("arrgetf r{dst}, r{obj}, f{field}, r{idx} :{}", repr.to_u8()),
        Op::ArrSetF { obj, field, idx, val, repr } => format!("arrsetf r{obj}, f{field}, r{idx}, r{val} :{}", repr.to_u8()),
        Op::EnumNew { dst, ty, member } => format!("enumnew r{dst}, t{ty}, m{member}"),
        Op::TidOf { dst, obj } => format!("tidof r{dst}, r{obj}"),
        Op::IsType { dst, obj, want } => format!("istype r{dst}, r{obj}, t{want}"),
        Op::IsTrait { dst, obj, want } => format!("istrait r{dst}, r{obj}, trait{want}"),
        Op::Unbox { dst, box_, ty } => format!("unbox r{dst}, r{box_}, t{ty}"),
        Op::Box { dst, val, ty } => format!("box r{dst}, r{val}, t{ty}"),
        Op::MakeClosure { dst, func, argv_off, argc } => format!(
            "closure r{dst}, f{func}({})",
            argv(*argv_off, *argc).iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", ")
        ),
        Op::Panic { msg } => format!("panic r{msg}"),
        Op::LoopHead => "loophead".to_string(),
        #[allow(unreachable_patterns)]
        Op::Pad { .. } => unreachable!("layout pin, never constructed"),
        Op::Conv { dst, src, from, to } => format!("conv r{dst}, r{src}, {} -> {}", from.name(), to.name()),
        Op::StrCodeAt { dst, s, idx } => format!("strcodeat r{dst}, r{s}, r{idx}"),
    }
}

/// Unused-kind helper kept for dump parity.
#[allow(dead_code)]
fn is_cell(kind: &TyKind) -> bool {
    !matches!(kind, TyKind::Nil | TyKind::Prim(_) | TyKind::Fn { .. })
}
