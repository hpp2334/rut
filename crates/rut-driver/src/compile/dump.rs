//! irDump — the demo page's IR pane rendering.

use rut_core::ops::{NOREG, Op};
use rut_core::types::TyKind;

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
        Op::IsIface { dst, obj, want } => format!("istrait r{dst}, r{obj}, trait{want}"),
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

