//! astDump — pretty-printer over the flat arena (RFC 0041 §3: the demo
//! page's AST pane). Indented one-node-per-line form; `@n` suffixes expose
//! the stable `NodeId`s (RFC 0030 §5) for tooling.

use crate::ast::*;
use crate::token::{FloatSuffix, IntSuffix};

pub fn dump(ast: &Ast) -> String {
    let mut out = String::new();
    if let NodeKind::Module { items } = &ast.node(ast.root).kind {
        out.push_str(&format!("@{} Module\n", ast.root.0));
        for it in items {
            dump_node(ast, *it, 1, &mut out);
        }
    }
    out
}

fn ind(n: usize) -> String {
    "  ".repeat(n)
}

fn dump_node(a: &Ast, id: NodeId, d: usize, out: &mut String) {
    let node = a.node(id);
    let s = &node.span;
    match &node.kind {
        NodeKind::Module { items } => {
            out.push_str(&format!("{}@{} Module [{},{})\n", ind(d), id.0, s.lo, s.hi));
            for it in items {
                dump_node(a, *it, d + 1, out);
            }
        }
        NodeKind::Import { names, from } => {
            let ns: Vec<&str> = names.iter().map(|&n| a.name(n)).collect();
            out.push_str(&format!(
                "{}@{} Import {{{}}} from {:?} [{},{}]\n",
                ind(d),
                id.0,
                ns.join(", "),
                from,
                s.lo,
                s.hi
            ));
        }
        NodeKind::ModuleLet { vis, name, ty, init } => {
            out.push_str(&format!(
                "{}@{} Let {} {}{} [{},{}]\n",
                ind(d),
                id.0,
                vis_str(*vis),
                a.name(*name),
                if ty.is_some() { ": T" } else { "" },
                s.lo,
                s.hi
            ));
            if let Some(t) = ty {
                dump_node(a, *t, d + 1, out);
            }
            dump_node(a, *init, d + 1, out);
        }
        NodeKind::Enum { vis, name, members } => {
            let ms: Vec<String> = members
                .iter()
                .map(|(m, v)| match v {
                    Some(v) => format!("{}={}", a.name(*m), v),
                    None => a.name(*m).to_string(),
                })
                .collect();
            out.push_str(&format!(
                "{}@{} Enum {} {} {{{}}} [{},{}]\n",
                ind(d),
                id.0,
                vis_str(*vis),
                a.name(*name),
                ms.join(", "),
                s.lo,
                s.hi
            ));
        }
        NodeKind::Dataclass { vis, name, generics, fields, methods }
        | NodeKind::Class { vis, name, generics, fields, methods } => {
            let kind = match &node.kind {
                NodeKind::Dataclass { .. } => "Dataclass",
                _ => "Class",
            };
            let gs: Vec<&str> = generics.iter().map(|&g| a.name(g)).collect();
            out.push_str(&format!(
                "{}@{} {} {}{}<{}> [{},{}]\n",
                ind(d),
                id.0,
                kind,
                vis_str(*vis),
                a.name(*name),
                gs.join(", "),
                s.lo,
                s.hi
            ));
            for f in fields {
                dump_node(a, *f, d + 1, out);
            }
            for m in methods {
                dump_node(a, *m, d + 1, out);
            }
        }
        NodeKind::Trait { vis, name, generics, requires, methods } => {
            out.push_str(&format!(
                "{}@{} Trait {} {} ({} requires, {} methods) [{},{}]\n",
                ind(d),
                id.0,
                vis_str(*vis),
                a.name(*name),
                requires.len(),
                methods.len(),
                s.lo,
                s.hi
            ));
            let _ = generics;
            for m in methods {
                dump_node(a, *m, d + 1, out);
            }
        }
        NodeKind::Impl { trait_ref, target, methods } => {
            let tn = type_str(a, *trait_ref);
            let gn = type_str(a, *target);
            out.push_str(&format!(
                "{}@{} Impl {} for {} ({} methods) [{},{}]\n",
                ind(d),
                id.0,
                tn,
                gn,
                methods.len(),
                s.lo,
                s.hi
            ));
            for m in methods {
                dump_node(a, *m, d + 1, out);
            }
        }
        NodeKind::Fn { vis, is_suspend, name, generics, params, ret, where_bounds, body } => {
            out.push_str(&format!(
                "{}@{} Fn {}{}{} ({}, {} wheres) [{},{}]\n",
                ind(d),
                id.0,
                vis_str(*vis),
                if *is_suspend { "suspend " } else { "" },
                a.name(*name),
                generics.len(),
                params.len(),
                s.lo,
                s.hi
            ));
            let _ = ret;
            let _ = where_bounds;
            for p in params {
                dump_node(a, *p, d + 1, out);
            }
            if let Some(r) = ret {
                dump_node(a, *r, d + 1, out);
            }
            dump_node(a, *body, d + 1, out);
        }
        NodeKind::SurfaceFn { vis, linkage, name, generics, params, ret } => {
            out.push_str(&format!(
                "{}@{} SurfaceFn {} {} {} ({}) [{},{}]\n",
                ind(d),
                id.0,
                vis_str(*vis),
                if *linkage == Linkage::Host { "host" } else { "extern" },
                a.name(*name),
                generics.len() + params.len(),
                s.lo,
                s.hi
            ));
            let _ = ret;
        }
        NodeKind::SurfaceClass { vis, linkage, name, extparams, members } => {
            out.push_str(&format!(
                "{}@{} SurfaceClass {} {} {} ({}) [{},{}]\n",
                ind(d),
                id.0,
                vis_str(*vis),
                if *linkage == Linkage::Host { "host" } else { "extern" },
                a.name(*name),
                extparams.len() + members.len(),
                s.lo,
                s.hi
            ));
        }
        NodeKind::FieldDecl { is_private, is_static, name, ty, init } => {
            out.push_str(&format!(
                "{}@{} Field {}{}{}: {}{}\n",
                ind(d),
                id.0,
                if *is_static { "static " } else { "" },
                if *is_private { "private " } else { "" },
                a.name(*name),
                type_str(a, *ty),
                if init.is_some() { " = init" } else { "" },
            ));
        }
        NodeKind::MethodDecl { is_private, is_suspend, name, generics, params, ret, body } => {
            out.push_str(&format!(
                "{}@{} Method {}{}{} ({} params{}{})\n",
                ind(d),
                id.0,
                if *is_suspend { "suspend " } else { "" },
                if *is_private { "private " } else { "" },
                a.name(*name),
                params.len(),
                if !generics.is_empty() { ", generic" } else { "" },
                if body.is_none() { ", bodiless" } else { "" },
            ));
            let _ = ret;
        }
        NodeKind::Param { is_mut, name, ty } => {
            out.push_str(&format!(
                "{}@{} Param {}{}{}\n",
                ind(d),
                id.0,
                if *is_mut { "mut " } else { "" },
                a.name(*name),
                match ty {
                    Some(t) => format!(": {}", type_str(a, *t)),
                    None => String::new(),
                }
            ));
        }
        NodeKind::SelfParam { is_mut } => {
            out.push_str(&format!(
                "{}@{} SelfParam{}\n",
                ind(d),
                id.0,
                if *is_mut { " mut" } else { "" }
            ));
        }
        NodeKind::Block { stmts } => {
            out.push_str(&format!("{}@{} Block ({} stmts)\n", ind(d), id.0, stmts.len()));
            for st in stmts {
                dump_node(a, *st, d + 1, out);
            }
        }
        NodeKind::LetStmt { is_mut, name, ty, init } => {
            out.push_str(&format!(
                "{}@{} LetStmt {}{}{}\n",
                ind(d),
                id.0,
                if *is_mut { "mut " } else { "" },
                a.name(*name),
                match ty {
                    Some(t) => format!(": {}", type_str(a, *t)),
                    None => String::new(),
                }
            ));
            dump_node(a, *init, d + 1, out);
        }
        NodeKind::If { cond, then, els } => {
            out.push_str(&format!("{}@{} If{}\n", ind(d), id.0, if els.is_some() { " else" } else { "" }));
            dump_node(a, *cond, d + 1, out);
            dump_node(a, *then, d + 1, out);
            if let Some(e) = els {
                dump_node(a, *e, d + 1, out);
            }
        }
        NodeKind::While { cond, body } => {
            out.push_str(&format!("{}@{} While\n", ind(d), id.0));
            dump_node(a, *cond, d + 1, out);
            dump_node(a, *body, d + 1, out);
        }
        NodeKind::ForOf { var, iter, body } => {
            out.push_str(&format!("{}@{} ForOf {}\n", ind(d), id.0, a.name(*var)));
            dump_node(a, *iter, d + 1, out);
            dump_node(a, *body, d + 1, out);
        }
        NodeKind::ForC { var, init, cond, update, body } => {
            out.push_str(&format!("{}@{} ForC {}\n", ind(d), id.0, a.name(*var)));
            dump_node(a, *init, d + 1, out);
            dump_node(a, *cond, d + 1, out);
            dump_node(a, *update, d + 1, out);
            dump_node(a, *body, d + 1, out);
        }
        NodeKind::Return { value } => {
            out.push_str(&format!("{}@{} Return\n", ind(d), id.0));
            if let Some(v) = value {
                dump_node(a, *v, d + 1, out);
            }
        }
        NodeKind::Break => out.push_str(&format!("{}@{} Break\n", ind(d), id.0)),
        NodeKind::Continue => out.push_str(&format!("{}@{} Continue\n", ind(d), id.0)),
        NodeKind::WhenStmt { scrut, arms } => {
            out.push_str(&format!("{}@{} WhenStmt\n", ind(d), id.0));
            dump_node(a, *scrut, d + 1, out);
            for arm in arms {
                dump_node(a, *arm, d + 1, out);
            }
        }
        NodeKind::ExprStmt(e) => {
            out.push_str(&format!("{}@{} ExprStmt\n", ind(d), id.0));
            dump_node(a, *e, d + 1, out);
        }
        NodeKind::WhenArm { pats, body } => {
            out.push_str(&format!("{}@{} Arm ({} pats)\n", ind(d), id.0, pats.len()));
            for p in pats {
                dump_node(a, *p, d + 1, out);
            }
            dump_node(a, *body, d + 1, out);
        }
        NodeKind::SelectArm { fut, bind, body } => {
            out.push_str(&format!(
                "{}@{} SelectArm{}\n",
                ind(d),
                id.0,
                match bind {
                    Some(b) => format!(" as {}", a.name(*b)),
                    None => String::new(),
                }
            ));
            dump_node(a, *fut, d + 1, out);
            dump_node(a, *body, d + 1, out);
        }
        NodeKind::PatLit(e) => {
            out.push_str(&format!("{}@{} PatLit\n", ind(d), id.0));
            dump_node(a, *e, d + 1, out);
        }
        NodeKind::PatPath { segs } => {
            out.push_str(&format!("{}@{} PatPath {}\n", ind(d), id.0, seg_str(a, segs)));
        }
        NodeKind::PatCtor { segs, args } => {
            let as_: Vec<String> = args
                .iter()
                .map(|x| match x {
                    Some(n) => a.name(*n).to_string(),
                    None => "_".to_string(),
                })
                .collect();
            out.push_str(&format!(
                "{}@{} PatCtor {}({})\n",
                ind(d),
                id.0,
                seg_str(a, segs),
                as_.join(", ")
            ));
        }
        NodeKind::PatWild => out.push_str(&format!("{}@{} PatWild\n", ind(d), id.0)),
        NodeKind::PatElse => out.push_str(&format!("{}@{} PatElse\n", ind(d), id.0)),
        NodeKind::TyPath { segs, is_dyn } => {
            out.push_str(&format!(
                "{}@{} Ty{} {}\n",
                ind(d),
                id.0,
                if *is_dyn { " dyn" } else { "" },
                seg_str(a, segs)
            ));
        }
        NodeKind::TyFn { params, ret } => {
            let ps: Vec<String> = params.iter().map(|&p| type_str(a, p)).collect();
            out.push_str(&format!(
                "{}@{} TyFn fn({}): {}\n",
                ind(d),
                id.0,
                ps.join(", "),
                type_str(a, *ret)
            ));
        }
        NodeKind::TyConst(e) => {
            out.push_str(&format!("{}@{} TyConst\n", ind(d), id.0));
            dump_node(a, *e, d + 1, out);
        }
        NodeKind::Lit(l) => {
            out.push_str(&format!("{}@{} Lit {}\n", ind(d), id.0, lit_str(l)));
        }
        NodeKind::Path { segs } => {
            out.push_str(&format!("{}@{} Path {}\n", ind(d), id.0, seg_str(a, segs)));
        }
        NodeKind::Call { callee, args } => {
            out.push_str(&format!("{}@{} Call ({})\n", ind(d), id.0, args.len()));
            dump_node(a, *callee, d + 1, out);
            for arg in args {
                dump_node(a, *arg, d + 1, out);
            }
        }
        NodeKind::Method { recv, name, generics, args } => {
            out.push_str(&format!(
                "{}@{} Method .{}{} ({})\n",
                ind(d),
                id.0,
                a.name(*name),
                if generics.is_empty() { String::new() } else { "<..>".to_string() },
                args.len()
            ));
            dump_node(a, *recv, d + 1, out);
            for arg in args {
                dump_node(a, *arg, d + 1, out);
            }
        }
        NodeKind::Field { recv, name } => {
            out.push_str(&format!("{}@{} Field .{}\n", ind(d), id.0, a.name(*name)));
            dump_node(a, *recv, d + 1, out);
        }
        NodeKind::Index { recv, idx } => {
            out.push_str(&format!("{}@{} Index\n", ind(d), id.0));
            dump_node(a, *recv, d + 1, out);
            dump_node(a, *idx, d + 1, out);
        }
        NodeKind::Unary { op, expr } => {
            out.push_str(&format!(
                "{}@{} Unary {:?}\n",
                ind(d),
                id.0,
                match op {
                    UnOp::Neg => "-",
                    UnOp::Not => "!",
                    UnOp::BitNot => "~",
                }
            ));
            dump_node(a, *expr, d + 1, out);
        }
        NodeKind::Binary { op, lhs, rhs } => {
            out.push_str(&format!("{}@{} Binary {:?}\n", ind(d), id.0, binop_str(*op)));
            dump_node(a, *lhs, d + 1, out);
            dump_node(a, *rhs, d + 1, out);
        }
        NodeKind::Assign { op, target, value } => {
            out.push_str(&format!(
                "{}@{} Assign {}\n",
                ind(d),
                id.0,
                match op {
                    Some(o) => binop_str(*o),
                    None => "=",
                }
            ));
            dump_node(a, *target, d + 1, out);
            dump_node(a, *value, d + 1, out);
        }
        NodeKind::Lambda { params, ret, body } => {
            out.push_str(&format!("{}@{} Lambda ({})\n", ind(d), id.0, params.len()));
            for p in params {
                dump_node(a, *p, d + 1, out);
            }
            let _ = ret;
            dump_node(a, *body, d + 1, out);
        }
        NodeKind::Try { expr } => {
            out.push_str(&format!("{}@{} Try `?`\n", ind(d), id.0));
            dump_node(a, *expr, d + 1, out);
        }
        NodeKind::FStr { parts } => {
            let desc: Vec<String> = parts
                .iter()
                .map(|p| match p {
                    FPartAst::Lit(s) => format!("{s:?}"),
                    FPartAst::Hole(_) => "{expr}".to_string(),
                })
                .collect();
            out.push_str(&format!("{}@{} FStr \"{}\"\n", ind(d), id.0, desc.join(" ")));
            for p in parts {
                if let FPartAst::Hole(e) = p {
                    dump_node(a, *e, d + 1, out);
                }
            }
        }
        NodeKind::Struct { ty, fields } => {
            let fs: Vec<String> = fields.iter().map(|(f, _)| a.name(*f).to_string()).collect();
            out.push_str(&format!(
                "{}@{} Struct {} {{{}}}\n",
                ind(d),
                id.0,
                type_str(a, *ty),
                fs.join(", ")
            ));
            for (_, v) in fields {
                dump_node(a, *v, d + 1, out);
            }
        }
        NodeKind::ArrayLit { elems } => {
            out.push_str(&format!("{}@{} ArrayLit ({})\n", ind(d), id.0, elems.len()));
            for e in elems {
                dump_node(a, *e, d + 1, out);
            }
        }
        NodeKind::WhenExpr { scrut, arms } => {
            out.push_str(&format!("{}@{} WhenExpr\n", ind(d), id.0));
            dump_node(a, *scrut, d + 1, out);
            for arm in arms {
                dump_node(a, *arm, d + 1, out);
            }
        }
        NodeKind::Await { expr } => {
            out.push_str(&format!("{}@{} Await\n", ind(d), id.0));
            dump_node(a, *expr, d + 1, out);
        }
        NodeKind::Select { arms } => {
            out.push_str(&format!("{}@{} Select ({})\n", ind(d), id.0, arms.len()));
            for arm in arms {
                dump_node(a, *arm, d + 1, out);
            }
        }
        NodeKind::Is { expr, ty } => {
            out.push_str(&format!("{}@{} Is {}\n", ind(d), id.0, type_str(a, *ty)));
            dump_node(a, *expr, d + 1, out);
        }
    }
}

