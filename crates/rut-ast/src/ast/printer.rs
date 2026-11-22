//! AstPrinter — the tree renderer (was `dump`): one mapping, two
//! renderers. [`AstPrinter::tree`] maps the whole tree from the root,
//! [`AstPrinter::subtree`] maps any node (an `AnyHandle` window —
//! `node_at` debugging, per-window ≡ comparisons); [`AstPrinter::json`]
//! emits the flattened tagged objects the demo's tree UI consumes (no
//! display strings on the wire — the demo derives every label from
//! `source` + small tables); [`AstPrinter::text`] renders the same
//! tree for the CLI `rut dump` in the labeled `label: value` /
//! `- item` style.
//!
//! The mapping dispatches the SLOT enums (`ItemRef`, `StmtRef`, …);
//! node ids are assigned in DFS pre-order (unique across the per-kind
//! arenas). `Error` slots print as `Error` rows carrying their diag
//! messages — the positional recovery view. The kind labels read the
//! `arenas!` macro's single `NodeKind::NAME` table — a new kind cannot
//! lack a printable name. An `AnyHandle` addressing a dead slot prints
//! the explicit `dead` leaf row — never a panic, never a wrong-arena
//! read. The WIRE IS FROZEN: the emitted JSON and text bytes are the
//! demo's and the corpus oracle's contract.

use crate::ast::error::ErrorNode;
use crate::ast::expr::ExprRef;
use crate::ast::handle::{AnyHandle, Handle, NodeKind};
use crate::ast::item::{
    AliasNode, BuiltinImplNode, BuiltinPrimitiveNode, BuiltinTyNode, EnumMemberNode, ImplNode,
    ModDeclNode, SurfaceFnNode, SurfaceStructNode, UseNode,
};
use crate::ast::item::{
    ClassNode, EnumNode, FieldDeclNode, FieldRef, FnNode, InterfaceNode, ItemRef, MemberRef,
    MethodDeclNode, MethodRef, ModuleLetNode, ModuleNode, ParamNode, ParamRef, SelfParamNode,
    StructNode, Vis,
};
use crate::ast::pat::{PatElseNode, PatRef, PatWildNode};
use crate::ast::stmt::{
    BreakNode, ContinueNode, ElseBranch, ForCNode, ForOfNode, IfNode, LetStmtNode, ReturnNode,
    StmtRef, WhenStmtNode, WhileNode,
};
use crate::ast::store::span_of_any;
use crate::ast::ty::TyRef;
use crate::{
    ArrayLitNode, ArrayRepeatNode, AssignNode, Ast, AsyncBlockNode, AwaitNode, BinOp, BinaryNode,
    BlockNode, CallNode, CastNode, FPartAst, FStrNode, FieldNode, IfExprNode, IndexNode, IsNode,
    LambdaNode, LitNode, MethodNode, PatLitNode, PatPathNode, PathNode, PathSeg, StructLitNode,
    TupleNode, TyArrayNode, TyFnNode, TyOptNode, TyPathNode, TyTupleNode, TyUnionNode, UnOp,
    UnaryNode, WhenArmNode, WhenExprNode,
};
use rut_core::IdentId;

// ---- the print tree ----

pub struct PrintNode {
    id: u32,
    kind: &'static str,
    span: (u32, u32),
    fields: Vec<PrintField>,
}
struct PrintField {
    pub label: &'static str,
    pub val: PrintVal,
}

/// Concrete companions — the Rust mirror of the demo's `d.ts` companion
/// interfaces. The JSON writer is a dumb serializer over these.
struct PrintSeg {
    pub name: String,
    /// the segment ident's recorded span
    pub span: (u32, u32),
    pub generics: Vec<PrintNode>,
}
struct PrintMember {
    pub ident: String,
}
struct PrintStructField {
    pub ident: String,
    /// the field name's recorded span
    pub span: (u32, u32),
    pub node: PrintNode,
}
struct PrintBound {
    pub ident: String,
    pub ty: PrintNode,
}
enum PrintFPart {
    Lit(String),
    Hole(PrintNode),
}

