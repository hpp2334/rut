//! AST — flat arena with typed node handles (RFC 0030 §5, contract C1).
//!
//! Nodes are plain records in one arena; children are `NodeHandle<T>`s —
//! typed indices into it, never `Box<Expr>`. Construction is bottom-up;
//! drop/clone are flat `Vec` ops, so no input can overflow the host stack
//! through the tree. Spans on every node; names are `IdentId`s into the
//! interner.
//!
//! `NodeKind` is split per category (`ItemKind`/`MemberKind`/`StmtKind`/
//! `ExprKind`/`PatKind`/`ArmKind`/`TypeKind`): a child slot's type says
//! which category it accepts, and closed slots (`fn` bodies are `Block`s,
//! method lists are `MethodDecl`s) name the exact variant. A wrong-category
//! link does not compile. `Block` is an expression — arm and lambda bodies
//! are block-or-expr — which is exactly how the compiler treats them.

use crate::span::Span;
use std::collections::HashMap;
use std::marker::PhantomData;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub u32);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IdentId(pub u32);

// ---- typed handles ----

/// A `NodeId` that carries the node's category (or exact variant) in its
/// type. Zero-sized at runtime.
pub struct NodeHandle<T> {
    id: NodeId,
    _ty: PhantomData<fn() -> T>,
}

impl<T> NodeHandle<T> {
    pub(crate) fn new(id: NodeId) -> Self {
        NodeHandle { id, _ty: PhantomData }
    }
    pub fn id(&self) -> NodeId {
        self.id
    }
}
impl<T> Clone for NodeHandle<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for NodeHandle<T> {}
impl<T> PartialEq for NodeHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<T> Eq for NodeHandle<T> {}
impl<T> std::hash::Hash for NodeHandle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}
impl<T> std::fmt::Debug for NodeHandle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.id)
    }
}
impl<T> From<NodeHandle<T>> for NodeId {
    fn from(h: NodeHandle<T>) -> NodeId {
        h.id
    }
}

// markers: categories (open slots) + variants (closed slots)
pub struct ModuleNode;
pub struct AnyItem;
pub struct IfNode;
pub struct AnyStmt;
pub struct AnyExpr;
pub struct AnyPat;
pub struct AnyTy;
pub struct AnyParam; // Param | SelfParam
pub struct AnyArm;
pub struct BlockNode;      // fn/method/loop/if-then bodies, block arms
pub struct FieldDeclNode;
pub struct MethodDeclNode;
pub struct FnNode;

macro_rules! upcast {
    ($from:ty, $to:ty) => {
        impl From<NodeHandle<$from>> for NodeHandle<$to> {
            fn from(h: NodeHandle<$from>) -> NodeHandle<$to> {
                NodeHandle::new(h.id())
            }
        }
    };
}
upcast!(ModuleNode, AnyItem);
upcast!(FnNode, AnyItem);
upcast!(IfNode, AnyStmt);
upcast!(BlockNode, AnyExpr);

pub struct Ast {
    pub nodes: Vec<Node>,
    pub interner: Interner,
    pub root: NodeHandle<ModuleNode>,
}

