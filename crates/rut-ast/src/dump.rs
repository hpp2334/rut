//! astDump — one mapping, two renderers (the demo page's AST
//! pane). `to_dump_tree` maps the typed arenas to a labeled field tree
//! once; `render_json` emits the flattened tagged objects the demo's
//! tree UI consumes (no display strings on the wire — the demo derives
//! every label from `source` + small tables); `render_text` renders the
//! same tree for the CLI `rut dump` in the labeled `label: value` /
//! `- item` style.
//!
//! The mapping dispatches the SLOT enums (`ItemRef`, `StmtRef`, …);
//! node ids are assigned in DFS pre-order (unique across the per-kind
//! arenas). `Error` slots dump as `Error` rows carrying their diag
//! messages — the positional recovery view.

use crate::error::ErrorNode;
use crate::expr::ExprRef;
use crate::handle::Handle;
use crate::item::{
    AliasNode, BuiltinImplNode, BuiltinPrimitiveNode, BuiltinTyNode, ImplNode, SurfaceFnNode,
    SurfaceStructNode,
};
use crate::item::{
    ClassNode, EnumNode, FieldDeclNode, FieldRef, FnNode, InterfaceNode, ItemRef, MemberRef,
    MethodDeclNode, MethodRef, ModuleLetNode, ModuleNode, ParamNode, ParamRef, SelfParamNode,
    StructNode, Vis,
};
use crate::pat::PatRef;
use crate::stmt::{
    ElseBranch, ForCNode, ForOfNode, IfNode, LetStmtNode, ReturnNode, StmtRef, WhenStmtNode,
    WhileNode,
};
use crate::ty::TyRef;
use crate::{
    ArrayLitNode, ArrayRepeatNode, AssignNode, Ast, AsyncBlockNode, AwaitNode, BinOp, BinaryNode,
    BlockNode, CallNode, CastNode, FPartAst, FStrNode, FieldNode, IfExprNode, IndexNode, IsNode,
    LambdaNode, LitNode, MethodNode, PatCtorNode, PatLitNode, PatPathNode, PathSeg, StructLitNode,
    TryNode, TupleNode, TyArrayNode, TyConstNode, TyFnNode, TyOptNode, TyPathNode, TyTupleNode,
    TyUnionNode, UnOp, UnaryNode, WhenArmNode, WhenExprNode,
};
use rut_core::IdentId;

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
}
pub struct DumpStructField {
    pub ident: String,
    pub node: DumpNode,
}
pub struct DumpBound {
    pub ident: String,
    pub ty: DumpNode,
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
    Bounds(Vec<DumpBound>),
    Segs(Vec<DumpSeg>),
    PatArgs(Vec<Option<String>>),
    FParts(Vec<DumpFPart>),
    /// stored strings: interned names
    Str(String),
    /// only emitted when true (quietness, not fabrication)
    Flag(bool),
    Vis(Vis),
    /// member visibility — only pushed when annotated (None = the
    /// unannotated module-private default)
    OptVis(Option<Vis>),
    Op(BinOp),
    UnOp(UnOp),
}

// ---- the single mapping ----

fn segs_dump(a: &Ast, segs: &[PathSeg], next: &mut u32) -> Vec<DumpSeg> {
    segs.iter()
        .map(|s| DumpSeg {
            name: a.name(s.name).to_string(),
            generics: s.generics.iter().map(|&g| ty_dump(a, g, next)).collect(),
        })
        .collect()
}

fn field(label: &'static str, val: DumpVal) -> DumpField {
    DumpField { label, val }
}

fn bounds_dump(a: &Ast, bounds: &[(IdentId, TyRef)], next: &mut u32) -> DumpVal {
    DumpVal::Bounds(
        bounds
            .iter()
            .map(|(n, t)| DumpBound {
                ident: a.name(*n).to_string(),
                ty: ty_dump(a, *t, next),
            })
            .collect(),
    )
}

fn error_dump(a: &Ast, h: Handle<ErrorNode>, next: &mut u32) -> DumpNode {
    let e = h.get(a);
    let mut fields = Vec::new();
    for d in &e.diags {
        fields.push(field("diag", DumpVal::Str(d.msg.clone())));
    }
    DumpNode {
        id: take_id(next),
        kind: "Error",
        span: (e.span.lo, e.span.hi),
        fields,
    }
}

fn take_id(next: &mut u32) -> u32 {
    let id = *next;
    *next += 1;
    id
}

