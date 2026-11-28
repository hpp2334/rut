//! The capsule store (D4): per-kind private arenas, the interner, the
//! root — and NOTHING else. NO `src` (the text is the [`Doc`](crate::Doc)'s
//! — text queries are `Doc::text(span)` joins), no flat kind enum, no
//! public fields.
//!
//! THE ARTIFACT LAW (v4): `Ast` stores PURE STRUCTURE — the typed
//! nodes, their spans/verdicts, the interner vocabulary, the derived
//! [`TreeIndex`]. Every door is a method; reads live on
//! nodes/handles with the store as ctx.
//!
//! THE MUTATION LAW: `&ctx` methods ask the store, `&mut ctx` methods
//! tell it — and only the in-crate pipeline tells (`intern`, `set_root`,
//! `allocate`, `set_errors`, and the edit lane's repair doors:
//! `reseal_root`/`touch_root_span`/`remove_any`/`index_invalidate`).
//! Node fields are never mutated outside those doors. The doors'
//! `pub(crate)` markers stay load-bearing under the seal: the sealed
//! module gates the PATHS, a method's own visibility gates the CALL —
//! drop the marker and the door is method-private, unreachable even
//! in-crate.

use once_cell::sync::OnceCell;
use rut_core::{IdentId, Interner};
use rut_lexer::diag::Diag;
use rut_lexer::span::Span;

use crate::ast::handle::{AnyHandle, Arena, Handle, HasSpan, NodeKind, NodeStore};
use crate::ast::index::TreeIndex;

// ---- the node inventory: one private arena per kind ----
//
// Every node kind in the tree, exactly once, as `field: Ty => Variant,
// "Wire"`. The macro wires the fields, the NodeStore impls (typed
// `allocate`/`get`), the `NodeKindId` enum (one variant per row — a
// kind CANNOT lack one: the arity forces it), and the NodeKind stamps
// — `KIND` is the row's own variant and `NAME` the single
// printable-name table (a kind CANNOT lack a name either; variant and
// label are separate columns because two pairs share labels — `Struct`
// the item and `Struct` the literal, `If` the statement and `If` the
// expression). The wire labels and `AnyHandle::kind_name` both read
// that one table.
macro_rules! arenas {
    ($($rest:tt)*) => {
        arenas_munch!(@emit []; $($rest)*);
    };
}

// the muncher — one expansion per kind (its own macro so the catch-all
// entry above can never re-match the recursion). A row may carry a
// trailing `(expr)` — the kind's absence rule for the completeness
// query (the slots whose `None` makes the node incomplete).
macro_rules! arenas_munch {
    // step: carry one row (variant, name, field, ty, incompleteness)
    (@emit [$($acc:tt)*];
        $field:ident : $ty:ty => $variant:ident , $name:literal $(($inc:expr))?
        $(, $($rest:tt)* )?
    ) => {
        arenas_munch!(
            @emit
            [$($acc)* [$variant $name $field [$ty] [$($inc)?]]]
            ;
            $( $($rest)* )?
        );
    };
    // base: every kind carries its row — emit the store
    (@emit [$(
        [$variant:ident $name:tt $field:ident [$ty:tt] [$($inc:tt)*]]
    )*];) => {
        /// The arena kind — one variant per row of the inventory,
        /// stamped by the `arenas!` macro (unique by construction).
        /// The CLOSED vocabulary the generic lanes match on:
        /// `AnyHandle::kind` carries it, and a bogus kind is
        /// unrepresentable — the compiler refuses it. The derived
        /// order is the inventory's row order (the index's
        /// deterministic tie-break rides it)
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum NodeKindId {
            $($variant),*
        }

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
            const KIND: NodeKindId = NodeKindId::$variant;
            const NAME: &'static str = $name;
        } )*
        // the completeness impl — default complete, or the row's
        // absence rule (a closure over the node, bound in the row's
        // own hygiene context)
        $( incomplete_impl!($ty, [$($inc)*]); )*

        /// the kind dispatch — one arm per stamped kind (the enum's
        /// own match; the macro's arity keeps it exhaustive). `None` =
        /// dead slot: the printer's `dead` row, never a wrong-arena
        /// read.
        pub(crate) fn span_of_any(ast: &Ast, h: AnyHandle) -> Option<Span> {
            match h.kind {
                $(
                    NodeKindId::$variant => ast.$field.get(h.idx).map(|n| n.span()),
                )*
            }
        }

        /// the completeness query's dispatch — the per-kind absence
        /// rules above, over the same inventory. A dead slot is not
        /// an incomplete node.
        pub(crate) fn incomplete_of_any(ast: &Ast, h: AnyHandle) -> bool {
            match h.kind {
                $(
                    NodeKindId::$variant => ast
                        .$field
                        .get(h.idx)
                        .is_some_and(|n| <$ty as crate::ast::handle::IsIncomplete>::is_incomplete(n)),
                )*
            }
        }

        /// the printable name of a kind — the same single table the
        /// printer's labels read (`AnyHandle::kind_name` rides this).
        /// Exhaustive by construction: a new row cannot lack its arm
        pub(crate) fn kind_name_of(kind: NodeKindId) -> &'static str {
            match kind {
                $( NodeKindId::$variant => $name, )*
            }
        }

        /// remove the node at a universal address — the kind dispatch
        /// over the arenas, punching the slot's hole (stale handles
        /// then die loudly on read, never read yesterday's node). The
        /// edit lane's door; `false` = dead slot.
        pub(crate) fn remove_any(ast: &mut Ast, h: AnyHandle) -> bool {
            match h.kind {
                $(
                    NodeKindId::$variant => {
                        <$ty as NodeStore>::arena_mut(ast).remove(h.idx).is_some()
                    }
                )*
            }
        }
    };
}

