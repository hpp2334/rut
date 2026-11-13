//! The capsule store (D4): per-kind private arenas, the interner, the
//! root — and NOTHING else. NO `src` (the text is the caller's —
//! `Parsed { ast, src }` is the pair artifact; text queries are span×src
//! joins), no flat kind enum, no public fields.
//!
//! THE ARTIFACT LAW (v3): `Ast` stores PURE STRUCTURE — the typed
//! nodes, their spans/verdicts, the interner vocabulary. Every door is
//! a method; reads live on nodes/handles with the store as ctx.
//!
//! THE MUTATION LAW: `&ctx` methods ask the store, `&mut ctx` methods
//! tell it — and only the parse lane tells (`intern`, `set_root`,
//! `allocate`, `set_errors`). Node fields are never mutated after
//! allocation; there is no API for it.

use rut_core::{IdentId, Interner};
use rut_lexer::diag::Diag;

use crate::ast::handle::{Arena, Handle, NodeStore};

// ---- the node inventory: one private arena per kind ----
//
// Every node kind in the tree, exactly once. The macro wires the
// fields, the NodeStore impls (typed `allocate`/`get`), nothing else —
// behavior lives on the nodes.
macro_rules! arenas {
    ($( $field:ident : $ty:ty ),* $(,)?) => {
        pub struct Ast {
            $( $field: Arena<$ty>, )*
            interner: Interner,
            root: Option<Handle<crate::ast::item::ModuleNode>>,
            /// the ordered diag rows (the FLAT view — the panel). The
            /// POSITIONAL view is the `ErrorNode`s in the streams; the
            /// same rows, two shapes. Parse-lane-written only.
            errors: Vec<Diag>,
        }

        impl Ast {
            /// the empty store — the parse lane's first call
            #[allow(clippy::new_without_default)] // a Default would
            // suggest Ast::default() elsewhere; the store is BORN in
            // the parse lane, one constructor, one story
            pub(crate) fn new() -> Ast {
                Ast {
                    $( $field: Arena::empty(), )*
                    interner: Interner::default(),
                    root: None,
                    errors: Vec::new(),
                }
            }
            /// construction (parse lane): land a complete node in its
            /// arena — the one allocation door behind every
            /// `XNode { .. }.allocate(ctx)`
            pub(crate) fn alloc<T: NodeStore>(&mut self, node: T) -> Handle<T> {
                let h = Handle::new(T::arena_mut(self).0.len() as u32);
                T::arena_mut(self).0.push(node);
                h
            }
            /// construction (parse lane): intern a name
            pub(crate) fn intern(&mut self, name: &str) -> IdentId {
                self.interner.intern(name)
            }
            /// construction (parse lane): seal the root
            pub(crate) fn set_root(&mut self, root: Handle<crate::ast::item::ModuleNode>) {
                self.root = Some(root);
            }
            /// construction (parse lane): the ordered diag rows
            pub(crate) fn set_errors(&mut self, diags: Vec<Diag>) {
                self.errors = diags;
            }

            // ---- reads ----
            /// the root's Copy handle — the walk door
            pub fn root(&self) -> Handle<crate::ast::item::ModuleNode> {
                self.root.expect("the parse lane always sets the root")
            }
            /// a name's spelling — the interner read
            pub fn name(&self, id: IdentId) -> &str {
                self.interner.name(id)
            }
            /// the whole vocabulary, by reference — the middle-end's
            /// door: its `Ctx` CLONES the interner (extending it with
            /// synthesized names and moving the result into the emitted
            /// `Program`), so it needs the store itself, not per-name
            /// reads. Read-only — interning stays the parse lane's.
            pub fn interner(&self) -> &Interner {
                &self.interner
            }
            /// the FLAT error view, source order (the panel). The
            /// positional view is the ErrorNodes in the streams.
            pub fn errors(&self) -> &[Diag] {
                &self.errors
            }
            /// O(1) — the fmt/compiler refusal gate
            pub fn is_clean(&self) -> bool {
                self.errors.is_empty()
            }
        }

        $( impl NodeStore for $ty {
            fn arena(ast: &Ast) -> &Arena<Self> { &ast.$field }
            fn arena_mut(ast: &mut Ast) -> &mut Arena<Self> { &mut ast.$field }
        } )*
        $( impl crate::ast::handle::HasSpan for $ty {
            fn span(&self) -> rut_lexer::span::Span { self.span }
        } )*
    };
}