impl Ast {
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }
    pub fn span(&self, id: NodeId) -> Span {
        self.nodes[id.0 as usize].span
    }
    pub fn name(&self, id: IdentId) -> &str {
        self.interner.name(id)
    }

    // full view (generic walks — dump)
    pub fn kind(&self, id: NodeId) -> &Kind {
        &self.nodes[id.0 as usize].kind
    }

    // category accessors — construction is typed, so the wrapper is
    // structural; a mismatch is unreachable by construction
    pub fn item(&self, h: NodeHandle<AnyItem>) -> &ItemKind {
        match &self.node(h.id()).kind {
            Kind::Item(k) => k,
            _ => unreachable!("item handle into a non-item node"),
        }
    }
    pub fn stmt(&self, h: NodeHandle<AnyStmt>) -> &StmtKind {
        match &self.node(h.id()).kind {
            Kind::Stmt(k) => k,
            _ => unreachable!("stmt handle into a non-stmt node"),
        }
    }
    pub fn expr(&self, h: NodeHandle<AnyExpr>) -> &ExprKind {
        match &self.node(h.id()).kind {
            Kind::Expr(k) => k,
            _ => unreachable!("expr handle into a non-expr node"),
        }
    }
    pub fn pat(&self, h: NodeHandle<AnyPat>) -> &PatKind {
        match &self.node(h.id()).kind {
            Kind::Pat(k) => k,
            _ => unreachable!("pattern handle into a non-pattern node"),
        }
    }
    pub fn ty(&self, h: NodeHandle<AnyTy>) -> &TypeKind {
        match &self.node(h.id()).kind {
            Kind::Type(k) => k,
            _ => unreachable!("type handle into a non-type node"),
        }
    }
    pub fn arm(&self, h: NodeHandle<AnyArm>) -> &ArmKind {
        match &self.node(h.id()).kind {
            Kind::Arm(k) => k,
            _ => unreachable!("arm handle into a non-arm node"),
        }
    }

    // variant accessors for closed slots
    pub fn module_items(&self, h: NodeHandle<ModuleNode>) -> &[NodeHandle<AnyItem>] {
        match self.item(h.into()) {
            ItemKind::Module { items } => items,
            _ => unreachable!("module handle into a non-module node"),
        }
    }
    pub fn fn_decl(&self, h: NodeHandle<FnNode>) -> &FnData {
        match self.item(h.into()) {
            ItemKind::Fn(d) => d,
            _ => unreachable!("fn handle into a non-fn node"),
        }
    }
    pub fn field_decl(&self, h: NodeHandle<FieldDeclNode>) -> &FieldDeclData {
        match &self.node(h.id()).kind {
            Kind::Member(MemberKind::FieldDecl(d)) => d,
            _ => unreachable!("field handle into a non-field node"),
        }
    }
    pub fn method_decl(&self, h: NodeHandle<MethodDeclNode>) -> &MethodDeclData {
        match &self.node(h.id()).kind {
            Kind::Member(MemberKind::MethodDecl(d)) => d,
            _ => unreachable!("method handle into a non-method node"),
        }
    }
    /// Param or SelfParam (fn/lambda/method parameter lists)
    pub fn param(&self, h: NodeHandle<AnyParam>) -> &MemberKind {
        match &self.node(h.id()).kind {
            Kind::Member(k) => k,
            _ => unreachable!("param handle into a non-param node"),
        }
    }
    pub fn block(&self, h: NodeHandle<BlockNode>) -> &[NodeHandle<AnyStmt>] {
        match self.expr(h.into()) {
            ExprKind::Block { stmts } => stmts,
            _ => unreachable!("block handle into a non-block node"),
        }
    }
    /// checked narrowing: block-or-expr slots (arm / lambda bodies)
    pub fn narrow_block(&self, h: NodeHandle<AnyExpr>) -> Option<NodeHandle<BlockNode>> {
        match self.expr(h) {
            ExprKind::Block { .. } => Some(NodeHandle::new(h.id())),
            _ => None,
        }
    }
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
    /// generic arguments — type nodes (incl. const-generic `TyConst`)
    pub generics: Vec<NodeHandle<AnyTy>>,
}

#[derive(Clone, Debug)]
pub enum Kind {
    Item(ItemKind),
    Member(MemberKind),
    Stmt(StmtKind),
    Expr(ExprKind),
    Pat(PatKind),
    Arm(ArmKind),
    Type(TypeKind),
}

pub struct Node {
    pub span: Span,
    pub kind: Kind,
}

// ---- items ----

#[derive(Clone, Debug)]
pub struct FnData {
    pub vis: Vis,
    pub is_suspend: bool,
    pub name: IdentId,
    pub generics: Vec<IdentId>,
    pub params: Vec<NodeHandle<AnyParam>>,
    pub ret: Option<NodeHandle<AnyTy>>,
    /// RFC 0013 §2 — admission-only
    pub where_bounds: Vec<(IdentId, NodeHandle<AnyTy>)>,
    pub body: NodeHandle<BlockNode>,
}

