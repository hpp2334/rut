//! astDump — one mapping, two renderers (RFC 0041 §3: the demo page's AST
//! pane). `to_dump_tree` maps the typed arena to a labeled field tree once;
//! `render_json` emits the flattened tagged objects the demo's tree UI
//! consumes (no display strings on the wire — the demo derives every label
//! from `source` + small tables); `render_text` renders the same tree for
//! the CLI `rut dump` in the labeled `label: value` / `- item` style.

use crate::ast::*;

// ---- the dump tree ----

pub struct DumpNode {
    pub id: u32,
    pub kind: &'static str,
    pub span: (u32, u32),
    pub fields: Vec<DumpField>,
}
pub struct DumpField {
    pub label: &'static str,
    pub val: DumpVal,
}

/// Concrete companions — the Rust mirror of the demo's `d.ts` companion
/// interfaces. The JSON writer is a dumb serializer over these.
pub struct DumpSeg {
    pub name: String,
    pub generics: Vec<DumpNode>,
}
pub struct DumpMember {
    pub ident: String,
    pub int: Option<i64>,
}
pub struct DumpStructField {
    pub ident: String,
    pub node: DumpNode,
}
pub struct DumpWhere {
    pub ident: String,
    pub ty: DumpNode,
}
pub struct DumpExtParam {
    pub ident: String,
    pub ty: Option<DumpNode>,
}
pub enum DumpFPart {
    Lit(String),
    Hole(DumpNode),
}

pub enum DumpVal {
    Node(Box<DumpNode>),
    Nodes(Vec<DumpNode>),
    Idents(Vec<String>),
    Members(Vec<DumpMember>),
    StructFields(Vec<DumpStructField>),
    Wheres(Vec<DumpWhere>),
    ExtParams(Vec<DumpExtParam>),
    Segs(Vec<DumpSeg>),
    PatArgs(Vec<Option<String>>),
    FParts(Vec<DumpFPart>),
    /// stored strings: interned names, Import `from`
    Str(String),
    /// only emitted when true (quietness, not fabrication)
    Flag(bool),
    Vis(Vis),
    /// member visibility — only pushed when annotated (None = the
    /// unannotated module-private default, RFC 0003 §2)
    OptVis(Option<Vis>),
    Op(BinOp),
    UnOp(UnOp),
}

// ---- the single mapping ----

fn segs_dump(a: &Ast, segs: &[PathSeg]) -> Vec<DumpSeg> {
    segs.iter()
        .map(|s| DumpSeg {
            name: a.name(s.name).to_string(),
            generics: s.generics.iter().map(|&g| node_dump(a, g.id())).collect(),
        })
        .collect()
}

fn field(label: &'static str, val: DumpVal) -> DumpField {
    DumpField { label, val }
}