fn item_dump(a: &Ast, r: ItemRef, next: &mut u32) -> DumpNode {
    let id = take_id(next);
    let span = (r.span(a).lo, r.span(a).hi);
    let mut fields: Vec<DumpField> = Vec::new();
    let kind: &'static str = match r {
        ItemRef::Module(h) => {
            module_fields(a, h.get(a), &mut fields, next);
            "Module"
        }
        ItemRef::ModDecl(h) => {
            let d = h.get(a);
            fields.push(field("vis", DumpVal::Vis(d.vis)));
            fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
            "ModDecl"
        }
        ItemRef::ModInline(h) => {
            let d = h.get(a);
            fields.push(field("vis", DumpVal::Vis(d.vis)));
            fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
            fields.push(field(
                "items",
                DumpVal::Nodes(d.items.iter().map(|&i| item_dump(a, i, next)).collect()),
            ));
            "ModInline"
        }
        ItemRef::Use(h) => {
            let d = h.get(a);
            fields.push(field("vis", DumpVal::Vis(d.vis)));
            fields.push(field(
                "path",
                DumpVal::Idents(d.path.iter().map(|&x| a.name(x).to_string()).collect()),
            ));
            fields.push(field(
                "names",
                DumpVal::Idents(d.names.iter().map(|&x| a.name(x).to_string()).collect()),
            ));
            "Use"
        }
        ItemRef::Alias(h) => {
            let d: &AliasNode = h.get(a);
            fields.push(field("vis", DumpVal::Vis(d.vis)));
            fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
            fields.push(field(
                "target",
                DumpVal::Node(Box::new(ty_dump(a, d.target, next))),
            ));
            "Alias"
        }
        ItemRef::ModuleLet(h) => {
            let d: &ModuleLetNode = h.get(a);
            fields.push(field("vis", DumpVal::Vis(d.vis)));
            fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
            if let Some(t) = d.ty {
                fields.push(field("ty", DumpVal::Node(Box::new(ty_dump(a, t, next)))));
            }
            fields.push(field(
                "init",
                DumpVal::Node(Box::new(expr_dump(a, d.init, next))),
            ));
            "ModuleLet"
        }
        ItemRef::Enum(h) => {
            let d: &EnumNode = h.get(a);
            fields.push(field("vis", DumpVal::Vis(d.vis)));
            fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
            fields.push(field(
                "members",
                DumpVal::Members(
                    d.members
                        .iter()
                        .map(|m| match m {
                            MemberRef::Member(mh) => DumpMember {
                                ident: a.name(mh.get(a).name).to_string(),
                            },
                            MemberRef::Error(_) => DumpMember {
                                ident: "<error>".to_string(),
                            },
                        })
                        .collect(),
                ),
            ));
            "Enum"
        }
        ItemRef::Struct(h) => {
            let d: &StructNode = h.get(a);
            decl_fields(
                a,
                &mut fields,
                d.vis,
                d.name,
                &d.generics,
                &d.fields,
                &d.methods,
                next,
            );
            "Struct"
        }
        ItemRef::Class(h) => {
            let d: &ClassNode = h.get(a);
            decl_fields(
                a,
                &mut fields,
                d.vis,
                d.name,
                &d.generics,
                &d.fields,
                &d.methods,
                next,
            );
            if d.newtype {
                fields.push(field("newtype", DumpVal::Flag(true)));
            }
            if !d.requires.is_empty() {
                fields.push(field("bounds", bounds_dump(a, &d.requires, next)));
            }
            "Class"
        }
        ItemRef::Interface(h) => {
            let d: &InterfaceNode = h.get(a);
            fields.push(field("vis", DumpVal::Vis(d.vis)));
            fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
            if !d.generics.is_empty() {
                fields.push(field(
                    "generics",
                    DumpVal::Idents(d.generics.iter().map(|&g| a.name(g).to_string()).collect()),
                ));
            }
            fields.push(field(
                "methods",
                DumpVal::Nodes(d.methods.iter().map(|m| method_dump(a, *m, next)).collect()),
            ));
            "Interface"
        }
        ItemRef::Impl(h) => {
            let d: &ImplNode = h.get(a);
            if !d.generics.is_empty() {
                fields.push(field(
                    "generics",
                    DumpVal::Idents(d.generics.iter().map(|&g| a.name(g).to_string()).collect()),
                ));
            }
            fields.push(field(
                "target",
                DumpVal::Node(Box::new(ty_dump(a, d.target, next))),
            ));
            fields.push(field(
                "methods",
                DumpVal::Nodes(d.methods.iter().map(|m| method_dump(a, *m, next)).collect()),
            ));
            "Impl"
        }
        ItemRef::Fn(h) => {
            let f: &FnNode = h.get(a);
            fields.push(field("vis", DumpVal::Vis(f.vis)));
            if f.is_async {
                fields.push(field("async", DumpVal::Flag(true)));
            }
            fields.push(field("name", DumpVal::Str(a.name(f.name).to_string())));
            if !f.generics.is_empty() {
                fields.push(field(
                    "generics",
                    DumpVal::Idents(f.generics.iter().map(|&g| a.name(g).to_string()).collect()),
                ));
            }
            fields.push(field(
                "params",
                DumpVal::Nodes(f.params.iter().map(|p| param_dump(a, *p, next)).collect()),
            ));
            if let Some(r) = f.ret {
                fields.push(field("ret", DumpVal::Node(Box::new(ty_dump(a, r, next)))));
            }
            if !f.bounds.is_empty() {
                fields.push(field("bounds", bounds_dump(a, &f.bounds, next)));
            }
            fields.push(field(
                "body",
                DumpVal::Node(Box::new(block_dump(a, f.body, next))),
            ));
            "Fn"
        }
        ItemRef::SurfaceFn(h) => {
            let d: &SurfaceFnNode = h.get(a);
            fields.push(field("vis", DumpVal::Vis(d.vis)));
            fields.push(field(
                "linkage",
                DumpVal::Str(
                    match d.linkage {
                        crate::item::Linkage::Host => "host",
                        crate::item::Linkage::Builtin { .. } => "builtin",
                    }
                    .to_string(),
                ),
            ));
            if d.is_async {
                fields.push(field("is_async", DumpVal::Flag(true)));
            }
            fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
            if !d.generics.is_empty() {
                fields.push(field(
                    "generics",
                    DumpVal::Idents(d.generics.iter().map(|&g| a.name(g).to_string()).collect()),
                ));
            }
            fields.push(field(
                "params",
                DumpVal::Nodes(d.params.iter().map(|p| param_dump(a, *p, next)).collect()),
            ));
            if let Some(r) = d.ret {
                fields.push(field("ret", DumpVal::Node(Box::new(ty_dump(a, r, next)))));
            }
            "SurfaceFn"
        }
        ItemRef::SurfaceStruct(h) => {
            let d: &SurfaceStructNode = h.get(a);
            fields.push(field("vis", DumpVal::Vis(d.vis)));
            fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
            fields.push(field(
                "fields",
                DumpVal::Nodes(d.fields.iter().map(|f| field_dump(a, *f, next)).collect()),
            ));
            "SurfaceStruct"
        }
        ItemRef::BuiltinTy(h) => {
            let d: &BuiltinTyNode = h.get(a);
            fields.push(field("vis", DumpVal::Vis(d.vis)));
            fields.push(field(
                "linkage",
                DumpVal::Str(
                    if d.ambient {
                        "prelude builtin"
                    } else {
                        "pub builtin"
                    }
                    .to_string(),
                ),
            ));
            fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
            if !d.generics.is_empty() {
                fields.push(field(
                    "generics",
                    DumpVal::Idents(d.generics.iter().map(|&g| a.name(g).to_string()).collect()),
                ));
            }
            fields.push(field(
                "members",
                DumpVal::Nodes(d.members.iter().map(|m| method_dump(a, *m, next)).collect()),
            ));
            "BuiltinTy"
        }
        ItemRef::BuiltinPrimitive(h) => {
            let d: &BuiltinPrimitiveNode = h.get(a);
            fields.push(field(
                "linkage",
                DumpVal::Str(
                    if d.ambient {
                        "prelude builtin"
                    } else {
                        "pub builtin"
                    }
                    .to_string(),
                ),
            ));
            fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
            fields.push(field(
                "members",
                DumpVal::Nodes(d.members.iter().map(|m| method_dump(a, *m, next)).collect()),
            ));
            "BuiltinPrimitive"
        }
        ItemRef::BuiltinImpl(h) => {
            let d: &BuiltinImplNode = h.get(a);
            fields.push(field("vis", DumpVal::Vis(d.vis)));
            fields.push(field("prim", DumpVal::Str(a.name(d.prim).to_string())));
            fields.push(field(
                "methods",
                DumpVal::Nodes(d.methods.iter().map(|m| method_dump(a, *m, next)).collect()),
            ));
            "BuiltinImpl"
        }
        ItemRef::Error(h) => return error_dump(a, h, next),
    };
    DumpNode {
        id,
        kind,
        span,
        fields,
    }
}