enum PrintVal {
    Node(Box<PrintNode>),
    Nodes(Vec<PrintNode>),
    Idents(Vec<String>),
    Members(Vec<PrintMember>),
    StructFields(Vec<PrintStructField>),
    Bounds(Vec<PrintBound>),
    Segs(Vec<PrintSeg>),
    FParts(Vec<PrintFPart>),
    /// stored strings: interned names
    Str(String),
    /// a raw span pair (`body` — the brace region): `[lo,hi]`, no
    /// fabrication
    Span(u32, u32),
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

fn segs_dump(a: &Ast, segs: &[PathSeg], next: &mut u32) -> Vec<PrintSeg> {
    segs.iter()
        .map(|s| PrintSeg {
            name: a.name(s.name.ident).to_string(),
            span: (s.name.span.lo, s.name.span.hi),
            generics: s.generics.iter().map(|&g| ty_dump(a, g, next)).collect(),
        })
        .collect()
}

fn field(label: &'static str, val: PrintVal) -> PrintField {
    PrintField { label, val }
}

fn bounds_dump(a: &Ast, bounds: &[(IdentId, TyRef)], next: &mut u32) -> PrintVal {
    PrintVal::Bounds(
        bounds
            .iter()
            .map(|(n, t)| PrintBound {
                ident: a.name(*n).to_string(),
                ty: ty_dump(a, *t, next),
            })
            .collect(),
    )
}

fn error_dump(a: &Ast, h: Handle<ErrorNode>, next: &mut u32) -> PrintNode {
    let e = h.get(a);
    let mut fields = Vec::new();
    for d in &e.diags {
        fields.push(field("diag", PrintVal::Str(d.msg.clone())));
    }
    PrintNode {
        id: take_id(next),
        kind: <ErrorNode as NodeKind>::NAME,
        span: (e.span.lo, e.span.hi),
        fields,
    }
}

fn take_id(next: &mut u32) -> u32 {
    let id = *next;
    *next += 1;
    id
}

fn item_dump(a: &Ast, r: ItemRef, next: &mut u32) -> PrintNode {
    let id = take_id(next);
    let span = (r.span(a).lo, r.span(a).hi);
    let mut fields: Vec<PrintField> = Vec::new();
    let kind: &'static str = match r {
        ItemRef::Module(h) => {
            module_fields(a, h.get(a), &mut fields, next);
            <ModuleNode as NodeKind>::NAME
        }
        ItemRef::ModDecl(h) => {
            let d = h.get(a);
            fields.push(field("vis", PrintVal::Vis(d.vis())));
            fields.push(field(
                "name",
                PrintVal::Str(a.name(d.name.ident).to_string()),
            ));
            <ModDeclNode as NodeKind>::NAME
        }
        ItemRef::Use(h) => {
            let d = h.get(a);
            fields.push(field("vis", PrintVal::Vis(d.vis())));
            fields.push(field(
                "path",
                PrintVal::Idents(d.path.iter().map(|n| a.name(n.ident).to_string()).collect()),
            ));
            fields.push(field(
                "names",
                PrintVal::Idents(
                    d.names
                        .iter()
                        .map(|n| a.name(n.ident).to_string())
                        .collect(),
                ),
            ));
            <UseNode as NodeKind>::NAME
        }
        ItemRef::Alias(h) => {
            let d: &AliasNode = h.get(a);
            fields.push(field("vis", PrintVal::Vis(d.vis())));
            fields.push(field(
                "name",
                PrintVal::Str(a.name(d.name.ident).to_string()),
            ));
            fields.push(field(
                "target",
                PrintVal::Node(Box::new(ty_dump(a, d.target, next))),
            ));
            <AliasNode as NodeKind>::NAME
        }
        ItemRef::ModuleLet(h) => {
            let d: &ModuleLetNode = h.get(a);
            fields.push(field("vis", PrintVal::Vis(d.vis())));
            fields.push(field(
                "name",
                PrintVal::Str(a.name(d.name.ident).to_string()),
            ));
            if let Some(t) = d.ty {
                fields.push(field("ty", PrintVal::Node(Box::new(ty_dump(a, t, next)))));
            }
            fields.push(field(
                "init",
                PrintVal::Node(Box::new(expr_dump(a, d.init, next))),
            ));
            <ModuleLetNode as NodeKind>::NAME
        }
        ItemRef::Enum(h) => {
            let d: &EnumNode = h.get(a);
            fields.push(field("vis", PrintVal::Vis(d.vis())));
            fields.push(field(
                "name",
                PrintVal::Str(a.name(d.name.ident).to_string()),
            ));
            fields.push(field(
                "body",
                PrintVal::Span(d.body_span.lo, d.body_span.hi),
            ));
            fields.push(field(
                "members",
                PrintVal::Members(
                    d.members
                        .iter()
                        .map(|m| match m {
                            MemberRef::Member(mh) => PrintMember {
                                ident: a.name(mh.get(a).name.ident).to_string(),
                            },
                            MemberRef::Error(_) => PrintMember {
                                ident: "<error>".to_string(),
                            },
                        })
                        .collect(),
                ),
            ));
            <EnumNode as NodeKind>::NAME
        }
        ItemRef::Struct(h) => {
            let d: &StructNode = h.get(a);
            decl_fields(
                a,
                &mut fields,
                (d.body_span.lo, d.body_span.hi),
                d.vis(),
                d.name.ident,
                &d.generics,
                &d.fields,
                &d.methods,
                next,
            );
            <StructNode as NodeKind>::NAME
        }
        ItemRef::Class(h) => {
            let d: &ClassNode = h.get(a);
            decl_fields(
                a,
                &mut fields,
                (d.body_span.lo, d.body_span.hi),
                d.vis(),
                d.name.ident,
                &d.generics,
                &d.fields,
                &d.methods,
                next,
            );
            if d.newtype {
                fields.push(field("newtype", PrintVal::Flag(true)));
            }
            if !d.requires.is_empty() {
                fields.push(field("bounds", bounds_dump(a, &d.requires, next)));
            }
            <ClassNode as NodeKind>::NAME
        }
        ItemRef::Interface(h) => {
            let d: &InterfaceNode = h.get(a);
            fields.push(field("vis", PrintVal::Vis(d.vis())));
            fields.push(field(
                "name",
                PrintVal::Str(a.name(d.name.ident).to_string()),
            ));
            fields.push(field(
                "body",
                PrintVal::Span(d.body_span.lo, d.body_span.hi),
            ));
            if !d.generics.is_empty() {
                fields.push(field(
                    "generics",
                    PrintVal::Idents(d.generics.iter().map(|&g| a.name(g).to_string()).collect()),
                ));
            }
            fields.push(field(
                "methods",
                PrintVal::Nodes(d.methods.iter().map(|m| method_dump(a, *m, next)).collect()),
            ));
            <InterfaceNode as NodeKind>::NAME
        }
        ItemRef::Impl(h) => {
            let d: &ImplNode = h.get(a);
            if !d.generics.is_empty() {
                fields.push(field(
                    "generics",
                    PrintVal::Idents(d.generics.iter().map(|&g| a.name(g).to_string()).collect()),
                ));
            }
            fields.push(field(
                "target",
                PrintVal::Node(Box::new(ty_dump(a, d.target, next))),
            ));
            fields.push(field(
                "methods",
                PrintVal::Nodes(d.methods.iter().map(|m| method_dump(a, *m, next)).collect()),
            ));
            <ImplNode as NodeKind>::NAME
        }
        ItemRef::Fn(h) => {
            let f: &FnNode = h.get(a);
            fields.push(field("vis", PrintVal::Vis(f.vis())));
            if f.is_async() {
                fields.push(field("async", PrintVal::Flag(true)));
            }
            fields.push(field(
                "name",
                PrintVal::Str(a.name(f.name.ident).to_string()),
            ));
            if !f.generics.is_empty() {
                fields.push(field(
                    "generics",
                    PrintVal::Idents(f.generics.iter().map(|&g| a.name(g).to_string()).collect()),
                ));
            }
            fields.push(field(
                "params",
                PrintVal::Nodes(f.params.iter().map(|p| param_dump(a, *p, next)).collect()),
            ));
            if let Some(r) = f.ret {
                fields.push(field("ret", PrintVal::Node(Box::new(ty_dump(a, r, next)))));
            }
            if !f.bounds.is_empty() {
                fields.push(field("bounds", bounds_dump(a, &f.bounds, next)));
            }
            fields.push(field(
                "body",
                PrintVal::Node(Box::new(block_dump(a, f.body, next))),
            ));
            <FnNode as NodeKind>::NAME
        }
        ItemRef::SurfaceFn(h) => {
            let d: &SurfaceFnNode = h.get(a);
            fields.push(field("vis", PrintVal::Vis(d.vis())));
            fields.push(field(
                "linkage",
                PrintVal::Str(
                    match d.linkage() {
                        crate::ast::item::Linkage::Host => "host",
                        crate::ast::item::Linkage::Builtin { .. } => "builtin",
                    }
                    .to_string(),
                ),
            ));
            if d.is_async() {
                fields.push(field("is_async", PrintVal::Flag(true)));
            }
            fields.push(field(
                "name",
                PrintVal::Str(a.name(d.name.ident).to_string()),
            ));
            if !d.generics.is_empty() {
                fields.push(field(
                    "generics",
                    PrintVal::Idents(d.generics.iter().map(|&g| a.name(g).to_string()).collect()),
                ));
            }
            fields.push(field(
                "params",
                PrintVal::Nodes(d.params.iter().map(|p| param_dump(a, *p, next)).collect()),
            ));
            if let Some(r) = d.ret {
                fields.push(field("ret", PrintVal::Node(Box::new(ty_dump(a, r, next)))));
            }
            <SurfaceFnNode as NodeKind>::NAME
        }
        ItemRef::SurfaceStruct(h) => {
            let d: &SurfaceStructNode = h.get(a);
            fields.push(field("vis", PrintVal::Vis(d.vis())));
            fields.push(field(
                "name",
                PrintVal::Str(a.name(d.name.ident).to_string()),
            ));
            fields.push(field(
                "body",
                PrintVal::Span(d.body_span.lo, d.body_span.hi),
            ));
            fields.push(field(
                "fields",
                PrintVal::Nodes(d.fields.iter().map(|f| field_dump(a, *f, next)).collect()),
            ));
            <SurfaceStructNode as NodeKind>::NAME
        }
        ItemRef::BuiltinTy(h) => {
            let d: &BuiltinTyNode = h.get(a);
            fields.push(field("vis", PrintVal::Vis(d.vis())));
            fields.push(field(
                "linkage",
                PrintVal::Str(
                    if d.ambient() {
                        "prelude builtin"
                    } else {
                        "pub builtin"
                    }
                    .to_string(),
                ),
            ));
            fields.push(field(
                "name",
                PrintVal::Str(a.name(d.name.ident).to_string()),
            ));
            fields.push(field(
                "body",
                PrintVal::Span(d.body_span.lo, d.body_span.hi),
            ));
            if !d.generics.is_empty() {
                fields.push(field(
                    "generics",
                    PrintVal::Idents(d.generics.iter().map(|&g| a.name(g).to_string()).collect()),
                ));
            }
            fields.push(field(
                "members",
                PrintVal::Nodes(d.members.iter().map(|m| method_dump(a, *m, next)).collect()),
            ));
            <BuiltinTyNode as NodeKind>::NAME
        }
        ItemRef::BuiltinPrimitive(h) => {
            let d: &BuiltinPrimitiveNode = h.get(a);
            fields.push(field(
                "linkage",
                PrintVal::Str(
                    if d.ambient() {
                        "prelude builtin"
                    } else {
                        "pub builtin"
                    }
                    .to_string(),
                ),
            ));
            fields.push(field(
                "name",
                PrintVal::Str(a.name(d.name.ident).to_string()),
            ));
            fields.push(field(
                "body",
                PrintVal::Span(d.body_span.lo, d.body_span.hi),
            ));
            fields.push(field(
                "members",
                PrintVal::Nodes(d.members.iter().map(|m| method_dump(a, *m, next)).collect()),
            ));
            <BuiltinPrimitiveNode as NodeKind>::NAME
        }
        ItemRef::BuiltinImpl(h) => {
            let d: &BuiltinImplNode = h.get(a);
            fields.push(field(
                "prim",
                PrintVal::Str(a.name(d.prim.ident).to_string()),
            ));
            fields.push(field(
                "methods",
                PrintVal::Nodes(d.methods.iter().map(|m| method_dump(a, *m, next)).collect()),
            ));
            <BuiltinImplNode as NodeKind>::NAME
        }
        ItemRef::Error(h) => return error_dump(a, h, next),
    };
    PrintNode {
        id,
        kind,
        span,
        fields,
    }
}