// the completeness impl — default complete, or the row's absence rule.
// The row spells a CLOSURE over the node (`|n: &Self| ..`), so its
// bindings live in the row's own hygiene context.
macro_rules! incomplete_impl {
    ($ty:ty, []) => {
        impl crate::ast::handle::IsIncomplete for $ty {}
    };
    ($ty:ty, [$($inc:tt)*]) => {
        impl crate::ast::handle::IsIncomplete for $ty {
            fn is_incomplete(&self) -> bool {
                #[allow(clippy::redundant_closure_for_method_calls)]
                ($($inc)*)(self)
            }
        }
    };
}

arenas! {
    // items
    modules: crate::ast::item::ModuleNode => Module, "Module",
    mod_decls: crate::ast::item::ModDeclNode => ModDecl, "ModDecl",
    uses: crate::ast::item::UseNode => Use, "Use" (
        |n: &Self| !n.closed || (n.names.is_empty() && n.group.is_none())
    ),
    aliases: crate::ast::item::AliasNode => Alias, "Alias",
    module_lets: crate::ast::item::ModuleLetNode => ModuleLet, "ModuleLet",
    enums: crate::ast::item::EnumNode => Enum, "Enum" (|n: &Self| !n.closed),
    structs: crate::ast::item::StructNode => Struct, "Struct" (|n: &Self| !n.closed),
    classes: crate::ast::item::ClassNode => Class, "Class" (|n: &Self| !n.closed),
    interfaces: crate::ast::item::InterfaceNode => Interface, "Interface" (|n: &Self| !n.closed),
    impls: crate::ast::item::ImplNode => Impl, "Impl" (|n: &Self| !n.closed),
    fns: crate::ast::item::FnNode => Fn, "Fn" (|n: &Self| !n.closed),
    surface_fns: crate::ast::item::SurfaceFnNode => SurfaceFn, "SurfaceFn",
    surface_structs: crate::ast::item::SurfaceStructNode => SurfaceStruct, "SurfaceStruct" (
        |n: &Self| !n.closed
    ),
    builtin_tys: crate::ast::item::BuiltinTyNode => BuiltinTy, "BuiltinTy" (|n: &Self| !n.closed),
    builtin_primitives: crate::ast::item::BuiltinPrimitiveNode => BuiltinPrimitive,
    "BuiltinPrimitive" (|n: &Self| !n.closed),
    builtin_impls: crate::ast::item::BuiltinImplNode => BuiltinImpl, "BuiltinImpl" (
        |n: &Self| !n.closed
    ),
    // members
    field_decls: crate::ast::item::FieldDeclNode => FieldDecl, "FieldDecl",
    method_decls: crate::ast::item::MethodDeclNode => MethodDecl, "MethodDecl",
    enum_members: crate::ast::item::EnumMemberNode => EnumMember, "EnumMember",
    params: crate::ast::item::ParamNode => Param, "Param",
    self_params: crate::ast::item::SelfParamNode => SelfParam, "SelfParam",
    // statements
    let_stmts: crate::ast::stmt::LetStmtNode => LetStmt, "LetStmt",
    if_stmts: crate::ast::stmt::IfNode => If, "If",
    whiles: crate::ast::stmt::WhileNode => While, "While",
    for_ofs: crate::ast::stmt::ForOfNode => ForOf, "ForOf",
    for_cs: crate::ast::stmt::ForCNode => ForC, "ForC",
    returns: crate::ast::stmt::ReturnNode => Return, "Return",
    breaks: crate::ast::stmt::BreakNode => Break, "Break",
    continues: crate::ast::stmt::ContinueNode => Continue, "Continue",
    when_stmts: crate::ast::stmt::WhenStmtNode => WhenStmt, "WhenStmt",
    // expressions
    blocks: crate::ast::expr::BlockNode => Block, "Block" (|n: &Self| !n.closed),
    lits: crate::ast::expr::LitNode => Lit, "Lit",
    paths: crate::ast::expr::PathNode => Path, "Path",
    selfs: crate::ast::expr::SelfNode => SelfExpr, "Self",
    calls: crate::ast::expr::CallNode => Call, "Call" (|n: &Self| n.rparen.is_none()),
    methods: crate::ast::expr::MethodNode => Method, "Method" (
        |n: &Self| n.member.is_none() || n.rparen.is_none()
    ),
    fields: crate::ast::expr::FieldNode => Field, "Field",
    indexes: crate::ast::expr::IndexNode => Index, "Index",
    asserts: crate::ast::expr::AssertNode => Assert, "Assert",
    unaries: crate::ast::expr::UnaryNode => Unary, "Unary",
    binaries: crate::ast::expr::BinaryNode => Binary, "Binary",
    assigns: crate::ast::expr::AssignNode => Assign, "Assign",
    lambdas: crate::ast::expr::LambdaNode => Lambda, "Lambda",
    fstrs: crate::ast::expr::FStrNode => FStr, "FStr",
    struct_lits: crate::ast::expr::StructLitNode => StructLit, "Struct",
    tuples: crate::ast::expr::TupleNode => Tuple, "Tuple",
    array_lits: crate::ast::expr::ArrayLitNode => ArrayLit, "ArrayLit",
    array_repeats: crate::ast::expr::ArrayRepeatNode => ArrayRepeat, "ArrayRepeat",
    when_exprs: crate::ast::expr::WhenExprNode => WhenExpr, "WhenExpr",
    if_exprs: crate::ast::expr::IfExprNode => IfExpr, "If",
    awaits: crate::ast::expr::AwaitNode => Await, "Await",
    async_blocks: crate::ast::expr::AsyncBlockNode => AsyncBlock, "AsyncBlock",
    is_exprs: crate::ast::expr::IsNode => Is, "Is",
    casts: crate::ast::expr::CastNode => Cast, "Cast",
    // types
    ty_paths: crate::ast::ty::TyPathNode => TyPath, "TyPath",
    self_tys: crate::ast::ty::SelfTyNode => SelfTy, "SelfTy",
    ty_fns: crate::ast::ty::TyFnNode => TyFn, "TyFn",
    ty_opts: crate::ast::ty::TyOptNode => TyOpt, "TyOpt",
    ty_arrays: crate::ast::ty::TyArrayNode => TyArray, "TyArray",
    ty_tuples: crate::ast::ty::TyTupleNode => TyTuple, "TyTuple",
    ty_unions: crate::ast::ty::TyUnionNode => TyUnion, "TyUnion",
    // patterns
    pat_lits: crate::ast::pat::PatLitNode => PatLit, "PatLit",
    pat_paths: crate::ast::pat::PatPathNode => PatPath, "PatPath",
    pat_wilds: crate::ast::pat::PatWildNode => PatWild, "PatWild",
    pat_elses: crate::ast::pat::PatElseNode => PatElse, "PatElse",
    // arms + recovery
    when_arms: crate::ast::expr::WhenArmNode => WhenArm, "WhenArm",
    error_nodes: crate::ast::error::ErrorNode => Error, "Error",
}

