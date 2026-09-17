//! Pipeline driver — RFC 0031: Ast ─► resolve ─► typecheck (fused with
//! body compilation, M1) ─► LIR ─► binary emit (RFC 0033). One entry point
//! for the CLI and the wasm demo: `compile_module`.

use rut_lir::check::{Ctx, FnKey, Inst};
use rut_lexer::diag::Diag;
use rut_ast::dump;
use rut_parser::{parse, Mode};
use rut_core::binary::{encode, Program};
use rut_core::ops::{NOREG, Op};
use rut_core::types::{TyKind, TY_F64, TY_I32, TY_OPAQUE, TY_STR, TY_NIL};
use rut_core::{IdentId, sym};

pub mod session;
pub use session::{Entry, Manifest, ManifestError, Module, ResolveError, Session};

pub mod graph;
pub use graph::{compile_graph, GraphOutput};

pub mod bundle;
pub use bundle::{crc32, parse_bundle, write_bundle, BundleError};

pub mod loader;
pub use loader::{
    compile_dir, expand_module_source, load_bundle_bytes, load_bundle_session, load_dir_session,
    load_path_session, pack_dir,
};

pub struct CompileOutput {
    pub diags: Vec<Diag>,
    pub ast_dump: String,
    /// the demo tree UI's structured AST (flattened tagged JSON)
    pub ast_json: String,
    pub ir_dump: String,
    pub binary: Option<Vec<u8>>,
}

/// A compiled module. `program` still carries scope-qualified ids — the
/// driver links a graph and flattens once (RFC 0035 §1).
pub struct ProgramOutput {
    pub diags: Vec<Diag>,
    pub ast_dump: String,
    pub ast_json: String,
    pub ir_dump: String,
    pub program: Option<Program>,
}

/// Compile one module under `scope`, binding used function surfaces
/// (RFC 0029 surface / RFC 0035 §1). Does not flatten or encode.
pub fn compile_program(
    src: &str,
    mode: Mode,
    module_name: &str,
    scope: rut_core::ScopeId,
    uses: &[(rut_core::ScopeId, rut_core::binary::Surface)],
) -> ProgramOutput {
    compile_program_resolved(src, mode, module_name, scope, uses, !uses.is_empty())
}