fn module_fields(a: &Ast, d: &ModuleNode, fields: &mut Vec<PrintField>, next: &mut u32) {
    fields.push(field(
        "items",
        PrintVal::Nodes(d.items.iter().map(|&i| item_dump(a, i, next)).collect()),
    ));
}

#[allow(clippy::too_many_arguments)]
fn decl_fields(
    a: &Ast,
    fields: &mut Vec<PrintField>,
    body: (u32, u32),
    vis: Vis,
    name: IdentId,
    generics: &[IdentId],
    fs: &[FieldRef],
    methods: &[MethodRef],
    next: &mut u32,
) {
    fields.push(field("vis", PrintVal::Vis(vis)));
    fields.push(field("name", PrintVal::Str(a.name(name).to_string())));
    fields.push(field("body", PrintVal::Span(body.0, body.1)));
    if !generics.is_empty() {
        fields.push(field(
            "generics",
            PrintVal::Idents(generics.iter().map(|&g| a.name(g).to_string()).collect()),
        ));
    }
    fields.push(field(
        "fields",
        PrintVal::Nodes(fs.iter().map(|f| field_dump(a, *f, next)).collect()),
    ));
    fields.push(field(
        "methods",
        PrintVal::Nodes(methods.iter().map(|m| method_dump(a, *m, next)).collect()),
    ));
}

fn field_dump(a: &Ast, r: FieldRef, next: &mut u32) -> PrintNode {
    match r {
        FieldRef::Field(h) => field_decl_dump(a, h, next),
        FieldRef::Error(h) => error_dump(a, h, next),
    }
}

fn field_decl_dump(a: &Ast, h: Handle<FieldDeclNode>, next: &mut u32) -> PrintNode {
    let d = h.get(a);
    let mut fields = Vec::new();
    if let Some(v) = d.vis() {
        fields.push(field("vis", PrintVal::OptVis(Some(v))));
    }
    if d.is_static() {
        fields.push(field("static", PrintVal::Flag(true)));
    }
    fields.push(field(
        "name",
        PrintVal::Str(a.name(d.name.ident).to_string()),
    ));
    fields.push(field(
        "ty",
        PrintVal::Node(Box::new(ty_dump(a, d.ty, next))),
    ));
    if let Some(i) = d.init {
        fields.push(field(
            "init",
            PrintVal::Node(Box::new(expr_dump(a, i, next))),
        ));
    }
    PrintNode {
        id: take_id(next),
        kind: <FieldDeclNode as NodeKind>::NAME,
        span: (d.span.lo, d.span.hi),
        fields,
    }
}

fn method_dump(a: &Ast, r: MethodRef, next: &mut u32) -> PrintNode {
    match r {
        MethodRef::Method(h) => method_decl_dump(a, h, next),
        MethodRef::Error(h) => error_dump(a, h, next),
    }
}

fn method_decl_dump(a: &Ast, h: Handle<MethodDeclNode>, next: &mut u32) -> PrintNode {
    let d = h.get(a);
    let mut fields = Vec::new();
    if let Some(v) = d.vis() {
        fields.push(field("vis", PrintVal::OptVis(Some(v))));
    }
    if d.is_async() {
        fields.push(field("async", PrintVal::Flag(true)));
    }
    if let Some(m) = d.marker() {
        fields.push(field("marker", PrintVal::Str(a.name(m).to_string())));
    }
    fields.push(field(
        "name",
        PrintVal::Str(a.name(d.name.ident).to_string()),
    ));
    if !d.generics.is_empty() {
        fields.push(field(
            "generics",
            PrintVal::Idents(d.generics.iter().map(|&g| a.name(g).to_string()).collect()),
        ));
    }
    fields.push(field(
        "params",
        PrintVal::Nodes(d.params.iter().map(|p| param_dump(a, *p, next)).collect()),
    ));
    if let Some(r) = d.ret {
        fields.push(field("ret", PrintVal::Node(Box::new(ty_dump(a, r, next)))));
    }
    if !d.bounds.is_empty() {
        fields.push(field("bounds", bounds_dump(a, &d.bounds, next)));
    }
    if let Some(b) = d.body {
        fields.push(field(
            "body",
            PrintVal::Node(Box::new(block_dump(a, b, next))),
        ));
    }
    PrintNode {
        id: take_id(next),
        kind: <MethodDeclNode as NodeKind>::NAME,
        span: (d.span.lo, d.span.hi),
        fields,
    }
}

fn param_dump(a: &Ast, r: ParamRef, next: &mut u32) -> PrintNode {
    match r {
        ParamRef::Param(h) => {
            let d: &ParamNode = h.get(a);
            let mut fields = Vec::new();
            fields.push(field(
                "name",
                PrintVal::Str(a.name(d.name.ident).to_string()),
            ));
            if let Some(t) = d.ty {
                fields.push(field("ty", PrintVal::Node(Box::new(ty_dump(a, t, next)))));
            }
            PrintNode {
                id: take_id(next),
                kind: <ParamNode as NodeKind>::NAME,
                span: (d.span.lo, d.span.hi),
                fields,
            }
        }
        ParamRef::SelfParam(h) => {
            let d: &SelfParamNode = h.get(a);
            let fields = Vec::new();
            PrintNode {
                id: take_id(next),
                kind: <SelfParamNode as NodeKind>::NAME,
                span: (d.span.lo, d.span.hi),
                fields,
            }
        }
        ParamRef::Error(h) => error_dump(a, h, next),
    }
}

