//! Resolve + collect — RFC 0031 §1 over the flat arena: module symbols,
//! the type table, traits & impls (the requires-graph shape lives here),
//! visibility. Body compilation (fused typecheck + codegen — see lir.rs)
//! runs over what this pass collects. Errors are Diags; a module with any
//! diag stops before emit (RFC 0030 §6).

use rut_ast::ast::*;
use rut_lexer::diag::Diag;
use rut_lexer::span::Span;
use rut_lexer::token::{FloatSuffix, IntSuffix};
use rut_core::binary::{ConstVal, FuncCode, TraitDesc};
use rut_core::types::*;
use rut_core::{Interner, sym};

mod collect;
mod inst;
mod resolve;

// ---- decl indices ----

#[derive(Clone, Debug)]
pub struct EnumDecl {
    pub ty: TypeId,
    /// member name → index
    pub members: Vec<IdentId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataKind {
    Dataclass,
    Class,
}

#[derive(Clone, Debug)]
pub struct DataDecl {
    pub kind: DataKind,
    pub ty: TypeId,
    /// the declaration's AST node — re-read to instantiate generics
    pub node: NodeHandle<AnyItem>,
    /// (name, ty, initializer, member vis — None = the module-private
    /// default, RFC 0003 §2; checked when modules load, M2) in decl order
    pub fields: Vec<(IdentId, TypeId, Option<NodeHandle<AnyExpr>>, Option<Vis>)>,
    /// inherent methods (name → MethodDecl node)
    pub methods: Vec<(IdentId, NodeHandle<MethodDeclNode>)>,
    pub generics: Vec<IdentId>,
}

#[derive(Clone, Debug)]
pub struct TraitDeclInfo {
    pub id: u32,
    pub node: NodeId,
    /// generic parameters (`trait Foo<T>`) — empty for non-generic ones
    pub generics: Vec<IdentId>,
}

#[derive(Clone, Debug)]
pub struct ImplDecl {
    pub trait_id: u32,
    /// the trait's source name (`Iterator`, a user trait); for an inherent
    /// impl, the target type's name
    pub trait_name: IdentId,
    pub target: TypeId,
    /// `impl Trait<T> for Vec<T>`: the generic class and the target's
    /// generic parameter idents. The impl's methods are monomorphized per
    /// instantiation through the `Inst` substitution; `None` for ordinary
    /// concrete impls.
    pub target_data: Option<(IdentId, Vec<IdentId>)>,
    /// the trait ref's type arguments, as written (`impl Iter<T>` →
    /// `[T]`, `impl Iterator<char>` → `[char]`). The element type of the
    /// sequence/iterator contracts is argument 0, resolved at the use site
    /// under the target substitution.
    pub trait_arg_nodes: Vec<NodeHandle<AnyTy>>,
    /// `impl T { .. }` — inherent methods (no trait involved); dispatch is
    /// always static (the receiver's concrete type names the impl)
    pub inherent: bool,
    pub methods: Vec<(IdentId, NodeHandle<MethodDeclNode>)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FnKey {
    Free(IdentId),
    Method { data: IdentId, name: IdentId },
    ImplMethod { idx: usize, name: IdentId },
    /// one lambda AST node (its enclosing fn is non-generic in this build,
    /// so one instantiation per node)
    Lambda(NodeId),
    /// the emit closure of a desugared `for (v of xs)` over the
    /// `__iterate` protocol (RFC 0012 §6): `body` with `v: E` bound;
    /// `break` → `return false`, `continue` → `return true`
    ForOfEmit { body: NodeId, var: IdentId },
}

/// A monomorphization instantiation: fn key + generic substitution.
/// `trait_origins` specializes trait-typed parameters per concrete
/// argument (a trait parameter IS an implicit generic bound, RFC 0012):
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
    /// emitted `Program` (RFC 0030 §5).
    pub interner: Interner,
    /// every name the module's `use` statements wrote — the binding
    /// gate for used surfaces (RFC 0028: the prelude is used,
    /// never ambient)
    pub used: std::collections::HashSet<IdentId>,
    pub diags: Vec<Diag>,
    pub types: TypeTable,
    pub traits: Vec<TraitDesc>,
    pub trait_decls: Vec<(IdentId, TraitDeclInfo)>,
    /// the sequence-contract trait id once referenced (RFC 0012 `Iter`) —
    /// the sequence-lowering path keys on this, never on the trait's name