fn module_fields(a: &Ast, d: &ModuleNode, fields: &mut Vec<DumpField>, next: &mut u32) {
    fields.push(field(
        "items",
        DumpVal::Nodes(d.items.iter().map(|&i| item_dump(a, i, next)).collect()),
    ));
}

#[allow(clippy::too_many_arguments)]
fn decl_fields(
    a: &Ast,
    fields: &mut Vec<DumpField>,
    vis: Vis,
    name: IdentId,
    generics: &[IdentId],
    fs: &[FieldRef],
    methods: &[MethodRef],
    next: &mut u32,
) {
    fields.push(field("vis", DumpVal::Vis(vis)));
    fields.push(field("name", DumpVal::Str(a.name(name).to_string())));
    if !generics.is_empty() {
        fields.push(field(
            "generics",
            DumpVal::Idents(generics.iter().map(|&g| a.name(g).to_string()).collect()),
        ));
    }
    fields.push(field(
        "fields",
        DumpVal::Nodes(fs.iter().map(|f| field_dump(a, *f, next)).collect()),
    ));
    fields.push(field(
        "methods",
        DumpVal::Nodes(methods.iter().map(|m| method_dump(a, *m, next)).collect()),
    ));
}

fn field_dump(a: &Ast, r: FieldRef, next: &mut u32) -> DumpNode {
    match r {
        FieldRef::Field(h) => field_decl_dump(a, h, next),
        FieldRef::Error(h) => error_dump(a, h, next),
    }
}