fn block_dump(a: &Ast, h: Handle<BlockNode>, next: &mut u32) -> PrintNode {
    let d = h.get(a);
    let mut fields = Vec::new();
    fields.push(field(
        "stmts",
        PrintVal::Nodes(d.stmts.iter().map(|s| stmt_dump(a, *s, next)).collect()),
    ));
    if let Some(t) = d.tail {
        fields.push(field(
            "tail",
            PrintVal::Node(Box::new(expr_dump(a, t, next))),
        ));
    }
    PrintNode {
        id: take_id(next),
        kind: <BlockNode as NodeKind>::NAME,
        span: (d.span.lo, d.span.hi),
        fields,
    }
}

fn stmt_dump(a: &Ast, r: StmtRef, next: &mut u32) -> PrintNode {
    let id = take_id(next);
    let span = (r.span(a).lo, r.span(a).hi);
    let mut fields: Vec<PrintField> = Vec::new();
    let kind: &'static str = match r {
        StmtRef::Let(h) => {
            let d: &LetStmtNode = h.get(a);
            if d.is_const {
                fields.push(field("const", PrintVal::Flag(true)));
            }
            fields.push(field(
                "name",
                PrintVal::Str(a.name(d.name.ident).to_string()),
            ));
            if let Some(names) = &d.destructure {
                fields.push(field(
                    "destructure",
                    PrintVal::Str(
                        names
                            .iter()
                            .map(|n| a.name(n.ident).to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ));
            }
            if let Some(t) = d.ty {
                fields.push(field("ty", PrintVal::Node(Box::new(ty_dump(a, t, next)))));
            }
            fields.push(field(
                "init",
                PrintVal::Node(Box::new(expr_dump(a, d.init, next))),
            ));
            <LetStmtNode as NodeKind>::NAME
        }
        StmtRef::If(h) => {
            let d: &IfNode = h.get(a);
            fields.push(field(
                "cond",
                PrintVal::Node(Box::new(expr_dump(a, d.cond, next))),
            ));
            fields.push(field(
                "then",
                PrintVal::Node(Box::new(block_dump(a, d.then, next))),
            ));
            if let Some(e) = d.els {
                fields.push(field(
                    "els",
                    PrintVal::Node(Box::new(else_dump(a, e, next))),
                ));
            }
            <IfNode as NodeKind>::NAME
        }
        StmtRef::While(h) => {
            let d: &WhileNode = h.get(a);
            fields.push(field(
                "cond",
                PrintVal::Node(Box::new(expr_dump(a, d.cond, next))),
            ));
            fields.push(field(
                "body",
                PrintVal::Node(Box::new(block_dump(a, d.body, next))),
            ));
            <WhileNode as NodeKind>::NAME
        }
        StmtRef::ForOf(h) => {
            let d: &ForOfNode = h.get(a);
            if d.is_const {
                fields.push(field("const", PrintVal::Flag(true)));
            }
            fields.push(field("var", PrintVal::Str(a.name(d.var.ident).to_string())));
            fields.push(field(
                "iter",
                PrintVal::Node(Box::new(expr_dump(a, d.iter, next))),
            ));
            fields.push(field(
                "body",
                PrintVal::Node(Box::new(block_dump(a, d.body, next))),
            ));
            <ForOfNode as NodeKind>::NAME
        }
        StmtRef::ForC(h) => {
            let d: &ForCNode = h.get(a);
            fields.push(field("var", PrintVal::Str(a.name(d.var.ident).to_string())));
            fields.push(field(
                "init",
                PrintVal::Node(Box::new(expr_dump(a, d.init, next))),
            ));
            fields.push(field(
                "cond",
                PrintVal::Node(Box::new(expr_dump(a, d.cond, next))),
            ));
            fields.push(field(
                "update",
                PrintVal::Node(Box::new(expr_dump(a, d.update, next))),
            ));
            fields.push(field(
                "body",
                PrintVal::Node(Box::new(block_dump(a, d.body, next))),
            ));
            <ForCNode as NodeKind>::NAME
        }
        StmtRef::Return(h) => {
            let d: &ReturnNode = h.get(a);
            if let Some(v) = d.value {
                fields.push(field(
                    "value",
                    PrintVal::Node(Box::new(expr_dump(a, v, next))),
                ));
            }
            <ReturnNode as NodeKind>::NAME
        }
        StmtRef::Break(_) => <BreakNode as NodeKind>::NAME,
        StmtRef::Continue(_) => <ContinueNode as NodeKind>::NAME,
        StmtRef::When(h) => {
            let d: &WhenStmtNode = h.get(a);
            fields.push(field(
                "scrut",
                PrintVal::Node(Box::new(expr_dump(a, d.scrut, next))),
            ));
            fields.push(field(
                "arms",
                PrintVal::Nodes(d.arms.iter().map(|&x| arm_dump(a, x, next)).collect()),
            ));
            <WhenStmtNode as NodeKind>::NAME
        }
        StmtRef::Expr(e) => {
            fields.push(field(
                "expr",
                PrintVal::Node(Box::new(expr_dump(a, e, next))),
            ));
            "ExprStmt"
        }
        StmtRef::Error(h) => return error_dump(a, h, next),
    };
    PrintNode {
        id,
        kind,
        span,
        fields,
    }
}

fn else_dump(a: &Ast, e: ElseBranch, next: &mut u32) -> PrintNode {
    match e {
        ElseBranch::If(h) => stmt_dump(a, StmtRef::If(h), next),
        ElseBranch::Block(h) => block_dump(a, h, next),
    }
}

fn arm_dump(a: &Ast, h: Handle<WhenArmNode>, next: &mut u32) -> PrintNode {
    let d = h.get(a);
    let fields = vec![
        field(
            "pats",
            PrintVal::Nodes(d.pats.iter().map(|p| pat_dump(a, *p, next)).collect()),
        ),
        field("body", PrintVal::Node(Box::new(expr_dump(a, d.body, next)))),
    ];
    PrintNode {
        id: take_id(next),
        kind: <WhenArmNode as NodeKind>::NAME,
        span: (d.span.lo, d.span.hi),
        fields,
    }
}

fn pat_dump(a: &Ast, r: PatRef, next: &mut u32) -> PrintNode {
    let id = take_id(next);
    let span = (r.span(a).lo, r.span(a).hi);
    let mut fields: Vec<PrintField> = Vec::new();
    let kind: &'static str = match r {
        PatRef::Lit(h) => {
            let d: &PatLitNode = h.get(a);
            fields.push(field(
                "expr",
                PrintVal::Node(Box::new(expr_dump(a, d.expr, next))),
            ));
            <PatLitNode as NodeKind>::NAME
        }
        PatRef::Path(h) => {
            let d: &PatPathNode = h.get(a);
            fields.push(field("segs", PrintVal::Segs(segs_dump(a, &d.segs, next))));
            <PatPathNode as NodeKind>::NAME
        }
        PatRef::Wild(_) => <PatWildNode as NodeKind>::NAME,
        PatRef::Else(_) => <PatElseNode as NodeKind>::NAME,
        PatRef::Error(h) => return error_dump(a, h, next),
    };
    PrintNode {
        id,
        kind,
        span,
        fields,
    }
}

