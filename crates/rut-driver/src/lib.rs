//! Pipeline driver — RFC 0031: Ast ─► resolve ─► typecheck (fused with
//! body compilation, M1) ─► LIR ─► binary emit (RFC 0033). One entry point
//! for the CLI and the wasm demo: `compile_module`.

use rut_lir::check::{Ctx, FnKey, Inst};
use rut_lexer::diag::Diag;
use rut_ast::dump;
use rut_parser::{parse, Mode};
use rut_core::binary::{encode, Program};
use rut_core::ops::Op;
use rut_core::types::{TyKind, TY_F64, TY_I32, TY_OPAQUE, TY_STR, TY_UNIT};

pub mod session;
pub use session::{Entry, Manifest, ManifestError, Module, ResolveError, Session};

pub mod graph;
pub use graph::{compile_graph, GraphOutput};

pub mod loader;
pub use loader::{compile_dir, expand_module_source, load_dir_session};

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

/// Compile one module under `scope`, binding imported function surfaces
/// (RFC 0029 surface / RFC 0035 §1). Does not flatten or encode.
pub fn compile_program(
    src: &str,
    mode: Mode,
    module_name: &str,
    scope: rut_core::ScopeId,
    imports: &[(rut_core::ScopeId, rut_core::binary::Surface)],
) -> ProgramOutput {
    compile_program_resolved(src, mode, module_name, scope, imports, !imports.is_empty())
}

