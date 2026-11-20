//! The capsule store (D4): per-kind private arenas, the interner, the
//! root — and NOTHING else. NO `src` (the text is the caller's —
//! `Parsed { ast, src }` is the pair artifact; text queries are span×src
//! joins), no flat kind enum, no public fields.
//!
//! THE ARTIFACT LAW (v3): `Ast` stores PURE STRUCTURE — the typed
//! nodes, their spans/verdicts, the interner vocabulary, the derived
//! [`TreeIndex`]. Every door is a method; reads live on
//! nodes/handles with the store as ctx.
//!
//! THE MUTATION LAW: `&ctx` methods ask the store, `&mut ctx` methods
//! tell it — and only the parse lane tells (`intern`, `set_root`,
//! `allocate`, `set_errors`). Node fields are never mutated after
//! allocation; there is no API for it. The doors' `pub(crate)`
//! markers stay load-bearing under the seal: the sealed module gates
//! the PATHS, a method's own visibility gates the CALL — drop the
//! marker and the door is method-private, unreachable even in-crate.

use once_cell::sync::OnceCell;
use rut_core::{IdentId, Interner};
use rut_lexer::diag::Diag;
use rut_lexer::span::Span;

use crate::ast::handle::{AnyHandle, Arena, Handle, HasSpan, NodeKind, NodeKindId, NodeStore};
use crate::ast::index::TreeIndex;

// ---- the node inventory: one private arena per kind ----
//
// Every node kind in the tree, exactly once, as `field: Ty => "Wire"`.
// The macro wires the fields, the NodeStore impls (typed
// `allocate`/`get`), and the NodeKind stamps — `KIND` is the running
// index over this list (dense, unique by construction: a kind CANNOT
// lack an id) and `NAME` the single printable-name table (a kind
// CANNOT lack a name — the arity forces the literal). The wire labels
// and `AnyHandle::kind_name` both read that one table.
macro_rules! arenas {
    ($($rest:tt)*) => {
        arenas_munch!(@emit [] 0u16; $($rest)*);
    };
}

// the muncher — one expansion per kind (its own macro so the catch-all
// entry above can never re-match the recursion)
macro_rules! arenas_munch {
    // step: carry one row (id, name, field, ty), bump the id
    (@emit [$($acc:tt)*] $next:tt;
        $field:ident : $ty:ty => $name:literal
        $(, $($rest:tt)* )?
    ) => {
        arenas_munch!(
            @emit
            [$($acc)* [$next $name $field [$ty]]]
            ($next + 1);
            $( $($rest)* )?
        );
    };
    // base: every kind carries its row — emit the store
    (@emit [$(
        [$n:tt $name:tt $field:ident [$ty:tt]]
    )*] $next:tt;) => {
        pub struct Ast {
            $( $field: Arena<$ty>, )*
            interner: Interner,
            root: Option<Handle<crate::ast::item::ModuleNode>>,
            /// the ordered diag rows (the FLAT view — the panel). The
            /// POSITIONAL view is the `ErrorNode`s in the streams; the
            /// same rows, two shapes. Parse-lane-written only.
            errors: Vec<Diag>,
            /// the derived tree index (parents + cursor rows) — plain
            /// data, built lazily on first read (cold parses pay on
            /// demand); the edit lane maintains it from Phase 3 on
            index: OnceCell<TreeIndex>,
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
                    index: OnceCell::new(),
                }
            }
            /// construction (parse lane): land a complete node in its
            /// arena — the one allocation door behind every
            /// `XNode { .. }.allocate(ctx)`
            pub(crate) fn alloc<T: NodeStore>(&mut self, node: T) -> Handle<T> {
                Handle::new(T::arena_mut(self).insert(node))
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
        $( impl NodeKind for $ty {
            const KIND: NodeKindId = $n;
            const NAME: &'static str = $name;
        } )*

        /// the kind dispatch — one probe per stamped kind, id-compared
        /// (the ids are the macro's own running index). `None` = dead
        /// slot or bogus id: the printer's `dead` row, never a
        /// wrong-arena read.
        pub(crate) fn span_of_any(ast: &Ast, h: AnyHandle) -> Option<Span> {
            let kind = h.kind;
            let idx = h.idx;
            $(
                if kind == <$ty as NodeKind>::KIND {
                    return ast.$field.get(idx).map(|n| n.span());
                }
            )*
            None
        }

        /// the printable name of a kind id — the same single table the
        /// printer's labels read (`AnyHandle::kind_name` rides this)
        pub(crate) fn kind_name_of(kind: NodeKindId) -> &'static str {
            $(
                if kind == <$ty as NodeKind>::KIND {
                    return <$ty as NodeKind>::NAME;
                }
            )*
            "unknown"
        }
    };
}

