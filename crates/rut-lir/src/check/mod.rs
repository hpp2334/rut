//! Resolve + collect over the flat arena: module symbols,
//! the type table, traits & impls (the requires-graph shape lives here),
//! visibility. Body compilation (fused typecheck + codegen — see lir.rs)
//! runs over what this pass collects. Errors are Diags; a module with any
//! diag stops before emit.

use rut_ast::ast::*;
use rut_lexer::diag::Diag;
use rut_lexer::span::Span;
use rut_lexer::token::{FloatSuffix, IntSuffix};
use rut_core::binary::{ConstVal, FuncCode, TraitDesc};
use rut_core::types::*;
use rut_core::{Interner, sym};

pub(crate) mod collect;
mod collect_impl;
mod externs;
mod impls;
mod inst;
mod resolve;

// ---- decl indices ----

#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub ty: TypeId,
    /// member name → index
    pub members: Vec<IdentId>,
    /// inherent methods (`impl Color { .. }`): statics and self
    /// methods alike, exactly the struct decl's slot
    pub methods: Vec<(IdentId, NodeHandle<MethodDeclNode>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataKind {
    Struct,
    Class,
}

#[derive(Clone, Debug)]
pub struct DataDecl {
    pub kind: DataKind,
    pub ty: TypeId,
    /// the declaration's AST node — re-read to instantiate generics
    pub node: NodeHandle<AnyItem>,
    /// (name, ty, initializer, member vis — None = the module-private
    /// default; checked when modules load, M2) in decl order
    pub fields: Vec<(IdentId, TypeId, Option<NodeHandle<AnyExpr>>, Option<Vis>)>,
    /// inherent methods (name → MethodDecl node)
    pub methods: Vec<(IdentId, NodeHandle<MethodDeclNode>)>,
    pub generics: Vec<IdentId>,
    /// The class's admission bounds
    /// (`class HashMap<K requires i8 | … | bytes, V>`, the nmapset key
    /// union); checked at every
    /// `mk_data_inst`. Empty for structs and for classes without bounds.
    pub requires: Vec<(IdentId, NodeHandle<AnyTy>)>,
}

#[derive(Clone, Debug)]
pub struct TraitDeclInfo {
    pub id: u32,
    pub node: NodeId,
    /// generic parameters (`trait Foo<T>`) — empty for non-generic ones
    pub generics: Vec<IdentId>,
}

/// What a validated type alias resolves to. A single target
/// binds the target's `TypeId`; a union alias is bound-only and never
/// becomes a value type. `Pending` marks an alias
/// mid-validation — re-entering it is a cycle.
#[derive(Clone, Copy, Debug)]
pub enum AliasTarget {
    Ty(TypeId),
    Union,
    Pending,
    Error,
}

/// `type X = A;` / `type X = A | B;` — declared in pass 1a,
/// target validated in pass 1b. Aliases are non-generic: one name, one
/// decl (the row form is repealed).
#[derive(Clone, Debug)]
pub struct AliasDecl {
    pub name: IdentId,
    pub node: NodeId,
    /// the target as written
    pub target: NodeHandle<AnyTy>,
    pub resolved: Option<AliasTarget>,
}

/// A generic type bound from a used module's surface: the declaring
/// package owns every instantiation of it. The consumer holds the
/// TEMPLATE (its field descriptors, with `#<param>` placeholder leaves)
/// so it can lay a concrete instantiation out — the bodies compile in
/// the owner, which the consumer's instantiation requests name.
#[derive(Clone, Debug)]
pub struct ExternGeneric {
    /// the declaring pkg's spec
    pub owner: String,
    /// the generic parameters, in declaration order
    pub params: Vec<IdentId>,
    /// the template row's id in THIS module's table (registered from the
    /// exporter's surface block; its field descriptors carry the
    /// `#<param>` placeholder leaves)
    pub template: TypeId,
    /// `class` — no outside record literal
    pub is_class: bool,
}

/// One consumer request routed to a declaring package's compile: an
/// instantiation (`decl<args>`) a consumer's code used, addressed to the
/// pkg that owns the bodies. The graph resolves requests after the walk
/// (canonical, sorted order): a source owner is recompiled with the
/// seeds; a packaged owner's binary must already carry the row.
#[derive(Clone, Debug)]
pub struct InstRequest {
    pub owner: String,
    /// the declaration's name, as the consumer spells it
    pub decl: IdentId,
    /// the concrete arguments, in the consumer's type space
    pub args: Vec<TypeId>,
    /// the methods whose mirrors this request exists for (the
    /// linkable-classes phase): an instantiation IS its bodies, but a
    /// body only compiles when some call site needs it — the owner
    /// seeds exactly the mirrored methods at the requested
    /// instantiation, never the whole method set (a body may only
    /// typecheck at some instantiations)
    pub methods: Vec<IdentId>,
    /// a GENERIC FN seed (the linkable-classes phase): `decl` names a
    /// fn, not a type, and `args` are the fn's type arguments — the
    /// owner queues the fn's monomorphized body
    pub is_fn: bool,
    /// a GENERIC-TARGET IMPL METHOD seed: `decl` names the trait,
    /// `args[0]` is the concrete target row (in the consumer's space,
    /// registered verbatim in the owner's seed block), and `methods[0]`
    /// the method whose body the owner mints + compiles
    pub is_impl: bool,
    pub impl_target: TypeId,
}

/// One capture of a lambda or a desugared `for..of` emit closure:
/// the binding's `(name, value type, is_mut)`, plus the capture law's
/// by-cell marker — `cell` is `Some(record type)` for a PROMOTED
/// binding, in which case the closure's parameter carries the shared
/// one-field cell's handle and the body reads/writes through it; the
/// FuncCode param list spells the CELL type (so `MakeClosure` retains
/// the cell), while `ty` stays the value type the body is typed
/// against.
#[derive(Clone, Copy, Debug)]
pub struct Capture {
    pub name: IdentId,
    pub ty: TypeId,
    pub is_mut: bool,
    pub cell: Option<TypeId>,
}

#[derive(Clone, Debug)]
pub struct ImplDecl {
    pub trait_id: u32,
    /// the trait's source name (`Iterable`, a user trait); for an inherent
    /// impl, the target type's name
    pub trait_name: IdentId,
    pub target: TypeId,
    /// `impl Trait<T> for Vec<T>`: the generic class and the target's
    /// generic parameter idents. The impl's methods are monomorphized per
    /// instantiation through the `Inst` substitution; `None` for ordinary
    /// concrete impls.
    pub target_data: Option<(IdentId, Vec<IdentId>)>,
    /// the trait ref's type arguments, as written (`impl Iter<T>` →
    /// `[T]`, `impl Iterable<char>` → `[char]`). The element type of the
    /// sequence/iterator contracts is argument 0, resolved at the use site
    /// under the target substitution.
    pub trait_arg_nodes: Vec<NodeHandle<AnyTy>>,
    /// A parameterized trait impl (`impl Readable<T> for Source<T>`, the
    /// phase-1 template form) — `trait_id` names the PLACEHOLDER trait
    /// instantiation (its args are `#param` types), so every exact-match
    /// consumer re-resolves the trait args per target instantiation (the
    /// dispatch half: vtable fills, iterate, `find_or_mint_impl`). The
    /// per-instantiation clones the mint registers carry the flag too, so
    /// the concrete-first law sees them as the template's own rows.
    pub is_template: bool,
    /// `impl T { .. }` — inherent methods (no trait involved); dispatch is
    /// always static (the receiver's concrete type names the impl)
    pub inherent: bool,
    pub methods: Vec<(IdentId, NodeHandle<MethodDeclNode>)>,
    /// the pkg whose source the impl block lives in (the splice-origin
    /// rule) — the instantiation ledger's owner anchor for the impl's
    /// monomorphized methods
    pub origin: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FnKey {
    Free(IdentId),
    Method { data: IdentId, name: IdentId },
    /// `slot_abi` — which ABI variant this is. A prim-target trait-impl
    /// method interns TWO variants: the slot ABI (vtable rows; `self`
    /// and `Self`-spelled params cross boxed, prologue unboxes) and the
    /// concrete ABI (bare-receiver static calls; params cross raw —
    /// P1 of the mapset perf plan). Ref-repr targets compile ONE
    /// variant (`slot_abi: false`): the ABIs coincide, the receiver
    /// already is a cell handle.
    ImplMethod { idx: usize, name: IdentId, slot_abi: bool },
    /// one lambda AST node (its enclosing fn is non-generic in this build,
    /// so one instantiation per node)
    Lambda(NodeId),
    /// the emit closure of a desugared `for (v of xs)` over the
    /// `iterate` protocol: `body` with `v: E` bound;
    /// `break` → `return false`, `continue` → `return true`
    ForOfEmit { body: NodeId, var: IdentId },
    /// An engine-backed thunk: a bodyless FuncCode whose
    /// `host_id` names the embedder's registered body — the sleep
    /// future's `Future::yield`. The VM joins it like any host fn.
    HostThunk(IdentId),
}

/// The engine-minted async machinery of one compiled async fn.
#[derive(Clone, Copy, Debug)]
pub struct AsyncLayout {
    /// the hidden frame type (locals are its cell-backed fields)
    pub frame_ty: TypeId,
    /// the checkpoint enum (`s0` + one member per await)
    pub ckpt_ty: TypeId,
    /// the `Future<ret>` instantiation the frame implements
    pub fut_inst: u32,
    /// the yield's vtable slot under that instantiation
    pub yield_slot: u32,
}

/// A monomorphization instantiation: fn key + generic substitution.
/// `trait_origins` specializes trait-typed parameters per concrete
/// argument (a trait parameter IS an implicit generic bound):
/// one clone of the fn per distinct origin list, each body statically
/// binding calls on those parameters.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Inst {
    pub key: FnKey,
    /// generic param → concrete type
    pub subst: Vec<(IdentId, TypeId)>,
    /// concrete origins for the fn's trait-obj parameters, in order
    pub trait_origins: Vec<TypeId>,
}

pub struct Ctx<'a> {
    pub ast: &'a Ast,
    /// the module's interner: a clone of the AST's at construction (so all
    /// source ids resolve), extended here with synthesized names — type
    /// instantiations (`Array<i32>`), trait instantiations. Moves into the
    /// emitted `Program`.
    pub interner: Interner,
    /// every name the module's `use` statements wrote — the binding
    /// gate for used surfaces (the prelude is used,
    /// never ambient)
    pub used: std::collections::HashSet<IdentId>,
    pub diags: Vec<Diag>,
    pub types: TypeTable,
    pub traits: Vec<TraitDesc>,
    pub trait_decls: Vec<(IdentId, TraitDeclInfo)>,
    /// type aliases: `type X = A;` / `type X = A | B;`
    pub aliases: Vec<AliasDecl>,
    /// the sequence-contract trait id once referenced (`Iter`) —
    /// the sequence-lowering path keys on this, never on the trait's name

    /// the iterable-contract trait id once referenced (`Iterable`)
    /// — `for..of` lowers through the registered impl
    pub funcs: Vec<FuncCode>,
    pub consts: Vec<ConstVal>,
    pub exports: Vec<(IdentId, u32)>,
    // decl tables
    pub enums: Vec<(IdentId, EnumDecl)>,
    pub datas: Vec<(IdentId, DataDecl)>,
    pub impls: Vec<ImplDecl>,
    pub lets: Vec<(IdentId, Option<NodeHandle<AnyTy>>, NodeHandle<AnyExpr>)>,
    // name → index maps
    pub fn_index: Vec<IdentId>,
    /// free fn decl nodes: (name, Fn node)
    pub fn_nodes: Vec<(IdentId, NodeHandle<FnNode>)>,
    /// lambda capture lists, filled when the lambda is created:
    /// lambda node → one `Capture` per capture. `cell: Some` marks a
    /// promoted binding — the closure shares the binding's slot cell
    /// (the capture law); `None` is the immediate-slot copy.
    pub lambda_info: std::collections::HashMap<NodeId, Vec<Capture>>,
    /// lambda signatures: body node → (resolved param types incl.
    /// expected-type inference, ret type, the creation site's
    /// substitution). The substitution re-arms the body compiler's type
    /// env — a lambda nested inside a generic fn/method resolves its
    /// annotations (and any lambda it creates) against the enclosing
    /// generics, which its own unit-local Inst doesn't carry.
    pub lambda_sigs: std::collections::HashMap<NodeId, (Vec<TypeId>, TypeId, Vec<(IdentId, TypeId)>)>,
    /// `entry fn` names: the host-callable surface
    pub entries: Vec<IdentId>,
    /// used functions, bound before body compilation:
    /// name -> signature + the exporter's scope-qualified function id
    pub extern_fns: std::collections::HashMap<IdentId, ExternFn>,
    /// The host future lane's minted machinery, one per async host fn
    /// this unit calls (phase 4): extern fn name -> layout (the hidden
    /// `#hframe@` frame type, the `__start` thunk, the Future inst).
    /// Idempotent per fn — the mint runs at the first call site.
    pub host_async: std::collections::HashMap<IdentId, HostAsyncLayout>,
    /// core's `builtin impl` numeric methods: method
    /// name → `(receiver prim, lowering id)` — ambient on primitives
    pub builtin_impls: std::collections::HashMap<IdentId, Vec<(TypeId, rut_core::ops::Intrinsic)>>,
    /// used constants (native modules: `calc`): name -> (type, bits)
    pub extern_consts: std::collections::HashMap<IdentId, (TypeId, u64)>,
    /// used types: name -> the exporter's scope-qualified type id
    pub extern_types: std::collections::HashMap<IdentId, TypeId>,
    /// used types that are `class` (no outside record literal)
    pub extern_classes: std::collections::HashSet<TypeId>,
    /// used core builtin containers: name -> constructor.
    /// The prelude is used, never ambient — `Array`/`Opaque` resolve
    /// only through this map
    pub extern_native_types: std::collections::HashMap<IdentId, rut_core::binary::NativeTy>,
    /// used core builtin traits: name -> contract
    /// (`Disposal`/`Index`/`Iterable`)
    pub extern_traits: std::collections::HashMap<IdentId, rut_core::binary::NativeTrait>,
    /// traits exported by used modules' surfaces:
    /// name -> the descriptor registered in this module's table. The
    /// name binds only when the module used it — the use-both gate.
    pub extern_trait_decls: std::collections::HashMap<IdentId, ExternTrait>,
    /// trait impls registered by used modules' surfaces
    pub extern_impls: Vec<ExternImpl>,
    /// used enums (the linkable-classes phase): name → (the enum's
    /// type id, its members as bound) — a used enum's member paths
    /// (`EncodeErrorKind.Depth`) resolve through this registry; the
    /// descriptor row itself rides the carried type block
    pub extern_enums: std::collections::HashMap<IdentId, (TypeId, Vec<(IdentId, i64)>)>,
    /// inherent method surfaces bound from used modules' surfaces (the
    /// linkable-classes phase): the class-method rows a consumer's
    /// `b.append(..)` / `Logger.new(..)` resolve through
    pub extern_inherents: Vec<ExternInherent>,
    /// generic fns bound from used modules' surfaces: name → the
    /// placeholder signature + parameter names. The call mints the
    /// mirror instantiation and requests the body from the owner.
    pub extern_generic_fns: std::collections::HashMap<IdentId, ExternGenericFn>,
    /// the emit-closure signature of each desugared `for..of`,
    /// recorded at the creation site and read when the queue compiles the fn:
    /// body node → (element type, captures, the loop var's cell when
    /// the loop var itself is captured — the emit closure binds it
    /// cell-backed and stores the incoming element into the shared
    /// cell at frame entry, so both loop forms see one variable)
    pub for_of_sigs:
        std::collections::HashMap<u32, (TypeId, Vec<Capture>, Option<TypeId>)>,
    /// used core compiler-lowered functions (`own`, `downcast`,
    /// `assert`/`panic`, the `str`/`bytes` natives): the name is callable
    /// only when bound
    pub extern_native_fns: std::collections::HashSet<IdentId>,
    /// Bound namespace heads (`Math` for `calc`) — the qualified
    /// access form `<namespace>.<member>`. Name-generic: the
    /// LIR routes by membership here, never by a hardcoded string.
    pub extern_namespaces: std::collections::HashSet<IdentId>,
    /// instantiated generic types: id -> (decl, type args)
    pub inst_data: std::collections::HashMap<TypeId, (IdentId, Vec<TypeId>)>,
    /// monomorphization cache: (decl, type args) -> id
    pub type_inst: std::collections::HashMap<(IdentId, Vec<TypeId>), TypeId>,
    /// generic-trait instantiation cache: (trait, type args) -> trait id
    pub trait_inst: std::collections::HashMap<(IdentId, Vec<TypeId>), u32>,
    /// the async weave's engine-minted types: the `RunContext`
    /// cx record (lazily interned once), and per-async-fn hidden frame
    /// types with their checkpoint enums. Engine frames are exactly the
    /// types `await` accepts in v1 — the probe reads the engine-reserved
    /// state field, so the operand must be an engine-woven future.
    pub run_context_ty: Option<TypeId>,
    pub engine_frames: std::collections::HashSet<TypeId>,
    /// engine-vtable fills the minted impls contribute: (target type,
    /// trait slot, func id) — the frame's `Future::yield` row and the
    /// sleep future's engine-backed row. `build_vtables` applies them
    /// after the ordinary impl walk (the minted impls have no AST
    /// method nodes for the ordinary path to walk).
    pub extra_vtable_fills: Vec<(TypeId, u32, u32)>,
    /// whether the engine-backed sleep future machinery is already minted
    pub sleep_minted: bool,
    /// per compiled async fn: the minted frame type, checkpoint enum,
    /// Future instantiation and its yield's vtable slot — minted by the
    /// weave before the body compiles, read by every call site
    pub async_layout: std::collections::HashMap<u32, AsyncLayout>,
    /// frame type → (yield's vtable slot) — the await expansion reads
    /// this to aim its `CallI` at the awaited frame's woven yield
    pub frame_yield_slot: std::collections::HashMap<TypeId, u32>,
    /// frame type → its checkpoint enum (the await expansion mints the
    /// next state's singleton into the awaited... no — into the PARKING
    /// frame; the map serves the weave's own bookkeeping and tests)
    pub async_fns: std::collections::HashSet<IdentId>,
    /// whether `use` is resolved by the driver (module loading on)
    pub allow_uses: bool,
    /// this unit's own pkg spec (= the module name). Every decl's
    /// origin IS its module — source crosses no boundary since the
    /// linkable-classes phase retired splicing, so the orphan rule's
    /// inputs are this spec and the bound names' exporters
    /// ([`Ctx::extern_origins`]).
    pub own_spec: String,
    /// the origin pkg of every bound (used) name: a used
    /// type's or trait's DECLARING pkg, recorded beside the binding —
    /// the origin map covers only spliced text, a linked dep contributes
    /// no text, so its names carry their exporter's spec here
    pub extern_origins: std::collections::HashMap<IdentId, String>,
    /// core's import-gated names (the `pub builtin` rows of
    /// `Surface::core`), keyed by IdentId — a resolution
    /// miss that hits it names the fix: `use core::{ Name }`
    pub pub_core: std::collections::HashMap<IdentId, &'static str>,
    // instantiation queue
    pub inst_map: std::collections::HashMap<Inst, u32>,
    queue: Vec<Inst>,
    /// the pkg whose source declared each record/enum: spliced decls
    /// carry their splice origin, linked generics their exporter, own
    /// decls this unit — the instantiation ledger's owner anchor
    /// (`(owner, decl, args)` keys are global across the graph)
    pub decl_owner: std::collections::HashMap<IdentId, String>,
    /// generic templates bound from used modules' surfaces (see
    /// [`ExternGeneric`])
    pub extern_generics: std::collections::HashMap<IdentId, ExternGeneric>,
    /// requests this unit routed to declaring packages (see
    /// [`InstRequest`]) — deduplicated, emission order
    pub inst_requests: Vec<InstRequest>,
    requests_seen: std::collections::HashSet<(String, String, Vec<TypeId>, String)>,
    /// instantiations the graph seeded into this unit (the owner side of
    /// a request): `(decl, args)` — materialized after collect, before
    /// the compile roots
    pub seeded_insts: Vec<(String, Vec<TypeId>, Vec<String>)>,
    /// the graph-seeded GENERIC FN bodies: `(fn name, type arguments)`
    /// — the owner side of a consumer's mirrored fn call
    pub seeded_fns: Vec<(String, Vec<TypeId>)>,
    /// the requesters' own impl registrations (trait text, target in
    /// the requester's scope, method rows scope-qualified likewise) —
    /// registered after `collect`, when the trait table exists
    pub seed_impls: Vec<(String, TypeId, Vec<(String, u32)>, Vec<(String, u32)>)>,
    /// the mirrored GENERIC-TARGET impl methods: `(trait text, target
    /// row in the requester's space, method text)` — the owner mints
    /// the template impl at the concrete target and compiles the body
    pub seeded_impl_methods: Vec<(String, TypeId, String)>,
    /// the instantiation ledger rows this unit minted (type + fn) —
    /// copied onto the Program for link's unification
    pub ledger_types: Vec<rut_core::binary::InstTy>,
    pub ledger_fns: Vec<rut_core::binary::InstFn>,
}