fn field_decl_dump(a: &Ast, h: Handle<FieldDeclNode>, next: &mut u32) -> DumpNode {
    let d = h.get(a);
    let mut fields = Vec::new();
    if let Some(v) = d.vis() {
        fields.push(field("vis", DumpVal::OptVis(Some(v))));
    }
    if d.is_static() {
        fields.push(field("static", DumpVal::Flag(true)));
    }
    fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
    fields.push(field("ty", DumpVal::Node(Box::new(ty_dump(a, d.ty, next)))));
    if let Some(i) = d.init {
        fields.push(field(
            "init",
            DumpVal::Node(Box::new(expr_dump(a, i, next))),
        ));
    }
    DumpNode {
        id: take_id(next),
        kind: "FieldDecl",
        span: (d.span.lo, d.span.hi),
        fields,
    }
}

fn method_dump(a: &Ast, r: MethodRef, next: &mut u32) -> DumpNode {
    match r {
        MethodRef::Method(h) => method_decl_dump(a, h, next),
        MethodRef::Error(h) => error_dump(a, h, next),
    }
}

fn method_decl_dump(a: &Ast, h: Handle<MethodDeclNode>, next: &mut u32) -> DumpNode {
    let d = h.get(a);
    let mut fields = Vec::new();
    if let Some(v) = d.vis() {
        fields.push(field("vis", DumpVal::OptVis(Some(v))));
    }
    if d.is_async() {
        fields.push(field("async", DumpVal::Flag(true)));
    }
    if let Some(m) = d.marker() {
        fields.push(field("marker", DumpVal::Str(a.name(m).to_string())));
    }
    fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
    if !d.generics.is_empty() {
        fields.push(field(
            "generics",
            DumpVal::Idents(d.generics.iter().map(|&g| a.name(g).to_string()).collect()),
        ));
    }
    fields.push(field(
        "params",
        DumpVal::Nodes(d.params.iter().map(|p| param_dump(a, *p, next)).collect()),
    ));
    if let Some(r) = d.ret {
        fields.push(field("ret", DumpVal::Node(Box::new(ty_dump(a, r, next)))));
    }
    if !d.bounds.is_empty() {
        fields.push(field("bounds", bounds_dump(a, &d.bounds, next)));
    }
    if let Some(b) = d.body {
        fields.push(field(
            "body",
            DumpVal::Node(Box::new(block_dump(a, b, next))),
        ));
    }
    DumpNode {
        id: take_id(next),
        kind: "MethodDecl",
        span: (d.span.lo, d.span.hi),
        fields,
    }
}

fn param_dump(a: &Ast, r: ParamRef, next: &mut u32) -> DumpNode {
    match r {
        ParamRef::Param(h) => {
            let d: &ParamNode = h.get(a);
            let mut fields = Vec::new();
            if d.is_mut {
                fields.push(field("mut", DumpVal::Flag(true)));
            }
            fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
            if let Some(t) = d.ty {
                fields.push(field("ty", DumpVal::Node(Box::new(ty_dump(a, t, next)))));
            }
            DumpNode {
                id: take_id(next),
                kind: "Param",
                span: (d.span.lo, d.span.hi),
                fields,
            }
        }
        ParamRef::SelfParam(h) => {
            let d: &SelfParamNode = h.get(a);
            let mut fields = Vec::new();
            if d.is_mut {
                fields.push(field("mut", DumpVal::Flag(true)));
            }
            DumpNode {
                id: take_id(next),
                kind: "SelfParam",
                span: (d.span.lo, d.span.hi),
                fields,
            }
        }
        ParamRef::Error(h) => error_dump(a, h, next),
    }
}

fn block_dump(a: &Ast, h: Handle<BlockNode>, next: &mut u32) -> DumpNode {
    let d = h.get(a);
    let mut fields = Vec::new();
    fields.push(field(
        "stmts",
        DumpVal::Nodes(d.stmts.iter().map(|s| stmt_dump(a, *s, next)).collect()),
    ));
    if let Some(t) = d.tail {
        fields.push(field(
            "tail",
            DumpVal::Node(Box::new(expr_dump(a, t, next))),
        ));
    }
    DumpNode {
        id: take_id(next),
        kind: "Block",
        span: (d.span.lo, d.span.hi),
        fields,
    }
}

