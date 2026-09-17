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

use rut_lexer::span::Span;
use std::marker::PhantomData;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(pub u32);

/// An interned name — re-exported from rut-core (RFC 0030 §5): the AST's
/// interner and rut-core's are one type, so well-known symbol ids
/// (`rut_core::sym::SELF` and friends) are valid in the AST's interner.
pub use rut_core::IdentId;

// ---- typed handles ----

/// A `NodeId` that carries the node's category (or exact variant) in its
/// type. Zero-sized at runtime.
pub struct NodeHandle<T> {
    id: NodeId,
    _ty: PhantomData<fn() -> T>,
}

impl<T> NodeHandle<T> {
    pub fn new(id: NodeId) -> Self {
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
/// Lives in rut-core so well-known symbols and every compiled artifact
/// share one representation; re-exported here for the parser and the AST.
pub use rut_core::Interner;

// ---- visibility (RFC 0003 §2) ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vis {
    /// `pub` — usable from anywhere
    Pub,
    /// `pub(mod)`
    Mod,
    /// `pub(super)`
    Super,
    /// `pub(self)` / unannotated module items **and members** — module-private
    /// (the default)
    Self_,
}

// ---- linkage (.d.rut surface decls, RFC 0029 §2 / RFC 0030 §3) ----

/// Who implements a surface declaration. The `extern` kind (a linked rut
/// package) is gone: rut→rut use statements go through the module loader and
/// host→rut entry points are `entry fn` — `host` is the one foreign-body
/// case left (embedding Rust).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Linkage {
    /// `host` — the embedding Rust (a registered `NativeModule`)
    Host,
    /// `builtin` — the engine itself, compiler-lowered (std:core only;
    /// nothing to register, the decl is a pure signature contract)
    Builtin,
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
    pub is_async: bool,
    /// `entry fn` — host-callable (RFC 0035 §3): lands in the binary's
    /// entry table; its signature must satisfy the crossing rule
    /// (RFC 0023 §2), checked at compile time
    pub entry: bool,
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
    Use { names: Vec<IdentId>, from: String },
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
    /// `host fn` / `builtin fn` — .d.rut only (RFC 0030 §3). `host`:
    /// embedding-Rust body, concrete signature over the crossing set
    /// (RFC 0023 §1); `builtin`: engine-lowered (generics allowed —
    /// nothing crosses a boundary).
    SurfaceFn {
        vis: Vis,
        linkage: Linkage,
        name: IdentId,
        generics: Vec<IdentId>,
        params: Vec<NodeHandle<AnyParam>>,
        ret: Option<NodeHandle<AnyTy>>,
    },
    /// `host struct` — .d.rut only: a flat record whose every field is
    /// a crossing type. Host fns take/return it; the host constructs and
    /// reads it through the field table (the shape is the whole surface).
    SurfaceDataclass {
        vis: Vis,
        name: IdentId,
        fields: Vec<NodeHandle<FieldDeclNode>>,
    },
    /// `builtin Name<..>` — .d.rut only, std:core only: an engine builtin
    /// type's member contract (`Option`, `Result`, `Opaque`, `Array`).
    /// Members are compiler-lowered (ops, RFC 0032 §1.1) — the decl exists
    /// so users and the LSP see every signature; no impl ever registers.
    BuiltinTy {
        vis: Vis,
        name: IdentId,
        generics: Vec<IdentId>,
        members: Vec<NodeHandle<MethodDeclNode>>, // bodiless
    },
    /// `builtin trait Name<..>` — .d.rut only, std:core only: a
    /// trait the ENGINE is woven into (compiler-backed impls /
    /// lowering hooks — `x[i]` through `Index`, `for (x of it)` through
    /// `Iterator`, rc-0 `Disposal`). Users still implement it with
    /// ordinary `impl` blocks; the `builtin` marker is the engine's
    /// reservation, not an access rule. Library contracts without
    /// engine knowledge (`Hashable`) stay plain `trait`.
    BuiltinTrait {
        vis: Vis,
        name: IdentId,
        generics: Vec<IdentId>,
        methods: Vec<NodeHandle<MethodDeclNode>>, // bodiless
    },
    Fn(FnData),
}