/// A used function: the exporter's scope-qualified id and signature.
/// `is_async`/`host` are the host future lane's fields (a decl/native
/// exporter's `host async fn`): the call site weaves into the minted
/// frame whose rows register under `host`.
#[derive(Clone, Debug)]
pub struct ExternFn {
    pub func: u32,
    pub params: Vec<TypeId>,
    pub ret: TypeId,
    pub is_async: bool,
    pub host: Option<String>,
}

/// The minted machinery of one async host fn (the host future lane,
/// phase 4): the hidden `#hframe@` frame type the call site
/// instantiates, the `__start` thunk's fid the call dispatches to, and
/// the `Future<ret>` instantiation the result widens through.
#[derive(Clone, Copy)]
pub struct HostAsyncLayout {
    pub frame_ty: TypeId,
    pub start_fid: u32,
    pub fut_inst: u32,
}

/// A trait exported by a used module's surface: the id of
/// the descriptor registered in THIS module's trait table, and the
/// trait's generic parameter count. The NAME resolves only when the
/// module used it — the gate is the use-both rule's enforcement point,
/// so an impl whose trait was never `use`d stays
/// invisible to dispatch but visible to the "use `I` .." diagnostic.
#[derive(Clone, Debug)]
pub struct ExternTrait {
    pub id: u32,
    pub generics: usize,
}