fn stmt_dump(a: &Ast, r: StmtRef, next: &mut u32) -> DumpNode {
    let id = take_id(next);
    let span = (r.span(a).lo, r.span(a).hi);
    let mut fields: Vec<DumpField> = Vec::new();
    let kind: &'static str = match r {
        StmtRef::Let(h) => {
            let d: &LetStmtNode = h.get(a);
            if d.is_mut {
                fields.push(field("mut", DumpVal::Flag(true)));
            }
            fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
            if let Some(names) = &d.destructure {
                fields.push(field(
                    "destructure",
                    DumpVal::Str(
                        names
                            .iter()
                            .map(|n| a.name(*n).to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ));
            }
            if let Some(t) = d.ty {
                fields.push(field("ty", DumpVal::Node(Box::new(ty_dump(a, t, next)))));
            }
            fields.push(field(
                "init",
                DumpVal::Node(Box::new(expr_dump(a, d.init, next))),
            ));
            "LetStmt"
        }
        StmtRef::If(h) => {
            let d: &IfNode = h.get(a);
            fields.push(field(
                "cond",
                DumpVal::Node(Box::new(expr_dump(a, d.cond, next))),
            ));
            fields.push(field(
                "then",
                DumpVal::Node(Box::new(block_dump(a, d.then, next))),
            ));
            if let Some(e) = d.els {
                fields.push(field("els", DumpVal::Node(Box::new(else_dump(a, e, next)))));
            }
            "If"
        }
        StmtRef::While(h) => {
            let d: &WhileNode = h.get(a);
            fields.push(field(
                "cond",
                DumpVal::Node(Box::new(expr_dump(a, d.cond, next))),
            ));
            fields.push(field(
                "body",
                DumpVal::Node(Box::new(block_dump(a, d.body, next))),
            ));
            "While"
        }
        StmtRef::ForOf(h) => {
            let d: &ForOfNode = h.get(a);
            if d.is_mut {
                fields.push(field("mut", DumpVal::Flag(true)));
            }
            fields.push(field("var", DumpVal::Str(a.name(d.var).to_string())));
            fields.push(field(
                "iter",
                DumpVal::Node(Box::new(expr_dump(a, d.iter, next))),
            ));
            fields.push(field(
                "body",
                DumpVal::Node(Box::new(block_dump(a, d.body, next))),
            ));
            "ForOf"
        }
        StmtRef::ForC(h) => {
            let d: &ForCNode = h.get(a);
            if d.is_mut {
                fields.push(field("mut", DumpVal::Flag(true)));
            }
            fields.push(field("var", DumpVal::Str(a.name(d.var).to_string())));
            fields.push(field(
                "init",
                DumpVal::Node(Box::new(expr_dump(a, d.init, next))),
            ));
            fields.push(field(
                "cond",
                DumpVal::Node(Box::new(expr_dump(a, d.cond, next))),
            ));
            fields.push(field(
                "update",
                DumpVal::Node(Box::new(expr_dump(a, d.update, next))),
            ));
            fields.push(field(
                "body",
                DumpVal::Node(Box::new(block_dump(a, d.body, next))),
            ));
            "ForC"
        }
        StmtRef::Return(h) => {
            let d: &ReturnNode = h.get(a);
            if let Some(v) = d.value {
                fields.push(field(
                    "value",
                    DumpVal::Node(Box::new(expr_dump(a, v, next))),
                ));
            }
            "Return"
        }
        StmtRef::Break(_) => "Break",
        StmtRef::Continue(_) => "Continue",
        StmtRef::When(h) => {
            let d: &WhenStmtNode = h.get(a);
            fields.push(field(
                "scrut",
                DumpVal::Node(Box::new(expr_dump(a, d.scrut, next))),
            ));
            fields.push(field(
                "arms",
                DumpVal::Nodes(d.arms.iter().map(|&x| arm_dump(a, x, next)).collect()),
            ));
            "WhenStmt"
        }
        StmtRef::Expr(e) => {
            fields.push(field(
                "expr",
                DumpVal::Node(Box::new(expr_dump(a, e, next))),
            ));
            "ExprStmt"
        }
        StmtRef::Error(h) => return error_dump(a, h, next),
    };
    DumpNode {
        id,
        kind,
        span,
        fields,
    }
}

fn else_dump(a: &Ast, e: ElseBranch, next: &mut u32) -> DumpNode {
    match e {
        ElseBranch::If(h) => stmt_dump(a, StmtRef::If(h), next),
        ElseBranch::Block(h) => block_dump(a, h, next),
    }
}

fn arm_dump(a: &Ast, h: Handle<WhenArmNode>, next: &mut u32) -> DumpNode {
    let d = h.get(a);
    let fields = vec![
        field(
            "pats",
            DumpVal::Nodes(d.pats.iter().map(|p| pat_dump(a, *p, next)).collect()),
        ),
        field("body", DumpVal::Node(Box::new(expr_dump(a, d.body, next)))),
    ];
    DumpNode {
        id: take_id(next),
        kind: "WhenArm",
        span: (d.span.lo, d.span.hi),
        fields,
    }
}

fn pat_dump(a: &Ast, r: PatRef, next: &mut u32) -> DumpNode {
    let id = take_id(next);
    let span = (r.span(a).lo, r.span(a).hi);
    let mut fields: Vec<DumpField> = Vec::new();
    let kind: &'static str = match r {
        PatRef::Lit(h) => {
            let d: &PatLitNode = h.get(a);
            fields.push(field(
                "expr",
                DumpVal::Node(Box::new(expr_dump(a, d.expr, next))),
            ));
            "PatLit"
        }
        PatRef::Path(h) => {
            let d: &PatPathNode = h.get(a);
            fields.push(field("segs", DumpVal::Segs(segs_dump(a, &d.segs, next))));
            "PatPath"
        }
        PatRef::Ctor(h) => {
            let d: &PatCtorNode = h.get(a);
            fields.push(field("segs", DumpVal::Segs(segs_dump(a, &d.segs, next))));
            fields.push(field(
                "args",
                DumpVal::PatArgs(
                    d.args
                        .iter()
                        .map(|x| x.map(|n| a.name(n).to_string()))
                        .collect(),
                ),
            ));
            "PatCtor"
        }
        PatRef::Wild(_) => "PatWild",
        PatRef::Else(_) => "PatElse",
        PatRef::Error(h) => return error_dump(a, h, next),
    };
    DumpNode {
        id,
        kind,
        span,
        fields,
    }
}