fn node_dump(a: &Ast, id: NodeId) -> DumpNode {
    let n = a.node(id);
    let span = (n.span.lo, n.span.hi);
    let mut fields: Vec<DumpField> = Vec::new();
    let kind: &'static str = match &n.kind {
        Kind::Item(k) => match k {
            ItemKind::Module { items } => {
                fields.push(field(
                    "items",
                    DumpVal::Nodes(items.iter().map(|&i| node_dump(a, i.id())).collect()),
                ));
                "Module"
            }
            ItemKind::Import { names, from } => {
                fields.push(field("names", DumpVal::Idents(names.iter().map(|&x| a.name(x).to_string()).collect())));
                fields.push(field("from", DumpVal::Str(from.clone())));
                "Import"
            }
            ItemKind::ModuleLet { vis, name, ty, init } => {
                fields.push(field("vis", DumpVal::Vis(*vis)));
                fields.push(field("name", DumpVal::Str(a.name(*name).to_string())));
                if let Some(t) = ty {
                    fields.push(field("ty", DumpVal::Node(Box::new(node_dump(a, t.id())))));
                }
                fields.push(field("init", DumpVal::Node(Box::new(node_dump(a, init.id())))));
                "ModuleLet"
            }
            ItemKind::Enum { vis, name, members } => {
                fields.push(field("vis", DumpVal::Vis(*vis)));
                fields.push(field("name", DumpVal::Str(a.name(*name).to_string())));
                fields.push(field(
                    "members",
                    DumpVal::Members(
                        members
                            .iter()
                            .map(|(m, v)| DumpMember { ident: a.name(*m).to_string(), int: *v })
                            .collect(),
                    ),
                ));
                "Enum"
            }
            ItemKind::Dataclass { vis, name, generics, fields: fs, methods } => {
                item_data_fields(a, &mut fields, *vis, *name, generics, fs, methods);
                "Dataclass"
            }
            ItemKind::Class { vis, name, generics, fields: fs, methods } => {
                item_data_fields(a, &mut fields, *vis, *name, generics, fs, methods);
                "Class"
            }
            ItemKind::Trait { vis, name, generics, requires, methods } => {
                fields.push(field("vis", DumpVal::Vis(*vis)));
                fields.push(field("name", DumpVal::Str(a.name(*name).to_string())));
                if !generics.is_empty() {
                    fields.push(field("generics", DumpVal::Idents(generics.iter().map(|&g| a.name(g).to_string()).collect())));
                }
                fields.push(field("requires", DumpVal::Nodes(requires.iter().map(|&t| node_dump(a, t.id())).collect())));
                fields.push(field("methods", DumpVal::Nodes(methods.iter().map(|&m| node_dump(a, m.id())).collect())));
                "Trait"
            }
            ItemKind::Impl { trait_ref, target, methods } => {
                fields.push(field("trait", DumpVal::Node(Box::new(node_dump(a, trait_ref.id())))));
                fields.push(field("target", DumpVal::Node(Box::new(node_dump(a, target.id())))));
                fields.push(field("methods", DumpVal::Nodes(methods.iter().map(|&m| node_dump(a, m.id())).collect())));
                "Impl"
            }
            ItemKind::Fn(f) => {
                fields.push(field("vis", DumpVal::Vis(f.vis)));
                if f.is_suspend {
                    fields.push(field("suspend", DumpVal::Flag(true)));
                }
                fields.push(field("name", DumpVal::Str(a.name(f.name).to_string())));
                if !f.generics.is_empty() {
                    fields.push(field("generics", DumpVal::Idents(f.generics.iter().map(|&g| a.name(g).to_string()).collect())));
                }
                fields.push(field("params", DumpVal::Nodes(f.params.iter().map(|&p| node_dump(a, p.id())).collect())));
                if let Some(r) = f.ret {
                    fields.push(field("ret", DumpVal::Node(Box::new(node_dump(a, r.id())))));
                }
                if !f.where_bounds.is_empty() {
                    fields.push(field(
                        "wheres",
                        DumpVal::Wheres(
                            f.where_bounds
                                .iter()
                                .map(|(n, t)| DumpWhere { ident: a.name(*n).to_string(), ty: node_dump(a, t.id()) })
                                .collect(),
                        ),
                    ));
                }
                fields.push(field("body", DumpVal::Node(Box::new(node_dump(a, f.body.id())))));
                "Fn"
            }
            ItemKind::SurfaceFn { vis, linkage, name, generics, params, ret } => {
                fields.push(field("vis", DumpVal::Vis(*vis)));
                fields.push(field("linkage", DumpVal::Str(if *linkage == Linkage::Host { "host" } else { "extern" }.to_string())));
                fields.push(field("name", DumpVal::Str(a.name(*name).to_string())));
                if !generics.is_empty() {
                    fields.push(field("generics", DumpVal::Idents(generics.iter().map(|&g| a.name(g).to_string()).collect())));
                }
                fields.push(field("params", DumpVal::Nodes(params.iter().map(|&p| node_dump(a, p.id())).collect())));
                if let Some(r) = ret {
                    fields.push(field("ret", DumpVal::Node(Box::new(node_dump(a, r.id())))));
                }
                "SurfaceFn"
            }
            ItemKind::SurfaceClass { vis, linkage, name, extparams, members } => {
                fields.push(field("vis", DumpVal::Vis(*vis)));
                fields.push(field("linkage", DumpVal::Str(if *linkage == Linkage::Host { "host" } else { "extern" }.to_string())));
                fields.push(field("name", DumpVal::Str(a.name(*name).to_string())));
                fields.push(field(
                    "extparams",
                    DumpVal::ExtParams(
                        extparams
                            .iter()
                            .map(|(n, t)| DumpExtParam {
                                ident: a.name(*n).to_string(),
                                ty: t.map(|x| node_dump(a, x.id())),
                            })
                            .collect(),
                    ),
                ));
                fields.push(field("members", DumpVal::Nodes(members.iter().map(|&m| node_dump(a, m.id())).collect())));
                "SurfaceClass"
            }
        },
        Kind::Member(k) => match k {
            MemberKind::FieldDecl(d) => {
                if let Some(v) = d.vis {
                    fields.push(field("vis", DumpVal::OptVis(Some(v))));
                }
                if d.is_static {
                    fields.push(field("static", DumpVal::Flag(true)));
                }
                fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
                fields.push(field("ty", DumpVal::Node(Box::new(node_dump(a, d.ty.id())))));
                if let Some(i) = d.init {
                    fields.push(field("init", DumpVal::Node(Box::new(node_dump(a, i.id())))));
                }
                "FieldDecl"
            }
            MemberKind::MethodDecl(d) => {
                if let Some(v) = d.vis {
                    fields.push(field("vis", DumpVal::OptVis(Some(v))));
                }
                if d.is_suspend {
                    fields.push(field("suspend", DumpVal::Flag(true)));
                }
                fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
                if !d.generics.is_empty() {
                    fields.push(field("generics", DumpVal::Idents(d.generics.iter().map(|&g| a.name(g).to_string()).collect())));
                }
                fields.push(field("params", DumpVal::Nodes(d.params.iter().map(|&p| node_dump(a, p.id())).collect())));
                if let Some(r) = d.ret {
                    fields.push(field("ret", DumpVal::Node(Box::new(node_dump(a, r.id())))));
                }
                if let Some(b) = d.body {
                    fields.push(field("body", DumpVal::Node(Box::new(node_dump(a, b.id())))));
                }
                "MethodDecl"
            }
            MemberKind::Param(d) => {
                if d.is_mut {
                    fields.push(field("mut", DumpVal::Flag(true)));
                }
                fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
                if let Some(t) = d.ty {
                    fields.push(field("ty", DumpVal::Node(Box::new(node_dump(a, t.id())))));
                }
                "Param"
            }
            MemberKind::SelfParam(d) => {
                if d.is_mut {
                    fields.push(field("mut", DumpVal::Flag(true)));
                }
                "SelfParam"
            }
        },
        Kind::Stmt(k) => match k {
            StmtKind::LetStmt { is_mut, name, ty, init } => {
                if *is_mut {
                    fields.push(field("mut", DumpVal::Flag(true)));
                }
                fields.push(field("name", DumpVal::Str(a.name(*name).to_string())));
                if let Some(t) = ty {
                    fields.push(field("ty", DumpVal::Node(Box::new(node_dump(a, t.id())))));
                }
                fields.push(field("init", DumpVal::Node(Box::new(node_dump(a, init.id())))));
                "LetStmt"
            }
            StmtKind::If { cond, then, els } => {
                fields.push(field("cond", DumpVal::Node(Box::new(node_dump(a, cond.id())))));
                fields.push(field("then", DumpVal::Node(Box::new(node_dump(a, then.id())))));
                if let Some(e) = els {
                    let id = match e {
                        ElseBranch::If(h) => h.id(),
                        ElseBranch::Block(h) => h.id(),
                    };
                    fields.push(field("els", DumpVal::Node(Box::new(node_dump(a, id)))));
                }
                "If"
            }
            StmtKind::While { cond, body } => {
                fields.push(field("cond", DumpVal::Node(Box::new(node_dump(a, cond.id())))));
                fields.push(field("body", DumpVal::Node(Box::new(node_dump(a, body.id())))));
                "While"
            }
            StmtKind::ForOf { var, iter, body } => {
                fields.push(field("var", DumpVal::Str(a.name(*var).to_string())));
                fields.push(field("iter", DumpVal::Node(Box::new(node_dump(a, iter.id())))));
                fields.push(field("body", DumpVal::Node(Box::new(node_dump(a, body.id())))));
                "ForOf"
            }
            StmtKind::ForC { var, init, cond, update, body } => {
                fields.push(field("var", DumpVal::Str(a.name(*var).to_string())));
                fields.push(field("init", DumpVal::Node(Box::new(node_dump(a, init.id())))));
                fields.push(field("cond", DumpVal::Node(Box::new(node_dump(a, cond.id())))));
                fields.push(field("update", DumpVal::Node(Box::new(node_dump(a, update.id())))));
                fields.push(field("body", DumpVal::Node(Box::new(node_dump(a, body.id())))));
                "ForC"
            }
            StmtKind::Return { value } => {
                if let Some(v) = value {
                    fields.push(field("value", DumpVal::Node(Box::new(node_dump(a, v.id())))));
                }
                "Return"
            }
            StmtKind::Break => "Break",
            StmtKind::Continue => "Continue",
            StmtKind::WhenStmt { scrut, arms } => {
                fields.push(field("scrut", DumpVal::Node(Box::new(node_dump(a, scrut.id())))));
                fields.push(field("arms", DumpVal::Nodes(arms.iter().map(|&x| node_dump(a, x.id())).collect())));
                "WhenStmt"
            }
            StmtKind::ExprStmt(e) => {
                fields.push(field("expr", DumpVal::Node(Box::new(node_dump(a, e.id())))));
                "ExprStmt"
            }
        },
        Kind::Arm(k) => match k {
            ArmKind::WhenArm { pats, body } => {
                fields.push(field("pats", DumpVal::Nodes(pats.iter().map(|&p| node_dump(a, p.id())).collect())));
                fields.push(field("body", DumpVal::Node(Box::new(node_dump(a, body.id())))));
                "WhenArm"
            }
            ArmKind::SelectArm { fut, bind, body } => {
                fields.push(field("fut", DumpVal::Node(Box::new(node_dump(a, fut.id())))));
                if let Some(b) = bind {
                    fields.push(field("bind", DumpVal::Str(a.name(*b).to_string())));
                }
                fields.push(field("body", DumpVal::Node(Box::new(node_dump(a, body.id())))));
                "SelectArm"
            }
        },
        Kind::Pat(k) => match k {
            PatKind::PatLit(e) => {
                fields.push(field("expr", DumpVal::Node(Box::new(node_dump(a, e.id())))));
                "PatLit"
            }
            PatKind::PatPath { segs } => {
                fields.push(field("segs", DumpVal::Segs(segs_dump(a, segs))));
                "PatPath"
            }
            PatKind::PatCtor { segs, args } => {
                fields.push(field("segs", DumpVal::Segs(segs_dump(a, segs))));
                fields.push(field("args", DumpVal::PatArgs(args.iter().map(|x| x.map(|n| a.name(n).to_string())).collect())));
                "PatCtor"
            }
            PatKind::PatWild => "PatWild",
            PatKind::PatElse => "PatElse",
        },
        Kind::Type(k) => match k {
            TypeKind::TyPath { segs, is_dyn } => {
                if *is_dyn {
                    fields.push(field("dyn", DumpVal::Flag(true)));
                }
                fields.push(field("segs", DumpVal::Segs(segs_dump(a, segs))));
                "TyPath"
            }
            TypeKind::TyFn { params, ret } => {
                fields.push(field("params", DumpVal::Nodes(params.iter().map(|&p| node_dump(a, p.id())).collect())));
                fields.push(field("ret", DumpVal::Node(Box::new(node_dump(a, ret.id())))));
                "TyFn"
            }
            TypeKind::TyConst(e) => {
                fields.push(field("expr", DumpVal::Node(Box::new(node_dump(a, e.id())))));
                "TyConst"
            }
        },
        Kind::Expr(k) => match k {
            ExprKind::Block { stmts } => {
                fields.push(field("stmts", DumpVal::Nodes(stmts.iter().map(|&s| node_dump(a, s.id())).collect())));
                "Block"
            }
            ExprKind::Lit(_) => "Lit", // value derived: src[lo..hi]
            ExprKind::Path { segs } => {
                fields.push(field("segs", DumpVal::Segs(segs_dump(a, segs))));
                "Path"
            }
            ExprKind::Call { callee, args } => {
                fields.push(field("callee", DumpVal::Node(Box::new(node_dump(a, callee.id())))));
                fields.push(field("args", DumpVal::Nodes(args.iter().map(|&x| node_dump(a, x.id())).collect())));
                "Call"
            }
            ExprKind::Method { recv, name, generics, args } => {
                fields.push(field("recv", DumpVal::Node(Box::new(node_dump(a, recv.id())))));
                fields.push(field("name", DumpVal::Str(a.name(*name).to_string())));
                if !generics.is_empty() {
                    fields.push(field("generics", DumpVal::Nodes(generics.iter().map(|&g| node_dump(a, g.id())).collect())));
                }
                fields.push(field("args", DumpVal::Nodes(args.iter().map(|&x| node_dump(a, x.id())).collect())));
                "Method"
            }
            ExprKind::Field { recv, name } => {
                fields.push(field("recv", DumpVal::Node(Box::new(node_dump(a, recv.id())))));
                fields.push(field("name", DumpVal::Str(a.name(*name).to_string())));
                "Field"
            }
            ExprKind::Index { recv, idx } => {
                fields.push(field("recv", DumpVal::Node(Box::new(node_dump(a, recv.id())))));
                fields.push(field("idx", DumpVal::Node(Box::new(node_dump(a, idx.id())))));
                "Index"
            }
            ExprKind::Unary { op, expr } => {
                fields.push(field("op", DumpVal::UnOp(*op)));
                fields.push(field("expr", DumpVal::Node(Box::new(node_dump(a, expr.id())))));
                "Unary"
            }
            ExprKind::Binary { op, lhs, rhs } => {
                fields.push(field("op", DumpVal::Op(*op)));
                fields.push(field("lhs", DumpVal::Node(Box::new(node_dump(a, lhs.id())))));
                fields.push(field("rhs", DumpVal::Node(Box::new(node_dump(a, rhs.id())))));
                "Binary"
            }
            ExprKind::Assign { op, target, value } => {
                if let Some(o) = op {
                    fields.push(field("op", DumpVal::Op(*o)));
                }
                fields.push(field("target", DumpVal::Node(Box::new(node_dump(a, target.id())))));
                fields.push(field("value", DumpVal::Node(Box::new(node_dump(a, value.id())))));
                "Assign"
            }
            ExprKind::Lambda { params, ret, body } => {
                fields.push(field("params", DumpVal::Nodes(params.iter().map(|&p| node_dump(a, p.id())).collect())));
                if let Some(r) = ret {
                    fields.push(field("ret", DumpVal::Node(Box::new(node_dump(a, r.id())))));
                }
                fields.push(field("body", DumpVal::Node(Box::new(node_dump(a, body.id())))));
                "Lambda"
            }
            ExprKind::Try { expr } => {
                fields.push(field("expr", DumpVal::Node(Box::new(node_dump(a, expr.id())))));
                "Try"
            }
            ExprKind::FStr { parts } => {
                fields.push(field(
                    "parts",
                    DumpVal::FParts(
                        parts
                            .iter()
                            .map(|p| match p {
                                FPartAst::Lit(s) => DumpFPart::Lit(s.clone()),
                                FPartAst::Hole(e) => DumpFPart::Hole(node_dump(a, e.id())),
                            })
                            .collect(),
                    ),
                ));
                "FStr"
            }
            ExprKind::Struct { ty, fields: fs } => {
                fields.push(field("ty", DumpVal::Node(Box::new(node_dump(a, ty.id())))));
                fields.push(field(
                    "fields",
                    DumpVal::StructFields(
                        fs.iter()
                            .map(|(n, v)| DumpStructField { ident: a.name(*n).to_string(), node: node_dump(a, v.id()) })
                            .collect(),
                    ),
                ));
                "Struct"
            }
            ExprKind::ArrayLit { elems } => {
                fields.push(field("elems", DumpVal::Nodes(elems.iter().map(|&e| node_dump(a, e.id())).collect())));
                "ArrayLit"
            }
            ExprKind::WhenExpr { scrut, arms } => {
                fields.push(field("scrut", DumpVal::Node(Box::new(node_dump(a, scrut.id())))));
                fields.push(field("arms", DumpVal::Nodes(arms.iter().map(|&x| node_dump(a, x.id())).collect())));
                "WhenExpr"
            }
            ExprKind::Await { expr } => {
                fields.push(field("expr", DumpVal::Node(Box::new(node_dump(a, expr.id())))));
                "Await"
            }
            ExprKind::Select { arms } => {
                fields.push(field("arms", DumpVal::Nodes(arms.iter().map(|&x| node_dump(a, x.id())).collect())));
                "Select"
            }
            ExprKind::Is { expr, ty } => {
                fields.push(field("expr", DumpVal::Node(Box::new(node_dump(a, expr.id())))));
                fields.push(field("ty", DumpVal::Node(Box::new(node_dump(a, ty.id())))));
                "Is"
            }
        },
    };
    DumpNode { id: id.0, kind, span, fields }
}

