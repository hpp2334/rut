//! The typed-arena AST — the tree half of the crate.
//!
//! THE CONTRACT (see AGENTS.md):
//! - this module is SEALED (`mod ast;` at the crate root — the same
//!   privacy wall as rut-lsp's `doc`): the vocabulary is
//!   reachable only through the flat re-exports below (which the
//!   crate root globs), never an `ast::` module path
//! - every node kind is its own struct; span + name + doc live
//!   IN the node (D1); per-kind private arenas (D2)
//! - `Ast` is a CAPSULE: plain mut doors for the parse lane
//!   (`new`/`intern`/`set_root`/`alloc`/`set_errors` — `pub(crate)`:
//!   they are methods on a publicly re-exported type, so the marker
//!   itself is the enforcement), reads
//!   (`root`/`name`/`errors`/`is_clean`) — behavior binds to
//!   nodes/handles with the store as ctx (D4/D5)
//! - `Ast` holds NO src — the [`Doc`](crate::Doc) is the artifact (the
//!   pipeline owns text + tokens + tree); text queries are
//!   `Doc::text(span)` joins (law v4)
//! - every child list is SLOTS (`Vec<XxxRef>`), every slot enum carries
//!   `Error(Handle<ErrorNode>)` — the recovery ladder (D10)
//! - docs ride decl nodes as `DocData` (spans, never text), populated
//!   at allocation — always on (D7)
//!
//! This module re-exports the whole vocabulary FLAT (and the crate
//! root globs it) — consumers never spell a module path; file layout
//! stays free to move.

mod doc;
mod error;
mod expr;
mod handle;
mod index;
mod item;
mod name;
mod pat;
mod printer;
mod stmt;
mod store;
mod ty;

pub use doc::{CommentData, DocData};
pub use error::ErrorNode;
pub use handle::{AnyHandle, Arena, Handle, NodeKind, NodeStore};
pub use name::Name;
pub use store::{Ast, NodeKindId};
// the construction macros ride the same chain (crate-root reachable
// via the root gate, crate-only visible — the capsule's allocation
// lane). The marker here is LOAD-BEARING, not redundant: macro_rules
// items are module-private at definition (no vis syntax), so a `pub
// use` binding of one caps at pub(crate) and E0364 refuses the `pub`
// spelling — the seal alone cannot publicize this chain.
pub(crate) use handle::{plain_alloc, slot_alloc, slot_any, slot_span};
// the edit lane's dispatch — the Doc pipeline (crate::document) walks
// and punches subtrees through these
pub(crate) use index::children_of;
pub use rut_core::IdentId;
pub use rut_lexer::span::Span;
pub(crate) use store::remove_any;

// items
pub use item::{
    AliasNode, BuiltinPrimitiveNode, BuiltinTyNode, ClassNode, EnumMemberNode, EnumNode,
    FieldDeclNode, FieldRef, FnNode, ImplNode, InterfaceNode, ItemRef, Linkage, MemberRef,
    MethodDeclNode, MethodRef, ModDeclNode, ModKind, ModifierNode, ModuleLetNode, ModuleNode,
    ParamNode, ParamRef, SelfParamNode, StructNode, SurfaceFnNode, SurfaceStructNode, UseNode, Vis,
};
// statements
pub use stmt::{
    BreakNode, ContinueNode, ElseBranch, ForCNode, ForOfNode, IfNode, LetStmtNode, ReturnNode,
    StmtRef, WhenStmtNode, WhileNode,
};
// expressions
pub use expr::{
    ArrayLitNode, ArrayRepeatNode, AssertNode, AssignNode, AsyncBlockNode, AwaitNode, BinOp,
    BinaryNode, BlockNode, CallNode, CastNode, ExprRef, FPartAst, FStrNode, FieldInit, FieldNode,
    IfExprNode, IndexNode, IsNode, LambdaNode, Lit, LitNode, MethodNode, PathNode, PathSeg,
    SelfNode, StructLitNode, TupleNode, UnOp, UnaryNode, WhenArmNode, WhenExprNode,
};
// types + patterns
pub use pat::{PatElseNode, PatLitNode, PatPathNode, PatRef, PatWildNode};
pub use ty::{
    SelfTyNode, TyArrayNode, TyFnNode, TyOptNode, TyPathNode, TyRef, TyTupleNode, TyUnionNode,
};

// the tree renderer — AstPrinter + PrintNode are the whole face; the
// value companions are private impl
pub use printer::{AstPrinter, PrintNode};