arenas! {
    // items
    modules: crate::ast::item::ModuleNode,
    mod_decls: crate::ast::item::ModDeclNode,
    mod_inlines: crate::ast::item::ModInlineNode,
    uses: crate::ast::item::UseNode,
    aliases: crate::ast::item::AliasNode,
    module_lets: crate::ast::item::ModuleLetNode,
    enums: crate::ast::item::EnumNode,
    structs: crate::ast::item::StructNode,
    classes: crate::ast::item::ClassNode,
    interfaces: crate::ast::item::InterfaceNode,
    impls: crate::ast::item::ImplNode,
    fns: crate::ast::item::FnNode,
    surface_fns: crate::ast::item::SurfaceFnNode,
    surface_structs: crate::ast::item::SurfaceStructNode,
    builtin_tys: crate::ast::item::BuiltinTyNode,
    builtin_primitives: crate::ast::item::BuiltinPrimitiveNode,
    builtin_impls: crate::ast::item::BuiltinImplNode,
    // members
    field_decls: crate::ast::item::FieldDeclNode,
    method_decls: crate::ast::item::MethodDeclNode,
    enum_members: crate::ast::item::EnumMemberNode,
    params: crate::ast::item::ParamNode,
    self_params: crate::ast::item::SelfParamNode,
    // statements
    let_stmts: crate::ast::stmt::LetStmtNode,
    if_stmts: crate::ast::stmt::IfNode,
    whiles: crate::ast::stmt::WhileNode,
    for_ofs: crate::ast::stmt::ForOfNode,
    for_cs: crate::ast::stmt::ForCNode,
    returns: crate::ast::stmt::ReturnNode,
    breaks: crate::ast::stmt::BreakNode,
    continues: crate::ast::stmt::ContinueNode,
    when_stmts: crate::ast::stmt::WhenStmtNode,
    // expressions
    blocks: crate::ast::expr::BlockNode,
    lits: crate::ast::expr::LitNode,
    paths: crate::ast::expr::PathNode,
    calls: crate::ast::expr::CallNode,
    methods: crate::ast::expr::MethodNode,
    fields: crate::ast::expr::FieldNode,
    indexes: crate::ast::expr::IndexNode,
    unaries: crate::ast::expr::UnaryNode,
    binaries: crate::ast::expr::BinaryNode,
    assigns: crate::ast::expr::AssignNode,
    lambdas: crate::ast::expr::LambdaNode,
    tries: crate::ast::expr::TryNode,
    fstrs: crate::ast::expr::FStrNode,
    struct_lits: crate::ast::expr::StructLitNode,
    tuples: crate::ast::expr::TupleNode,
    array_lits: crate::ast::expr::ArrayLitNode,
    array_repeats: crate::ast::expr::ArrayRepeatNode,
    when_exprs: crate::ast::expr::WhenExprNode,
    if_exprs: crate::ast::expr::IfExprNode,
    awaits: crate::ast::expr::AwaitNode,
    async_blocks: crate::ast::expr::AsyncBlockNode,
    is_exprs: crate::ast::expr::IsNode,
    casts: crate::ast::expr::CastNode,
    // types
    ty_paths: crate::ast::ty::TyPathNode,
    ty_fns: crate::ast::ty::TyFnNode,
    ty_opts: crate::ast::ty::TyOptNode,
    ty_arrays: crate::ast::ty::TyArrayNode,
    ty_tuples: crate::ast::ty::TyTupleNode,
    ty_consts: crate::ast::ty::TyConstNode,
    ty_unions: crate::ast::ty::TyUnionNode,
    // patterns
    pat_lits: crate::ast::pat::PatLitNode,
    pat_paths: crate::ast::pat::PatPathNode,
    pat_ctors: crate::ast::pat::PatCtorNode,
    pat_wilds: crate::ast::pat::PatWildNode,
    pat_elses: crate::ast::pat::PatElseNode,
    // arms + recovery
    when_arms: crate::ast::expr::WhenArmNode,
    error_nodes: crate::ast::error::ErrorNode,
}