fn item_data_fields(
    a: &Ast,
    fields: &mut Vec<DumpField>,
    vis: Vis,
    name: IdentId,
    generics: &[IdentId],
    fs: &[NodeHandle<FieldDeclNode>],
    methods: &[NodeHandle<MethodDeclNode>],
) {
    fields.push(field("vis", DumpVal::Vis(vis)));
    fields.push(field("name", DumpVal::Str(a.name(name).to_string())));
    if !generics.is_empty() {
        fields.push(field("generics", DumpVal::Idents(generics.iter().map(|&g| a.name(g).to_string()).collect())));
    }
    fields.push(field("fields", DumpVal::Nodes(fs.iter().map(|&f| node_dump(a, f.id())).collect())));
    fields.push(field("methods", DumpVal::Nodes(methods.iter().map(|&m| node_dump(a, m.id())).collect())));
}

/// Build the dump tree from a parsed module.
pub fn to_dump_tree(ast: &Ast) -> DumpNode {
    node_dump(ast, ast.root.id())
}

// ---- renderer 1: JSON (flattened tagged objects, no display strings) ----

fn json_escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn vis_tag(v: Vis) -> &'static str {
    match v {
        Vis::Pub => "pub",
        Vis::Mod => "mod",
        Vis::Super => "super",
        Vis::Self_ => "self",
    }
}