/// As [`compile_program`] but with an explicit `allow_imports` flag — the
/// graph compiler resolves every specifier itself (or inlines it), so it
/// passes `true` even when a module's only imports were source-inlined.
pub fn compile_program_resolved(
    src: &str,
    mode: Mode,
    module_name: &str,
    scope: rut_core::ScopeId,
    imports: &[(rut_core::ScopeId, rut_core::binary::Surface)],
    allow_imports: bool,
) -> ProgramOutput {
    let (mut ast, mut diags) = parse(src, mode);
    let tree = dump::to_dump_tree(&ast);
    let ast_dump = dump::render_text(&tree, src);
    let ast_json = dump::render_json(&tree);
    if !diags.is_empty() {
        return ProgramOutput { diags, ast_dump, ast_json, ir_dump: String::new(), program: None };
    }
    // Intern every imported surface name, so a namespace import (`Math`)
    // resolves its members by name even though the member name is never
    // written in `import { .. }` (RFC 0029 surface).
    for (_, surface) in imports {
        for f in &surface.funcs {
            ast.interner.intern(&f.name);
        }
        for c in &surface.consts {
            ast.interner.intern(&c.name);
        }
    }
    let mut ctx = Ctx::new_scoped(&ast, scope);
    ctx.allow_imports = allow_imports;
    for (dep_scope, surface) in imports {
        // types first: descriptors must be in the table before any own type
        // is interned (TypeTable::import_block)
        ctx.import_types(surface.types.clone(), &surface.scope_blocks);
        for f in &surface.funcs {
            if let Some(id) = ctx.ast.interner.lookup(&f.name) {
                if let Some(i) = f.intrinsic {
                    ctx.add_extern_intrinsic(id, i);
                } else {
                    ctx.add_extern_fn(id, rut_core::pack(*dep_scope, f.local), f.params.clone(), f.ret);
                }
            }
        }
        for c in &surface.consts {
            if let Some(id) = ctx.ast.interner.lookup(&c.name) {
                ctx.add_extern_const(id, c.ty, c.bits);
            }
        }
        for t in &surface.type_exports {
            if let Some(id) = ctx.ast.interner.lookup(&t.name) {
                ctx.add_extern_type(id, rut_core::pack(*dep_scope, t.local), t.is_class);
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
    if let Some(m) = ctx.lookup_name("main") {
        if ctx.find_free_fn(m) {
            roots.push(Inst { key: FnKey::Free(m), subst: vec![] });
        }
    }
    for name in ctx.entries.clone() {
        if ctx.find_free_fn(name) {
            roots.push(Inst { key: FnKey::Free(name), subst: vec![] });
        }
    }
    // library surface: every non-generic `pub fn` is importable, so its body
    // must be compiled even when nothing local calls it (RFC 0029 surface)
    for (name, node) in ctx.fn_nodes.clone() {
        let is_pub = ctx.exports.iter().any(|(n, _)| *n == ctx.name(name));
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
    let mut exports = Vec::new();
    let mut names: Vec<String> = ctx
        .entries
        .iter()
        .map(|&n| ctx.name(n).to_string())
        .collect();
    if ctx.exports.iter().any(|(n, _)| n == "main") && !names.iter().any(|n| n == "main") {
        names.push("main".to_string());
    }
    for n in names {
        if let Some(id) = ctx.lookup_name(&n) {
            if let Some(&f) = ctx.inst_map.get(&Inst { key: FnKey::Free(id), subst: vec![] }) {
                exports.push((n, f));
            }
        }
    }
    let funcs = std::mem::take(&mut ctx.funcs);
    let ir_dump = ir_dump_of(&funcs);
    // exported surface: every `pub` fn + its signature, for importers
    let mut surface = rut_core::binary::Surface::default();
    for (name, _) in &ctx.exports {
        if let Some(id) = ctx.lookup_name(name) {
            if let Some(&fid) = ctx.inst_map.get(&Inst { key: FnKey::Free(id), subst: vec![] }) {
                if let Some(f) = funcs.get(fid as usize) {
                    surface.funcs.push(rut_core::binary::SurfaceFn {
                        name: name.clone(),
                        params: f.params.clone(),
                        ret: f.ret,
                        local: fid,
                        intrinsic: None,
                    });
                }
            }
        }
    }
    // type surface: the whole non-boot block (so `(scope, local)` ids and
    // field layouts resolve in an importer) + the exported names
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
                name: ctx.name(*name).to_string(),
                local: rut_core::local_of(d.ty),
                is_class: d.kind == rut_lir::check::DataKind::Class,
                is_generic: !d.generics.is_empty(),
            });
        }
        for (name, e) in &ctx.enums {
            surface.type_exports.push(rut_core::binary::SurfaceType {
                name: ctx.name(*name).to_string(),
                local: rut_core::local_of(e.ty),
                is_class: false,
                is_generic: false,
            });
        }
    }
    let program = Program {
        name: module_name.to_string(),
        scope,
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

/// The `std:string` body — a mutable UTF-8 string builder (rut source).
pub fn std_string_source() -> String {
    include_str!("../../../rut/std-string/string.rut").to_string()
}

/// The `std:log` body (RFC 0028) — an ordinary rut module over the host
/// function `rt:log::emit`.
pub fn std_log_source() -> String {
    include_str!("../../../rut/std-log/log.rut").to_string()
}

/// Mount the standard modules every rut program expects: `std:collection`
/// and `std:string` (rut source), `std:math` (float host fns + constants +
/// compiler intrinsics), and the `std:log` logger over the `rt:log` native
/// module (RFC 0022/0026/0028).
pub fn mount_std(session: &mut Session) {
    let _ = session.register_module(
        "std:collection",
        Module { source: Some(std_collection_source()), ..Default::default() },
    );
    let _ = session.register_module(
        "std:string",
        Module { source: Some(std_string_source()), ..Default::default() },
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
        Module { host_funcs, consts, intrinsics, ..Default::default() },
    );
}

/// Mount `std:log` over its `rt:log` native module (the logger; RFC 0028).
pub fn mount_std_log(session: &mut Session) {
    let _ = session.register_module(
        "rt:log",
        Module {
            host_funcs: vec![
                ("create_logger".to_string(), vec![TY_STR], TY_OPAQUE),
                ("logger_log".to_string(), vec![TY_OPAQUE, TY_I32, TY_STR], TY_UNIT),
            ],
            ..Default::default()
        },
    );
    let _ = session.register_module(
        "std:log",
        Module { source: Some(std_log_source()), inline: true, ..Default::default() },
    );
}

/// Full pipeline over one module: resolve its `import` statements against a
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
        Module { source: Some(src.to_string()), ..Default::default() },
    ) {
        diags.push(Diag::new(rut_lexer::span::Span::new(0, 0), e.to_string()));
        return CompileOutput { diags, ast_dump, ast_json, ir_dump: String::new(), binary: None };
    }
    let g = compile_graph(&session, &spec);
    let ir_dump = g.program.as_ref().map(|p| ir_dump_of(&p.funcs)).unwrap_or_default();
    let binary = g.program.map(|p| encode(&p));
    CompileOutput { diags: g.diags, ast_dump, ast_json, ir_dump, binary }
}

