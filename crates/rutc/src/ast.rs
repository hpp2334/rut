//! AST — flat arena, ESTree-style (RFC 0030 §5, contract C1).
//!
//! Nodes are plain records in one arena; children are `NodeId`s into it —
//! never `Box<Expr>`. Construction is bottom-up; drop/clone are flat `Vec`
//! ops, so no input can overflow the host stack through the tree. Spans on
//! every node; names are `IdentId`s into the interner.

use crate::span::Span;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IdentId(pub u32);

pub struct Ast {
    pub nodes: Vec<Node>,
    pub interner: Interner,
    pub root: NodeId,
}

impl Ast {
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }
    pub fn name(&self, id: IdentId) -> &str {
        self.interner.name(id)
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub span: Span,
    pub kind: NodeKind,
}

/// String interner — names are indices, not `String` keys (RFC 0030 §5).
#[derive(Default)]
pub struct Interner {
    names: Vec<String>,
    map: HashMap<String, IdentId>,
}

impl Interner {
    pub fn intern(&mut self, s: &str) -> IdentId {
        if let Some(id) = self.map.get(s) {
            return *id;
        }
        let id = IdentId(self.names.len() as u32);
        self.names.push(s.to_string());
        self.map.insert(s.to_string(), id);
        id
    }
    /// interner lookup — find an existing name's id (no interning)
    pub fn lookup(&self, s: &str) -> Option<IdentId> {
        self.map.get(s).copied()
    }
    pub fn name(&self, id: IdentId) -> &str {
        &self.names[id.0 as usize]
    }
}

// ---- visibility (RFC 0003 §2) ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vis {
    /// `export` — importable from anywhere
    Pub,
    /// `export(mod)`
    Mod,
    /// `export(super)`
    Super,
    /// `export(self)` / unannotated — module-private (the default)
    Self_,
}

// ---- linkage (.d.rut surface decls, RFC 0029 §2 / RFC 0030 §3) ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linkage {
    Host,
    Extern,
}

#[derive(Clone, Debug)]
pub struct PathSeg {
    pub name: IdentId,
    /// generic arguments — type args, or const args as expression nodes
    pub generics: Vec<NodeId>,
}

#[derive(Clone, Debug)]
pub enum NodeKind {
    // ---- items ----
    Module { items: Vec<NodeId> },
    Import { names: Vec<IdentId>, from: String },
    ModuleLet { vis: Vis, name: IdentId, ty: Option<NodeId>, init: NodeId },
    Enum { vis: Vis, name: IdentId, members: Vec<(IdentId, Option<i64>)> },
    Dataclass {
        vis: Vis,
        name: IdentId,
        generics: Vec<IdentId>,
        fields: Vec<NodeId>,
        methods: Vec<NodeId>,
    },
    Class {
        vis: Vis,
        name: IdentId,
        generics: Vec<IdentId>,
        fields: Vec<NodeId>,
        methods: Vec<NodeId>,
    },
    Trait {
        vis: Vis,
        name: IdentId,
        generics: Vec<IdentId>,
        requires: Vec<NodeId>, // type nodes — naming position, bare
        methods: Vec<NodeId>,  // MethodDecl nodes (no bodies)
    },
    Impl {
        trait_ref: NodeId, // type node
        target: NodeId,    // type node
        methods: Vec<NodeId>,
    },
    Fn {
        vis: Vis,
        is_suspend: bool,
        name: IdentId,
        generics: Vec<IdentId>,
        params: Vec<NodeId>,
        ret: Option<NodeId>,
        where_bounds: Vec<(IdentId, NodeId)>, // RFC 0013 §2 — admission-only
        body: NodeId,
    },
    /// `host fn` / `extern fn` — .d.rut only (RFC 0030 §3)
    SurfaceFn {
        vis: Vis,
        linkage: Linkage,
        name: IdentId,
        generics: Vec<IdentId>,
        params: Vec<NodeId>,
        ret: Option<NodeId>,
    },
    /// `host class` / `extern class` — .d.rut only
    SurfaceClass {
        vis: Vis,
        linkage: Linkage,
        name: IdentId,
        extparams: Vec<(IdentId, Option<NodeId>)>,
        members: Vec<NodeId>, // bodiless MethodDecl nodes
    },