/// As [`compile_program`] but with an explicit `allow_uses` flag — the
/// graph compiler resolves every specifier itself (or inlines it), so it
/// passes `true` even when a module's only uses were source-inlined.
pub fn compile_program_resolved(
    src: &str,
    mode: Mode,
    module_name: &str,
    scope: rut_core::ScopeId,
    uses: &[(rut_core::ScopeId, rut_core::binary::Surface)],
    allow_uses: bool,
) -> ProgramOutput {
    let (mut ast, mut diags) = parse(src, mode);
    let tree = dump::to_dump_tree(&ast);
    let ast_dump = dump::render_text(&tree, src);
    let ast_json = dump::render_json(&tree);
    if !diags.is_empty() {
        return ProgramOutput { diags, ast_dump, ast_json, ir_dump: String::new(), program: None };
    }
    // Intern every used surface name, so a namespace use (`Math`)
    // resolves its members by name even though the member name is never
    // written in `use { .. }` (RFC 0029 surface). Surface names are
    // ids in the exporter's interner (`surface.names`) — re-interned by
    // text into this module's.
    for (_, surface) in uses {
        for f in &surface.funcs {
            ast.interner.intern(surface.names.name(f.name));
        }
        for c in &surface.consts {
            ast.interner.intern(surface.names.name(c.name));
        }
    }
    let mut ctx = Ctx::new_scoped(&ast, scope);
    ctx.allow_uses = allow_uses;
    // the binding gate: the names this module's `use` statements wrote
    // (RFC 0028/0029) — read off the AST before any binding runs
    for it in ast.module_items(ast.root).to_vec() {
        if let rut_ast::ast::ItemKind::Use { names, .. } = ast.item(it) {
            ctx.used.extend(names.iter().copied());
        }
    }
    for (dep_scope, surface) in uses {
        // types first: descriptors must be in the table before any own type
        // is interned (TypeTable::use_block); names re-intern from the
        // exporter's interner
        ctx.use_types(surface.types.clone(), &surface.names, &surface.scope_blocks);
        for f in &surface.funcs {
            if let Some(id) = ctx.ast.interner.lookup(surface.names.name(f.name)) {
                if let Some(i) = f.intrinsic {
                    ctx.add_extern_intrinsic(id, i);
                } else {
                    ctx.add_extern_fn(id, rut_core::pack(*dep_scope, f.local), f.params.clone(), f.ret);
                }
            }
        }
        for c in &surface.consts {
            if let Some(id) = ctx.ast.interner.lookup(surface.names.name(c.name)) {
                ctx.add_extern_const(id, c.ty, c.bits);
            }
        }
        for t in &surface.type_exports {
            if let Some(id) = ctx.ast.interner.lookup(surface.names.name(t.name)) {
                ctx.add_extern_type(id, rut_core::pack(*dep_scope, t.local), t.is_class);
            }
        }
        // std:core's native surface (RFC 0028): builtin containers,
        // traits, and compiler-lowered fns — bound only when the
        // module wrote the name: `use { Array } from "std:core"`
        // gates `Array`, nothing else. The prelude is used, never
        // ambient.
        for (n, kind) in &surface.native_types {
            if let Some(id) = ctx.ast.interner.lookup(surface.names.name(*n)) {
                if ctx.used.contains(&id) {
                    ctx.add_extern_native_type(id, *kind);
                }
            }
        }
        for (n, native) in &surface.native_traits {
            if let Some(id) = ctx.ast.interner.lookup(surface.names.name(*n)) {
                if ctx.used.contains(&id) {
                    ctx.add_extern_trait(id, *native);
                }
            }
        }
        for n in &surface.native_fns {
            if let Some(id) = ctx.ast.interner.lookup(surface.names.name(*n)) {
                if ctx.used.contains(&id) {
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
    ctx.collect();
    // the entry surface's crossing contract is compile-time (RFC 0035 §3 /
    // 0023 §2): bad signatures are source diagnostics, never call-time
    // surprises for the embedder
    ctx.check_entries();
    if !ctx.diags.is_empty() {
        diags.append(&mut ctx.diags.clone());
        return ProgramOutput { diags, ast_dump, ast_json, ir_dump: String::new(), program: None };
    }
    // module lets (load-time expression check, RFC 0003 §1)
    ctx.compile_module_lets();
    // compilation roots: the conventional `main` (RFC 0003 §1) and every
    // `entry fn` — the host-callable surface (RFC 0035 §3). A module may
    // have either, both, or neither (pure library shape).
    let mut roots: Vec<Inst> = Vec::new();
    if ctx.find_free_fn(sym::MAIN) {
        roots.push(Inst { key: FnKey::Free(sym::MAIN), subst: vec![] });
    }
    for name in ctx.entries.clone() {
        if ctx.find_free_fn(name) {
            roots.push(Inst { key: FnKey::Free(name), subst: vec![] });
        }
    }
    // library surface: every non-generic `pub fn` is usable, so its body
    // must be compiled even when nothing local calls it (RFC 0029 surface)
    for (name, node) in ctx.fn_nodes.clone() {
        let is_pub = ctx.exports.iter().any(|(n, _)| *n == name);
        if !is_pub {
            continue;
        }
        if !ctx.ast.fn_decl(node).generics.is_empty() {
            continue;
        }
        roots.push(Inst { key: FnKey::Free(name), subst: vec![] });
    }
    for root in roots {
        if ctx.compile_queue(root).is_err() {
            diags.append(&mut ctx.diags);
            return ProgramOutput { diags, ast_dump, ast_json, ir_dump: String::new(), program: None };
        }
    }
    if !ctx.diags.is_empty() {
        diags.append(&mut ctx.diags);
        return ProgramOutput { diags, ast_dump, ast_json, ir_dump: String::new(), program: None };
    }
    // global trait-method slots: same enumeration order as Ctx::trait_slot
    let mut trait_slots = Vec::new();
    for (i, t) in ctx.traits.iter().enumerate() {
        for m in 0..t.methods.len() {
            trait_slots.push((i as u32, m as u32));
        }
    }
    let vtables = ctx.build_vtables();
    // finalize the entry table (RFC 0035 §3): `entry fn`s — plus the
    // conventional `main` when it is exported.
    let mut exports: Vec<(IdentId, u32)> = Vec::new();
    let mut names: Vec<IdentId> = ctx.entries.clone();
    if ctx.exports.iter().any(|(n, _)| *n == sym::MAIN) && !names.contains(&sym::MAIN) {
        names.push(sym::MAIN);
    }
    for n in names {
        if let Some(&f) = ctx.inst_map.get(&Inst { key: FnKey::Free(n), subst: vec![] }) {
            exports.push((n, f));
        }
    }
    let funcs = std::mem::take(&mut ctx.funcs);
    let ir_dump = ir_dump_of(&funcs, &ctx.interner);
    // exported surface: every `pub` fn + its signature, for using modules
    let mut surface = rut_core::binary::Surface::default();
    for (name, _) in &ctx.exports {
        if let Some(&fid) = ctx.inst_map.get(&Inst { key: FnKey::Free(*name), subst: vec![] }) {
            if let Some(f) = funcs.get(fid as usize) {
                surface.funcs.push(rut_core::binary::SurfaceFn {
                    name: *name,
                    params: f.params.clone(),
                    ret: f.ret,
                    local: fid,
                    intrinsic: None,
                });
            }
        }
    }
    // type surface: the whole non-boot block (so `(scope, local)` ids and
    // field layouts resolve in a using module) + the exported names
    {
        let boot_len = ctx.types.boot_len as usize;
        surface.types = ctx.types.types[boot_len..].to_vec();
        for s in 0..ctx.types.scope_base.len() as u32 {
            if s == rut_core::BOOT_SCOPE as u32 {
                continue;
            }
            let base = ctx.types.scope_base[s as usize];
            if base >= ctx.types.boot_len {
                surface.scope_blocks.push((s as rut_core::ScopeId, base - ctx.types.boot_len));
            }
        }
        for (name, d) in &ctx.datas {
            surface.type_exports.push(rut_core::binary::SurfaceType {
                name: *name,
                local: rut_core::local_of(d.ty),
                is_class: d.kind == rut_lir::check::DataKind::Class,
                is_generic: !d.generics.is_empty(),
            });
        }
        for (name, e) in &ctx.enums {
            surface.type_exports.push(rut_core::binary::SurfaceType {
                name: *name,
                local: rut_core::local_of(e.ty),
                is_class: false,
                is_generic: false,
            });
        }
    }
    // the program's interner moves out of the Ctx; the surface carries a
    // clone so it stays self-contained when it crosses to a using module
    let interner = std::mem::take(&mut ctx.interner);
    surface.names = interner.clone();
    let program = Program {
        name: module_name.to_string(),
        scope,
        interner,
        surface,
        types: ctx.types,
        traits: ctx.traits,
        trait_slots,
        vtables,
        consts: ctx.consts,
        funcs,
        exports,
        ..Default::default()
    };
    ProgramOutput { diags, ast_dump, ast_json, ir_dump, program: Some(program) }
}

/// The `std:collection` body, assembled from its parts (no filesystem): the
/// relative includes are merged so the module can be mounted in-memory by
/// wasm hosts and tests alike (RFC 0035 §1).
pub fn std_collection_source() -> String {
    format!(
        "{}\n{}",
        include_str!("../../../rut/std-collection/hash.rut"),
        include_str!("../../../rut/std-collection/vec.rut"),
    )
}

/// The `std:log` body (RFC 0028) — an ordinary rut module over the host
/// function `rt:log::emit`.
pub fn std_log_source() -> String {
    include_str!("../../../rut/std-log/log.rut").to_string()
}

/// Mount `std:core` — the prelude surface (RFC 0028): the builtin
/// containers (`Array`/`Opaque`), the builtin traits
/// (`Disposal`/`Index`/`Iterator`), and the compiler-lowered functions
/// (`downcast`, `assert`/`panic`, `make_ptr`/`on_drop`, the `str`/`bytes`
/// natives). v1.1 removed `Option`/`Result`/`own` — use sites diagnose
/// with the removal. A
/// native module with no body: its surface is
/// [`rut_core::binary::Surface::core`], the single source of truth
/// (`rut/std-core/core.d.rut` mirrors it for the LSP). Nothing here is
/// ambient — every name must be used.
pub fn mount_std_core(session: &mut Session) {
    // the surface is symbol-id based; the host-facing mount table is
    // string-based — `sym::text` bridges at this boundary only
    let core = rut_core::binary::Surface::core();
    let txt = |id: rut_core::IdentId| -> String {
        rut_core::sym::text(id).unwrap_or_default().to_string()
    };
    let _ = session.register_module(
        "std:core",
        Module {
            native_types: core.native_types.iter().map(|(n, k)| (txt(*n), *k)).collect(),
            native_traits: core.native_traits.iter().map(|(n, k)| (txt(*n), *k)).collect(),
            native_fns: core.native_fns.iter().map(|n| txt(*n)).collect(),
            ..Default::default()
        },
    );
}

/// Mount the standard modules every rut program expects: `std:collection`
/// (rut source), `std:math` (float host fns + constants + compiler
/// intrinsics), and the `std:log` logger over the `rt:log` native module
/// (RFC 0022/0026/0028).
pub fn mount_std(session: &mut Session) {
    mount_std_core(session);
    let _ = session.register_module(
        "std:collection",
        Module { source: Some(std_collection_source()), ..Default::default() },
    );
    mount_std_math(session);
    mount_std_log(session);
}

/// Mount `std:math` — a native module (RFC 0028): `f64` host functions
/// (bodies in `rut-std`), `f64` constants, and width-polymorphic integer
/// intrinsics the LIR expands inline (RFC 0032 §1.1 R2).
pub fn mount_std_math(session: &mut Session) {
    use rut_core::ops::Intrinsic;
    let u = |name: &str| (name.to_string(), vec![TY_F64], TY_F64);
    let b = |name: &str| (name.to_string(), vec![TY_F64, TY_F64], TY_F64);
    let host_funcs = vec![
        u("sqrt"), u("floor"), u("ceil"), u("round"), u("trunc"),
        u("exp"), u("ln"), u("log2"), u("log10"),
        u("sin"), u("cos"), u("tan"), u("asin"), u("acos"), u("atan"),
        u("sinh"), u("cosh"), u("tanh"),
        b("pow"), b("atan2"), b("hypot"), b("copysign"),
        ("fma".to_string(), vec![TY_F64, TY_F64, TY_F64], TY_F64),
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
        c("NAN", f64::NAN),
        c("EPSILON", f64::EPSILON),
        c("MAX", f64::MAX),
        c("MIN", f64::MIN),
        c("MIN_POSITIVE", f64::MIN_POSITIVE),
    ];

    let i2 = |name: &str, id: Intrinsic| (name.to_string(), id, 2usize);
    let i1 = |name: &str, id: Intrinsic| (name.to_string(), id, 1usize);
    let intrinsics = vec![
        i2("wrapping_add", Intrinsic::WrappingAdd),
        i2("wrapping_sub", Intrinsic::WrappingSub),
        i2("wrapping_mul", Intrinsic::WrappingMul),
        i2("wrapping_shl", Intrinsic::WrappingShl),
        i2("saturating_add", Intrinsic::SaturatingAdd),
        i2("saturating_sub", Intrinsic::SaturatingSub),
        i2("saturating_mul", Intrinsic::SaturatingMul),
        i2("checked_add", Intrinsic::CheckedAdd),
        i2("checked_sub", Intrinsic::CheckedSub),
        i2("checked_mul", Intrinsic::CheckedMul),
        i1("abs", Intrinsic::Abs),
        i2("min", Intrinsic::Min),
        i2("max", Intrinsic::Max),
        i1("signum", Intrinsic::Signum),
    ];

    let _ = session.register_module(
        "std:math",
        Module {
            namespace: Some("Math".to_string()),
            host_funcs,
            consts,
            intrinsics,
            ..Default::default()
        },
    );
}

/// Mount `std:log` over its `rt:log` native module (the logger; RFC 0028).
pub fn mount_std_log(session: &mut Session) {
    let _ = session.register_module(
        "rt:log",
        Module {
            host_funcs: vec![
                ("create_logger".to_string(), vec![TY_STR], TY_OPAQUE),
                ("logger_log".to_string(), vec![TY_OPAQUE, TY_I32, TY_STR], TY_NIL),
            ],
            ..Default::default()
        },
    );
    let _ = session.register_module(
        "std:log",
        Module { source: Some(std_log_source()), inline: true, ..Default::default() },
    );
}

/// Full pipeline over one module: resolve its `use` statements against a
/// session with the standard modules mounted, then link, flatten, encode.
pub fn compile_module(src: &str, mode: Mode, module_name: &str) -> CompileOutput {
    let (ast, mut diags) = parse(src, mode);
    let tree = dump::to_dump_tree(&ast);
    let ast_dump = dump::render_text(&tree, src);
    let ast_json = dump::render_json(&tree);
    if !diags.is_empty() {
        return CompileOutput { diags, ast_dump, ast_json, ir_dump: String::new(), binary: None };
    }
    let mut session = Session::new();
    mount_std(&mut session);
    let spec = format!("app:{module_name}");
    if let Err(e) = session.register_module(
        &spec,
        Module { source: Some(src.to_string()), is_decl: mode == Mode::Decl, ..Default::default() },
    ) {
        diags.push(Diag::new(rut_lexer::span::Span::new(0, 0), e.to_string()));
        return CompileOutput { diags, ast_dump, ast_json, ir_dump: String::new(), binary: None };
    }
    let g = compile_graph(&session, &spec);
    let ir_dump = g.program.as_ref().map(|p| ir_dump_of(&p.funcs, &p.interner)).unwrap_or_default();
    let binary = g.program.map(|p| encode(&p));
    CompileOutput { diags: g.diags, ast_dump, ast_json, ir_dump, binary }
}

/// irDump — the demo page's IR pane (RFC 0041 §3): per-function typed
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
        Op::MakePtr { dst, src, .. } => format!("makeptr r{dst}, r{src}"),
        Op::CloneVal { dst, src, .. } => format!("cloneval r{dst}, r{src}"),
        Op::ValEq { dst, a, b, .. } => format!("valeq r{dst}, r{a}, r{b}"),
        Op::OnDrop { obj, cleanup } => format!("ondrop r{obj}, r{cleanup}"),
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
        Op::ArrGetRef { dst, arr, idx, ty } => format!("arrgetref r{dst}, r{arr}, r{idx} t{ty}"),
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
        Op::Assert { cond, msg } => format!("assert r{cond}, {:?}", msg.map(|m| format!("r{m}"))),
        Op::LoopHead => "loophead".to_string(),
        #[allow(unreachable_patterns)]
        Op::Pad { .. } => unreachable!("layout pin, never constructed"),
        Op::Conv { dst, src, from, to } => format!("conv r{dst}, r{src}, {} -> {}", from.name(), to.name()),
        Op::StrCharAt { dst, s, idx } => format!("strcharat r{dst}, r{s}, r{idx}"),
    }
}

/// Unused-kind helper kept for dump parity.
#[allow(dead_code)]
fn is_cell(kind: &TyKind) -> bool {
    !matches!(kind, TyKind::Nil | TyKind::Prim(_) | TyKind::Fn { .. })
}