#[derive(Clone, Debug)]
pub enum ItemKind {
    Module { items: Vec<NodeHandle<AnyItem>> },
    Import { names: Vec<IdentId>, from: String },
    ModuleLet { vis: Vis, name: IdentId, ty: Option<NodeHandle<AnyTy>>, init: NodeHandle<AnyExpr> },
    Enum { vis: Vis, name: IdentId, members: Vec<(IdentId, Option<i64>)> },
    Dataclass {
        vis: Vis,
        name: IdentId,
        generics: Vec<IdentId>,
        fields: Vec<NodeHandle<FieldDeclNode>>,
        methods: Vec<NodeHandle<MethodDeclNode>>,
    },
    Class {
        vis: Vis,
        name: IdentId,
        generics: Vec<IdentId>,
        fields: Vec<NodeHandle<FieldDeclNode>>,
        methods: Vec<NodeHandle<MethodDeclNode>>,
    },
    Trait {
        vis: Vis,
        name: IdentId,
        generics: Vec<IdentId>,
        requires: Vec<NodeHandle<AnyTy>>, // type nodes — naming position, bare
        methods: Vec<NodeHandle<MethodDeclNode>>, // bodiless MethodDecls
    },
    Impl {
        trait_ref: NodeHandle<AnyTy>,
        target: NodeHandle<AnyTy>,
        methods: Vec<NodeHandle<MethodDeclNode>>,
    },
    /// `host fn` / `extern fn` — .d.rut only (RFC 0030 §3)
    SurfaceFn {
        vis: Vis,
        linkage: Linkage,
        name: IdentId,
        generics: Vec<IdentId>,
        params: Vec<NodeHandle<AnyParam>>,
        ret: Option<NodeHandle<AnyTy>>,
    },
    /// `host class` / `extern class` — .d.rut only
    SurfaceClass {
        vis: Vis,
        linkage: Linkage,
        name: IdentId,
        extparams: Vec<(IdentId, Option<NodeHandle<AnyTy>>)>,
        members: Vec<NodeHandle<MethodDeclNode>>, // bodiless
    },
    Fn(FnData),
}

// ---- members ----

/// dataclass/class field: `private`? `static`? name: ty (= init)?
#[derive(Clone, Debug)]
pub struct FieldDeclData {
    pub is_private: bool,
    pub is_static: bool,
    pub name: IdentId,
    pub ty: NodeHandle<AnyTy>,
    pub init: Option<NodeHandle<AnyExpr>>,
}

/// `private`? `suspend`? fn name<..>(self, ..): T { .. }
#[derive(Clone, Debug)]
pub struct MethodDeclData {
    pub is_private: bool,
    pub is_suspend: bool,
    pub name: IdentId,
    pub generics: Vec<IdentId>,
    pub params: Vec<NodeHandle<AnyParam>>, // Param / SelfParam
    pub ret: Option<NodeHandle<AnyTy>>,
    pub body: Option<NodeHandle<BlockNode>>, // None in traits / surface classes
}

/// `mut`? name: ty — or a type-less lambda param `(x)`
#[derive(Clone, Debug)]
pub struct ParamData {
    pub is_mut: bool,
    pub name: IdentId,
    pub ty: Option<NodeHandle<AnyTy>>,
}

/// `self` / `mut self`
#[derive(Clone, Debug)]
pub struct SelfParamData {
    pub is_mut: bool,
}

#[derive(Clone, Debug)]
pub enum MemberKind {
    FieldDecl(FieldDeclData),
    MethodDecl(MethodDeclData),
    Param(ParamData),
    SelfParam(SelfParamData),
}

// ---- statements ----

/// `else if` chains re-enter `If`; plain `else` is a block
#[derive(Clone, Copy, Debug)]
pub enum ElseBranch {
    If(NodeHandle<IfNode>),
    Block(NodeHandle<BlockNode>),
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    LetStmt { is_mut: bool, name: IdentId, ty: Option<NodeHandle<AnyTy>>, init: NodeHandle<AnyExpr> },
    If { cond: NodeHandle<AnyExpr>, then: NodeHandle<BlockNode>, els: Option<ElseBranch> },
    While { cond: NodeHandle<AnyExpr>, body: NodeHandle<BlockNode> },
    ForOf { var: IdentId, iter: NodeHandle<AnyExpr>, body: NodeHandle<BlockNode> },
    ForC { var: IdentId, init: NodeHandle<AnyExpr>, cond: NodeHandle<AnyExpr>, update: NodeHandle<AnyExpr>, body: NodeHandle<BlockNode> },
    Return { value: Option<NodeHandle<AnyExpr>> },
    Break,
    Continue,
    WhenStmt { scrut: NodeHandle<AnyExpr>, arms: Vec<NodeHandle<AnyArm>> },
    ExprStmt(NodeHandle<AnyExpr>),
}

// ---- arms ----