// ---- members ----

/// struct/class field: `pub(..)`? `static`? name: ty (= init)?
/// `vis: None` = unannotated — module-private, the RFC 0003 §2 default
#[derive(Clone, Debug)]
pub struct FieldDeclData {
    pub vis: Option<Vis>,
    pub is_static: bool,
    pub name: IdentId,
    pub ty: NodeHandle<AnyTy>,
    pub init: Option<NodeHandle<AnyExpr>>,
}

/// `pub(..)`? `async`? fn name<..>(self, ..) -> T { .. }
/// `vis: None` = unannotated — module-private, the RFC 0003 §2 default
#[derive(Clone, Debug)]
pub struct MethodDeclData {
    pub vis: Option<Vis>,
    pub is_async: bool,
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
    LetStmt { is_mut: bool, name: IdentId, destructure: Option<Vec<IdentId>>, ty: Option<NodeHandle<AnyTy>>, init: NodeHandle<AnyExpr> },
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
    /// path type — a trait name in type position is the object type
    /// (RFC 0030 §2); the bare name spells it
    TyPath { segs: Vec<PathSeg> },
    /// fn type: `fn(Store, P) -> R` — params are bare types (RFC 0013 §1)
    TyFn { params: Vec<NodeHandle<AnyTy>>, ret: NodeHandle<AnyTy> },
    /// pointer type `*T` (RFC 0005) — nil-able, rc-backed reference
    TyPtr { inner: NodeHandle<AnyTy> },
    /// tuple type `(A, B, ..)` — a record with numeric fields (RFC 0007)
    TyTuple { elems: Vec<NodeHandle<AnyTy>> },
    /// const-generic argument (the `N` in `Array<T, N>`) — an expression
    TyConst(NodeHandle<AnyExpr>),
}

// ---- expressions ----

#[derive(Clone, Debug)]
pub enum ExprKind {
    /// block expression — fn bodies, loop bodies, block arms/lambda bodies;
    /// its value is the last statement's (nil if none)
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
    /// struct / `Self { .. }` literal — classes have no instance literal
    Struct { ty: NodeHandle<AnyTy>, fields: Vec<(IdentId, NodeHandle<AnyExpr>)> },
    /// tuple expression `(a, b, ..)` — a record with numeric fields
    /// (RFC 0007; `()` is rejected — the empty value is `nil`, v1.2)
    Tuple { elems: Vec<NodeHandle<AnyExpr>> },
    /// `[e1, .., en] : Array<T, n>` (RFC 0007 §1)
    ArrayLit { elems: Vec<NodeHandle<AnyExpr>> },
    WhenExpr { scrut: NodeHandle<AnyExpr>, arms: Vec<NodeHandle<AnyArm>> },
    Await { expr: NodeHandle<AnyExpr> },
    /// `select { ... }` — only reachable as `await select { .. }` (RFC 0019 §3)
    Select { arms: Vec<NodeHandle<AnyArm>> },
    /// `expr is Type` — relational precedence, non-associative (RFC 0012 §3)
    Is { expr: NodeHandle<AnyExpr>, ty: NodeHandle<AnyTy> },
    /// `expr as T` — the numeric cast, truncating like C/Rust; binds
    /// tighter than `*`, left-associative, RHS a naming position
    /// restricted to the numeric primitives (RFC 0007 §1)
    Cast { expr: NodeHandle<AnyExpr>, ty: NodeHandle<AnyTy> },
}

#[derive(Clone, Debug)]
pub enum FPartAst {
    Lit(String),
    Hole(NodeHandle<AnyExpr>),
}

#[derive(Clone, Debug)]
pub enum Lit {
    Int(u64, Option<rut_lexer::token::IntSuffix>),
    Float(u64 /*f64 bits*/, Option<rut_lexer::token::FloatSuffix>),
    Str(String),
    RawStr(String),
    Bool(bool),
    /// `nil` — the null pointer literal (RFC 0005)
    Nil,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
    /// `*p` — pointer dereference, copies the pointee out (RFC 0005)
    Deref,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or, // logical short-circuit
    BitAnd, BitOr, BitXor, Shl, Shr,
}
