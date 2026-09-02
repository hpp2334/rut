//! Pipeline driver — RFC 0031: Ast ─► resolve ─► typecheck (fused with
//! body compilation, M1) ─► LIR ─► binary emit (RFC 0033). One entry point
//! for the CLI and the wasm demo: `compile_module`.

use crate::check::{Ctx, FnKey, Inst};
use crate::diag::Diag;
use crate::dump;
use crate::parser::{self, Mode};
use rut_core::binary::{encode, Program};
use rut_core::ops::Op;
use rut_core::types::TyKind;

pub struct CompileOutput {
    pub diags: Vec<Diag>,
    pub ast_dump: String,
    /// the demo tree UI's structured AST (flattened tagged JSON)
    pub ast_json: String,
    pub ir_dump: String,
    pub binary: Option<Vec<u8>>,
}

/// Full pipeline over one module. Diags stop before emit (RFC 0030 §6).
pub fn compile_module(src: &str, mode: Mode, module_name: &str) -> CompileOutput {
    let (ast, mut diags) = parser::parse(src, mode);
    let tree = dump::to_dump_tree(&ast);
    let ast_dump = dump::render_text(&tree, src);
    let ast_json = dump::render_json(&tree);
    if !diags.is_empty() {
        return CompileOutput { diags, ast_dump, ast_json, ir_dump: String::new(), binary: None };
    }
    let mut ctx = Ctx::new(&ast);
    ctx.collect();
    if !ctx.diags.is_empty() {
        diags.append(&mut ctx.diags.clone());
        return CompileOutput { diags, ast_dump, ast_json, ir_dump: String::new(), binary: None };
    }
    // module lets (load-time expression check, RFC 0003 §1)
    ctx.compile_module_lets();
    // entry: conventionally `main` (RFC 0003 §1) — required for a runnable
    // binary; a module without one still compiles (library shape)
    let main_id = ctx.lookup_name("main");
    let mut root: Option<Inst> = None;
    if let Some(m) = main_id {
        if ctx.find_free_fn(m) {
            root = Some(Inst { key: FnKey::Free(m), subst: vec![] });
        }
    }
    if let Some(root) = root {
        if ctx.compile_queue(root).is_err() {
            diags.append(&mut ctx.diags);
            return CompileOutput { diags, ast_dump, ast_json, ir_dump: String::new(), binary: None };
        }
    }
    if !ctx.diags.is_empty() {
        diags.append(&mut ctx.diags);
        return CompileOutput { diags, ast_dump, ast_json, ir_dump: String::new(), binary: None };
    }
    // global trait-method slots: same enumeration order as Ctx::trait_slot
    let mut trait_slots = Vec::new();
    for (i, t) in ctx.traits.iter().enumerate() {
        for m in 0..t.methods.len() {
            trait_slots.push((i as u32, m as u32));
        }
    }
    let vtables = ctx.build_vtables();
    // finalize exports: names → function ids
    let mut exports = Vec::new();
    for (n, _) in ctx.exports.clone() {
        if let Some(id) = ctx.lookup_name(&n) {
            if let Some(&f) = ctx.inst_map.get(&Inst { key: FnKey::Free(id), subst: vec![] }) {
                exports.push((n.clone(), f));
            }
        }
    }
    let funcs = std::mem::take(&mut ctx.funcs);
    // drop placeholder/empty entries never compiled (reserved, not emitted)
    let ir_dump = ir_dump_of(&funcs);
    let prog = Program {
        name: module_name.to_string(),
        types: ctx.types,
        traits: ctx.traits,
        trait_slots,
        vtables,
        consts: ctx.consts,
        funcs,
        exports,
    };
    let binary = encode(&prog);
    CompileOutput { diags, ast_dump, ast_json, ir_dump, binary: Some(binary) }
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
        Op::Arith { op, ty, dst, a, b } => format!("arith {:?} t{ty} r{dst}, r{a}, r{b}", op),
        Op::Wrap { op, ty, dst, a, b } => format!("wrap {:?} t{ty} r{dst}, r{a}, r{b}", op),
        Op::Bit { op, ty, dst, a, b } => format!("bit {:?} t{ty} r{dst}, r{a}, r{b}", op),
        Op::Cmp { op, ty, dst, a, b } => format!("cmp {:?} t{ty} r{dst}, r{a}, r{b}", op),
        Op::Not { dst, a } => format!("not r{dst}, r{a}"),
        Op::Neg { ty, dst, a } => format!("neg t{ty} r{dst}, r{a}"),
        Op::StrCmp { eq, dst, a, b } => format!("strcmp {eq} r{dst}, r{a}, r{b}"),
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
        Op::GetF { dst, obj, field } => format!("getf r{dst}, r{obj}, f{field}"),
        Op::SetF { obj, field, val } => format!("setf r{obj}, f{field}, r{val}"),
        Op::Own { dst, src, ty } => format!("own r{dst}, r{src}, t{ty}"),
        Op::ArrNew { dst, ty, len } => format!("arrnew r{dst}, t{ty}, r{len}"),
        Op::ArrLit { dst, ty, elems } => format!(
            "arrlit r{dst}, t{ty}, [{}]",
            elems.iter().map(|r| format!("r{r}")).collect::<Vec<_>>().join(", ")
        ),
        Op::ArrGet { dst, arr, idx } => format!("arrget r{dst}, r{arr}, r{idx}"),
        Op::ArrSet { arr, idx, val } => format!("arrset r{arr}, r{idx}, r{val}"),
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
        Op::Conv { dst, src, from, to } => format!("conv r{dst}, r{src}, t{from} -> t{to}"),
        Op::StrCharAt { dst, s, idx } => format!("strcharat r{dst}, r{s}, r{idx}"),
    }
}

/// Unused-kind helper kept for dump parity.
#[allow(dead_code)]
fn is_cell(kind: &TyKind) -> bool {
    !matches!(kind, TyKind::Unit | TyKind::Prim(_) | TyKind::Fn { .. })
}