    /// the iterator-contract trait id once referenced (RFC 0012 `Iterator`)
    /// — `for..of` lowers to `next` when a type implements it and not `Iter`
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
    /// lambda node → (name, ty) per capture (by value — v1 deviation
    /// from RFC 0013 §1's by-reference capture, documented)
    pub lambda_info: std::collections::HashMap<NodeId, Vec<(IdentId, TypeId)>>,
    /// lambda signatures: body node → (resolved param types incl.
    /// expected-type inference, ret type)
    pub lambda_sigs: std::collections::HashMap<NodeId, (Vec<TypeId>, TypeId)>,
    /// `entry fn` names (RFC 0035 §3): the host-callable surface
    pub entries: Vec<IdentId>,
    /// used functions, bound before body compilation (RFC 0029 surface):
    /// name -> signature + the exporter's scope-qualified function id
    pub extern_fns: std::collections::HashMap<IdentId, ExternFn>,
    /// used constants (native modules: `calc`): name -> (type, bits)
    pub extern_consts: std::collections::HashMap<IdentId, (TypeId, u64)>,
    /// used types: name -> the exporter's scope-qualified type id
    pub extern_types: std::collections::HashMap<IdentId, TypeId>,
    /// used types that are `class` (no outside record literal)
    pub extern_classes: std::collections::HashSet<TypeId>,
    /// used core builtin containers (RFC 0028): name -> constructor.
    /// The prelude is used, never ambient — `Array`/`Opaque` resolve
    /// only through this map
    pub extern_native_types: std::collections::HashMap<IdentId, rut_core::binary::NativeTy>,
    /// used core builtin traits: name -> contract
    /// (`Disposal`/`Index`/`Iterator`)
    pub extern_traits: std::collections::HashMap<IdentId, rut_core::binary::NativeTrait>,
    /// traits exported by used modules' surfaces (RFC 0012 §5):
    /// name -> the descriptor registered in this module's table. The
    /// name binds only when the module used it — the use-both gate.
    pub extern_trait_decls: std::collections::HashMap<IdentId, ExternTrait>,
    /// trait impls registered by used modules' surfaces (RFC 0012 §2)
    pub extern_impls: Vec<ExternImpl>,
    /// the emit-closure signature of each desugared `for..of` (RFC 0012 §6),
    /// recorded at the creation site and read when the queue compiles the fn:
    /// body node → (element type, captures)
    pub for_of_sigs: std::collections::HashMap<u32, (TypeId, Vec<(IdentId, TypeId)>)>,
    /// used core compiler-lowered functions (`own`, `downcast`,
    /// `assert`/`panic`, the `str`/`bytes` natives): the name is callable
    /// only when bound
    pub extern_native_fns: std::collections::HashSet<IdentId>,
    /// Bound namespace heads (`Math` for `calc`) — the qualified
    /// access form `<namespace>.<member>` (RFC 0028). Name-generic: the
    /// LIR routes by membership here, never by a hardcoded string.
    pub extern_namespaces: std::collections::HashSet<IdentId>,
    /// instantiated generic types: id -> (decl, type args)
    pub inst_data: std::collections::HashMap<TypeId, (IdentId, Vec<TypeId>)>,
    /// monomorphization cache: (decl, type args) -> id
    pub type_inst: std::collections::HashMap<(IdentId, Vec<TypeId>), TypeId>,
    /// generic-trait instantiation cache: (trait, type args) -> trait id
    pub trait_inst: std::collections::HashMap<(IdentId, Vec<TypeId>), u32>,
    /// whether `use` is resolved by the driver (module loading on)
    pub allow_uses: bool,
    // instantiation queue
    pub inst_map: std::collections::HashMap<Inst, u32>,
    queue: Vec<Inst>,
}

/// A used function: the exporter's scope-qualified id and signature.
#[derive(Clone, Debug)]
pub struct ExternFn {
    pub func: u32,
    pub params: Vec<TypeId>,
    pub ret: TypeId,
    /// `Some` for a compiler-lowered intrinsic (RFC 0032 §1.1 R2): no
    /// `FuncCode` — `rut-lir` expands the call inline.
    pub intrinsic: Option<rut_core::ops::Intrinsic>,
}

/// A trait exported by a used module's surface (RFC 0012 §5): the id of
/// the descriptor registered in THIS module's trait table, and the
/// trait's generic parameter count. The NAME resolves only when the
/// module used it — the gate is the use-both rule's enforcement point
/// (RFC 0012 §6), so an impl whose trait was never `use`d stays
/// invisible to dispatch but visible to the "use `I` .." diagnostic.
#[derive(Clone, Debug)]
pub struct ExternTrait {
    pub id: u32,
    pub generics: usize,
}

/// A trait impl registered by another module's surface (RFC 0012 §2:
/// trait impls may live in any module). Dispatch and widening consult
/// these exactly like local impls; the methods are the exporter's
/// scope-qualified fn ids, called directly (static dispatch) and
/// carried into vtable fills (link merges the rows).
#[derive(Clone, Debug)]
pub struct ExternImpl {
    pub trait_id: u32,
    pub trait_name: IdentId,
    pub target: TypeId,
    /// trait method name → the exporter's scope-qualified fn id
    pub methods: Vec<(IdentId, u32)>,
}

/// Where a satisfying impl was found (RFC 0012 §4): a local impl block
/// (its methods monomorphize here) or another module's registration
/// (its compiled fns are called through scope-qualified ids).
#[derive(Clone, Copy, Debug)]
pub enum ImplHit {
    Local(usize),
    Extern(usize),
}

pub type TcResult<T> = Result<T, ()>;

impl<'a> Ctx<'a> {
    pub fn new(ast: &'a Ast) -> Ctx<'a> {
        Ctx::new_scoped(ast, 1)
    }

    /// A module compiled under its own scope (RFC 0035 §1).
    pub fn new_scoped(ast: &'a Ast, scope: rut_core::ScopeId) -> Ctx<'a> {
        Ctx {
            ast,
            // the interner clones the AST's — every source id resolves
            // identically; synthesized names intern here only
            interner: ast.interner.clone(),
            used: std::collections::HashSet::new(),
            diags: Vec::new(),
            types: TypeTable::boot_scoped(scope),
            traits: Vec::new(),
            trait_decls: Vec::new(),
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
            extern_consts: std::collections::HashMap::new(),
            extern_types: std::collections::HashMap::new(),
            extern_classes: std::collections::HashSet::new(),
            extern_native_types: std::collections::HashMap::new(),
            extern_traits: std::collections::HashMap::new(),
            extern_trait_decls: std::collections::HashMap::new(),
            extern_impls: Vec::new(),
            for_of_sigs: std::collections::HashMap::new(),
            extern_native_fns: std::collections::HashSet::new(),
            extern_namespaces: std::collections::HashSet::new(),
            inst_data: std::collections::HashMap::new(),
            type_inst: std::collections::HashMap::new(),
            trait_inst: std::collections::HashMap::new(),
            allow_uses: false,
            inst_map: std::collections::HashMap::new(),
            queue: Vec::new(),
        }
    }

    /// Bind a used function before body compilation.
    pub fn add_extern_fn(&mut self, name: IdentId, func: u32, params: Vec<TypeId>, ret: TypeId) {
        self.extern_fns.insert(name, ExternFn { func, params, ret, intrinsic: None });
    }

    /// Bind a used compiler-lowered intrinsic (no `FuncCode`).
    pub fn add_extern_intrinsic(&mut self, name: IdentId, intrinsic: rut_core::ops::Intrinsic) {
        self.extern_fns.insert(name, ExternFn { func: 0, params: vec![], ret: TY_I32, intrinsic: Some(intrinsic) });
    }

    pub fn extern_fn(&self, name: IdentId) -> Option<&ExternFn> {
        self.extern_fns.get(&name)
    }

    /// Bind a used constant (native modules: `calc::PI`).
    pub fn add_extern_const(&mut self, name: IdentId, ty: TypeId, bits: u64) {
        self.extern_consts.insert(name, (ty, bits));
    }

    pub fn extern_const(&self, name: IdentId) -> Option<(TypeId, u64)> {
        self.extern_consts.get(&name).copied()
    }

    /// Bind a used type name to the exporter's scope-qualified id.
    pub fn add_extern_type(&mut self, name: IdentId, ty: TypeId, is_class: bool) {
        self.extern_types.insert(name, ty);
        if is_class {
            self.extern_classes.insert(ty);
        }
    }

    /// Bind a used core builtin container (`Array`/`Option`/
    /// `Result`/`Opaque` — RFC 0028): the type constructor is the
    /// compiler's own; the binding gates the NAME.
    pub fn add_extern_native_type(&mut self, name: IdentId, kind: rut_core::binary::NativeTy) {
        self.extern_native_types.insert(name, kind);
    }

    /// Bind a used core builtin trait (`Disposal`/`Index`/
    /// `Iterator`): registered as a trait on first reference, like a
    /// declared one — but only for modules that named it.
    pub fn add_extern_trait(&mut self, name: IdentId, native: rut_core::binary::NativeTrait) {
        self.extern_traits.insert(name, native);
    }

    /// Bind a trait from a used module's surface (RFC 0012 §5): the
    /// descriptor joins THIS module's trait table (so slot numbering,
    /// widening and vtables treat it like a declared trait). The name
    /// resolves only when the module used it — the use-both gate's
    /// enforcement point (RFC 0012 §6). Method names re-intern from the
    /// surface's interner; parameter/ret ids pass through verbatim —
    /// they are packed with the scopes the surface's type blocks were
    /// registered under (`use_types`).
    ///
    /// `name` is the consumer-side name (already looked up); `None`
    /// registers the descriptor without a name binding (an impl's trait
    /// the module never named — visible to the use-gate diagnostic,
    /// invisible to resolution).
    pub fn add_extern_trait_decl(
        &mut self,
        name: Option<IdentId>,
        desc: &rut_core::binary::SurfaceTrait,
        surface_names: &Interner,
    ) -> u32 {
        let tname = self.intern(surface_names.name(desc.name));
        let mut methods = Vec::with_capacity(desc.methods.len());
        for m in &desc.methods {
            methods.push(rut_core::binary::TraitMethod {
                name: self.intern(surface_names.name(m.name)),
                params: m.params.clone(),
                ret: m.ret,
            });
        }
        let id = self.traits.len() as u32;
        self.traits.push(TraitDesc { name: tname, methods });
        if let Some(n) = name {
            self.extern_trait_decls.insert(n, ExternTrait { id, generics: desc.generics });
        }
        id
    }

    /// Bind a trait impl registered by a used module's surface
    /// (RFC 0012 §2 — the impl may live in any module). `methods` pair
    /// each trait method with the exporter's scope-qualified fn id.
    pub fn add_extern_impl(
        &mut self,
        trait_name: IdentId,
        trait_id: u32,
        target: TypeId,
        methods: Vec<(IdentId, u32)>,
    ) {
        self.extern_impls.push(ExternImpl { trait_id, trait_name, target, methods });
    }

    /// The id of a used module's exported trait, if the module used the
    /// name (the use-both gate).
    pub fn extern_trait(&self, name: IdentId) -> Option<&ExternTrait> {
        self.extern_trait_decls.get(&name)
    }

    /// Bind a used core compiler-lowered function (`own`,
    /// `downcast`, `assert`/`panic`, the `str`/`bytes` natives).
    pub fn add_extern_native_fn(&mut self, name: IdentId) {
        self.extern_native_fns.insert(name);
    }

    /// Bind a namespace head (RFC 0028): `use calc::{Math}`.
    pub fn add_extern_namespace(&mut self, name: IdentId) {
        self.extern_namespaces.insert(name);
    }

    /// Is `name` a bound namespace head?
    pub fn is_extern_namespace(&self, name: IdentId) -> bool {
        self.extern_namespaces.contains(&name)
    }

    /// The `core` not-in-scope diagnostic (RFC 0028): the prelude is
    /// used, never ambient. A v1.1-removed name diagnoses with its
    /// replacement instead. `None` when `n` is not a prelude name —
    /// the caller keeps its ordinary message.
    pub fn not_in_core_scope(&self, n: IdentId) -> Option<String> {
        let text = self.interner.name(n);
        if let Some(msg) = rut_core::binary::removed_core(text) {
            return Some(msg.to_string());
        }
        rut_core::binary::is_core_name(n).then(|| {
            format!(
                "`{text}` is not in scope — `use core::{{{text}}}` (RFC 0028: the prelude is used, never implicit)"
            )
        })
    }

    /// Use another module's type descriptors so `(scope, local)` ids
    /// resolve for typechecking and layout (RFC 0035 §1). Descriptor and
    /// field names re-intern from the exporter's interner into this
    /// module's — name ids are only comparable within one interner
    /// (well-known ids pass through: they mean the same name everywhere).
    /// `tmap` maps the exporter's trait-table indices to THIS module's
    /// trait ids: trait-typed descriptors (`[trait] Shape`) carried
    /// across must name the trait here (RFC 0015 §6).
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

    pub fn err(&mut self, span: Span, msg: impl Into<String>) {
        self.diags.push(Diag::new(span, msg));
    }

    /// The crossing rule (RFC 0023 §2): what an `entry fn` signature may
    /// carry. Primitives, `str`, `nil`, `bytes` (the binary buffer,
    /// RFC 0004), `Option`/`Result` over crossable types — and `Opaque`,
    /// the host-held box (RFC 0014): the ONE cell shape an embedder may
    /// keep and pass back. Every other cell (`TodoList`, `Vec<Todo>`,
    /// a trait object, `Vec<u8>` itself, …) stays inside the VM.
    pub fn crosses_boundary(&self, ty: TypeId) -> bool {
        self.types.crosses_boundary(ty)
    }

    /// Compile-time enforcement of the crossing rule on every `entry fn`
    /// (RFC 0035 §3): a bad surface is a source diagnostic with span and
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
                        "`entry fn {fname}` cannot be generic — the published signature is one concrete shape (RFC 0023 §2)"
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
                            "`entry fn {fname}`: parameter `{}` is `{}` — only primitives, `str`, `bytes`, `Opaque`, and `Option`/`Result` over those cross the host boundary (RFC 0023 §2)",
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
                            "`entry fn {fname}` returns `{}` — only primitives, `str`, `bytes`, `Opaque`, and `Option`/`Result` over those cross the host boundary (RFC 0023 §2)",
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
        // a used module's exported trait (RFC 0012 §5) — the name is
        // bound only when the module used it (the gate)
        self.extern_trait_decls.get(&name).map(|t| t.id)
    }
    pub fn trait_by_id(&self, id: u32) -> &TraitDesc {
        &self.traits[id as usize]
    }
    pub fn find_impl(&self, trait_id: u32, target: TypeId) -> Option<usize> {
        self.impls
            .iter()
            .position(|i| !i.inherent && i.trait_id == trait_id && i.target == target)
    }

    /// The impl satisfying `(trait, target)` wherever it lives: a local
    /// impl block, or another module's surface registration (RFC 0012
    /// §2/§5). Extern impls are gated on the trait's name having been
    /// used — an unused trait's impl is invisible to dispatch.
    pub fn find_impl_ex(&self, trait_id: u32, target: TypeId) -> Option<ImplHit> {
        if let Some(idx) = self.find_impl(trait_id, target) {
            return Some(ImplHit::Local(idx));
        }
        self.extern_impls.iter().position(|im| {
            im.trait_id == trait_id
                && im.target == target
                && self.extern_trait_decls.contains_key(&im.trait_name)
        }).map(ImplHit::Extern)
    }

    /// Extern impls on `target` whose method set contains `name`
    /// (registered by any module, RFC 0012 §2) — the use-gate diagnostic
    /// reads these even when the trait's name was never used.
    pub fn extern_impl_method(&self, target: TypeId, name: IdentId) -> Option<usize> {
        self.extern_impls.iter().position(|im| {
            im.target == target && im.methods.iter().any(|(n, _)| *n == name)
        })
    }
    pub fn impls_of(&self, target: TypeId) -> Vec<usize> {
        self.impls
            .iter()
            .enumerate()
            .filter(|(_, i)| !i.inherent && i.target == target)
            .map(|(k, _)| k)
            .collect()
    }

    /// Resolve a signature type under `env`, with `self_ty` spelling the
    /// method's `Self` (the impl target inside an impl block). Bare
    /// `Self` outside an impl is the caller's diagnostic.
    pub(crate) fn resolve_sig_ty(
        &mut self,
        node: NodeHandle<AnyTy>,
        env: &[(IdentId, TypeId)],
        self_ty: Option<TypeId>,
    ) -> TypeId {
        if let TypeKind::TyPath { segs, .. } = self.ast.ty(node) {
            if segs.len() == 1 && segs[0].generics.is_empty() && segs[0].name == sym::SELF_TY {
                if let Some(t) = self_ty {
                    return t;
                }
            }
        }
        self.resolve_type(node, env)
    }

    /// global trait-method slot id (RFC 0015 §6: assigned per trait
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
        self.types.intern(RutType {
            name,
            kind: TyKind::Array { elem },
        })
    }
    /// A trait-typed value (`i: I`) — the trait object type: a cell handle whose cell's own
    /// type reaches the vtable (RFC 0015 §6)
    pub fn mk_trait_obj(&mut self, trait_id: u32) -> TypeId {
        let tname = self.interner.name(self.traits[trait_id as usize].name).to_string();
        let name = self.intern(&format!("[trait] {tname}"));
        self.types.intern(RutType {
            name,
            kind: TyKind::TraitObj { trait_id },
        })
    }
    /// `*T` (RFC 0005) — nil-able rc-backed pointer
    pub fn mk_ptr(&mut self, elem: TypeId) -> TypeId {
        let name = self.intern(&format!("*{}", self.type_name(elem)));
        self.types.intern(RutType {
            name,
            kind: TyKind::Ptr { elem },
        })
    }
    /// `(A, B, ..)` (RFC 0007) — a record with numeric field names
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
/// (`Bool`/`Char` are excluded; RFC 0007 §1).
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