impl Ast {
    /// the derived tree index — built on first read (the lazy cell);
    /// the edit lane invalidates it across edits (the next read
    /// rebuilds — the same deterministic walk a cold parse runs)
    pub(crate) fn index(&self) -> &TreeIndex {
        self.index.get_or_init(|| TreeIndex::build(self))
    }
    /// the edit lane's index door — drop the derived index so the next
    /// read rebuilds it over the edited tree (v1 maintenance policy:
    /// one deterministic rebuild per version)
    pub(crate) fn index_invalidate(&mut self) {
        self.index.take();
    }
    /// the edit lane's splice door — replace the root module's item
    /// list wholesale (the lex-fence prefix, the surviving items, the
    /// freshly reparsed window) and re-seal its span over the new
    /// text. The root handle stays: the module's slot is repaired in
    /// place, never reallocated.
    pub(crate) fn reseal_root(&mut self, items: Vec<crate::ItemRef>, eof_hi: u32) {
        let root = self.root.expect("the parse lane always sets the root");
        let m = <crate::ModuleNode as NodeStore>::arena_mut(self)
            .get_mut(root.idx)
            .expect("the root slot is live");
        m.items = items;
        m.span = Span::new(0, eof_hi);
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
    /// the generic completeness query — absence-derived per kind
    /// (a method's member or a call's `rparen` absent, a use without
    /// leaf names): the partial-shape rung's door. The tree always has
    /// something at the cursor; the query says whether it is whole.
    pub fn is_incomplete(&self, h: AnyHandle) -> bool {
        incomplete_of_any(self, h)
    }
}
