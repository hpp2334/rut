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

use crate::handle::{Arena, Handle, NodeStore};

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
            root: Option<Handle<crate::item::ModuleNode>>,
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
            pub fn new() -> Ast {
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
            pub fn alloc<T: NodeStore>(&mut self, node: T) -> Handle<T> {
                let h = Handle::new(T::arena_mut(self).0.len() as u32);
                T::arena_mut(self).0.push(node);
                h
            }
            /// construction (parse lane): intern a name
            pub fn intern(&mut self, name: &str) -> IdentId {
                self.interner.intern(name)
            }
            /// construction (parse lane): seal the root
            pub fn set_root(&mut self, root: Handle<crate::item::ModuleNode>) {
                self.root = Some(root);
            }
            /// construction (parse lane): the ordered diag rows
            pub fn set_errors(&mut self, diags: Vec<Diag>) {
                self.errors = diags;
            }

            // ---- reads ----
            /// the root's Copy handle — the walk door
            pub fn root(&self) -> Handle<crate::item::ModuleNode> {
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
        $( impl crate::handle::HasSpan for $ty {
            fn span(&self) -> rut_lexer::span::Span { self.span }
        } )*
    };
}

arenas! {
    // items
    modules: crate::item::ModuleNode,
    mod_decls: crate::item::ModDeclNode,
    mod_inlines: crate::item::ModInlineNode,
    uses: crate::item::UseNode,
    aliases: crate::item::AliasNode,
    module_lets: crate::item::ModuleLetNode,
    enums: crate::item::EnumNode,
    structs: crate::item::StructNode,
    classes: crate::item::ClassNode,
    interfaces: crate::item::InterfaceNode,
    impls: crate::item::ImplNode,
    fns: crate::item::FnNode,
    surface_fns: crate::item::SurfaceFnNode,
    surface_structs: crate::item::SurfaceStructNode,
    builtin_tys: crate::item::BuiltinTyNode,
    builtin_primitives: crate::item::BuiltinPrimitiveNode,
    builtin_impls: crate::item::BuiltinImplNode,
    // members
    field_decls: crate::item::FieldDeclNode,
    method_decls: crate::item::MethodDeclNode,
    enum_members: crate::item::EnumMemberNode,
    params: crate::item::ParamNode,
    self_params: crate::item::SelfParamNode,
    // statements
    let_stmts: crate::stmt::LetStmtNode,
    if_stmts: crate::stmt::IfNode,
    whiles: crate::stmt::WhileNode,
    for_ofs: crate::stmt::ForOfNode,
    for_cs: crate::stmt::ForCNode,
    returns: crate::stmt::ReturnNode,
    breaks: crate::stmt::BreakNode,
    continues: crate::stmt::ContinueNode,
    when_stmts: crate::stmt::WhenStmtNode,
    // expressions
    blocks: crate::expr::BlockNode,
    lits: crate::expr::LitNode,
    paths: crate::expr::PathNode,
    calls: crate::expr::CallNode,
    methods: crate::expr::MethodNode,
    fields: crate::expr::FieldNode,
    indexes: crate::expr::IndexNode,
    unaries: crate::expr::UnaryNode,
    binaries: crate::expr::BinaryNode,
    assigns: crate::expr::AssignNode,
    lambdas: crate::expr::LambdaNode,
    tries: crate::expr::TryNode,
    fstrs: crate::expr::FStrNode,
    struct_lits: crate::expr::StructLitNode,
    tuples: crate::expr::TupleNode,
    array_lits: crate::expr::ArrayLitNode,
    array_repeats: crate::expr::ArrayRepeatNode,
    when_exprs: crate::expr::WhenExprNode,
    if_exprs: crate::expr::IfExprNode,
    awaits: crate::expr::AwaitNode,
    async_blocks: crate::expr::AsyncBlockNode,
    is_exprs: crate::expr::IsNode,
    casts: crate::expr::CastNode,
    // types
    ty_paths: crate::ty::TyPathNode,
    ty_fns: crate::ty::TyFnNode,
    ty_opts: crate::ty::TyOptNode,
    ty_arrays: crate::ty::TyArrayNode,
    ty_tuples: crate::ty::TyTupleNode,
    ty_consts: crate::ty::TyConstNode,
    ty_unions: crate::ty::TyUnionNode,
    // patterns
    pat_lits: crate::pat::PatLitNode,
    pat_paths: crate::pat::PatPathNode,
    pat_ctors: crate::pat::PatCtorNode,
    pat_wilds: crate::pat::PatWildNode,
    pat_elses: crate::pat::PatElseNode,
    // arms + recovery
    when_arms: crate::expr::WhenArmNode,
    error_nodes: crate::error::ErrorNode,
}