/// A trait impl registered by another module's surface.
/// Dispatch and widening consult
/// these exactly like local impls; the methods are the exporter's
/// scope-qualified fn ids, called directly (static dispatch) and
/// carried into vtable fills (link merges the rows). `methods` binds
/// the slot-ABI variant, `methods_concrete` the concrete one (identical
/// ids for single-ABI impls — see [`SurfaceImpl`]).
#[derive(Clone, Debug)]
pub struct ExternImpl {
    pub trait_id: u32,
    pub trait_name: IdentId,
    pub target: TypeId,
    /// trait method name → the exporter's scope-qualified fn id
    pub methods: Vec<(IdentId, u32)>,
    /// trait method name → the exporter's scope-qualified fn id
    /// (concrete-ABI variant; falls back to `methods` when absent)
    pub methods_concrete: Vec<(IdentId, u32)>,
    /// a parameterized impl head's trait arguments as placeholder rows
    pub trait_args: Vec<TypeId>,
    /// the pkg that mints this impl's per-instantiation bodies. A
    /// DECLARED trait's row leaves it None — the owner is the trait's
    /// declaring pkg, read off `extern_origins`. A NATIVE trait's row
    /// (`impl Iterable<E> for Flow<E>`) carries it: no decl names an
    /// owner, and the impl's exporter is the pkg that compiles the
    /// bodies (the shape-only row's mint anchor).
    pub origin: Option<String>,
}