fn node_json(n: &DumpNode, out: &mut String) {
    out.push_str(&format!("{{\"id\":{},\"kind\":", n.id));
    json_escape(n.kind, out);
    out.push_str(&format!(",\"span\":[{},{}]", n.span.0, n.span.1));
    for f in &n.fields {
        out.push(',');
        json_escape(f.label, out);
        out.push(':');
        val_json(&f.val, out);
    }
    out.push('}');
}

fn val_json(v: &DumpVal, out: &mut String) {
    match v {
        DumpVal::Node(n) => node_json(n, out),
        DumpVal::Nodes(ns) => {
            out.push('[');
            for (i, n) in ns.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                node_json(n, out);
            }
            out.push(']');
        }
        DumpVal::Idents(xs) => {
            out.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                json_escape(x, out);
            }
            out.push(']');
        }
        DumpVal::Members(ms) => {
            out.push('[');
            for (i, m) in ms.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('{');
                json_escape("ident", out);
                out.push(':');
                json_escape(&m.ident, out);
                if let Some(v) = m.int {
                    out.push_str(&format!(",\"int\":{v}"));
                }
                out.push('}');
            }
            out.push(']');
        }
        DumpVal::StructFields(fs) => {
            out.push('[');
            for (i, f) in fs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('{');
                json_escape("ident", out);
                out.push(':');
                json_escape(&f.ident, out);
                out.push_str(",\"node\":");
                node_json(&f.node, out);
                out.push('}');
            }
            out.push(']');
        }
        DumpVal::Wheres(ws) => {
            out.push('[');
            for (i, w) in ws.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('{');
                json_escape("ident", out);
                out.push(':');
                json_escape(&w.ident, out);
                out.push_str(",\"ty\":");
                node_json(&w.ty, out);
                out.push('}');
            }
            out.push(']');
        }
        DumpVal::ExtParams(ps) => {
            out.push('[');
            for (i, p) in ps.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('{');
                json_escape("ident", out);
                out.push(':');
                json_escape(&p.ident, out);
                if let Some(t) = &p.ty {
                    out.push_str(",\"ty\":");
                    node_json(t, out);
                }
                out.push('}');
            }
            out.push(']');
        }
        DumpVal::Segs(segs) => {
            out.push('[');
            for (i, s) in segs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('{');
                json_escape("name", out);
                out.push(':');
                json_escape(&s.name, out);
                if !s.generics.is_empty() {
                    out.push_str(",\"generics\":[");
                    for (j, g) in s.generics.iter().enumerate() {
                        if j > 0 {
                            out.push(',');
                        }
                        node_json(g, out);
                    }
                    out.push(']');
                }
                out.push('}');
            }
            out.push(']');
        }
        DumpVal::PatArgs(xs) => {
            out.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                match x {
                    Some(s) => json_escape(s, out),
                    None => out.push_str("null"),
                }
            }
            out.push(']');
        }
        DumpVal::FParts(ps) => {
            out.push('[');
            for (i, p) in ps.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                match p {
                    DumpFPart::Lit(s) => {
                        out.push_str("{\"kind\":\"FStrLit\",\"str\":");
                        json_escape(s, out);
                        out.push('}');
                    }
                    DumpFPart::Hole(n) => node_json(n, out),
                }
            }
            out.push(']');
        }
        DumpVal::Str(s) => json_escape(s, out),
        DumpVal::Flag(b) => out.push_str(if *b { "true" } else { "false" }),
        DumpVal::Vis(v) => out.push_str(&format!("\"{}\"", vis_tag(*v))),
        DumpVal::OptVis(v) => match v {
            Some(v) => out.push_str(&format!("\"{}\"", vis_tag(*v))),
            None => out.push_str("null"),
        },
        DumpVal::Op(op) => out.push_str(&format!("\"{:?}\"", op)),
        DumpVal::UnOp(op) => out.push_str(&format!("\"{:?}\"", op)),
    }
}