    // ---- members & statements ----
    /// dataclass/class field: `private`? `static`? name: ty (= init)?
    FieldDecl {
        is_private: bool,
        is_static: bool,
        name: IdentId,
        ty: NodeId,
        init: Option<NodeId>,
    },
    /// `private`? `suspend`? fn name<..>(self, ..): T { .. }
    MethodDecl {
        is_private: bool,
        is_suspend: bool,
        name: IdentId,
        generics: Vec<IdentId>,
        params: Vec<NodeId>, // Param / SelfParam
        ret: Option<NodeId>,
        body: Option<NodeId>, // None in traits / surface classes
    },
    /// `mut`? name: ty — or a type-less lambda param `(x)`
    Param { is_mut: bool, name: IdentId, ty: Option<NodeId> },
    /// `self` / `mut self`
    SelfParam { is_mut: bool },

    Block { stmts: Vec<NodeId> },
    LetStmt { is_mut: bool, name: IdentId, ty: Option<NodeId>, init: NodeId },
    If { cond: NodeId, then: NodeId, els: Option<NodeId> },
    While { cond: NodeId, body: NodeId },
    ForOf { var: IdentId, iter: NodeId, body: NodeId },
    ForC { var: IdentId, init: NodeId, cond: NodeId, update: NodeId, body: NodeId },
    Return { value: Option<NodeId> },
    Break,
    Continue,
    WhenStmt { scrut: NodeId, arms: Vec<NodeId> },
    ExprStmt(NodeId),

    // ---- arms ----
    WhenArm { pats: Vec<NodeId>, body: NodeId },
    /// `fut -> body` / `fut as name -> body` (RFC 0019 §3)
    SelectArm { fut: NodeId, bind: Option<IdentId>, body: NodeId },

    // ---- patterns (RFC 0008 §2; constructor/binding forms per corpus) ----
    /// literal pattern (int/float/bool/char/string) — expr node, literal only
    PatLit(NodeId),
    /// enum-member path: `Color.Red`
    PatPath { segs: Vec<PathSeg> },
    /// constructor pattern: `Ok(cfg)`, `Option<T>.Some(x)`, `Some(_)`,
    /// payloadless `Option<T>.None`; args are binding names (`None` = `_`)
    PatCtor { segs: Vec<PathSeg>, args: Vec<Option<IdentId>> },
    PatWild,
    PatElse,

    // ---- types ----
    /// path type, optionally `dyn`-prefixed in value positions (RFC 0030 §2)
    TyPath { segs: Vec<PathSeg>, is_dyn: bool },
    /// fn type: `fn(Store, P): R` — params are bare types (RFC 0013 §1)
    TyFn { params: Vec<NodeId>, ret: NodeId },
    /// const-generic argument (the `N` in `Array<T, N>`) — an expression
    TyConst(NodeId),

    // ---- expressions ----
    Lit(Lit),
    /// dotted path, possibly with generic args on any segment:
    /// `primes`, `Color.Red`, `Vec.from`, `MyMap<K, V>.new`, `Self`
    Path { segs: Vec<PathSeg> },
    Call { callee: NodeId, args: Vec<NodeId> },
    Method { recv: NodeId, name: IdentId, generics: Vec<NodeId>, args: Vec<NodeId> },
    Field { recv: NodeId, name: IdentId },
    Index { recv: NodeId, idx: NodeId },
    Unary { op: UnOp, expr: NodeId },
    Binary { op: BinOp, lhs: NodeId, rhs: NodeId },
    Assign { op: Option<BinOp>, target: NodeId, value: NodeId },
    Lambda { params: Vec<NodeId>, ret: Option<NodeId>, body: NodeId },
    Try { expr: NodeId }, // postfix `?`
    FStr { parts: Vec<FPartAst> },
    /// dataclass / `Self { .. }` literal — classes have no instance literal
    Struct { ty: NodeId, fields: Vec<(IdentId, NodeId)> },
    /// `[e1, .., en] : Array<T, n>` (RFC 0007 §1)
    ArrayLit { elems: Vec<NodeId> },
    WhenExpr { scrut: NodeId, arms: Vec<NodeId> },
    Await { expr: NodeId },
    /// `select { ... }` — only reachable as `await select { .. }` (RFC 0019 §3)
    Select { arms: Vec<NodeId> },
    /// `expr is Type` — relational precedence, non-associative (RFC 0012 §3)
    Is { expr: NodeId, ty: NodeId },
}

#[derive(Clone, Debug)]
pub enum FPartAst {
    Lit(String),
    Hole(NodeId),
}

#[derive(Clone, Debug)]
pub enum Lit {
    Int(u64, Option<crate::token::IntSuffix>),
    Float(u64 /*f64 bits*/, Option<crate::token::FloatSuffix>),
    Str(String),
    RawStr(String),
    Char(char),
    Bool(bool),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or, // logical short-circuit
    BitAnd, BitOr, BitXor, Shl, Shr,
    // wrapping family (RFC 0004 §3)
    WrapAdd, WrapSub, WrapMul, WrapShl,
}
