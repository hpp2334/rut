//! The typed-arena AST — the tree half of the crate.
//!
//! THE CONTRACT (see AGENTS.md):
//! - every node kind is its own struct; span + name_span + doc live
//!   IN the node (D1); per-kind private arenas (D2)
//! - `Ast` is a CAPSULE: plain mut doors for the parse lane
//!   (`new`/`intern`/`set_root`/`alloc`/`set_errors` — `pub(crate)`:
//!   the parse lane lives in this crate), reads
//!   (`root`/`name`/`errors`/`is_clean`) — behavior binds to
//!   nodes/handles with the store as ctx (D4/D5)
//! - `Ast` holds NO src — `Parsed { ast, src }` is the pair artifact;
//!   text queries are span×src joins (`Span::text(src)`) (law v3)
//! - every child list is SLOTS (`Vec<XxxRef>`), every slot enum carries
//!   `Error(Handle<ErrorNode>)` — the recovery ladder (D10)
//! - docs ride decl nodes as `DocData` (spans, never text), populated
//!   at allocation when `include_comments` is set (D7)
//!
//! This module re-exports the whole vocabulary FLAT (and the crate
//! root globs it) — consumers never spell a module path; file layout
//! stays free to move.

mod store;
mod doc;
mod dump;
mod error;
mod expr;
mod handle;
mod item;
mod pat;
mod stmt;
mod ty;

pub use store::Ast;
pub use doc::{CommentData, DocData};
pub use error::ErrorNode;
pub use handle::{Arena, Handle, NodeStore};
// the construction macros ride the same chain (crate-root reachable,
// crate-only visible — the capsule's allocation lane)
pub(crate) use handle::{plain_alloc, slot_alloc, slot_span};
pub use rut_core::IdentId;
pub use rut_lexer::span::Span;

// items
pub use item::{
    AliasNode, BuiltinImplNode, BuiltinPrimitiveNode, BuiltinTyNode, ClassNode, EnumMemberNode,
    EnumNode, FieldDeclNode, FieldRef, FnNode, ImplNode, InterfaceNode, ItemRef, Linkage,
    MemberRef, MethodDeclNode, MethodRef, ModDeclNode, ModInlineNode, ModKind, ModifierNode,
    ModuleLetNode, ModuleNode, ParamNode, ParamRef, SelfParamNode, StructNode, SurfaceFnNode,
    SurfaceStructNode, UseNode, Vis,
};
// statements
pub use stmt::{
    BreakNode, ContinueNode, ElseBranch, ForCNode, ForOfNode, IfNode, LetStmtNode, ReturnNode,
    StmtRef, WhenStmtNode, WhileNode,
};
// expressions
pub use expr::{
    ArrayLitNode, ArrayRepeatNode, AssignNode, AsyncBlockNode, AwaitNode, BinOp, BinaryNode,
    BlockNode, CallNode, CastNode, ExprRef, FPartAst, FStrNode, FieldNode, IfExprNode, IndexNode,
    IsNode, LambdaNode, Lit, LitNode, MethodNode, PathNode, PathSeg, StructLitNode, TryNode,
    TupleNode, UnOp, UnaryNode, WhenArmNode, WhenExprNode,
};
// types + patterns
pub use pat::{PatCtorNode, PatElseNode, PatLitNode, PatPathNode, PatRef, PatWildNode};
pub use ty::{
    TyArrayNode, TyConstNode, TyFnNode, TyOptNode, TyPathNode, TyRef, TyTupleNode, TyUnionNode,
};

pub use dump::{render_json, render_text, to_dump_tree, DumpNode};