/// irDump — the demo page's IR pane (RFC 0041 §3): per-function typed
/// register tables and op listings.
pub fn ir_dump_of(funcs: &[rut_core::binary::FuncCode]) -> String {
    let mut out = String::new();
    for (i, f) in funcs.iter().enumerate() {
        if f.code.is_empty() {
            continue; // reserved placeholder, never compiled
        }
        out.push_str(&format!(
            "fn #{} {}({}) -> {}\n",
            i,
            f.name,
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
            out.push_str(&format!("  {:4} {}\n", pc, op_str(op)));
        }
    }
    out
}

fn f_type_name(_funcs: &[rut_core::binary::FuncCode], ty: u32) -> String {
    ty.to_string()
}

fn op_str(op: &Op) -> String {
    match op {
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
        Op::BrTable { idx, table, default } => {
            format!("brtable r{idx}, [{}] default L{default}", table.iter().map(|t| format!("L{t}")).collect::<Vec<_>>().join(", "))
        }
        Op::Call { func, args, dst } => format!(
            "call f{func}({}) -> {}",
            args.iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", "),
            dst.map(|d| format!("r{d}")).unwrap_or("_".into())
        ),
        Op::CallM { func, recv, args, dst } => format!(
            "callm f{func} r{recv}({}) -> {}",
            args.iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", "),
            dst.map(|d| format!("r{d}")).unwrap_or("_".into())
        ),
        Op::CallI { slot, recv, args, dst } => format!(
            "calli slot{slot} r{recv}({}) -> {}",
            args.iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", "),
            dst.map(|d| format!("r{d}")).unwrap_or("_".into())
        ),
        Op::CallNat { nat, recv, args, dst } => format!(
            "callnat {nat:?} {}({}) -> {}",
            recv.map(|r| format!("r{r}")).unwrap_or("_".into()),
            args.iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", "),
            dst.map(|d| format!("r{d}")).unwrap_or("_".into())
        ),
        Op::CallFn { fval, args, dst } => format!(
            "callfn r{fval}({}) -> {}",
            args.iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", "),
            dst.map(|d| format!("r{d}")).unwrap_or("_".into())
        ),
        Op::Ret { val } => format!("ret {}", val.map(|r| format!("r{r}")).unwrap_or("_".into())),
        Op::NewCell { dst, ty } => format!("newcell r{dst}, t{ty}"),
        Op::MakeRecord { dst, ty, vals } => format!(
            "makerecord r{dst}, t{ty}, [{}]",
            vals.iter().map(|v| format!("r{v}")).collect::<Vec<_>>().join(", ")
        ),
        Op::GetF { dst, obj, field, repr } => format!("getf r{dst}, r{obj}, f{field} :{}", repr.to_u8()),
        Op::SetF { obj, field, val, repr } => format!("setf r{obj}, f{field}, r{val} :{}", repr.to_u8()),
        Op::Own { dst, src, ty } => format!("own r{dst}, r{src}, t{ty}"),
        Op::ArrNew { dst, ty, len, .. } => format!("arrnew r{dst}, t{ty}, r{len}"),
        Op::ArrLit { dst, ty, elems } => format!(
            "arrlit r{dst}, t{ty}, [{}]",
            elems.iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", ")
        ),
        Op::ArrGet { dst, arr, idx, repr } => format!("arrget r{dst}, r{arr}, r{idx} :{}", repr.to_u8()),
        Op::ArrSet { arr, idx, val, repr } => format!("arrset r{arr}, r{idx}, r{val} :{}", repr.to_u8()),
        Op::ArrGetF { dst, obj, field, idx, repr } => format!("arrgetf r{dst}, r{obj}, f{field}, r{idx} :{}", repr.to_u8()),
        Op::ArrSetF { obj, field, idx, val, repr } => format!("arrsetf r{obj}, f{field}, r{idx}, r{val} :{}", repr.to_u8()),
        Op::EnumNew { dst, ty, member } => format!("enumnew r{dst}, t{ty}, m{member}"),
        Op::OptSome { dst, ty, val } => format!("optsome r{dst}, t{ty}, r{val}"),
        Op::OptNone { dst, ty } => format!("optnone r{dst}, t{ty}"),
        Op::ResOk { dst, ty, val } => format!("resok r{dst}, t{ty}, r{val}"),
        Op::ResErr { dst, ty, val } => format!("reserr r{dst}, t{ty}, r{val}"),
        Op::SumIs { dst, v, want_err } => format!("sumis r{dst}, r{v}, err={want_err}"),
        Op::Unwrap { dst, v, want_err } => format!("unwrap r{dst}, r{v}, err={want_err}"),
        Op::UnwrapOr { dst, v, default } => format!("unwrapor r{dst}, r{v}, r{default}"),
        Op::Expect { dst, v, msg } => format!("expect r{dst}, r{v}, r{msg}"),
        Op::TidOf { dst, obj } => format!("tidof r{dst}, r{obj}"),
        Op::IsType { dst, obj, want } => format!("istype r{dst}, r{obj}, t{want}"),
        Op::IsTrait { dst, obj, want } => format!("istrait r{dst}, r{obj}, trait{want}"),
        Op::Unbox { dst, box_, ty } => format!("unbox r{dst}, r{box_}, t{ty}"),
        Op::Box { dst, val, ty } => format!("box r{dst}, r{val}, t{ty}"),
        Op::MakeClosure { dst, func, captures } => format!(
            "closure r{dst}, f{func}({})",
            captures.iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", ")
        ),
        Op::Panic { msg } => format!("panic r{msg}"),
        Op::Assert { cond, msg } => format!("assert r{cond}, {:?}", msg.map(|m| format!("r{m}"))),
        Op::LoopHead => "loophead".to_string(),
        Op::Conv { dst, src, from, to } => format!("conv r{dst}, r{src}, {} -> {}", from.name(), to.name()),
        Op::StrCharAt { dst, s, idx } => format!("strcharat r{dst}, r{s}, r{idx}"),
    }
}

/// Unused-kind helper kept for dump parity.
#[allow(dead_code)]
fn is_cell(kind: &TyKind) -> bool {
    !matches!(kind, TyKind::Unit | TyKind::Prim(_) | TyKind::Fn { .. })
}