/// The demo tree: `{"id":..,"kind":..,"span":[lo,hi],<label>:<val>,...}`.
pub fn render_json(root: &DumpNode) -> String {
    let mut out = String::new();
    node_json(root, &mut out);
    out
}

// ---- renderer 2: text (CLI `rut dump`) ----

fn vis_str(v: Vis) -> &'static str {
    match v {
        Vis::Pub => "pub",
        Vis::Mod => "pub(mod)",
        Vis::Super => "pub(super)",
        Vis::Self_ => "pub(self)",
    }
}

fn binop_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*", BinOp::Div => "/", BinOp::Mod => "%",
        BinOp::Eq => "==", BinOp::Ne => "!=", BinOp::Lt => "<", BinOp::Gt => ">", BinOp::Le => "<=", BinOp::Ge => ">=",
        BinOp::And => "&&", BinOp::Or => "||",
        BinOp::BitAnd => "&", BinOp::BitOr => "|", BinOp::BitXor => "^", BinOp::Shl => "<<", BinOp::Shr => ">>",
    }
}

fn unop_str(op: UnOp) -> &'static str {
    match op {
        UnOp::Neg => "-",
        UnOp::Not => "!",
        UnOp::BitNot => "~",
    }
}

fn ind(n: usize) -> String {
    "  ".repeat(n)
}