arenas! {
    // items
    modules: crate::ast::item::ModuleNode => "Module",
    mod_decls: crate::ast::item::ModDeclNode => "ModDecl",
    uses: crate::ast::item::UseNode => "Use",
    aliases: crate::ast::item::AliasNode => "Alias",
    module_lets: crate::ast::item::ModuleLetNode => "ModuleLet",
    enums: crate::ast::item::EnumNode => "Enum",
    structs: crate::ast::item::StructNode => "Struct",
    classes: crate::ast::item::ClassNode => "Class",
    interfaces: crate::ast::item::InterfaceNode => "Interface",
    impls: crate::ast::item::ImplNode => "Impl",
    fns: crate::ast::item::FnNode => "Fn",
    surface_fns: crate::ast::item::SurfaceFnNode => "SurfaceFn",
    surface_structs: crate::ast::item::SurfaceStructNode => "SurfaceStruct",
    builtin_tys: crate::ast::item::BuiltinTyNode => "BuiltinTy",
    builtin_primitives: crate::ast::item::BuiltinPrimitiveNode => "BuiltinPrimitive",
    builtin_impls: crate::ast::item::BuiltinImplNode => "BuiltinImpl",
    // members
    field_decls: crate::ast::item::FieldDeclNode => "FieldDecl",
    method_decls: crate::ast::item::MethodDeclNode => "MethodDecl",
    enum_members: crate::ast::item::EnumMemberNode => "EnumMember",
    params: crate::ast::item::ParamNode => "Param",
    self_params: crate::ast::item::SelfParamNode => "SelfParam",
    // statements
    let_stmts: crate::ast::stmt::LetStmtNode => "LetStmt",
    if_stmts: crate::ast::stmt::IfNode => "If",
    whiles: crate::ast::stmt::WhileNode => "While",
    for_ofs: crate::ast::stmt::ForOfNode => "ForOf",
    for_cs: crate::ast::stmt::ForCNode => "ForC",
    returns: crate::ast::stmt::ReturnNode => "Return",
    breaks: crate::ast::stmt::BreakNode => "Break",
    continues: crate::ast::stmt::ContinueNode => "Continue",
    when_stmts: crate::ast::stmt::WhenStmtNode => "WhenStmt",
    // expressions
    blocks: crate::ast::expr::BlockNode => "Block",
    lits: crate::ast::expr::LitNode => "Lit",
    paths: crate::ast::expr::PathNode => "Path",
    calls: crate::ast::expr::CallNode => "Call",
    methods: crate::ast::expr::MethodNode => "Method",
    fields: crate::ast::expr::FieldNode => "Field",
    indexes: crate::ast::expr::IndexNode => "Index",
    unaries: crate::ast::expr::UnaryNode => "Unary",
    binaries: crate::ast::expr::BinaryNode => "Binary",
    assigns: crate::ast::expr::AssignNode => "Assign",
    lambdas: crate::ast::expr::LambdaNode => "Lambda",
    fstrs: crate::ast::expr::FStrNode => "FStr",
    struct_lits: crate::ast::expr::StructLitNode => "Struct",
    tuples: crate::ast::expr::TupleNode => "Tuple",
    array_lits: crate::ast::expr::ArrayLitNode => "ArrayLit",
    array_repeats: crate::ast::expr::ArrayRepeatNode => "ArrayRepeat",
    when_exprs: crate::ast::expr::WhenExprNode => "WhenExpr",
    if_exprs: crate::ast::expr::IfExprNode => "If",
    awaits: crate::ast::expr::AwaitNode => "Await",
    async_blocks: crate::ast::expr::AsyncBlockNode => "AsyncBlock",
    is_exprs: crate::ast::expr::IsNode => "Is",
    casts: crate::ast::expr::CastNode => "Cast",
    // types
    ty_paths: crate::ast::ty::TyPathNode => "TyPath",
    ty_fns: crate::ast::ty::TyFnNode => "TyFn",
    ty_opts: crate::ast::ty::TyOptNode => "TyOpt",
    ty_arrays: crate::ast::ty::TyArrayNode => "TyArray",
    ty_tuples: crate::ast::ty::TyTupleNode => "TyTuple",
    ty_unions: crate::ast::ty::TyUnionNode => "TyUnion",
    // patterns
    pat_lits: crate::ast::pat::PatLitNode => "PatLit",
    pat_paths: crate::ast::pat::PatPathNode => "PatPath",
    pat_wilds: crate::ast::pat::PatWildNode => "PatWild",
    pat_elses: crate::ast::pat::PatElseNode => "PatElse",
    // arms + recovery
    when_arms: crate::ast::expr::WhenArmNode => "WhenArm",
    error_nodes: crate::ast::error::ErrorNode => "Error",
}

impl Ast {
    /// the derived tree index — built on first read (the lazy cell);
    /// the edit lane (Phase 3) maintains it across edits
    pub(crate) fn index(&self) -> &TreeIndex {
        self.index.get_or_init(|| TreeIndex::build(self))
    }
    /// the deepest node whose span contains `pos` — the cursor's door
    /// (a position in dead space between nodes answers `None`)
    pub fn node_at(&self, pos: u32) -> Option<AnyHandle> {
        self.index().node_at(pos)
    }
    /// the node's parent address — `None` at the root (and for stale
    /// addresses)
    pub fn parent(&self, h: AnyHandle) -> Option<AnyHandle> {
        self.index().parent(h)
    }
    /// the node's span — the kind dispatch over the arenas. A stale
    /// (dead-slot) address is a caller bug: the printer probes
    /// `span_of_any` first and prints the explicit `dead` row instead.
    pub fn span_of(&self, h: AnyHandle) -> Span {
        span_of_any(self, h).expect("stale AnyHandle — the slot is dead")
    }
    /// the ancestor chain, the node itself first — the generic walk's
    /// spine (`node_at` → `ancestors` → `downcast`)
    pub fn ancestors(&self, h: AnyHandle) -> impl Iterator<Item = AnyHandle> + '_ {
        std::iter::successors(Some(h), |&cur| self.parent(cur))
    }
}