fn ty_dump(a: &Ast, r: TyRef, next: &mut u32) -> DumpNode {
    let id = take_id(next);
    let span = (r.span(a).lo, r.span(a).hi);
    let mut fields: Vec<DumpField> = Vec::new();
    let kind: &'static str = match r {
        TyRef::Path(h) => {
            let d: &TyPathNode = h.get(a);
            fields.push(field("segs", DumpVal::Segs(segs_dump(a, &d.segs, next))));
            "TyPath"
        }
        TyRef::Fn(h) => {
            let d: &TyFnNode = h.get(a);
            fields.push(field(
                "params",
                DumpVal::Nodes(d.params.iter().map(|p| ty_dump(a, *p, next)).collect()),
            ));
            fields.push(field(
                "ret",
                DumpVal::Node(Box::new(ty_dump(a, d.ret, next))),
            ));
            "TyFn"
        }
        TyRef::Opt(h) => {
            let d: &TyOptNode = h.get(a);
            fields.push(field(
                "inner",
                DumpVal::Node(Box::new(ty_dump(a, d.inner, next))),
            ));
            "TyOpt"
        }
        TyRef::Array(h) => {
            let d: &TyArrayNode = h.get(a);
            fields.push(field(
                "elem",
                DumpVal::Node(Box::new(ty_dump(a, d.elem, next))),
            ));
            "TyArray"
        }
        TyRef::Tuple(h) => {
            let d: &TyTupleNode = h.get(a);
            fields.push(field(
                "elems",
                DumpVal::Nodes(d.elems.iter().map(|e| ty_dump(a, *e, next)).collect()),
            ));
            "TyTuple"
        }
        TyRef::Const(h) => {
            let d: &TyConstNode = h.get(a);
            fields.push(field(
                "expr",
                DumpVal::Node(Box::new(expr_dump(a, d.expr, next))),
            ));
            "TyConst"
        }
        TyRef::Union(h) => {
            let d: &TyUnionNode = h.get(a);
            fields.push(field(
                "elems",
                DumpVal::Nodes(d.elems.iter().map(|e| ty_dump(a, *e, next)).collect()),
            ));
            "TyUnion"
        }
        TyRef::Error(h) => return error_dump(a, h, next),
    };
    DumpNode {
        id,
        kind,
        span,
        fields,
    }
}