/// One used class's inherent method surface (the linkable-classes
/// phase): the exporter's class row and its method signatures + fn
/// ids. `target` is packed with the exporter's scope — a plain
/// class's own row, a generic class's TEMPLATE row (signatures spell
/// the `#<param>` placeholder rows a call site substitutes per
/// instantiation; `local` is zero — the bodies ride the owner-side
/// request machinery, keyed by the class name via
/// [`Ctx::extern_generics`]).
#[derive(Clone, Debug)]
pub struct ExternInherent {
    pub target: TypeId,
    /// (name, argument types with the receiver excluded, return, fn
    /// local, instance-vs-class, the method's own generic parameters)
    pub methods: Vec<(IdentId, Vec<TypeId>, TypeId, u32, bool, Vec<IdentId>)>,
}

/// A used module's exported GENERIC fn: the owner pkg, the parameter
/// names in order, and the `#<param>`-spelled signature the call site
/// typechecks (and infers) against.
#[derive(Clone, Debug)]
pub struct ExternGenericFn {
    pub owner: String,
    pub params: Vec<IdentId>,
    pub args: Vec<TypeId>,
    pub ret: TypeId,
}

/// Where a satisfying impl was found: a local impl block
/// (its methods monomorphize here) or another module's registration
/// (its compiled fns are called through scope-qualified ids).
#[derive(Clone, Copy, Debug)]
pub enum ImplHit {
    Local(usize),
    Extern(usize),
}

/// One `requires` member: a concrete type (exact `TypeId`
/// equality) or a trait (any registered impl satisfies, via the
/// registry).
#[derive(Clone, Copy, Debug)]
pub(crate) enum BoundMember {
    Concrete(TypeId),
    Trait(u32),
}

/// Display text of a bound/target type node — `A`, `A | B`, `Array<T>`
/// (diagnostics name the bound).
pub(crate) fn bound_ty_str(ctx: &Ctx, node: NodeHandle<AnyTy>) -> String {
    match ctx.ast.ty(node) {
        TypeKind::TyPath { segs } => {
            let mut s = segs
                .iter()
                .map(|sg| ctx.name(sg.name).to_string())
                .collect::<Vec<_>>()
                .join(".");
            let last = segs.last().expect("path has a segment");
            if !last.generics.is_empty() {
                s.push_str(&format!(
                    "<{}>",
                    last.generics.iter().map(|&g| bound_ty_str(ctx, g)).collect::<Vec<_>>().join(", ")
                ));
            }
            s
        }
        TypeKind::TyUnion { elems } => elems
            .iter()
            .map(|&e| bound_ty_str(ctx, e))
            .collect::<Vec<_>>()
            .join(" | "),
        _ => String::new(),
    }
}