fn ty_dump(a: &Ast, r: TyRef, next: &mut u32) -> PrintNode {
    let id = take_id(next);
    let span = (r.span(a).lo, r.span(a).hi);
    let mut fields: Vec<PrintField> = Vec::new();
    let kind: &'static str = match r {
        TyRef::Path(h) => {
            let d: &TyPathNode = h.get(a);
            fields.push(field("segs", PrintVal::Segs(segs_dump(a, &d.segs, next))));
            <TyPathNode as NodeKind>::NAME
        }
        TyRef::Self_(h) => {
            let _d: &crate::ast::ty::SelfTyNode = h.get(a);
            <crate::ast::ty::SelfTyNode as NodeKind>::NAME
        }
        TyRef::Fn(h) => {
            let d: &TyFnNode = h.get(a);
            fields.push(field(
                "params",
                PrintVal::Nodes(d.params.iter().map(|p| ty_dump(a, *p, next)).collect()),
            ));
            fields.push(field(
                "ret",
                PrintVal::Node(Box::new(ty_dump(a, d.ret, next))),
            ));
            <TyFnNode as NodeKind>::NAME
        }
        TyRef::Opt(h) => {
            let d: &TyOptNode = h.get(a);
            fields.push(field(
                "inner",
                PrintVal::Node(Box::new(ty_dump(a, d.inner, next))),
            ));
            <TyOptNode as NodeKind>::NAME
        }
        TyRef::Array(h) => {
            let d: &TyArrayNode = h.get(a);
            fields.push(field(
                "elem",
                PrintVal::Node(Box::new(ty_dump(a, d.elem, next))),
            ));
            <TyArrayNode as NodeKind>::NAME
        }
        TyRef::Tuple(h) => {
            let d: &TyTupleNode = h.get(a);
            fields.push(field(
                "elems",
                PrintVal::Nodes(d.elems.iter().map(|e| ty_dump(a, *e, next)).collect()),
            ));
            <TyTupleNode as NodeKind>::NAME
        }
        TyRef::Union(h) => {
            let d: &TyUnionNode = h.get(a);
            fields.push(field(
                "elems",
                PrintVal::Nodes(d.elems.iter().map(|e| ty_dump(a, *e, next)).collect()),
            ));
            <TyUnionNode as NodeKind>::NAME
        }
        TyRef::Error(h) => return error_dump(a, h, next),
    };
    PrintNode {
        id,
        kind,
        span,
        fields,
    }
}

fn expr_dump(a: &Ast, r: ExprRef, next: &mut u32) -> PrintNode {
    let id = take_id(next);
    let span = (r.span(a).lo, r.span(a).hi);
    let mut fields: Vec<PrintField> = Vec::new();
    let kind: &'static str = match r {
        ExprRef::Block(h) => return block_dump(a, h, next),
        ExprRef::AsyncBlock(h) => {
            let d: &AsyncBlockNode = h.get(a);
            fields.push(field(
                "body",
                PrintVal::Node(Box::new(block_dump(a, d.body, next))),
            ));
            <AsyncBlockNode as NodeKind>::NAME
        }
        ExprRef::Lit(h) => {
            let _d: &LitNode = h.get(a);
            "Lit" // value derived: src[lo..hi]
        }
        ExprRef::Path(h) => {
            let d: &crate::ast::expr::PathNode = h.get(a);
            fields.push(field("segs", PrintVal::Segs(segs_dump(a, &d.segs, next))));
            <PathNode as NodeKind>::NAME
        }
        ExprRef::Self_(h) => {
            let _d: &crate::ast::expr::SelfNode = h.get(a);
            <crate::ast::expr::SelfNode as NodeKind>::NAME // value derived: src[lo..hi]
        }
        ExprRef::Call(h) => {
            let d: &CallNode = h.get(a);
            fields.push(field(
                "callee",
                PrintVal::Node(Box::new(expr_dump(a, d.callee, next))),
            ));
            fields.push(field(
                "args",
                PrintVal::Nodes(d.args.iter().map(|x| expr_dump(a, *x, next)).collect()),
            ));
            <CallNode as NodeKind>::NAME
        }
        ExprRef::Method(h) => {
            let d: &MethodNode = h.get(a);
            fields.push(field(
                "recv",
                PrintVal::Node(Box::new(expr_dump(a, d.recv, next))),
            ));
            if let Some(m) = d.member {
                fields.push(field("name", PrintVal::Str(a.name(m.ident).to_string())));
            }
            if !d.generics.is_empty() {
                fields.push(field(
                    "generics",
                    PrintVal::Nodes(d.generics.iter().map(|g| ty_dump(a, *g, next)).collect()),
                ));
            }
            fields.push(field(
                "args",
                PrintVal::Nodes(d.args.iter().map(|x| expr_dump(a, *x, next)).collect()),
            ));
            <MethodNode as NodeKind>::NAME
        }
        ExprRef::Field(h) => {
            let d: &FieldNode = h.get(a);
            fields.push(field(
                "recv",
                PrintVal::Node(Box::new(expr_dump(a, d.recv, next))),
            ));
            fields.push(field(
                "name",
                PrintVal::Str(a.name(d.name.ident).to_string()),
            ));
            <FieldNode as NodeKind>::NAME
        }
        ExprRef::Index(h) => {
            let d: &IndexNode = h.get(a);
            fields.push(field(
                "recv",
                PrintVal::Node(Box::new(expr_dump(a, d.recv, next))),
            ));
            fields.push(field(
                "idx",
                PrintVal::Node(Box::new(expr_dump(a, d.idx, next))),
            ));
            <IndexNode as NodeKind>::NAME
        }
        ExprRef::Unary(h) => {
            let d: &UnaryNode = h.get(a);
            fields.push(field("op", PrintVal::UnOp(d.op)));
            fields.push(field(
                "expr",
                PrintVal::Node(Box::new(expr_dump(a, d.expr, next))),
            ));
            <UnaryNode as NodeKind>::NAME
        }
        ExprRef::Binary(h) => {
            let d: &BinaryNode = h.get(a);
            fields.push(field("op", PrintVal::Op(d.op)));
            fields.push(field(
                "lhs",
                PrintVal::Node(Box::new(expr_dump(a, d.lhs, next))),
            ));
            fields.push(field(
                "rhs",
                PrintVal::Node(Box::new(expr_dump(a, d.rhs, next))),
            ));
            <BinaryNode as NodeKind>::NAME
        }
        ExprRef::Assign(h) => {
            let d: &AssignNode = h.get(a);
            if let Some(o) = d.op {
                fields.push(field("op", PrintVal::Op(o)));
            }
            fields.push(field(
                "target",
                PrintVal::Node(Box::new(expr_dump(a, d.target, next))),
            ));
            fields.push(field(
                "value",
                PrintVal::Node(Box::new(expr_dump(a, d.value, next))),
            ));
            <AssignNode as NodeKind>::NAME
        }
        ExprRef::Lambda(h) => {
            let d: &LambdaNode = h.get(a);
            fields.push(field(
                "params",
                PrintVal::Nodes(d.params.iter().map(|p| param_dump(a, *p, next)).collect()),
            ));
            if let Some(r) = d.ret {
                fields.push(field("ret", PrintVal::Node(Box::new(ty_dump(a, r, next)))));
            }
            fields.push(field(
                "body",
                PrintVal::Node(Box::new(expr_dump(a, d.body, next))),
            ));
            <LambdaNode as NodeKind>::NAME
        }
        ExprRef::FStr(h) => {
            let d: &FStrNode = h.get(a);
            fields.push(field(
                "parts",
                PrintVal::FParts(
                    d.parts
                        .iter()
                        .map(|p| match p {
                            FPartAst::Lit(s) => PrintFPart::Lit(s.clone()),
                            FPartAst::Hole(e) => PrintFPart::Hole(expr_dump(a, *e, next)),
                        })
                        .collect(),
                ),
            ));
            <FStrNode as NodeKind>::NAME
        }
        ExprRef::Struct(h) => {
            let d: &StructLitNode = h.get(a);
            fields.push(field(
                "ty",
                PrintVal::Node(Box::new(ty_dump(a, d.ty, next))),
            ));
            fields.push(field(
                "fields",
                PrintVal::StructFields(
                    d.fields
                        .iter()
                        .map(|fi| PrintStructField {
                            ident: a.name(fi.name.ident).to_string(),
                            span: (fi.name.span.lo, fi.name.span.hi),
                            node: expr_dump(a, fi.value, next),
                        })
                        .collect(),
                ),
            ));
            <StructLitNode as NodeKind>::NAME
        }
        ExprRef::Tuple(h) => {
            let d: &TupleNode = h.get(a);
            fields.push(field(
                "elems",
                PrintVal::Nodes(d.elems.iter().map(|e| expr_dump(a, *e, next)).collect()),
            ));
            <TupleNode as NodeKind>::NAME
        }
        ExprRef::ArrayLit(h) => {
            let d: &ArrayLitNode = h.get(a);
            fields.push(field(
                "elems",
                PrintVal::Nodes(d.elems.iter().map(|e| expr_dump(a, *e, next)).collect()),
            ));
            <ArrayLitNode as NodeKind>::NAME
        }
        ExprRef::ArrayRepeat(h) => {
            let d: &ArrayRepeatNode = h.get(a);
            fields.push(field(
                "value",
                PrintVal::Node(Box::new(expr_dump(a, d.value, next))),
            ));
            fields.push(field(
                "count",
                PrintVal::Node(Box::new(expr_dump(a, d.count, next))),
            ));
            <ArrayRepeatNode as NodeKind>::NAME
        }
        ExprRef::WhenExpr(h) => {
            let d: &WhenExprNode = h.get(a);
            fields.push(field(
                "scrut",
                PrintVal::Node(Box::new(expr_dump(a, d.scrut, next))),
            ));
            fields.push(field(
                "arms",
                PrintVal::Nodes(d.arms.iter().map(|&x| arm_dump(a, x, next)).collect()),
            ));
            <WhenExprNode as NodeKind>::NAME
        }
        ExprRef::If(h) => {
            let d: &IfExprNode = h.get(a);
            fields.push(field(
                "cond",
                PrintVal::Node(Box::new(expr_dump(a, d.cond, next))),
            ));
            fields.push(field(
                "then",
                PrintVal::Node(Box::new(block_dump(a, d.then, next))),
            ));
            if let Some(e) = d.els {
                fields.push(field(
                    "els",
                    PrintVal::Node(Box::new(else_dump(a, e, next))),
                ));
            }
            <IfExprNode as NodeKind>::NAME
        }
        ExprRef::Await(h) => {
            let d: &AwaitNode = h.get(a);
            fields.push(field(
                "expr",
                PrintVal::Node(Box::new(expr_dump(a, d.expr, next))),
            ));
            <AwaitNode as NodeKind>::NAME
        }
        ExprRef::Is(h) => {
            let d: &IsNode = h.get(a);
            fields.push(field(
                "expr",
                PrintVal::Node(Box::new(expr_dump(a, d.expr, next))),
            ));
            fields.push(field(
                "ty",
                PrintVal::Node(Box::new(ty_dump(a, d.ty, next))),
            ));
            <IsNode as NodeKind>::NAME
        }
        ExprRef::Cast(h) => {
            let d: &CastNode = h.get(a);
            fields.push(field(
                "expr",
                PrintVal::Node(Box::new(expr_dump(a, d.expr, next))),
            ));
            fields.push(field(
                "ty",
                PrintVal::Node(Box::new(ty_dump(a, d.ty, next))),
            ));
            <CastNode as NodeKind>::NAME
        }
        ExprRef::Error(h) => return error_dump(a, h, next),
    };
    PrintNode {
        id,
        kind,
        span,
        fields,
    }
}