fn expr_dump(a: &Ast, r: ExprRef, next: &mut u32) -> DumpNode {
    let id = take_id(next);
    let span = (r.span(a).lo, r.span(a).hi);
    let mut fields: Vec<DumpField> = Vec::new();
    let kind: &'static str = match r {
        ExprRef::Block(h) => return block_dump(a, h, next),
        ExprRef::AsyncBlock(h) => {
            let d: &AsyncBlockNode = h.get(a);
            fields.push(field(
                "body",
                DumpVal::Node(Box::new(block_dump(a, d.body, next))),
            ));
            "AsyncBlock"
        }
        ExprRef::Lit(h) => {
            let _d: &LitNode = h.get(a);
            "Lit" // value derived: src[lo..hi]
        }
        ExprRef::Path(h) => {
            let d: &crate::expr::PathNode = h.get(a);
            fields.push(field("segs", DumpVal::Segs(segs_dump(a, &d.segs, next))));
            "Path"
        }
        ExprRef::Call(h) => {
            let d: &CallNode = h.get(a);
            fields.push(field(
                "callee",
                DumpVal::Node(Box::new(expr_dump(a, d.callee, next))),
            ));
            fields.push(field(
                "args",
                DumpVal::Nodes(d.args.iter().map(|x| expr_dump(a, *x, next)).collect()),
            ));
            "Call"
        }
        ExprRef::Method(h) => {
            let d: &MethodNode = h.get(a);
            fields.push(field(
                "recv",
                DumpVal::Node(Box::new(expr_dump(a, d.recv, next))),
            ));
            fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
            if !d.generics.is_empty() {
                fields.push(field(
                    "generics",
                    DumpVal::Nodes(d.generics.iter().map(|g| ty_dump(a, *g, next)).collect()),
                ));
            }
            fields.push(field(
                "args",
                DumpVal::Nodes(d.args.iter().map(|x| expr_dump(a, *x, next)).collect()),
            ));
            "Method"
        }
        ExprRef::Field(h) => {
            let d: &FieldNode = h.get(a);
            fields.push(field(
                "recv",
                DumpVal::Node(Box::new(expr_dump(a, d.recv, next))),
            ));
            fields.push(field("name", DumpVal::Str(a.name(d.name).to_string())));
            "Field"
        }
        ExprRef::Index(h) => {
            let d: &IndexNode = h.get(a);
            fields.push(field(
                "recv",
                DumpVal::Node(Box::new(expr_dump(a, d.recv, next))),
            ));
            fields.push(field(
                "idx",
                DumpVal::Node(Box::new(expr_dump(a, d.idx, next))),
            ));
            "Index"
        }
        ExprRef::Unary(h) => {
            let d: &UnaryNode = h.get(a);
            fields.push(field("op", DumpVal::UnOp(d.op)));
            fields.push(field(
                "expr",
                DumpVal::Node(Box::new(expr_dump(a, d.expr, next))),
            ));
            "Unary"
        }
        ExprRef::Binary(h) => {
            let d: &BinaryNode = h.get(a);
            fields.push(field("op", DumpVal::Op(d.op)));
            fields.push(field(
                "lhs",
                DumpVal::Node(Box::new(expr_dump(a, d.lhs, next))),
            ));
            fields.push(field(
                "rhs",
                DumpVal::Node(Box::new(expr_dump(a, d.rhs, next))),
            ));
            "Binary"
        }
        ExprRef::Assign(h) => {
            let d: &AssignNode = h.get(a);
            if let Some(o) = d.op {
                fields.push(field("op", DumpVal::Op(o)));
            }
            fields.push(field(
                "target",
                DumpVal::Node(Box::new(expr_dump(a, d.target, next))),
            ));
            fields.push(field(
                "value",
                DumpVal::Node(Box::new(expr_dump(a, d.value, next))),
            ));
            "Assign"
        }
        ExprRef::Lambda(h) => {
            let d: &LambdaNode = h.get(a);
            fields.push(field(
                "params",
                DumpVal::Nodes(d.params.iter().map(|p| param_dump(a, *p, next)).collect()),
            ));
            if let Some(r) = d.ret {
                fields.push(field("ret", DumpVal::Node(Box::new(ty_dump(a, r, next)))));
            }
            fields.push(field(
                "body",
                DumpVal::Node(Box::new(expr_dump(a, d.body, next))),
            ));
            "Lambda"
        }
        ExprRef::Try(h) => {
            let d: &TryNode = h.get(a);
            fields.push(field(
                "expr",
                DumpVal::Node(Box::new(expr_dump(a, d.expr, next))),
            ));
            "Try"
        }
        ExprRef::FStr(h) => {
            let d: &FStrNode = h.get(a);
            fields.push(field(
                "parts",
                DumpVal::FParts(
                    d.parts
                        .iter()
                        .map(|p| match p {
                            FPartAst::Lit(s) => DumpFPart::Lit(s.clone()),
                            FPartAst::Hole(e) => DumpFPart::Hole(expr_dump(a, *e, next)),
                        })
                        .collect(),
                ),
            ));
            "FStr"
        }
        ExprRef::Struct(h) => {
            let d: &StructLitNode = h.get(a);
            fields.push(field("ty", DumpVal::Node(Box::new(ty_dump(a, d.ty, next)))));
            fields.push(field(
                "fields",
                DumpVal::StructFields(
                    d.fields
                        .iter()
                        .map(|(n, v)| DumpStructField {
                            ident: a.name(*n).to_string(),
                            node: expr_dump(a, *v, next),
                        })
                        .collect(),
                ),
            ));
            "Struct"
        }
        ExprRef::Tuple(h) => {
            let d: &TupleNode = h.get(a);
            fields.push(field(
                "elems",
                DumpVal::Nodes(d.elems.iter().map(|e| expr_dump(a, *e, next)).collect()),
            ));
            "Tuple"
        }
        ExprRef::ArrayLit(h) => {
            let d: &ArrayLitNode = h.get(a);
            fields.push(field(
                "elems",
                DumpVal::Nodes(d.elems.iter().map(|e| expr_dump(a, *e, next)).collect()),
            ));
            "ArrayLit"
        }
        ExprRef::ArrayRepeat(h) => {
            let d: &ArrayRepeatNode = h.get(a);
            fields.push(field(
                "value",
                DumpVal::Node(Box::new(expr_dump(a, d.value, next))),
            ));
            fields.push(field(
                "count",
                DumpVal::Node(Box::new(expr_dump(a, d.count, next))),
            ));
            "ArrayRepeat"
        }
        ExprRef::WhenExpr(h) => {
            let d: &WhenExprNode = h.get(a);
            fields.push(field(
                "scrut",
                DumpVal::Node(Box::new(expr_dump(a, d.scrut, next))),
            ));
            fields.push(field(
                "arms",
                DumpVal::Nodes(d.arms.iter().map(|&x| arm_dump(a, x, next)).collect()),
            ));
            "WhenExpr"
        }
        ExprRef::If(h) => {
            let d: &IfExprNode = h.get(a);
            fields.push(field(
                "cond",
                DumpVal::Node(Box::new(expr_dump(a, d.cond, next))),
            ));
            fields.push(field(
                "then",
                DumpVal::Node(Box::new(block_dump(a, d.then, next))),
            ));
            if let Some(e) = d.els {
                fields.push(field("els", DumpVal::Node(Box::new(else_dump(a, e, next)))));
            }
            "If"
        }
        ExprRef::Await(h) => {
            let d: &AwaitNode = h.get(a);
            fields.push(field(
                "expr",
                DumpVal::Node(Box::new(expr_dump(a, d.expr, next))),
            ));
            "Await"
        }
        ExprRef::Is(h) => {
            let d: &IsNode = h.get(a);
            fields.push(field(
                "expr",
                DumpVal::Node(Box::new(expr_dump(a, d.expr, next))),
            ));
            fields.push(field("ty", DumpVal::Node(Box::new(ty_dump(a, d.ty, next)))));
            "Is"
        }
        ExprRef::Cast(h) => {
            let d: &CastNode = h.get(a);
            fields.push(field(
                "expr",
                DumpVal::Node(Box::new(expr_dump(a, d.expr, next))),
            ));
            fields.push(field("ty", DumpVal::Node(Box::new(ty_dump(a, d.ty, next)))));
            "Cast"
        }
        ExprRef::Error(h) => return error_dump(a, h, next),
    };
    DumpNode {
        id,
        kind,
        span,
        fields,
    }
}