pub type TcResult<T> = Result<T, ()>;

impl<'a> Ctx<'a> {
    pub fn new(ast: &'a Ast) -> Ctx<'a> {
        Ctx::new_scoped(ast, 1)
    }

    /// A module compiled under its own scope.
    pub fn new_scoped(ast: &'a Ast, scope: rut_core::ScopeId) -> Ctx<'a> {
        // the interner clones the AST's — every source id resolves
        // identically; synthesized names intern here only.
        let mut interner = ast.interner.clone();
        let pub_core = rut_core::binary::pub_core_map(&mut interner);
        Ctx {
            ast,
            interner,
            used: std::collections::HashSet::new(),
            diags: Vec::new(),
            types: TypeTable::boot_scoped(scope),
            traits: Vec::new(),
            trait_decls: Vec::new(),
            aliases: Vec::new(),
            funcs: Vec::new(),
            consts: Vec::new(),
            exports: Vec::new(),
            enums: Vec::new(),
            datas: Vec::new(),
            impls: Vec::new(),
            lets: Vec::new(),
            fn_index: Vec::new(),
            fn_nodes: Vec::new(),
            lambda_info: std::collections::HashMap::new(),
            lambda_sigs: std::collections::HashMap::new(),
            entries: Vec::new(),
            extern_fns: std::collections::HashMap::new(),
            host_async: std::collections::HashMap::new(),
            builtin_impls: std::collections::HashMap::new(),
            extern_consts: std::collections::HashMap::new(),
            extern_types: std::collections::HashMap::new(),
            extern_classes: std::collections::HashSet::new(),
            extern_native_types: std::collections::HashMap::new(),
            extern_traits: std::collections::HashMap::new(),
            extern_trait_decls: std::collections::HashMap::new(),
            extern_impls: Vec::new(),
            extern_inherents: Vec::new(),
            extern_enums: std::collections::HashMap::new(),
            extern_generic_fns: std::collections::HashMap::new(),
            for_of_sigs: std::collections::HashMap::new(),
            extern_native_fns: std::collections::HashSet::new(),
            extern_namespaces: std::collections::HashSet::new(),
            inst_data: std::collections::HashMap::new(),
            type_inst: std::collections::HashMap::new(),
            trait_inst: std::collections::HashMap::new(),
            run_context_ty: None,
            engine_frames: std::collections::HashSet::new(),
            extra_vtable_fills: Vec::new(),
            sleep_minted: false,
            async_layout: std::collections::HashMap::new(),
            frame_yield_slot: std::collections::HashMap::new(),
            async_fns: std::collections::HashSet::new(),
            allow_uses: false,
            own_spec: String::new(),
            extern_origins: std::collections::HashMap::new(),
            pub_core,
            inst_map: std::collections::HashMap::new(),
            queue: Vec::new(),
            decl_owner: std::collections::HashMap::new(),
            extern_generics: std::collections::HashMap::new(),
            inst_requests: Vec::new(),
            requests_seen: std::collections::HashSet::new(),
            seeded_insts: Vec::new(),
            seeded_fns: Vec::new(),
            seeded_impl_methods: Vec::new(),
            seed_impls: Vec::new(),
            ledger_types: Vec::new(),
            ledger_fns: Vec::new(),
        }
    }

    /// The `core` not-in-scope diagnostic: an import-gated core name
    /// (the `pub builtin` spellings) names the fix exactly —
    /// `` `Disposal` is not in scope — `use core::{ Disposal }` ``.
    /// `None` when `n` is not one — the caller keeps its ordinary
    /// message (retired names are ordinary identifiers: they fall to
    /// the caller's plain unknown-name diagnostic).
    pub fn not_in_core_scope(&self, n: IdentId) -> Option<String> {
        let text = self.interner.name(n);
        if let Some(name) = self.pub_core.get(&n) {
            return Some(format!("`{name}` is not in scope — `use core::{{ {name} }}`"));
        }
        rut_core::binary::is_core_name(n).then(|| {
            format!(
                "`{text}` is not in scope — `use core::{{{text}}}` (the prelude is used, never implicit)"
            )
        })
    }

    /// Use another module's type descriptors so `(scope, local)` ids
    /// resolve for typechecking and layout. Descriptor and
    /// field names re-intern from the exporter's interner into this
    /// module's — name ids are only comparable within one interner
    /// (well-known ids pass through: they mean the same name everywhere).
    /// `tmap` maps the exporter's trait-table indices to THIS module's
    /// trait ids: trait-typed descriptors (`[trait] Shape`) carried
    /// across must name the trait here.
    pub fn use_types(
        &mut self,
        descs: Vec<RutType>,
        names: &Interner,
        blocks: &[(rut_core::ScopeId, u32)],
        tmap: &std::collections::HashMap<u32, u32>,
    ) {
        let mut map: std::collections::HashMap<IdentId, IdentId> = std::collections::HashMap::new();
        let mut re = |interner: &mut Interner, id: IdentId| -> IdentId {
            if id.0 < names.well_known_len() {
                return id;
            }
            *map.entry(id).or_insert_with(|| interner.intern(names.name(id)))
        };
        let descs = descs
            .into_iter()
            .map(|mut d| {
                d.name = re(&mut self.interner, d.name);
                match &mut d.kind {
                    TyKind::Data { fields } => {
                        for f in fields {
                            f.name = re(&mut self.interner, f.name);
                        }
                    }
                    TyKind::Enum { members } => {
                        for (n, _) in members.iter_mut() {
                            *n = re(&mut self.interner, *n);
                        }
                    }
                    TyKind::TraitObj { trait_id } => {
                        if let Some(&g) = tmap.get(trait_id) {
                            *trait_id = g;
                        }
                    }
                    _ => {}
                }
                d
            })
            .collect();
        self.types.use_block(descs, blocks);
    }

    /// The request seeds' requester rows, re-interned through
    /// `source` into THIS unit's interner — without appending. The
    /// attach side merges every seed group's rows into ONE run per
    /// scope (two requesters' rows share a scope — the classes of a
    /// common dependency — and a second base registration would strand
    /// the first group's ids), then appends once.
    pub fn stage_seed_rows(
        &mut self,
        rows: Vec<((rut_core::ScopeId, u32), RutType)>,
        source: &Interner,
    ) -> Vec<((rut_core::ScopeId, u32), RutType)> {
        let mut map: std::collections::HashMap<IdentId, IdentId> = std::collections::HashMap::new();
        let mut re = |interner: &mut Interner, id: IdentId| -> IdentId {
            if id.0 < source.well_known_len() {
                return id;
            }
            *map.entry(id).or_insert_with(|| interner.intern(source.name(id)))
        };
        rows.into_iter()
            .map(|(key, mut d)| {
                d.name = re(&mut self.interner, d.name);
                match &mut d.kind {
                    TyKind::Data { fields } => {
                        for f in fields {
                            f.name = re(&mut self.interner, f.name);
                        }
                    }
                    TyKind::Enum { members } => {
                        for (n, _) in members.iter_mut() {
                            *n = re(&mut self.interner, *n);
                        }
                    }
                    _ => {}
                }
                (key, d)
            })
            .collect()
    }

    /// [`Self::use_staged_seed_rows`] for rows already re-interned into
    /// this unit's interner ([`Self::stage_seed_rows`]): append +
    /// register, no re-mapping pass.
    pub fn use_staged_seed_rows(
        &mut self,
        descs: Vec<RutType>,
        blocks: &[(rut_core::ScopeId, u32)],
    ) {
        self.types.use_seed_rows(descs, blocks);
    }

    pub fn err(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diag::new(span, msg));
    }

    /// The crossing rule: what an `entry fn` signature may
    /// carry. Primitives, `str`, `nil`, `bytes` (the binary buffer),
    /// `?T` over crossable, anonymous tuples of crossable types — and
    /// `Opaque`, the host-held box: the ONE cell shape an embedder may
    /// keep and pass back. Every other cell (`TodoList`, `Vec<Todo>`,
    /// a trait object, `Vec<u8>` itself, …) stays inside the VM.
    pub fn crosses_boundary(&self, ty: TypeId) -> bool {
        self.types.crosses_boundary(&self.interner, ty)
    }

    /// Compile-time enforcement of the crossing rule on every `entry fn`
    ///: a bad surface is a source diagnostic with span and
    /// parameter name — never a runtime "cannot call" surprise.
    pub fn check_entries(&mut self) {
        let entries = self.entries.clone();
        for name in entries {
            let Some((_, node)) = self.fn_nodes.iter().find(|(n, _)| *n == name).cloned() else {
                continue;
            };
            let fd = self.ast.fn_decl(node).clone();
            let fname = self.name(name).to_string();
            if !fd.generics.is_empty() {
                self.err(
                    self.ast.span(node.id()),
                    format!(
                        "`entry fn {fname}` cannot be generic — the published signature is one concrete shape"
                    ),
                );
                continue;
            }
            for p in &fd.params {
                let MemberKind::Param(pd) = self.ast.param(*p) else { continue };
                let Some(t) = pd.ty else { continue };
                let ty = self.resolve_type(t, &[]);
                if !self.crosses_boundary(ty) {
                    self.err(
                        self.ast.span(p.id()),
                        format!(
                            "`entry fn {fname}`: parameter `{}` is `{}` — only primitives, `str`, `nil`, `bytes`, `?T` over those, anonymous tuples of crossable types, and `opaque` cross the host boundary",
                            self.name(pd.name),
                            self.type_name(ty)
                        ),
                    );
                }
            }
            if let Some(r) = fd.ret {
                let ty = self.resolve_type(r, &[]);
                if !self.crosses_boundary(ty) {
                    self.err(
                        self.ast.span(r.id()),
                        format!(
                            "`entry fn {fname}` returns `{}` — only primitives, `str`, `nil`, `bytes`, `?T` over those, anonymous tuples of crossable types, and `opaque` cross the host boundary",
                            self.type_name(ty)
                        ),
                    );
                }
            }
        }
    }

    pub fn name(&self, id: IdentId) -> &str {
        self.interner.name(id)
    }

    /// A type's display name — synthesized instantiation names resolve
    /// through the Ctx's interner (the AST's copy predates them).
    pub fn type_name(&self, id: TypeId) -> &str {
        self.interner.name(self.types.type_at(id).name)
    }

    /// Intern a synthesized name (type instantiations, trait shapes).
    /// Source identifiers are interned by the parser — never call this
    /// for them.
    pub fn intern(&mut self, s: &str) -> IdentId {
        self.interner.intern(s)
    }

    /// look a name up in the interner
    pub fn lookup_name(&self, s: &str) -> Option<IdentId> {
        self.interner.lookup(s)
    }

    pub fn find_enum(&self, name: IdentId) -> Option<&EnumDecl> {
        self.enums.iter().find(|(n, _)| *n == name).map(|(_, d)| d)
    }
    pub fn find_data(&self, name: IdentId) -> Option<&DataDecl> {
        self.datas.iter().find(|(n, _)| *n == name).map(|(_, d)| d)
    }
    pub fn find_trait(&self, name: IdentId) -> Option<&TraitDeclInfo> {
        self.trait_decls.iter().find(|(n, _)| *n == name).map(|(_, d)| d)
    }
    pub fn find_alias(&self, name: IdentId) -> Option<&AliasDecl> {
        self.aliases.iter().find(|a| a.name == name)
    }

    /// this unit's own pkg spec. Every definition in the unit is the
    /// unit's own — the orphan rule's locality input, trivial since
    /// splicing retired.
    /// The pkg that owns `decl`'s instantiations: instantiation happens
    /// where the body lives. Every own decl's bodies live in this unit,
    /// a linked generic's in its exporter — the ledger key's owner
    /// anchor.
    pub fn owner_of_data(&self, decl: IdentId) -> String {
        self.decl_owner
            .get(&decl)
            .cloned()
            .unwrap_or_else(|| self.own_spec.clone())
    }

    /// Route an instantiation request to its declaring package
    /// (deduplicated — one request per `owner + decl + args`).
    pub fn request_inst(&mut self, owner: String, decl: IdentId, args: Vec<TypeId>) {
        let key = (owner.clone(), self.name(decl).to_string(), args.clone(), String::new());
        if self.requests_seen.insert(key) {
            self.inst_requests.push(InstRequest { owner, decl, args, methods: vec![], is_fn: false, is_impl: false, impl_target: 0 });
        }
    }

    /// Route a mirrored METHOD body request (the linkable-classes
    /// phase): the mirror stub's instantiation + the method whose body
    /// must exist in the owner. Dedup per `owner + decl + args +
    /// method`; several methods on one instantiation ride as several
    /// rows that the owner merges.
    pub fn request_inst_method(
        &mut self,
        owner: String,
        decl: IdentId,
        args: Vec<TypeId>,
        method: IdentId,
    ) {
        let key = (
            owner.clone(),
            self.name(decl).to_string(),
            args.clone(),
            self.name(method).to_string(),
        );
        if self.requests_seen.insert(key) {
            self.inst_requests.push(InstRequest {
                owner,
                decl,
                args,
                methods: vec![method],
                is_fn: false,
                is_impl: false,
                impl_target: 0,
            });
        }
    }

    /// Route a mirrored GENERIC-TARGET impl method request (the
    /// linkable-classes phase): the trait + the concrete target row (in
    /// the consumer's space — the seed block registers it verbatim) +
    /// the method whose body the owner must mint + compile.
    pub fn request_inst_impl_method(
        &mut self,
        owner: String,
        trait_name: IdentId,
        target: TypeId,
        method: IdentId,
    ) {
        let key = (
            owner.clone(),
            format!("impl:{}", self.name(trait_name)),
            vec![target],
            self.name(method).to_string(),
        );
        if self.requests_seen.insert(key) {
            
            self.inst_requests.push(InstRequest {
                owner,
                decl: trait_name,
                args: vec![target],
                methods: vec![method],
                is_fn: false,
                is_impl: true,
                impl_target: target,
            });
        }
    }

    /// Route a mirrored GENERIC FN body request (the linkable-classes
    /// phase): `name` is the fn, `args` its type arguments in the
    /// consumer's space (the seed block carries their rows to the
    /// owner, so the ids spell identically there).
    pub fn request_generic_fn_body(&mut self, owner: String, name: IdentId, args: Vec<TypeId>) {
        let key = (owner.clone(), format!("fn:{}", self.name(name)), args.clone(), String::new());
        if self.requests_seen.insert(key) {
            self.inst_requests.push(InstRequest {
                owner,
                decl: name,
                args,
                methods: vec![],
                is_fn: true,
                is_impl: false,
                impl_target: 0,
            });
        }
    }

    /// Do two type ids spell the SAME owner-anchored instantiation?
    /// Every unit lays its own mirror row for a foreign instantiation —
    /// distinct ids pre-link, one row at link. The checker's
    /// identity-sensitive comparisons read the ledger: same
    /// `(owner, decl, args)` key, same type. A row this unit only
    /// REGISTERED (a linked dep's returned instantiation) carries no
    /// ledger row — its display name is its identity here, the same
    /// spelling the ledger's rows were built with.
    pub fn same_instantiation(&self, a: TypeId, b: TypeId) -> bool {
        if a == b {
            return true;
        }
        let key_of = |ty: TypeId| -> Option<String> {
            if self.ledger_types.iter().any(|r| r.ty == ty) {
                return Some(self.type_name(ty).to_string());
            }
            let name = self.type_name(ty).to_string();
            name.contains('<').then_some(name)
        };
        match (key_of(a), key_of(b)) {
            (Some(ka), Some(kb)) => ka == kb,
            _ => false,
        }
    }

    /// Settle an alias's target (pass 1b): each member must
    /// resolve as a type or trait — located early errors; forward refs
    /// are legal, a cycle (`type A = B; type B = A;`) diagnoses at the
    /// re-entered alias. The target resolves HERE, at the declaration —
    /// used or not (`type Foo = NotAType;` diagnoses even in a dead
    /// program); there is no deferring alias path.
    pub(crate) fn validate_alias(&mut self, name: IdentId) {
        let Some(idx) = self.aliases.iter().position(|a| a.name == name) else {
            return;
        };
        match self.aliases[idx].resolved {
            Some(AliasTarget::Pending) => {
                let sp = self.ast.span(self.aliases[idx].node);
                let n = self.aliases[idx].name;
                self.err(
                    sp,
                    format!(
                        "recursive type alias `{}` — an alias chain must end at a real type",
                        self.name(n)
                    ),
                );
                self.aliases[idx].resolved = Some(AliasTarget::Error);
                return;
            }
            Some(_) => return,
            None => {}
        }
        self.aliases[idx].resolved = Some(AliasTarget::Pending);
        let target = self.aliases[idx].target;
        match self.ast.ty(target) {
            TypeKind::TyUnion { elems } => {
                // a union alias: every member must resolve (as a type or
                // trait); the union itself never becomes a value type
                for e in elems.clone() {
                    self.resolve_type(e, &[]);
                }
                if matches!(self.aliases[idx].resolved, Some(AliasTarget::Pending)) {
                    self.aliases[idx].resolved = Some(AliasTarget::Union);
                }
            }
            _ => {
                let t = self.resolve_type(target, &[]);
                if matches!(self.aliases[idx].resolved, Some(AliasTarget::Pending)) {
                    self.aliases[idx].resolved = Some(AliasTarget::Ty(t));
                }
            }
        }
    }

    /// Single-target aliases as export rows: the alias name
    /// binds the TARGET's id, so importers need zero changes —
    /// cross-module transparency by construction. Union aliases are
    /// module-local, never exported.
    pub fn alias_exports(&self) -> Vec<(IdentId, TypeId)> {
        self.aliases
            .iter()
            .filter_map(|a| match a.resolved {
                Some(AliasTarget::Ty(t)) => Some((a.name, t)),
                _ => None,
            })
            .collect()
    }

    pub fn find_let(&self, name: IdentId) -> Option<&(IdentId, Option<NodeHandle<AnyTy>>, NodeHandle<AnyExpr>)> {
        self.lets.iter().find(|(n, _, _)| *n == name)
    }
    pub fn find_free_fn(&self, name: IdentId) -> bool {
        self.fn_index.contains(&name)
    }
    pub fn trait_id_of(&self, name: IdentId) -> Option<u32> {
        // a generic trait has no uninstantiated id (`u32::MAX` sentinel)
        if let Some(t) = self.find_trait(name) {
            return (t.id != u32::MAX).then_some(t.id);
        }
        // a used module's exported trait — the name is
        // bound only when the module used it (the gate)
        self.extern_trait_decls.get(&name).map(|t| t.id)
    }
    pub fn trait_by_id(&self, id: u32) -> &TraitDesc {
        &self.traits[id as usize]
    }
    /// Resolve a signature type under `env`, with `self_ty` spelling the
    /// method's `Self` (the impl target inside an impl block). Bare
    /// `Self` outside an impl is the caller's diagnostic. The driver's
    /// surface build resolves exported methods' template signatures
    /// through this (the `#<param>` placeholder env).
    pub fn resolve_sig_ty(
        &mut self,
        node: NodeHandle<AnyTy>,
        env: &[(IdentId, TypeId)],
        self_ty: Option<TypeId>,
    ) -> TypeId {
        self.resolve_sig_ty_deep(node, env, self_ty)
    }

    /// Signature resolution with `Self` handled at ANY structural depth —
    /// `fn decode(r) -> (?Self, ?E)` spells Self inside a PAIR (the
    /// rut-json batch phase 1, gap 2's signature half). Bare `Self`
    /// binds `self_ty`; structure (`?`, tuples, arrays) recurses; leaves
    /// fall through to the ordinary resolver.
    pub(crate) fn resolve_sig_ty_deep(
        &mut self,
        node: NodeHandle<AnyTy>,
        env: &[(IdentId, TypeId)],
        self_ty: Option<TypeId>,
    ) -> TypeId {
        let is_self = |segs: &Vec<rut_ast::ast::PathSeg>| {
            segs.len() == 1 && segs[0].generics.is_empty() && segs[0].name == sym::SELF_TY
        };
        match self.ast.ty(node) {
            TypeKind::TyPath { segs, .. } if is_self(segs) => {
                if let Some(t) = self_ty {
                    return t;
                }
            }
            TypeKind::TyOpt { inner } => {
                if let TypeKind::TyPath { segs, .. } = self.ast.ty(*inner) {
                    if is_self(segs) {
                        if let Some(t) = self_ty {
                            return self.mk_opt(t);
                        }
                    }
                }
                let elem = self.resolve_sig_ty_deep(*inner, env, self_ty);
                return self.mk_opt(elem);
            }
            TypeKind::TyTuple { elems } => {
                let mut etys = Vec::new();
                for e in elems {
                    etys.push(self.resolve_sig_ty_deep(*e, env, self_ty));
                }
                return self.mk_tuple(etys);
            }
            TypeKind::TyArray { elem } => {
                let et = self.resolve_sig_ty_deep(*elem, env, self_ty);
                return self.mk_array(et);
            }
            _ => {}
        }
        self.resolve_type(node, env)
    }

    /// global trait-method slot id (assigned per trait
    /// instantiation; v1 non-generic traits only)
    pub fn trait_slot(&self, trait_id: u32, method: u32) -> Option<u32> {
        let mut slot = 0;
        for (i, t) in self.traits.iter().enumerate() {
            for m in 0..t.methods.len() {
                if i as u32 == trait_id && m as u32 == method {
                    return Some(slot);
                }
                slot += 1;
            }
        }
        None
    }

    // ---- type construction ----
    // synthesized type names intern into the Ctx's interner — structural
    // dedup keys on (name id, kind), and equal shapes always intern equal
    // name text, so the id compare in `TypeTable::intern` stays sound

    pub fn mk_array(&mut self, elem: TypeId) -> TypeId {
        let name = self.intern(&format!("Array<{}>", self.type_name(elem)));
        let id = self.types.intern(RutType {
            name,
            kind: TyKind::Array { elem },
        });
        
        id
    }
    /// A trait-typed value (`i: I`) — the trait object type: a cell handle whose cell's own
    /// type reaches the vtable
    pub fn mk_trait_obj(&mut self, trait_id: u32) -> TypeId {
        let tname = self.interner.name(self.traits[trait_id as usize].name).to_string();
        let name = self.intern(&format!("[trait] {tname}"));
        self.types.intern(RutType {
            name,
            kind: TyKind::TraitObj { trait_id },
        })
    }
    /// `?T` — nil-able cell (the old `*T` pointer)
    /// `Weak<T>` — the weak reference. Generic like
    /// `Array { elem }`: interned per instantiation, no boot row.
    pub fn mk_weak(&mut self, elem: TypeId) -> TypeId {
        let name = self.intern(&format!("Weak<{}>", self.type_name(elem)));
        self.types.intern(RutType {
            name,
            kind: TyKind::Weak { elem },
        })
    }
    /// `?T` — a nil-able cell (the `mk_array`
    /// shape). The three crossing elems answer their BOOT rows (the
    /// legal-host-returns phase): a source-spelled `?str`/`?bytes`/
    /// `?opaque` and a `.d.rut` row's `-> ?T` are ONE type — the boot
    /// id — so a wrapper fn returning a host row's optional widens by
    /// identity, never by shape (the nominal law). The boot rows ARE
    /// `Opt { elem }` of exactly these elems, and every program's table
    /// carries them (boot_len).
    pub fn mk_opt(&mut self, elem: TypeId) -> TypeId {
        match elem {
            TY_STR => return TY_OPT_STR,
            TY_BYTES => return TY_OPT_BYTES,
            TY_OPAQUE => return TY_OPT_OPAQUE,
            _ => {}
        }
        let name = self.intern(&format!("?{}", self.type_name(elem)));
        self.types.intern(RutType {
            name,
            kind: TyKind::Opt { elem },
        })
    }
    /// `(A, B, ..)` — a record with numeric field names
    pub fn mk_tuple(&mut self, elems: Vec<TypeId>) -> TypeId {
        let name = self.intern(&format!(
            "({})",
            elems.iter().map(|&t| self.type_name(t)).collect::<Vec<_>>().join(", ")
        ));
        let fields = elems
            .into_iter()
            .enumerate()
            .map(|(i, ty)| FieldInfo { name: self.intern(&i.to_string()), ty })
            .collect();
        self.types.intern(RutType {
            name,
            kind: TyKind::Data { fields },
        })
    }
    pub fn mk_fn_ty(&mut self, params: Vec<TypeId>, ret: TypeId) -> TypeId {
        let ps: Vec<String> = params.iter().map(|&p| self.type_name(p).to_string()).collect();
        let name = self.intern(&format!("fn({}) -> {}", ps.join(", "), self.type_name(ret)));
        self.types.intern(RutType {
            name,
            kind: TyKind::Fn { params, ret },
        })
    }
}

pub type NodeId2 = IdentId;

pub(crate) fn seg_str(ctx: &Ctx, segs: &[PathSeg]) -> String {
    segs.iter()
        .map(|s| ctx.name(s.name).to_string())
        .collect::<Vec<_>>()
        .join(".")
}

// int suffix → type id
pub fn int_suffix_ty(s: IntSuffix) -> TypeId {
    match s {
        IntSuffix::U8 => TY_U8, IntSuffix::U16 => TY_U16, IntSuffix::U32 => TY_U32, IntSuffix::U64 => TY_U64,
        IntSuffix::I8 => TY_I8, IntSuffix::I16 => TY_I16, IntSuffix::I32 => TY_I32, IntSuffix::I64 => TY_I64,
    }
}

/// The ten numeric primitives — the only `as` cast source/target kinds
/// (`Bool` is excluded).
pub fn numeric_prim(p: PrimTy) -> bool {
    matches!(
        p,
        PrimTy::U8 | PrimTy::U16 | PrimTy::U32 | PrimTy::U64
            | PrimTy::I8 | PrimTy::I16 | PrimTy::I32 | PrimTy::I64
            | PrimTy::F32 | PrimTy::F64
    )
}

pub fn float_suffix_ty(s: FloatSuffix) -> TypeId {
    match s {
        FloatSuffix::F32 => TY_F32,
        FloatSuffix::F64 => TY_F64,
    }
}