/// the whole tree from the root — the one mapping.
fn tree_of(ast: &Ast) -> PrintNode {
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

fn node_json(n: &PrintNode, out: &mut String) {
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

fn val_json(v: &PrintVal, out: &mut String) {
    match v {
        PrintVal::Node(n) => node_json(n, out),
        PrintVal::Nodes(ns) => {
            out.push('[');
            for (i, n) in ns.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                node_json(n, out);
            }
            out.push(']');
        }
        PrintVal::Idents(xs) => {
            out.push('[');
            for (i, x) in xs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                json_escape(x, out);
            }
            out.push(']');
        }
        PrintVal::Members(ms) => {
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
        PrintVal::StructFields(fs) => {
            out.push('[');
            for (i, f) in fs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('{');
                json_escape("ident", out);
                out.push(':');
                json_escape(&f.ident, out);
                out.push_str(&format!(",\"span\":[{},{}]", f.span.0, f.span.1));
                out.push_str(",\"node\":");
                node_json(&f.node, out);
                out.push('}');
            }
            out.push(']');
        }
        PrintVal::Bounds(ws) => {
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
        PrintVal::Segs(segs) => {
            out.push('[');
            for (i, s) in segs.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('{');
                json_escape("name", out);
                out.push(':');
                json_escape(&s.name, out);
                out.push_str(&format!(",\"span\":[{},{}]", s.span.0, s.span.1));
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
        PrintVal::FParts(ps) => {
            out.push('[');
            for (i, p) in ps.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                match p {
                    PrintFPart::Lit(s) => {
                        out.push_str("{\"kind\":\"FStrLit\",\"str\":");
                        json_escape(s, out);
                        out.push('}');
                    }
                    PrintFPart::Hole(n) => node_json(n, out),
                }
            }
            out.push(']');
        }
        PrintVal::Str(s) => json_escape(s, out),
        PrintVal::Span(lo, hi) => out.push_str(&format!("[{lo},{hi}]")),
        PrintVal::Flag(b) => out.push_str(if *b { "true" } else { "false" }),
        PrintVal::Vis(v) => out.push_str(&format!("\"{}\"", vis_tag(*v))),
        PrintVal::OptVis(v) => match v {
            Some(v) => out.push_str(&format!("\"{}\"", vis_tag(*v))),
            None => out.push_str("null"),
        },
        PrintVal::Op(op) => out.push_str(&format!("\"{:?}\"", op)),
        PrintVal::UnOp(op) => out.push_str(&format!("\"{:?}\"", op)),
    }
}

/// The demo tree: `{"id":..,"kind":..,"span":[lo,hi],<label>:<val>,...}`.
fn render_json(root: &PrintNode) -> String {
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
    }
}

fn ind(n: usize) -> String {
    "  ".repeat(n)
}