fn vis_str(v: Vis) -> &'static str {
    match v {
        Vis::Pub => "export",
        Vis::Mod => "export(mod)",
        Vis::Super => "export(super)",
        Vis::Self_ => "export(self)",
    }
}

fn seg_str(a: &Ast, segs: &[PathSeg]) -> String {
    segs.iter()
        .map(|s| {
            if s.generics.is_empty() {
                a.name(s.name).to_string()
            } else {
                format!("{}<..>", a.name(s.name))
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn type_str(a: &Ast, id: NodeId) -> String {
    match &a.node(id).kind {
        NodeKind::TyPath { segs, is_dyn } => {
            let base = seg_str(a, segs);
            if *is_dyn {
                format!("dyn {base}")
            } else {
                base
            }
        }
        NodeKind::TyFn { params, ret } => {
            let ps: Vec<String> = params.iter().map(|&p| type_str(a, p)).collect();
            format!("fn({}): {}", ps.join(", "), type_str(a, *ret))
        }
        NodeKind::TyConst(e) => format!("const {}", expr_str(a, *e)),
        _ => "<ty?>".to_string(),
    }
}

fn expr_str(a: &Ast, id: NodeId) -> String {
    match &a.node(id).kind {
        NodeKind::Lit(l) => lit_str(l),
        NodeKind::Path { segs } => seg_str(a, segs),
        NodeKind::Unary { op, expr } => format!("{}{}", match op { UnOp::Neg => "-", UnOp::Not => "!", UnOp::BitNot => "~" }, expr_str(a, *expr)),
        _ => "<expr?>".to_string(),
    }
}

fn lit_str(l: &Lit) -> String {
    match l {
        Lit::Int(v, None) => format!("{v}"),
        Lit::Int(v, Some(s)) => format!("{v}{}", int_suffix_str(*s)),
        Lit::Float(bits, None) => format!("{}", f64::from_bits(*bits)),
        Lit::Float(bits, Some(s)) => format!("{}{}", f64::from_bits(*bits), match s {
            FloatSuffix::F32 => "f32",
            FloatSuffix::F64 => "f64",
        }),
        Lit::Str(s) | Lit::RawStr(s) => format!("{s:?}"),
        Lit::Char(c) => format!("'{c}'"),
        Lit::Bool(b) => format!("{b}"),
    }
}

fn int_suffix_str(s: IntSuffix) -> &'static str {
    match s {
        IntSuffix::U8 => "u8", IntSuffix::U16 => "u16", IntSuffix::U32 => "u32", IntSuffix::U64 => "u64",
        IntSuffix::I8 => "i8", IntSuffix::I16 => "i16", IntSuffix::I32 => "i32", IntSuffix::I64 => "i64",
    }
}

fn binop_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*", BinOp::Div => "/", BinOp::Mod => "%",
        BinOp::Eq => "==", BinOp::Ne => "!=", BinOp::Lt => "<", BinOp::Gt => ">", BinOp::Le => "<=", BinOp::Ge => ">=",
        BinOp::And => "&&", BinOp::Or => "||",
        BinOp::BitAnd => "&", BinOp::BitOr => "|", BinOp::BitXor => "^", BinOp::Shl => "<<", BinOp::Shr => ">>",
        BinOp::WrapAdd => "&+", BinOp::WrapSub => "&-", BinOp::WrapMul => "&*", BinOp::WrapShl => "&<<",
    }
}