fn header(n: &DumpNode, src: &str) -> String {
    // collapsed-row label: prefer the `name` field; literals slice the source
    let name = n.fields.iter().find(|f| f.label == "name").and_then(|f| match &f.val {
        DumpVal::Str(s) => Some(s.clone()),
        _ => None,
    });
    let name = name.unwrap_or_else(|| {
        if n.kind == "Lit" {
            src.get(n.span.0 as usize..n.span.1 as usize).unwrap_or("").to_string()
        } else {
            String::new()
        }
    });
    if name.is_empty() {
        format!("@{} {} [{},{})", n.id, n.kind, n.span.0, n.span.1)
    } else {
        format!("@{} {} {} [{},{})", n.id, n.kind, name, n.span.0, n.span.1)
    }
}

fn node_text(n: &DumpNode, d: usize, src: &str, out: &mut String) {
    out.push_str(&format!("{}{}\n", ind(d), header(n, src)));
    for f in &n.fields {
        val_text(&f.label, &f.val, d + 1, src, out);
    }
}

fn val_text(label: &str, v: &DumpVal, d: usize, src: &str, out: &mut String) {
    let i = ind(d);
    match v {
        DumpVal::Node(n) => {
            out.push_str(&format!("{i}{label}:\n"));
            node_text(n, d + 1, src, out);
        }
        DumpVal::Nodes(ns) => {
            out.push_str(&format!("{i}{label}:\n"));
            for n in ns {
                node_text(n, d + 1, src, out);
            }
        }
        DumpVal::Idents(xs) => {
            out.push_str(&format!("{i}{label}:\n"));
            for x in xs {
                out.push_str(&format!("{}- {x}\n", ind(d + 1)));
            }
        }
        DumpVal::Members(ms) => {
            out.push_str(&format!("{i}{label}:\n"));
            for m in ms {
                match m.int {
                    Some(v) => out.push_str(&format!("{}- {} = {v}\n", ind(d + 1), m.ident)),
                    None => out.push_str(&format!("{}- {}\n", ind(d + 1), m.ident)),
                }
            }
        }
        DumpVal::StructFields(fs) => {
            out.push_str(&format!("{i}{label}:\n"));
            for f in fs {
                out.push_str(&format!("{}- {}:\n", ind(d + 1), f.ident));
                node_text(&f.node, d + 2, src, out);
            }
        }
        DumpVal::Wheres(ws) => {
            out.push_str(&format!("{i}{label}:\n"));
            for w in ws {
                out.push_str(&format!("{}- {} requires\n", ind(d + 1), w.ident));
                node_text(&w.ty, d + 2, src, out);
            }
        }
        DumpVal::ExtParams(ps) => {
            out.push_str(&format!("{i}{label}:\n"));
            for p in ps {
                out.push_str(&format!("{}- {}\n", ind(d + 1), p.ident));
                if let Some(t) = &p.ty {
                    node_text(t, d + 2, src, out);
                }
            }
        }
        DumpVal::Segs(segs) => {
            out.push_str(&format!("{i}{label}:\n"));
            for s in segs {
                if s.generics.is_empty() {
                    out.push_str(&format!("{}- {}\n", ind(d + 1), s.name));
                } else {
                    out.push_str(&format!("{}- {}<{} generics>\n", ind(d + 1), s.name, s.generics.len()));
                    for g in &s.generics {
                        node_text(g, d + 2, src, out);
                    }
                }
            }
        }
        DumpVal::PatArgs(xs) => {
            out.push_str(&format!("{i}{label}:\n"));
            for x in xs {
                match x {
                    Some(s) => out.push_str(&format!("{}- {s}\n", ind(d + 1))),
                    None => out.push_str(&format!("{}- _\n", ind(d + 1))),
                }
            }
        }
        DumpVal::FParts(ps) => {
            out.push_str(&format!("{i}{label}:\n"));
            for p in ps {
                match p {
                    DumpFPart::Lit(s) => out.push_str(&format!("{}- {s:?}\n", ind(d + 1))),
                    DumpFPart::Hole(n) => node_text(n, d + 1, src, out),
                }
            }
        }
        DumpVal::Str(s) => out.push_str(&format!("{i}{label}: {s}\n")),
        DumpVal::Flag(b) => out.push_str(&format!("{i}{label}: {b}\n")),
        DumpVal::Vis(v) => out.push_str(&format!("{i}{label}: {}\n", vis_str(*v))),
        DumpVal::OptVis(v) => {
            if let Some(v) = v {
                out.push_str(&format!("{i}{label}: {}\n", vis_str(*v)))
            }
        }
        DumpVal::Op(op) => out.push_str(&format!("{i}{label}: {}\n", binop_str(*op))),
        DumpVal::UnOp(op) => out.push_str(&format!("{i}{label}: {}\n", unop_str(*op))),
    }
}

/// CLI text dump — same tree, labeled style. Spans on every node header;
/// `Lit` values are sliced from the source.
pub fn render_text(root: &DumpNode, src: &str) -> String {
    let mut out = String::new();
    node_text(root, 0, src, &mut out);
    out
}