fn header(n: &PrintNode, src: &str) -> String {
    // collapsed-row label: prefer the `name` field; literals slice the source
    let name = n
        .fields
        .iter()
        .find(|f| f.label == "name")
        .and_then(|f| match &f.val {
            PrintVal::Str(s) => Some(s.clone()),
            _ => None,
        });
    let name = name.unwrap_or_else(|| {
        if n.kind == <LitNode as NodeKind>::NAME {
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

fn node_text(n: &PrintNode, d: usize, src: &str, out: &mut String) {
    out.push_str(&format!("{}{}\n", ind(d), header(n, src)));
    for f in &n.fields {
        val_text(f.label, &f.val, d + 1, src, out);
    }
}

fn val_text(label: &str, v: &PrintVal, d: usize, src: &str, out: &mut String) {
    let i = ind(d);
    match v {
        PrintVal::Node(n) => {
            out.push_str(&format!("{i}{label}:\n"));
            node_text(n, d + 1, src, out);
        }
        PrintVal::Nodes(ns) => {
            out.push_str(&format!("{i}{label}:\n"));
            for n in ns {
                node_text(n, d + 1, src, out);
            }
        }
        PrintVal::Idents(xs) => {
            out.push_str(&format!("{i}{label}:\n"));
            for x in xs {
                out.push_str(&format!("{}- {x}\n", ind(d + 1)));
            }
        }
        PrintVal::Members(ms) => {
            out.push_str(&format!("{i}{label}:\n"));
            for m in ms {
                out.push_str(&format!("{}- {}\n", ind(d + 1), m.ident));
            }
        }
        PrintVal::StructFields(fs) => {
            out.push_str(&format!("{i}{label}:\n"));
            for f in fs {
                out.push_str(&format!("{}- {}:\n", ind(d + 1), f.ident));
                node_text(&f.node, d + 2, src, out);
            }
        }
        PrintVal::Bounds(ws) => {
            out.push_str(&format!("{i}{label}:\n"));
            for w in ws {
                out.push_str(&format!("{}- {} requires\n", ind(d + 1), w.ident));
                node_text(&w.ty, d + 2, src, out);
            }
        }
        PrintVal::Segs(segs) => {
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
        PrintVal::FParts(ps) => {
            out.push_str(&format!("{i}{label}:\n"));
            for p in ps {
                match p {
                    PrintFPart::Lit(s) => out.push_str(&format!("{}- {s:?}\n", ind(d + 1))),
                    PrintFPart::Hole(n) => node_text(n, d + 1, src, out),
                }
            }
        }
        PrintVal::Str(s) => out.push_str(&format!("{i}{label}: {s}\n")),
        PrintVal::Span(lo, hi) => out.push_str(&format!("{i}{label}: [{lo},{hi})\n")),
        PrintVal::Flag(b) => out.push_str(&format!("{i}{label}: {b}\n")),
        PrintVal::Vis(v) => out.push_str(&format!("{i}{label}: {}\n", vis_str(*v))),
        PrintVal::OptVis(v) => {
            if let Some(v) = v {
                out.push_str(&format!("{i}{label}: {}\n", vis_str(*v)))
            }
        }
        PrintVal::Op(op) => out.push_str(&format!("{i}{label}: {}\n", binop_str(*op))),
        PrintVal::UnOp(op) => out.push_str(&format!("{i}{label}: {}\n", unop_str(*op))),
    }
}

/// CLI text style — same tree, labeled. Spans on every node header;
/// `Lit` values are sliced from the source.
fn render_text(root: &PrintNode, src: &str) -> String {
    let mut out = String::new();
    node_text(root, 0, src, &mut out);
    out
}

// ---- the printer's face: the named doors over the one mapping ----

/// The tree renderer — stateless; the doors are associated functions.
/// `tree`/`subtree` map, `json`/`text` render; `json_of`/`text_of`
/// are the conveniences (one call, no juggling the pair).
pub struct AstPrinter;

impl AstPrinter {
    /// the whole tree from the root — the one mapping (the demo wire's
    /// source; the ids are this single walk's DFS order)
    pub fn tree(ast: &Ast) -> PrintNode {
        tree_of(ast)
    }

    /// any node's window — the same mapping from an arbitrary address
    /// (serves `node_at` debugging and per-window comparisons in the
    /// edit≡cold property tests). A stale address prints the explicit
    /// `dead` leaf row.
    pub fn subtree(ast: &Ast, h: AnyHandle) -> PrintNode {
        let mut next = 0u32;
        print_node_of(ast, h, &mut next)
    }

    /// the flattened tagged objects the demo's tree UI consumes —
    /// FROZEN bytes
    pub fn json(t: &PrintNode) -> String {
        render_json(t)
    }

    /// the CLI's labeled text style (spans on every header; `Lit`
    /// values sliced from the source)
    pub fn text(t: &PrintNode, src: &str) -> String {
        render_text(t, src)
    }

    /// tree → json in one call
    pub fn json_of(ast: &Ast) -> String {
        Self::json(&Self::tree(ast))
    }

    /// tree → text in one call (src is the NORMALIZED text every span
    /// indexes)
    pub fn text_of(ast: &Ast, src: &str) -> String {
        Self::text(&Self::tree(ast), src)
    }
}

/// the dead hole — an address whose slot holds no node (or whose kind
/// id is bogus). Explicit, debuggable, never a panic, never a
/// wrong-arena read.
fn dead_node(next: &mut u32) -> PrintNode {
    PrintNode {
        id: take_id(next),
        kind: "dead",
        span: (0, 0),
        fields: Vec::new(),
    }
}

/// one enum member row — the tree index treats members as real nodes
/// (they own spans), though the whole-tree mapping renders them as
/// ident strings inside `Enum`. Reachable only through `subtree`.
fn enum_member_print(a: &Ast, h: Handle<EnumMemberNode>, next: &mut u32) -> PrintNode {
    let d = h.get(a);
    let fields = vec![field(
        "name",
        PrintVal::Str(a.name(d.name.ident).to_string()),
    )];
    PrintNode {
        id: take_id(next),
        kind: <EnumMemberNode as NodeKind>::NAME,
        span: (d.span.lo, d.span.hi),
        fields,
    }
}

/// the `AnyHandle` dispatch — the kind check picks the mapping arm
/// (never a handle into the wrong arena); a dead slot falls to the
/// `dead` row.
fn print_node_of(a: &Ast, h: AnyHandle, next: &mut u32) -> PrintNode {
    if span_of_any(a, h).is_none() {
        return dead_node(next);
    }
    // items
    if let Some(x) = h.downcast::<ModuleNode>() {
        return item_dump(a, ItemRef::Module(x), next);
    }
    if let Some(x) = h.downcast::<ModDeclNode>() {
        return item_dump(a, ItemRef::ModDecl(x), next);
    }
    if let Some(x) = h.downcast::<UseNode>() {
        return item_dump(a, ItemRef::Use(x), next);
    }
    if let Some(x) = h.downcast::<AliasNode>() {
        return item_dump(a, ItemRef::Alias(x), next);
    }
    if let Some(x) = h.downcast::<ModuleLetNode>() {
        return item_dump(a, ItemRef::ModuleLet(x), next);
    }
    if let Some(x) = h.downcast::<EnumNode>() {
        return item_dump(a, ItemRef::Enum(x), next);
    }
    if let Some(x) = h.downcast::<StructNode>() {
        return item_dump(a, ItemRef::Struct(x), next);
    }
    if let Some(x) = h.downcast::<ClassNode>() {
        return item_dump(a, ItemRef::Class(x), next);
    }
    if let Some(x) = h.downcast::<InterfaceNode>() {
        return item_dump(a, ItemRef::Interface(x), next);
    }
    if let Some(x) = h.downcast::<ImplNode>() {
        return item_dump(a, ItemRef::Impl(x), next);
    }
    if let Some(x) = h.downcast::<FnNode>() {
        return item_dump(a, ItemRef::Fn(x), next);
    }
    if let Some(x) = h.downcast::<SurfaceFnNode>() {
        return item_dump(a, ItemRef::SurfaceFn(x), next);
    }
    if let Some(x) = h.downcast::<SurfaceStructNode>() {
        return item_dump(a, ItemRef::SurfaceStruct(x), next);
    }
    if let Some(x) = h.downcast::<BuiltinTyNode>() {
        return item_dump(a, ItemRef::BuiltinTy(x), next);
    }
    if let Some(x) = h.downcast::<BuiltinPrimitiveNode>() {
        return item_dump(a, ItemRef::BuiltinPrimitive(x), next);
    }
    if let Some(x) = h.downcast::<BuiltinImplNode>() {
        return item_dump(a, ItemRef::BuiltinImpl(x), next);
    }
    if let Some(x) = h.downcast::<ErrorNode>() {
        return error_dump(a, x, next);
    }
    // members
    if let Some(x) = h.downcast::<FieldDeclNode>() {
        return field_decl_dump(a, x, next);
    }
    if let Some(x) = h.downcast::<MethodDeclNode>() {
        return method_decl_dump(a, x, next);
    }
    if let Some(x) = h.downcast::<EnumMemberNode>() {
        return enum_member_print(a, x, next);
    }
    if let Some(x) = h.downcast::<ParamNode>() {
        return param_dump(a, ParamRef::Param(x), next);
    }
    if let Some(x) = h.downcast::<SelfParamNode>() {
        return param_dump(a, ParamRef::SelfParam(x), next);
    }
    // statements
    if let Some(x) = h.downcast::<LetStmtNode>() {
        return stmt_dump(a, StmtRef::Let(x), next);
    }
    if let Some(x) = h.downcast::<IfNode>() {
        return stmt_dump(a, StmtRef::If(x), next);
    }
    if let Some(x) = h.downcast::<WhileNode>() {
        return stmt_dump(a, StmtRef::While(x), next);
    }
    if let Some(x) = h.downcast::<ForOfNode>() {
        return stmt_dump(a, StmtRef::ForOf(x), next);
    }
    if let Some(x) = h.downcast::<ForCNode>() {
        return stmt_dump(a, StmtRef::ForC(x), next);
    }
    if let Some(x) = h.downcast::<ReturnNode>() {
        return stmt_dump(a, StmtRef::Return(x), next);
    }
    if let Some(x) = h.downcast::<BreakNode>() {
        return stmt_dump(a, StmtRef::Break(x), next);
    }
    if let Some(x) = h.downcast::<ContinueNode>() {
        return stmt_dump(a, StmtRef::Continue(x), next);
    }
    if let Some(x) = h.downcast::<WhenStmtNode>() {
        return stmt_dump(a, StmtRef::When(x), next);
    }
    // expressions
    if let Some(x) = h.downcast::<BlockNode>() {
        return block_dump(a, x, next);
    }
    if let Some(x) = h.downcast::<LitNode>() {
        return expr_dump(a, ExprRef::Lit(x), next);
    }
    if let Some(x) = h.downcast::<PathNode>() {
        return expr_dump(a, ExprRef::Path(x), next);
    }
    if let Some(x) = h.downcast::<CallNode>() {
        return expr_dump(a, ExprRef::Call(x), next);
    }
    if let Some(x) = h.downcast::<MethodNode>() {
        return expr_dump(a, ExprRef::Method(x), next);
    }
    if let Some(x) = h.downcast::<FieldNode>() {
        return expr_dump(a, ExprRef::Field(x), next);
    }
    if let Some(x) = h.downcast::<IndexNode>() {
        return expr_dump(a, ExprRef::Index(x), next);
    }
    if let Some(x) = h.downcast::<UnaryNode>() {
        return expr_dump(a, ExprRef::Unary(x), next);
    }
    if let Some(x) = h.downcast::<BinaryNode>() {
        return expr_dump(a, ExprRef::Binary(x), next);
    }
    if let Some(x) = h.downcast::<AssignNode>() {
        return expr_dump(a, ExprRef::Assign(x), next);
    }
    if let Some(x) = h.downcast::<LambdaNode>() {
        return expr_dump(a, ExprRef::Lambda(x), next);
    }
    if let Some(x) = h.downcast::<FStrNode>() {
        return expr_dump(a, ExprRef::FStr(x), next);
    }
    if let Some(x) = h.downcast::<StructLitNode>() {
        return expr_dump(a, ExprRef::Struct(x), next);
    }
    if let Some(x) = h.downcast::<TupleNode>() {
        return expr_dump(a, ExprRef::Tuple(x), next);
    }
    if let Some(x) = h.downcast::<ArrayLitNode>() {
        return expr_dump(a, ExprRef::ArrayLit(x), next);
    }
    if let Some(x) = h.downcast::<ArrayRepeatNode>() {
        return expr_dump(a, ExprRef::ArrayRepeat(x), next);
    }
    if let Some(x) = h.downcast::<WhenExprNode>() {
        return expr_dump(a, ExprRef::WhenExpr(x), next);
    }
    if let Some(x) = h.downcast::<IfExprNode>() {
        return expr_dump(a, ExprRef::If(x), next);
    }
    if let Some(x) = h.downcast::<AwaitNode>() {
        return expr_dump(a, ExprRef::Await(x), next);
    }
    if let Some(x) = h.downcast::<AsyncBlockNode>() {
        return expr_dump(a, ExprRef::AsyncBlock(x), next);
    }
    if let Some(x) = h.downcast::<IsNode>() {
        return expr_dump(a, ExprRef::Is(x), next);
    }
    if let Some(x) = h.downcast::<CastNode>() {
        return expr_dump(a, ExprRef::Cast(x), next);
    }
    // types
    if let Some(x) = h.downcast::<TyPathNode>() {
        return ty_dump(a, TyRef::Path(x), next);
    }
    if let Some(x) = h.downcast::<TyFnNode>() {
        return ty_dump(a, TyRef::Fn(x), next);
    }
    if let Some(x) = h.downcast::<TyOptNode>() {
        return ty_dump(a, TyRef::Opt(x), next);
    }
    if let Some(x) = h.downcast::<TyArrayNode>() {
        return ty_dump(a, TyRef::Array(x), next);
    }
    if let Some(x) = h.downcast::<TyTupleNode>() {
        return ty_dump(a, TyRef::Tuple(x), next);
    }
    if let Some(x) = h.downcast::<TyUnionNode>() {
        return ty_dump(a, TyRef::Union(x), next);
    }
    // patterns
    if let Some(x) = h.downcast::<PatLitNode>() {
        return pat_dump(a, PatRef::Lit(x), next);
    }
    if let Some(x) = h.downcast::<PatPathNode>() {
        return pat_dump(a, PatRef::Path(x), next);
    }
    if let Some(x) = h.downcast::<PatWildNode>() {
        return pat_dump(a, PatRef::Wild(x), next);
    }
    if let Some(x) = h.downcast::<PatElseNode>() {
        return pat_dump(a, PatRef::Else(x), next);
    }
    // arms
    if let Some(x) = h.downcast::<WhenArmNode>() {
        return arm_dump(a, x, next);
    }
    // the inventory covers every stamped id — unreachable in practice
    dead_node(next)
}