/// Build the dump tree from a parsed module.
pub fn to_dump_tree(ast: &Ast) -> DumpNode {
    let mut next = 0u32;
    item_dump(ast, ItemRef::Module(ast.root()), &mut next)
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
        Vis::Super => "super",
        Vis::Pkg => "pkg",
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
        DumpVal::Bounds(ws) => {
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
        Vis::Super => "pub(super)",
        Vis::Pkg => "pub(pkg)",
        // the unannotated default — the `pub(self)` spelling is gone
        Vis::Self_ => "private",
    }
}

fn binop_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::Ne => "!=",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::Le => "<=",
        BinOp::Ge => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
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
    let name = n
        .fields
        .iter()
        .find(|f| f.label == "name")
        .and_then(|f| match &f.val {
            DumpVal::Str(s) => Some(s.clone()),
            _ => None,
        });
    let name = name.unwrap_or_else(|| {
        if n.kind == "Lit" {
            src.get(n.span.0 as usize..n.span.1 as usize)
                .unwrap_or("")
                .to_string()
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
        val_text(f.label, &f.val, d + 1, src, out);
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
                out.push_str(&format!("{}- {}\n", ind(d + 1), m.ident));
            }
        }
        DumpVal::StructFields(fs) => {
            out.push_str(&format!("{i}{label}:\n"));
            for f in fs {
                out.push_str(&format!("{}- {}:\n", ind(d + 1), f.ident));
                node_text(&f.node, d + 2, src, out);
            }
        }
        DumpVal::Bounds(ws) => {
            out.push_str(&format!("{i}{label}:\n"));
            for w in ws {
                out.push_str(&format!("{}- {} requires\n", ind(d + 1), w.ident));
                node_text(&w.ty, d + 2, src, out);
            }
        }
        DumpVal::Segs(segs) => {
            out.push_str(&format!("{i}{label}:\n"));
            for s in segs {
                if s.generics.is_empty() {
                    out.push_str(&format!("{}- {}\n", ind(d + 1), s.name));
                } else {
                    out.push_str(&format!(
                        "{}- {}<{} generics>\n",
                        ind(d + 1),
                        s.name,
                        s.generics.len()
                    ));
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