#[derive(Clone, Debug)]
pub enum ArmKind {
    WhenArm { pats: Vec<NodeHandle<AnyPat>>, body: NodeHandle<AnyExpr> },
    /// `fut -> body` / `fut as name -> body` (RFC 0019 §3)
    SelectArm { fut: NodeHandle<AnyExpr>, bind: Option<IdentId>, body: NodeHandle<AnyExpr> },
}

// ---- patterns (RFC 0008 §2; constructor/binding forms per corpus) ----

#[derive(Clone, Debug)]
pub enum PatKind {
    /// literal pattern (int/float/bool/char/string) — expr node, literal only
    PatLit(NodeHandle<AnyExpr>),
    /// enum-member path: `Color.Red`
    PatPath { segs: Vec<PathSeg> },
    /// constructor pattern: `Ok(cfg)`, `Option<T>.Some(x)`, `Some(_)`,
    /// payloadless `Option<T>.None`; args are binding names (`None` = `_`)
    PatCtor { segs: Vec<PathSeg>, args: Vec<Option<IdentId>> },
    PatWild,
    PatElse,
}

// ---- types ----

/// `TypeKind` (not `TyKind`) — rut-core's type-table kind owns that name
#[derive(Clone, Debug)]
pub enum TypeKind {
    /// path type, optionally `dyn`-prefixed in value positions (RFC 0030 §2)
    TyPath { segs: Vec<PathSeg>, is_dyn: bool },
    /// fn type: `fn(Store, P): R` — params are bare types (RFC 0013 §1)
    TyFn { params: Vec<NodeHandle<AnyTy>>, ret: NodeHandle<AnyTy> },
    /// const-generic argument (the `N` in `Array<T, N>`) — an expression
    TyConst(NodeHandle<AnyExpr>),
}

// ---- expressions ----

#[derive(Clone, Debug)]
pub enum ExprKind {
    /// block expression — fn bodies, loop bodies, block arms/lambda bodies;
    /// its value is the last statement's (unit if none)
    Block { stmts: Vec<NodeHandle<AnyStmt>> },
    Lit(Lit),
    /// dotted path, possibly with generic args on any segment:
    /// `primes`, `Color.Red`, `Vec.from`, `MyMap<K, V>.new`, `Self`
    Path { segs: Vec<PathSeg> },
    Call { callee: NodeHandle<AnyExpr>, args: Vec<NodeHandle<AnyExpr>> },
    Method { recv: NodeHandle<AnyExpr>, name: IdentId, generics: Vec<NodeHandle<AnyTy>>, args: Vec<NodeHandle<AnyExpr>> },
    Field { recv: NodeHandle<AnyExpr>, name: IdentId },
    Index { recv: NodeHandle<AnyExpr>, idx: NodeHandle<AnyExpr> },
    Unary { op: UnOp, expr: NodeHandle<AnyExpr> },
    Binary { op: BinOp, lhs: NodeHandle<AnyExpr>, rhs: NodeHandle<AnyExpr> },
    Assign { op: Option<BinOp>, target: NodeHandle<AnyExpr>, value: NodeHandle<AnyExpr> },
    Lambda { params: Vec<NodeHandle<AnyParam>>, ret: Option<NodeHandle<AnyTy>>, body: NodeHandle<AnyExpr> },
    Try { expr: NodeHandle<AnyExpr> }, // postfix `?`
    FStr { parts: Vec<FPartAst> },
    /// dataclass / `Self { .. }` literal — classes have no instance literal
    Struct { ty: NodeHandle<AnyTy>, fields: Vec<(IdentId, NodeHandle<AnyExpr>)> },
    /// `[e1, .., en] : Array<T, n>` (RFC 0007 §1)
    ArrayLit { elems: Vec<NodeHandle<AnyExpr>> },
    WhenExpr { scrut: NodeHandle<AnyExpr>, arms: Vec<NodeHandle<AnyArm>> },
    Await { expr: NodeHandle<AnyExpr> },
    /// `select { ... }` — only reachable as `await select { .. }` (RFC 0019 §3)
    Select { arms: Vec<NodeHandle<AnyArm>> },
    /// `expr is Type` — relational precedence, non-associative (RFC 0012 §3)
    Is { expr: NodeHandle<AnyExpr>, ty: NodeHandle<AnyTy> },
}

#[derive(Clone, Debug)]
pub enum FPartAst {
    Lit(String),
    Hole(NodeHandle<AnyExpr>),
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
